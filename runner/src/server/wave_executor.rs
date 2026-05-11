use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use reqwest::Client;
use serde_json::Value;
use tokio::sync::{mpsc, watch};
use tracing::error;

use previa_runner::{Pipeline, RuntimeEnvGroup, RuntimeSpec};

use crate::server::load_wave::{
    calculate_dispatch_tick_ms, local_rps_limit, sample_intensity, timeline_end_ms,
};
use crate::server::metrics::{MetricsSnapshotScope, WaveMetricsSnapshot};
use crate::server::models::{LoadProfile, LoadTestMetrics};
use crate::server::runtime::RuntimeSampler;
use crate::server::sse::{SseMessage, send_sse_or_cancel};
use crate::server::wave_dispatcher::{
    PipelineCursor, WaveDispatcherChannels, WaveDispatcherConfig, WaveDispatcherShared,
    WaveDispatcherSnapshot, spawn_wave_dispatcher_thread,
};
use crate::server::wave_metrics_actor::{WaveMetricEvent, run_wave_metrics_actor};
use crate::server::wave_scheduler::{WaveSchedulerMetric, spawn_wave_scheduler_thread};
use crate::server::wave_sender::{ReadyWaveRequest, WaveSender, spawn_wave_sender_thread};

const WAVE_LIVE_METRICS_INTERVAL_MS: u64 = 1_000;
const WAVE_LIVE_BUCKET_LAG_MS: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GraceDrainDecision {
    Continue,
    Drained,
    GraceTimeout,
    Cancelled,
}

