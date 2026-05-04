use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use reqwest::Client;
use serde_json::Value;
use tokio::sync::{mpsc, watch};
use tracing::error;

use previa_runner::{
    Pipeline, PipelineStep, RuntimeEnvGroup, RuntimeSpec, StepExecutionResult, prepare_http_step,
};

use crate::server::load_wave::{
    calculate_dispatch_tick_ms, local_rps_limit, sample_intensity, timeline_end_ms,
};
use crate::server::metrics::WaveMetricsSnapshot;
use crate::server::models::{LoadProfile, LoadTestMetrics};
use crate::server::runtime::RuntimeSampler;
use crate::server::sse::{SseMessage, send_sse_or_cancel};
use crate::server::wave_emitter::{StartLagClass, classify_start_lag};
use crate::server::wave_metrics_actor::{WaveMetricEvent, run_wave_metrics_actor};
use crate::server::wave_scheduler::{WaveDispatchSlot, WaveSchedulerMetric, run_wave_scheduler};
use crate::server::wave_sender::{ReadyWaveRequest, WaveObserverEvent, WaveSender};

#[derive(Debug)]
struct PipelineCursor {
    step_index: usize,
    attempt: usize,
    context: HashMap<String, StepExecutionResult>,
    pipeline_started_at: Instant,
}

impl PipelineCursor {
    fn new(started_at: Instant) -> Self {
        Self {
            step_index: 0,
            attempt: 1,
            context: HashMap::new(),
            pipeline_started_at: started_at,
        }
    }
}

type ObserverEvent = WaveObserverEvent<PipelineCursor>;
const OBSERVER_EVENTS_PER_TICK_BUDGET: usize = 1024;

fn next_cursor_for_slot(
    ready: &mut VecDeque<PipelineCursor>,
    create: impl FnOnce() -> PipelineCursor,
) -> PipelineCursor {
    ready.pop_front().unwrap_or_else(create)
}

struct DispatchSlotRequestArgs<'a> {
    slot: WaveDispatchSlot,
    ready: &'a mut VecDeque<PipelineCursor>,
    pipeline: &'a Pipeline,
    specs: &'a Arc<Vec<RuntimeSpec>>,
    env_groups: &'a Arc<Vec<RuntimeEnvGroup>>,
    selected_env_group_slug: &'a Option<String>,
    request_tx: &'a mpsc::UnboundedSender<ReadyWaveRequest<PipelineCursor>>,
    metric_tx: &'a mpsc::UnboundedSender<WaveMetricEvent>,
    ready_to_send: &'a Arc<AtomicUsize>,
    missed_starts: &'a Arc<AtomicUsize>,
    started: Instant,
    tick_ms: u64,
    token: &'a tokio_util::sync::CancellationToken,
}

async fn dispatch_slot_requests(args: DispatchSlotRequestArgs<'_>) {
    for _ in 0..args.slot.planned_starts {
        if args.token.is_cancelled() {
            break;
        }

        let was_ready_empty = args.ready.is_empty();
        let cursor = next_cursor_for_slot(args.ready, || PipelineCursor::new(Instant::now()));
        if was_ready_empty && cursor.step_index == 0 && cursor.context.is_empty() {
            let _ = args.metric_tx.send(WaveMetricEvent::PipelineStarted);
        }

        let Some(step) = args.pipeline.steps.get(cursor.step_index).cloned() else {
            record_terminal_pipeline(args.metric_tx, cursor, false, None).await;
            continue;
        };
        let max_attempts = max_attempts_for_step(&step);
        let prepared = match prepare_http_step(
            &step,
            &cursor.context,
            Some(args.specs.as_slice()),
            Some(args.env_groups.as_slice()),
            args.selected_env_group_slug.as_deref(),
            cursor.attempt,
            max_attempts,
        ) {
            Ok(prepared) => prepared,
            Err(result) => {
                handle_prepare_error(
                    result,
                    cursor,
                    args.ready,
                    args.pipeline,
                    args.metric_tx,
                    args.missed_starts,
                )
                .await;
                continue;
            }
        };

        let actual_elapsed_ms = args.started.elapsed().as_millis() as u64;
        if classify_start_lag(args.slot.elapsed_ms, actual_elapsed_ms, args.tick_ms)
            == StartLagClass::RuntimeLagged
        {
            args.missed_starts.fetch_add(1, Ordering::SeqCst);
            let _ = args.metric_tx.send(WaveMetricEvent::RuntimeLaggedStart);
        }

        args.ready_to_send.fetch_add(1, Ordering::SeqCst);
        if args
            .request_tx
            .send(ReadyWaveRequest {
                step,
                context: cursor.context.clone(),
                cursor,
                prepared,
                specs: Arc::clone(args.specs),
                env_groups: Arc::clone(args.env_groups),
                selected_env_group_slug: args.selected_env_group_slug.clone(),
            })
            .is_err()
        {
            args.ready_to_send.fetch_sub(1, Ordering::SeqCst);
            error!("wave sender stopped before accepting prepared request");
            break;
        }
    }
}

