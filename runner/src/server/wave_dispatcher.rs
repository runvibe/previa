use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use tokio::sync::{Mutex, mpsc, watch};
use tracing::error;

use previa_runner::{
    Pipeline, PipelineStep, RuntimeEnvGroup, RuntimeSpec, StepExecutionResult, prepare_http_step,
};

use crate::server::wave_emitter::{StartLagClass, classify_start_lag};
use crate::server::wave_metrics_actor::WaveMetricEvent;
use crate::server::wave_scheduler::{WaveDispatchSlot, slot_is_expired};
use crate::server::wave_sender::{ReadyWaveRequest, WaveObserverEvent};

#[derive(Debug)]
pub struct PipelineCursor {
    pub step_index: usize,
    pub attempt: usize,
    pub context: HashMap<String, StepExecutionResult>,
    pub pipeline_started_at: Instant,
}

impl PipelineCursor {
    pub fn new(started_at: Instant) -> Self {
        Self {
            step_index: 0,
            attempt: 1,
            context: HashMap::new(),
            pipeline_started_at: started_at,
        }
    }
}

pub type ObserverEvent = WaveObserverEvent<PipelineCursor>;

#[derive(Debug)]
pub struct WavePrepareIntent {
    pub cursor: PipelineCursor,
    pub scheduled_elapsed_ms: u64,
    pub target_start_elapsed_ms: u64,
    pub expires_at_elapsed_ms: u64,
}