fn grace_drain_decision(
    response_in_flight: usize,
    ready_to_send: usize,
    is_cancelled: bool,
    now: tokio::time::Instant,
    deadline: tokio::time::Instant,
) -> GraceDrainDecision {
    if response_in_flight == 0 && ready_to_send == 0 {
        return GraceDrainDecision::Drained;
    }

    if is_cancelled {
        return GraceDrainDecision::Cancelled;
    }

    if now >= deadline {
        return GraceDrainDecision::GraceTimeout;
    }

    GraceDrainDecision::Continue
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
    let (slot_tx, slot_rx) = mpsc::channel(1024);
    let (scheduler_metric_tx, mut scheduler_metric_rx) =
        mpsc::unbounded_channel::<WaveSchedulerMetric>();
    let (metric_tx, metric_rx) = mpsc::unbounded_channel::<WaveMetricEvent>();
    let (snapshot_tx, mut snapshot_rx) = watch::channel(LoadTestMetrics::default());
    let (dispatcher_snapshot_tx, dispatcher_snapshot_rx) =
        watch::channel(WaveDispatcherSnapshot::default());
    let (request_tx, request_rx) = mpsc::unbounded_channel::<ReadyWaveRequest<PipelineCursor>>();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let metrics_task = tokio::spawn(run_wave_metrics_actor(metric_rx, snapshot_tx));
    let missed_bridge = Arc::clone(&missed_starts);
    let metric_bridge_tx = metric_tx.clone();
    let metric_bridge = tokio::spawn(async move {
        while let Some(event) = scheduler_metric_rx.recv().await {
            match event {
                WaveSchedulerMetric::SlotBackpressure { dropped_starts, .. } => {
                    missed_bridge.fetch_add(dropped_starts, Ordering::SeqCst);
                    let _ = metric_bridge_tx.send(WaveMetricEvent::Scheduler(event));
                }
                WaveSchedulerMetric::SchedulerLag { missed_starts, .. } => {
                    missed_bridge.fetch_add(missed_starts, Ordering::SeqCst);
                    let _ = metric_bridge_tx.send(WaveMetricEvent::Scheduler(event));
                }
                WaveSchedulerMetric::DispatchScheduled { .. } => {
                    let _ = metric_bridge_tx.send(WaveMetricEvent::Scheduler(event));
                }
                WaveSchedulerMetric::SlotEnqueued { elapsed_ms, count } => {
                    let _ =
                        metric_bridge_tx.send(WaveMetricEvent::SlotEnqueued { elapsed_ms, count });
                    let _ = metric_bridge_tx.send(WaveMetricEvent::Scheduler(event));
                }
            }
        }
    });
    let scheduler_token = token.child_token();
    let scheduler_thread = spawn_wave_scheduler_thread(
        load.clone(),
        tick_ms,
        slot_tx,
        scheduler_metric_tx,
        scheduler_token.clone(),
    );
    let dispatcher_token = token.child_token();
    let dispatcher_handle = spawn_wave_dispatcher_thread(
        WaveDispatcherConfig {
            pipeline: Arc::clone(&pipeline),
            specs: Arc::clone(&specs),
            env_groups: Arc::clone(&env_groups),
            selected_env_group_slug: selected_env_group_slug.clone(),
            started,
            tick_ms,
        },
        WaveDispatcherChannels {
            slot_rx,
            request_tx: request_tx.clone(),
            observer_rx: event_rx,
            metric_tx: metric_tx.clone(),
            snapshot_tx: dispatcher_snapshot_tx,
        },
        WaveDispatcherShared {
            ready_to_send: Arc::clone(&ready_to_send),
            missed_starts: Arc::clone(&missed_starts),
        },
        dispatcher_token.clone(),
    );
    let sender = WaveSender::new(
        Arc::clone(&http_client),
        started,
        metric_tx.clone(),
        Arc::clone(&response_in_flight),
        Arc::clone(&ready_to_send),
        request_rx,
        event_tx,
        observer_token.clone(),
    );
    let sender_handle = spawn_wave_sender_thread(sender);
    let mut next_live_metrics_at_ms = 0u64;
    let mut next_live_bucket_from_ms = 0u64;

    while !token.is_cancelled() && started.elapsed().as_millis() as u64 <= end_ms {
        let elapsed_ms = started.elapsed().as_millis() as u64;
        if elapsed_ms >= next_live_metrics_at_ms {
            if let Some(through_elapsed_ms) = closed_bucket_through_elapsed_ms(elapsed_ms) {
                if through_elapsed_ms >= next_live_bucket_from_ms {
                    let dispatcher_snapshot = *dispatcher_snapshot_rx.borrow();
                    let scheduled_total = snapshot_rx.borrow().scheduled_starts.unwrap_or_default();
                    send_metrics_snapshot(SnapshotArgs {
                        load: &load,
                        started,
                        end_ms,
                        tick_ms,
                        scheduled_total,
                        missed_total: missed_starts.load(Ordering::SeqCst),
                        ready_requests: dispatcher_snapshot.ready_requests(),
                        response_in_flight: response_in_flight.load(Ordering::SeqCst),
                        metric_tx: &metric_tx,
                        snapshot_rx: &mut snapshot_rx,
                        runtime_sampler: &runtime_sampler,
                        tx: &tx,
                        token: &token,
                        event: "metrics",
                        duration_ms: None,
                        scope: MetricsSnapshotScope::LiveWindow {
                            from_elapsed_ms: next_live_bucket_from_ms,
                            through_elapsed_ms,
                        },
                    })
                    .await;
                    next_live_bucket_from_ms = through_elapsed_ms.saturating_add(1_000);
                }
            }
            next_live_metrics_at_ms = elapsed_ms.saturating_add(WAVE_LIVE_METRICS_INTERVAL_MS);
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(tick_ms.min(250))).await;
    }

    let grace_deadline =
        tokio::time::Instant::now() + tokio::time::Duration::from_millis(load.grace_period_ms);
    loop {
        match grace_drain_decision(
            response_in_flight.load(Ordering::SeqCst),
            ready_to_send.load(Ordering::SeqCst),
            token.is_cancelled(),
            tokio::time::Instant::now(),
            grace_deadline,
        ) {
            GraceDrainDecision::Continue => {}
            GraceDrainDecision::Drained
            | GraceDrainDecision::GraceTimeout
            | GraceDrainDecision::Cancelled => break,
        }

        let elapsed_ms = started.elapsed().as_millis() as u64;
        if elapsed_ms >= next_live_metrics_at_ms {
            if let Some(through_elapsed_ms) = closed_bucket_through_elapsed_ms(elapsed_ms) {
                if through_elapsed_ms >= next_live_bucket_from_ms {
                    let scheduled_total = snapshot_rx.borrow().scheduled_starts.unwrap_or_default();
                    let ready_requests = dispatcher_snapshot_rx.borrow().ready_requests();
                    send_metrics_snapshot(SnapshotArgs {
                        load: &load,
                        started,
                        end_ms,
                        tick_ms,
                        scheduled_total,
                        missed_total: missed_starts.load(Ordering::SeqCst),
                        ready_requests,
                        response_in_flight: response_in_flight.load(Ordering::SeqCst),
                        metric_tx: &metric_tx,
                        snapshot_rx: &mut snapshot_rx,
                        runtime_sampler: &runtime_sampler,
                        tx: &tx,
                        token: &token,
                        event: "metrics",
                        duration_ms: None,
                        scope: MetricsSnapshotScope::LiveWindow {
                            from_elapsed_ms: next_live_bucket_from_ms,
                            through_elapsed_ms,
                        },
                    })
                    .await;
                    next_live_bucket_from_ms = through_elapsed_ms.saturating_add(1_000);
                }
            }
            next_live_metrics_at_ms = elapsed_ms.saturating_add(WAVE_LIVE_METRICS_INTERVAL_MS);
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(tick_ms.min(250))).await;
    }

    scheduler_token.cancel();
    if let Err(err) = scheduler_thread.join() {
        error!("wave scheduler thread panicked: {:?}", err);
    }
    dispatcher_handle.stop();
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
    sender_handle.stop();

    let scheduled_total = snapshot_rx.borrow().scheduled_starts.unwrap_or_default();
    let ready_requests = dispatcher_snapshot_rx.borrow().ready_requests();
    if send_metrics_snapshot(SnapshotArgs {
        load: &load,
        started,
        end_ms,
        tick_ms,
        scheduled_total,
        missed_total: missed_starts.load(Ordering::SeqCst),
        ready_requests,
        response_in_flight: response_in_flight.load(Ordering::SeqCst),
        metric_tx: &metric_tx,
        snapshot_rx: &mut snapshot_rx,
        runtime_sampler: &runtime_sampler,
        tx: &tx,
        token: &token,
        event: "metrics",
        duration_ms: None,
        scope: MetricsSnapshotScope::Full,
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
    scope: MetricsSnapshotScope,
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
        scope: args.scope,
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

fn closed_bucket_through_elapsed_ms(elapsed_ms: u64) -> Option<u64> {
    elapsed_ms
        .checked_sub(WAVE_LIVE_BUCKET_LAG_MS)
        .map(|value| (value / 1_000).saturating_mul(1_000))
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

    #[test]
    fn grace_drain_decision_finishes_when_no_work_is_pending_before_deadline() {
        let now = tokio::time::Instant::now();
        let deadline = now + tokio::time::Duration::from_secs(30);

        assert_eq!(
            grace_drain_decision(0, 0, false, now, deadline),
            GraceDrainDecision::Drained,
        );
    }

    #[test]
    fn grace_drain_decision_continues_until_pending_work_drains_or_deadline_hits() {
        let now = tokio::time::Instant::now();
        let deadline = now + tokio::time::Duration::from_secs(30);

        assert_eq!(
            grace_drain_decision(1, 0, false, now, deadline),
            GraceDrainDecision::Continue,
        );
        assert_eq!(
            grace_drain_decision(0, 1, false, now, deadline),
            GraceDrainDecision::Continue,
        );
        assert_eq!(
            grace_drain_decision(1, 0, false, deadline, deadline),
            GraceDrainDecision::GraceTimeout,
        );
    }
}