pub async fn run_wave_load(
    load: LoadProfile,
    pipeline: Pipeline,
    _selected_key: Option<String>,
    selected_env_group_slug: Option<String>,
    specs: Vec<RuntimeSpec>,
    env_groups: Vec<RuntimeEnvGroup>,
    tx: mpsc::UnboundedSender<SseMessage>,
    token: tokio_util::sync::CancellationToken,
) {
    let tick_ms = calculate_dispatch_tick_ms(&load);
    let started = Instant::now();
    let end_ms = timeline_end_ms(&load);
    let pipeline = Arc::new(pipeline);
    let specs = Arc::new(specs);
    let env_groups = Arc::new(env_groups);
    let runtime_sampler = Arc::new(tokio::sync::Mutex::new(RuntimeSampler::new()));
    let response_in_flight = Arc::new(AtomicUsize::new(0));
    let ready_to_send = Arc::new(AtomicUsize::new(0));
    let missed_starts = Arc::new(AtomicUsize::new(0));
    let observer_token = token.child_token();
    let http_client = Arc::new(Client::new());
    let mut ready = VecDeque::new();
    let (slot_tx, mut slot_rx) = mpsc::channel::<WaveDispatchSlot>(1024);
    let (scheduler_metric_tx, mut scheduler_metric_rx) =
        mpsc::unbounded_channel::<WaveSchedulerMetric>();
    let (metric_tx, metric_rx) = mpsc::unbounded_channel::<WaveMetricEvent>();
    let (snapshot_tx, mut snapshot_rx) = watch::channel(LoadTestMetrics::default());
    let (request_tx, request_rx) = mpsc::unbounded_channel::<ReadyWaveRequest<PipelineCursor>>();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<ObserverEvent>();
    let metrics_task = tokio::spawn(run_wave_metrics_actor(metric_rx, snapshot_tx));
    let missed_bridge = Arc::clone(&missed_starts);
    let metric_bridge_tx = metric_tx.clone();
    let metric_bridge = tokio::spawn(async move {
        while let Some(event) = scheduler_metric_rx.recv().await {
            match event {
                WaveSchedulerMetric::SlotBackpressure { dropped_starts } => {
                    missed_bridge.fetch_add(dropped_starts, Ordering::SeqCst);
                    let _ = metric_bridge_tx.send(WaveMetricEvent::Scheduler(event));
                }
                WaveSchedulerMetric::DispatchScheduled { .. }
                | WaveSchedulerMetric::SchedulerLag { .. } => {}
            }
        }
    });
    let scheduler_task = tokio::spawn(run_wave_scheduler(
        load.clone(),
        tick_ms,
        slot_tx,
        scheduler_metric_tx,
        token.child_token(),
    ));
    let sender = WaveSender::new(
        Arc::clone(&http_client),
        metric_tx.clone(),
        Arc::clone(&response_in_flight),
        Arc::clone(&ready_to_send),
        request_rx,
        event_tx,
        observer_token.clone(),
    );
    let sender_task = tokio::spawn(sender.run());

    while let Some(slot) = slot_rx.recv().await {
        if token.is_cancelled() {
            break;
        }

        let _ = metric_tx.send(WaveMetricEvent::Scheduler(
            WaveSchedulerMetric::DispatchScheduled {
                count: slot.planned_starts,
            },
        ));
        if slot.scheduler_lag_ms > 0 || slot.missed_due_to_scheduler_lag > 0 {
            missed_starts.fetch_add(slot.missed_due_to_scheduler_lag, Ordering::SeqCst);
            let _ = metric_tx.send(WaveMetricEvent::Scheduler(
                WaveSchedulerMetric::SchedulerLag {
                    lag_ms: slot.scheduler_lag_ms,
                    missed_starts: slot.missed_due_to_scheduler_lag,
                },
            ));
        }

        dispatch_slot_requests(DispatchSlotRequestArgs {
            slot,
            ready: &mut ready,
            pipeline: &pipeline,
            specs: &specs,
            env_groups: &env_groups,
            selected_env_group_slug: &selected_env_group_slug,
            request_tx: &request_tx,
            metric_tx: &metric_tx,
            ready_to_send: &ready_to_send,
            missed_starts: &missed_starts,
            started,
            tick_ms,
            token: &token,
        })
        .await;

        drain_observer_events_budgeted(
            &mut event_rx,
            &mut ready,
            &pipeline,
            &metric_tx,
            OBSERVER_EVENTS_PER_TICK_BUDGET,
        )
        .await;

        send_metrics_snapshot(SnapshotArgs {
            load: &load,
            started,
            end_ms,
            tick_ms,
            scheduled_total: slot.scheduled_total,
            missed_total: missed_starts.load(Ordering::SeqCst),
            ready_requests: ready
                .len()
                .saturating_add(ready_to_send.load(Ordering::SeqCst)),
            response_in_flight: response_in_flight.load(Ordering::SeqCst),
            metric_tx: &metric_tx,
            snapshot_rx: &mut snapshot_rx,
            runtime_sampler: &runtime_sampler,
            tx: &tx,
            token: &token,
            event: "metrics",
            duration_ms: None,
        })
        .await;
    }

    let grace_deadline =
        tokio::time::Instant::now() + tokio::time::Duration::from_millis(load.grace_period_ms);
    while response_in_flight.load(Ordering::SeqCst) > 0 || ready_to_send.load(Ordering::SeqCst) > 0
    {
        drain_all_observer_events(&mut event_rx, &mut ready, &pipeline, &metric_tx).await;
        if token.is_cancelled() || tokio::time::Instant::now() >= grace_deadline {
            break;
        }

        let scheduled_total = snapshot_rx.borrow().scheduled_starts.unwrap_or_default();
        send_metrics_snapshot(SnapshotArgs {
            load: &load,
            started,
            end_ms,
            tick_ms,
            scheduled_total,
            missed_total: missed_starts.load(Ordering::SeqCst),
            ready_requests: ready
                .len()
                .saturating_add(ready_to_send.load(Ordering::SeqCst)),
            response_in_flight: response_in_flight.load(Ordering::SeqCst),
            metric_tx: &metric_tx,
            snapshot_rx: &mut snapshot_rx,
            runtime_sampler: &runtime_sampler,
            tx: &tx,
            token: &token,
            event: "metrics",
            duration_ms: None,
        })
        .await;

        tokio::time::sleep(tokio::time::Duration::from_millis(tick_ms.min(250))).await;
    }

    scheduler_task.abort();
    let _ = scheduler_task.await;
    drop(request_tx);
    if response_in_flight.load(Ordering::SeqCst) > 0 {
        observer_token.cancel();
        let observer_shutdown_deadline =
            tokio::time::Instant::now() + tokio::time::Duration::from_secs(2);
        while response_in_flight.load(Ordering::SeqCst) > 0
            && tokio::time::Instant::now() < observer_shutdown_deadline
        {
            tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;
        }
    }
    if let Err(err) = sender_task.await {
        if !err.is_cancelled() {
            error!("wave sender task failed: {err}");
        }
    }
    drain_all_observer_events(&mut event_rx, &mut ready, &pipeline, &metric_tx).await;

    let scheduled_total = snapshot_rx.borrow().scheduled_starts.unwrap_or_default();
    if send_metrics_snapshot(SnapshotArgs {
        load: &load,
        started,
        end_ms,
        tick_ms,
        scheduled_total,
        missed_total: missed_starts.load(Ordering::SeqCst),
        ready_requests: ready
            .len()
            .saturating_add(ready_to_send.load(Ordering::SeqCst)),
        response_in_flight: response_in_flight.load(Ordering::SeqCst),
        metric_tx: &metric_tx,
        snapshot_rx: &mut snapshot_rx,
        runtime_sampler: &runtime_sampler,
        tx: &tx,
        token: &token,
        event: "metrics",
        duration_ms: None,
    })
    .await
        && !token.is_cancelled()
    {
        let complete = snapshot_rx.borrow().clone();
        let _ = send_sse_or_cancel(
            &tx,
            "complete",
            serde_json::to_value(complete).unwrap_or(Value::Null),
            &token,
        );
    }
    drop(metric_tx);
    let _ = metric_bridge.await;
    let _ = metrics_task.await;
}