pub struct WavePrepareError {
    pub result: StepExecutionResult,
    pub cursor: PipelineCursor,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WaveDispatcherSnapshot {
    pub ready_continuations: usize,
    pub ready_to_send: usize,
}

impl WaveDispatcherSnapshot {
    pub fn ready_requests(self) -> usize {
        self.ready_continuations.saturating_add(self.ready_to_send)
    }
}

pub struct WaveDispatcherConfig {
    pub pipeline: Arc<Pipeline>,
    pub specs: Arc<Vec<RuntimeSpec>>,
    pub env_groups: Arc<Vec<RuntimeEnvGroup>>,
    pub selected_env_group_slug: Option<String>,
    pub started: Instant,
    pub tick_ms: u64,
}

pub struct WaveDispatcherChannels {
    pub slot_rx: mpsc::Receiver<WaveDispatchSlot>,
    pub request_tx: mpsc::UnboundedSender<ReadyWaveRequest<PipelineCursor>>,
    pub observer_rx: mpsc::UnboundedReceiver<ObserverEvent>,
    pub metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    pub snapshot_tx: watch::Sender<WaveDispatcherSnapshot>,
}

pub struct WaveDispatcherShared {
    pub ready_to_send: Arc<AtomicUsize>,
    pub missed_starts: Arc<AtomicUsize>,
}

pub struct WaveDispatcherHandle {
    token: tokio_util::sync::CancellationToken,
    join: std::thread::JoinHandle<()>,
}

impl WaveDispatcherHandle {
    pub fn stop(self) {
        self.token.cancel();
        if let Err(err) = self.join.join() {
            error!("wave dispatcher thread panicked: {:?}", err);
        }
    }
}

const OBSERVER_EVENTS_PER_TICK_BUDGET: usize = 1024;

pub fn spread_deadlines(slot_elapsed_ms: u64, tick_ms: u64, count: usize) -> Vec<u64> {
    if count == 0 {
        return Vec::new();
    }

    (0..count)
        .map(|index| slot_elapsed_ms.saturating_add((index as u64 * tick_ms) / count as u64))
        .collect()
}

pub fn next_cursor_for_slot(
    ready: &mut VecDeque<PipelineCursor>,
    create: impl FnOnce() -> PipelineCursor,
) -> PipelineCursor {
    ready.pop_front().unwrap_or_else(create)
}

pub async fn drain_observer_events_budgeted(
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

pub async fn drain_all_observer_events(
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

pub struct DispatchSlotPrepareArgs<'a> {
    pub slot: WaveDispatchSlot,
    pub ready: &'a mut VecDeque<PipelineCursor>,
    pub pipeline: &'a Pipeline,
    pub prepare_tx: &'a mpsc::UnboundedSender<WavePrepareIntent>,
    pub metric_tx: &'a mpsc::UnboundedSender<WaveMetricEvent>,
    pub missed_starts: &'a Arc<AtomicUsize>,
    pub started: Instant,
    pub tick_ms: u64,
    pub token: &'a tokio_util::sync::CancellationToken,
}

pub async fn dispatch_slot_prepare_intents(args: DispatchSlotPrepareArgs<'_>) {
    let actual_elapsed_ms = args.started.elapsed().as_millis() as u64;
    if slot_is_expired(&args.slot, actual_elapsed_ms) {
        args.missed_starts
            .fetch_add(args.slot.planned_starts, Ordering::SeqCst);
        let _ = args
            .metric_tx
            .send(WaveMetricEvent::DispatcherLaggedStarts {
                elapsed_ms: args.slot.elapsed_ms,
                count: args.slot.planned_starts,
            });
        return;
    }

    for target_start_elapsed_ms in
        spread_deadlines(args.slot.elapsed_ms, args.tick_ms, args.slot.planned_starts)
    {
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
        drop(step);

        let actual_elapsed_ms = args.started.elapsed().as_millis() as u64;
        if classify_start_lag(args.slot.elapsed_ms, actual_elapsed_ms, args.tick_ms)
            == StartLagClass::RuntimeLagged
        {
            args.missed_starts.fetch_add(1, Ordering::SeqCst);
            let _ = args.metric_tx.send(WaveMetricEvent::RuntimeLaggedStart {
                elapsed_ms: actual_elapsed_ms,
            });
        }

        if args
            .prepare_tx
            .send(WavePrepareIntent {
                cursor,
                scheduled_elapsed_ms: args.slot.elapsed_ms,
                target_start_elapsed_ms,
                expires_at_elapsed_ms: args.slot.expires_at_elapsed_ms,
            })
            .is_err()
        {
            error!("wave prepare workers stopped before accepting cursor");
            break;
        }
    }
}

struct WavePrepareWorkerConfig {
    pipeline: Arc<Pipeline>,
    specs: Arc<Vec<RuntimeSpec>>,
    env_groups: Arc<Vec<RuntimeEnvGroup>>,
    selected_env_group_slug: Option<String>,
    started: Instant,
    request_tx: mpsc::UnboundedSender<ReadyWaveRequest<PipelineCursor>>,
    prepare_error_tx: mpsc::UnboundedSender<WavePrepareError>,
    prepare_progress_tx: mpsc::UnboundedSender<()>,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    ready_to_send: Arc<AtomicUsize>,
    token: tokio_util::sync::CancellationToken,
}

async fn run_wave_prepare_worker(
    config: WavePrepareWorkerConfig,
    prepare_rx: Arc<Mutex<mpsc::UnboundedReceiver<WavePrepareIntent>>>,
) {
    loop {
        let intent = {
            let mut rx = prepare_rx.lock().await;
            tokio::select! {
                _ = config.token.cancelled() => return,
                maybe_intent = rx.recv() => maybe_intent,
            }
        };
        let Some(intent) = intent else {
            return;
        };
        if config.token.is_cancelled() {
            return;
        }

        prepare_and_enqueue_wave_request(&config, intent).await;
    }
}

async fn prepare_and_enqueue_wave_request(
    config: &WavePrepareWorkerConfig,
    intent: WavePrepareIntent,
) {
    let Some(step) = config.pipeline.steps.get(intent.cursor.step_index).cloned() else {
        let _ = config.metric_tx.send(WaveMetricEvent::PipelineFinished {
            duration_ms: intent.cursor.pipeline_started_at.elapsed().as_millis() as f64,
            success: false,
        });
        return;
    };
    let max_attempts = max_attempts_for_step(&step);
    let prepared = match prepare_http_step(
        &step,
        &intent.cursor.context,
        Some(config.specs.as_slice()),
        Some(config.env_groups.as_slice()),
        config.selected_env_group_slug.as_deref(),
        intent.cursor.attempt,
        max_attempts,
    ) {
        Ok(prepared) => prepared,
        Err(result) => {
            let _ = config.prepare_error_tx.send(WavePrepareError {
                result,
                cursor: intent.cursor,
            });
            return;
        }
    };

    let prepared_elapsed_ms = config.started.elapsed().as_millis() as u64;
    let _ = config.metric_tx.send(WaveMetricEvent::RequestPrepared {
        elapsed_ms: prepared_elapsed_ms,
    });

    let enqueue_elapsed_ms = config.started.elapsed().as_millis() as u64;
    config.ready_to_send.fetch_add(1, Ordering::SeqCst);
    if config
        .request_tx
        .send(ReadyWaveRequest {
            step,
            context: intent.cursor.context.clone(),
            cursor: intent.cursor,
            prepared,
            specs: Arc::clone(&config.specs),
            env_groups: Arc::clone(&config.env_groups),
            selected_env_group_slug: config.selected_env_group_slug.clone(),
            scheduled_elapsed_ms: intent.scheduled_elapsed_ms,
            target_start_elapsed_ms: intent.target_start_elapsed_ms,
            expires_at_elapsed_ms: intent.expires_at_elapsed_ms,
            sender_enqueued_elapsed_ms: enqueue_elapsed_ms,
        })
        .is_err()
    {
        config.ready_to_send.fetch_sub(1, Ordering::SeqCst);
        error!("wave sender stopped before accepting prepared request");
        return;
    }

    let _ = config.metric_tx.send(WaveMetricEvent::RequestEnqueued {
        elapsed_ms: enqueue_elapsed_ms,
    });
    let _ = config.prepare_progress_tx.send(());
}

pub async fn run_wave_dispatcher_loop(
    config: WaveDispatcherConfig,
    mut channels: WaveDispatcherChannels,
    shared: WaveDispatcherShared,
    token: tokio_util::sync::CancellationToken,
) {
    let mut ready = VecDeque::new();
    let (prepare_tx, prepare_rx) = mpsc::unbounded_channel::<WavePrepareIntent>();
    let prepare_rx = Arc::new(Mutex::new(prepare_rx));
    let (prepare_error_tx, mut prepare_error_rx) = mpsc::unbounded_channel::<WavePrepareError>();
    let (prepare_progress_tx, mut prepare_progress_rx) = mpsc::unbounded_channel::<()>();
    let mut prepare_handles = Vec::new();
    for _ in 0..prepare_worker_threads() {
        prepare_handles.push(tokio::spawn(run_wave_prepare_worker(
            WavePrepareWorkerConfig {
                pipeline: Arc::clone(&config.pipeline),
                specs: Arc::clone(&config.specs),
                env_groups: Arc::clone(&config.env_groups),
                selected_env_group_slug: config.selected_env_group_slug.clone(),
                started: config.started,
                request_tx: channels.request_tx.clone(),
                prepare_error_tx: prepare_error_tx.clone(),
                prepare_progress_tx: prepare_progress_tx.clone(),
                metric_tx: channels.metric_tx.clone(),
                ready_to_send: Arc::clone(&shared.ready_to_send),
                token: token.clone(),
            },
            Arc::clone(&prepare_rx),
        )));
    }
    drop(prepare_error_tx);
    drop(prepare_progress_tx);

    while !token.is_cancelled() {
        tokio::select! {
            biased;

            _ = token.cancelled() => break,
            Some(slot) = channels.slot_rx.recv() => {
                dispatch_slot_prepare_intents(DispatchSlotPrepareArgs {
                    slot,
                    ready: &mut ready,
                    pipeline: &config.pipeline,
                    prepare_tx: &prepare_tx,
                    metric_tx: &channels.metric_tx,
                    missed_starts: &shared.missed_starts,
                    started: config.started,
                    tick_ms: config.tick_ms,
                    token: &token,
                })
                .await;

                drain_observer_events_budgeted(
                    &mut channels.observer_rx,
                    &mut ready,
                    &config.pipeline,
                    &channels.metric_tx,
                    OBSERVER_EVENTS_PER_TICK_BUDGET,
                )
                .await;

                publish_dispatcher_snapshot(&channels.snapshot_tx, &ready, &shared.ready_to_send);
            }
            Some(event) = channels.observer_rx.recv() => {
                handle_step_result(
                    event.result,
                    event.cursor,
                    &mut ready,
                    &config.pipeline,
                    &channels.metric_tx,
                )
                .await;
                publish_dispatcher_snapshot(&channels.snapshot_tx, &ready, &shared.ready_to_send);
            }
            Some(error) = prepare_error_rx.recv() => {
                handle_prepare_error(
                    error.result,
                    error.cursor,
                    &mut ready,
                    &config.pipeline,
                    &channels.metric_tx,
                    &shared.missed_starts,
                )
                .await;
                publish_dispatcher_snapshot(&channels.snapshot_tx, &ready, &shared.ready_to_send);
            }
            Some(()) = prepare_progress_rx.recv() => {
                publish_dispatcher_snapshot(&channels.snapshot_tx, &ready, &shared.ready_to_send);
            }
            else => break,
        }
    }

    drop(prepare_tx);
    drain_all_observer_events(
        &mut channels.observer_rx,
        &mut ready,
        &config.pipeline,
        &channels.metric_tx,
    )
    .await;
    publish_dispatcher_snapshot(&channels.snapshot_tx, &ready, &shared.ready_to_send);
    for handle in prepare_handles {
        handle.abort();
        let _ = handle.await;
    }
}

pub fn spawn_wave_dispatcher_thread(
    config: WaveDispatcherConfig,
    channels: WaveDispatcherChannels,
    shared: WaveDispatcherShared,
    token: tokio_util::sync::CancellationToken,
) -> WaveDispatcherHandle {
    let dispatcher_token = token.child_token();
    let thread_token = dispatcher_token.clone();
    let join = std::thread::Builder::new()
        .name("previa-wave-dispatcher".to_owned())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(prepare_worker_threads().saturating_add(1))
                .thread_name("previa-wave-dispatcher")
                .enable_all()
                .build()
                .expect("failed to build previa wave dispatcher runtime");
            runtime.block_on(run_wave_dispatcher_loop(
                config,
                channels,
                shared,
                thread_token,
            ));
        })
        .expect("failed to spawn previa wave dispatcher thread");