async fn drain_observer_events_budgeted(
    event_rx: &mut mpsc::UnboundedReceiver<ObserverEvent>,
    ready: &mut VecDeque<PipelineCursor>,
    pipeline: &Pipeline,
    metric_tx: &mpsc::UnboundedSender<WaveMetricEvent>,
    budget: usize,
) -> usize {
    let mut drained = 0usize;
    while drained < budget {
        let Ok(event) = event_rx.try_recv() else {
            break;
        };
        handle_step_result(event.result, event.cursor, ready, pipeline, metric_tx).await;
        drained += 1;
    }
    drained
}

async fn drain_all_observer_events(
    event_rx: &mut mpsc::UnboundedReceiver<ObserverEvent>,
    ready: &mut VecDeque<PipelineCursor>,
    pipeline: &Pipeline,
    metric_tx: &mpsc::UnboundedSender<WaveMetricEvent>,
) {
    while drain_observer_events_budgeted(
        event_rx,
        ready,
        pipeline,
        metric_tx,
        OBSERVER_EVENTS_PER_TICK_BUDGET,
    )
    .await
        > 0
    {}
}

async fn handle_step_result(
    result: StepExecutionResult,
    mut cursor: PipelineCursor,
    ready: &mut VecDeque<PipelineCursor>,
    pipeline: &Pipeline,
    metric_tx: &mpsc::UnboundedSender<WaveMetricEvent>,
) {
    let Some(step) = pipeline.steps.get(cursor.step_index) else {
        record_terminal_pipeline(metric_tx, cursor, false, Some(&result)).await;
        return;
    };
    let max_attempts = max_attempts_for_step(step);

    if result.status == "error" && cursor.attempt < max_attempts {
        cursor.attempt += 1;
        ready.push_back(cursor);
        return;
    }

    if result.status == "error" {
        record_terminal_pipeline(metric_tx, cursor, false, Some(&result)).await;
        return;
    }

    cursor.context.insert(result.step_id.clone(), result);
    cursor.step_index += 1;
    cursor.attempt = 1;

    if cursor.step_index >= pipeline.steps.len() {
        record_terminal_pipeline(metric_tx, cursor, true, None).await;
    } else {
        ready.push_back(cursor);
    }
}

async fn handle_prepare_error(
    result: StepExecutionResult,
    mut cursor: PipelineCursor,
    ready: &mut VecDeque<PipelineCursor>,
    pipeline: &Pipeline,
    metric_tx: &mpsc::UnboundedSender<WaveMetricEvent>,
    missed_starts: &Arc<AtomicUsize>,
) {
    let max_attempts = pipeline
        .steps
        .get(cursor.step_index)
        .map(max_attempts_for_step)
        .unwrap_or(1);

    if cursor.attempt < max_attempts {
        cursor.attempt += 1;
        ready.push_back(cursor);
        return;
    }

    if cursor.step_index > 0 {
        missed_starts.fetch_add(1, Ordering::SeqCst);
        let _ = metric_tx.send(WaveMetricEvent::DependencyLimitedStarts(1));
    }
    record_terminal_pipeline(metric_tx, cursor, false, Some(&result)).await;
}

async fn record_terminal_pipeline(
    metric_tx: &mpsc::UnboundedSender<WaveMetricEvent>,
    cursor: PipelineCursor,
    success: bool,
    result: Option<&StepExecutionResult>,
) {
    let duration_ms = cursor.pipeline_started_at.elapsed().as_millis() as f64;
    let _ = metric_tx.send(WaveMetricEvent::PipelineFinished {
        duration_ms,
        success,
    });
    if !success {
        if let Some(result) = result {
            let http_status = result.response.as_ref().map(|response| response.status);
            let error = result.error.as_deref().unwrap_or("pipeline failed");
            let _ = metric_tx.send(WaveMetricEvent::ErrorSample {
                step_id: result.step_id.clone(),
                http_status,
                error: error.to_owned(),
            });
        }
    }
}

fn max_attempts_for_step(step: &PipelineStep) -> usize {
    step.retry.unwrap_or(0).saturating_add(1)
}

struct SnapshotArgs<'a> {
    load: &'a LoadProfile,
    started: Instant,
    end_ms: u64,
    tick_ms: u64,
    scheduled_total: usize,
    missed_total: usize,
    ready_requests: usize,
    response_in_flight: usize,
    metric_tx: &'a mpsc::UnboundedSender<WaveMetricEvent>,
    snapshot_rx: &'a mut watch::Receiver<LoadTestMetrics>,
    runtime_sampler: &'a Arc<tokio::sync::Mutex<RuntimeSampler>>,
    tx: &'a mpsc::UnboundedSender<SseMessage>,
    token: &'a tokio_util::sync::CancellationToken,
    event: &'static str,
    duration_ms: Option<u64>,
}