    WaveDispatcherHandle {
        token: dispatcher_token,
        join,
    }
}

fn prepare_worker_threads() -> usize {
    std::env::var("RUNNER_WAVE_PREPARE_THREADS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|value| value.get())
                .unwrap_or(2)
                .clamp(2, 8)
        })
}

fn publish_dispatcher_snapshot(
    snapshot_tx: &watch::Sender<WaveDispatcherSnapshot>,
    ready: &VecDeque<PipelineCursor>,
    ready_to_send: &Arc<AtomicUsize>,
) {
    let _ = snapshot_tx.send(WaveDispatcherSnapshot {
        ready_continuations: ready.len(),
        ready_to_send: ready_to_send.load(Ordering::SeqCst),
    });
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

#[cfg(test)]
mod tests {
    use super::*;

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
            PipelineCursor::new(Instant::now())
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

    #[test]
    fn dispatcher_snapshot_counts_ready_continuations_and_ready_to_send() {
        let snapshot = WaveDispatcherSnapshot {
            ready_continuations: 7,
            ready_to_send: 5,
        };

        assert_eq!(snapshot.ready_requests(), 12);
    }

    #[test]
    fn spread_deadlines_evenly_inside_tick() {
        assert_eq!(
            spread_deadlines(10_000, 1_000, 5),
            vec![10_000, 10_200, 10_400, 10_600, 10_800]
        );
        assert_eq!(
            spread_deadlines(10_000, 100, 4),
            vec![10_000, 10_025, 10_050, 10_075]
        );
    }

    #[test]
    fn spread_deadlines_handles_single_start() {
        assert_eq!(spread_deadlines(10_000, 1_000, 1), vec![10_000]);
        assert!(spread_deadlines(10_000, 1_000, 0).is_empty());
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

    #[tokio::test]
    async fn expired_dispatch_slot_records_lag_without_enqueuing_requests() {
        let pipeline = Pipeline {
            id: Some("p".to_owned()),
            name: "pipeline".to_owned(),
            description: None,
            steps: Vec::new(),
        };
        let (prepare_tx, mut prepare_rx) = mpsc::unbounded_channel();
        let (metric_tx, mut metric_rx) = mpsc::unbounded_channel();
        let missed_starts = Arc::new(AtomicUsize::new(0));
        let token = tokio_util::sync::CancellationToken::new();
        let mut ready = VecDeque::new();
        let started = Instant::now() - std::time::Duration::from_millis(1_000);

        dispatch_slot_prepare_intents(DispatchSlotPrepareArgs {
            slot: WaveDispatchSlot {
                elapsed_ms: 100,
                expires_at_elapsed_ms: 200,
                planned_starts: 7,
                target_rps_limit: 70.0,
                scheduled_total: 7,
                scheduler_lag_ms: 0,
                missed_due_to_scheduler_lag: 0,
            },
            ready: &mut ready,
            pipeline: &pipeline,
            prepare_tx: &prepare_tx,
            metric_tx: &metric_tx,
            missed_starts: &missed_starts,
            started,
            tick_ms: 100,
            token: &token,
        })
        .await;

        assert_eq!(missed_starts.load(Ordering::SeqCst), 7);
        assert!(prepare_rx.try_recv().is_err());
        assert!(matches!(
            metric_rx.try_recv(),
            Ok(WaveMetricEvent::DispatcherLaggedStarts { count: 7, .. })
        ));
    }

    #[tokio::test]
    async fn dispatch_slot_enqueues_prepare_intents_without_preparing_inline() {
        let pipeline = Pipeline {
            id: Some("p".to_owned()),
            name: "pipeline".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "s1".to_owned(),
                name: "GET".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: "http://example.test/users".to_owned(),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: None,
                retry: None,
                asserts: Vec::new(),
            }],
        };
        let (prepare_tx, mut prepare_rx) = mpsc::unbounded_channel();
        let (metric_tx, _metric_rx) = mpsc::unbounded_channel();
        let missed_starts = Arc::new(AtomicUsize::new(0));
        let token = tokio_util::sync::CancellationToken::new();
        let mut ready = VecDeque::new();

        dispatch_slot_prepare_intents(DispatchSlotPrepareArgs {
            slot: WaveDispatchSlot {
                elapsed_ms: 0,
                expires_at_elapsed_ms: 1_000,
                planned_starts: 500,
                target_rps_limit: 5_000.0,
                scheduled_total: 500,
                scheduler_lag_ms: 0,
                missed_due_to_scheduler_lag: 0,
            },
            ready: &mut ready,
            pipeline: &pipeline,
            prepare_tx: &prepare_tx,
            metric_tx: &metric_tx,
            missed_starts: &missed_starts,
            started: Instant::now(),
            tick_ms: 100,
            token: &token,
        })
        .await;

        let mut count = 0usize;
        while prepare_rx.try_recv().is_ok() {
            count += 1;
        }

        assert_eq!(count, 500);
        assert_eq!(missed_starts.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn prepared_request_carries_slot_deadline_to_sender() {
        let pipeline = Pipeline {
            id: Some("p".to_owned()),
            name: "pipeline".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "s1".to_owned(),
                name: "GET".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: "http://example.test/users".to_owned(),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: None,
                retry: None,
                asserts: Vec::new(),
            }],
        };
        let (prepare_tx, mut prepare_rx) = mpsc::unbounded_channel();
        let (metric_tx, _metric_rx) = mpsc::unbounded_channel();
        let missed_starts = Arc::new(AtomicUsize::new(0));
        let token = tokio_util::sync::CancellationToken::new();
        let mut ready = VecDeque::new();
        let started = Instant::now();

        dispatch_slot_prepare_intents(DispatchSlotPrepareArgs {
            slot: WaveDispatchSlot {
                elapsed_ms: 4_200,
                expires_at_elapsed_ms: 4_300,
                planned_starts: 1,
                target_rps_limit: 10.0,
                scheduled_total: 1,
                scheduler_lag_ms: 0,
                missed_due_to_scheduler_lag: 0,
            },
            ready: &mut ready,
            pipeline: &pipeline,
            prepare_tx: &prepare_tx,
            metric_tx: &metric_tx,
            missed_starts: &missed_starts,
            started,
            tick_ms: 100,
            token: &token,
        })
        .await;

        let intent = prepare_rx.try_recv().expect("prepare intent should exist");
        assert_eq!(intent.scheduled_elapsed_ms, 4_200);
        assert_eq!(intent.target_start_elapsed_ms, 4_200);
        assert_eq!(intent.expires_at_elapsed_ms, 4_300);
    }

    #[tokio::test]
    async fn dispatcher_thread_consumes_fresh_slot_and_enqueues_ready_request() {
        let pipeline = Arc::new(Pipeline {
            id: Some("p".to_owned()),
            name: "pipeline".to_owned(),
            description: None,
            steps: vec![PipelineStep {
                id: "s1".to_owned(),
                name: "GET".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: "http://example.test/users".to_owned(),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: None,
                retry: None,
                asserts: Vec::new(),
            }],
        });
        let specs = Arc::new(Vec::new());
        let env_groups = Arc::new(Vec::new());
        let (slot_tx, slot_rx) = mpsc::channel(8);
        let (request_tx, mut request_rx) = mpsc::unbounded_channel();
        let (observer_tx, observer_rx) = mpsc::unbounded_channel();
        drop(observer_tx);
        let (metric_tx, _metric_rx) = mpsc::unbounded_channel();
        let (snapshot_tx, mut snapshot_rx) = watch::channel(WaveDispatcherSnapshot::default());
        let ready_to_send = Arc::new(AtomicUsize::new(0));
        let missed_starts = Arc::new(AtomicUsize::new(0));
        let token = tokio_util::sync::CancellationToken::new();

        let handle = spawn_wave_dispatcher_thread(
            WaveDispatcherConfig {
                pipeline,
                specs,
                env_groups,
                selected_env_group_slug: None,
                started: Instant::now(),
                tick_ms: 100,
            },
            WaveDispatcherChannels {
                slot_rx,
                request_tx,
                observer_rx,
                metric_tx,
                snapshot_tx,
            },
            WaveDispatcherShared {
                ready_to_send: Arc::clone(&ready_to_send),
                missed_starts: Arc::clone(&missed_starts),
            },
            token,
        );

        slot_tx
            .send(WaveDispatchSlot {
                elapsed_ms: 0,
                expires_at_elapsed_ms: 1_000,
                planned_starts: 1,
                target_rps_limit: 10.0,
                scheduled_total: 1,
                scheduler_lag_ms: 0,
                missed_due_to_scheduler_lag: 0,
            })
            .await
            .unwrap();

        let request =
            tokio::time::timeout(std::time::Duration::from_millis(300), request_rx.recv())
                .await
                .expect("dispatcher should enqueue request")
                .expect("request channel should stay open");

        assert_eq!(request.step.id, "s1");
        assert_eq!(ready_to_send.load(Ordering::SeqCst), 1);

        tokio::time::timeout(std::time::Duration::from_millis(300), async {
            loop {
                if snapshot_rx.borrow().ready_requests() == 1 {
                    break;
                }
                snapshot_rx.changed().await.unwrap();
            }
        })
        .await
        .expect("snapshot should include prepared request");

        handle.stop();
    }
}