async fn send_metrics_snapshot(args: SnapshotArgs<'_>) -> bool {
    let runtime = {
        let mut sampler = args.runtime_sampler.lock().await;
        sampler.snapshot()
    };
    let elapsed_ms = args.started.elapsed().as_millis() as u64;
    let _ = args.metric_tx.send(WaveMetricEvent::Snapshot {
        wave: wave_snapshot(
            args.load,
            elapsed_ms,
            args.end_ms,
            args.tick_ms,
            args.response_in_flight,
            args.scheduled_total,
            args.missed_total,
            args.ready_requests,
        ),
        runtime,
        duration_ms: args.duration_ms,
    });

    let _ = tokio::time::timeout(
        tokio::time::Duration::from_millis(100),
        args.snapshot_rx.changed(),
    )
    .await;
    let snapshot = args.snapshot_rx.borrow().clone();
    send_sse_or_cancel(
        args.tx,
        args.event,
        serde_json::to_value(snapshot).unwrap_or(Value::Null),
        args.token,
    )
}

fn wave_snapshot(
    load: &LoadProfile,
    elapsed_ms: u64,
    end_ms: u64,
    tick_ms: u64,
    response_in_flight: usize,
    scheduled_starts: usize,
    missed_starts: usize,
    ready_requests: usize,
) -> WaveMetricsSnapshot {
    let load_phase_active = elapsed_ms <= end_ms;
    let target_intensity = if load_phase_active {
        sample_intensity(load, elapsed_ms)
    } else {
        0.0
    };
    let target_rps_limit = if load_phase_active {
        local_rps_limit(load, elapsed_ms)
    } else {
        0.0
    };

    WaveMetricsSnapshot {
        target_intensity,
        target_rps_limit,
        in_flight: response_in_flight,
        runner_max_rps: load.runner_max_rps,
        tick_ms,
        scheduled_starts,
        missed_starts,
        ready_requests,
        active_pipelines: response_in_flight.saturating_add(ready_requests),
        outstanding_requests: response_in_flight,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};
    use std::time::Instant;

    #[test]
    fn next_cursor_prefers_ready_continuations_before_starting_new_pipeline() {
        let mut ready = VecDeque::new();
        ready.push_back(PipelineCursor {
            step_index: 2,
            attempt: 1,
            context: HashMap::new(),
            pipeline_started_at: Instant::now(),
        });
        let mut started_new = false;

        let cursor = next_cursor_for_slot(&mut ready, || {
            started_new = true;
            PipelineCursor {
                step_index: 0,
                attempt: 1,
                context: HashMap::new(),
                pipeline_started_at: Instant::now(),
            }
        });

        assert_eq!(cursor.step_index, 2);
        assert!(!started_new);
    }

    #[test]
    fn next_cursor_starts_new_pipeline_when_no_continuation_is_ready() {
        let mut ready = VecDeque::new();
        let cursor = next_cursor_for_slot(&mut ready, || PipelineCursor::new(Instant::now()));

        assert_eq!(cursor.step_index, 0);
        assert_eq!(cursor.attempt, 1);
        assert!(cursor.context.is_empty());
    }

    #[tokio::test]
    async fn observer_drain_respects_per_tick_budget() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ObserverEvent>();
        let mut ready = VecDeque::new();
        let pipeline = Pipeline {
            id: Some("p".to_owned()),
            name: "pipeline".to_owned(),
            description: None,
            steps: Vec::new(),
        };
        let (metric_tx, mut metric_rx) = mpsc::unbounded_channel::<WaveMetricEvent>();

        for _ in 0..3 {
            tx.send(WaveObserverEvent {
                cursor: PipelineCursor::new(Instant::now()),
                result: StepExecutionResult {
                    step_id: "missing".to_owned(),
                    status: "error".to_owned(),
                    request: None,
                    response: None,
                    error: Some("synthetic".to_owned()),
                    duration: Some(0),
                    attempts: None,
                    attempt: Some(1),
                    max_attempts: Some(1),
                    assert_results: None,
                },
            })
            .unwrap();
        }

        let drained =
            drain_observer_events_budgeted(&mut rx, &mut ready, &pipeline, &metric_tx, 2).await;

        assert_eq!(drained, 2);
        assert!(matches!(
            metric_rx.try_recv(),
            Ok(WaveMetricEvent::PipelineFinished { success: false, .. })
        ));
        assert!(
            rx.try_recv().is_ok(),
            "one event should remain for a later tick"
        );
    }
}
