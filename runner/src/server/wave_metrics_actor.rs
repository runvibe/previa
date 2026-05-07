use tokio::sync::{mpsc, watch};

use crate::server::metrics::{MetricsAccumulator, MetricsSnapshotScope, WaveMetricsSnapshot};
#[cfg(test)]
use crate::server::models::LoadMetricsSnapshotMode;
use crate::server::models::{LoadTestMetrics, RunnerInfoResponse};
use crate::server::wave_scheduler::WaveSchedulerMetric;

#[derive(Debug, Clone)]
pub enum WaveMetricEvent {
    Scheduler(WaveSchedulerMetric),
    PipelineStarted,
    DispatchStarted {
        elapsed_ms: u64,
    },
    SlotEnqueued {
        elapsed_ms: u64,
        count: usize,
    },
    RequestPrepared {
        elapsed_ms: u64,
    },
    RequestEnqueued {
        elapsed_ms: u64,
    },
    SendTaskSpawned {
        elapsed_ms: u64,
    },
    SendStarted {
        elapsed_ms: u64,
    },
    SenderStartLag {
        elapsed_ms: u64,
        lag_ms: u64,
    },
    HttpStarted {
        elapsed_ms: u64,
    },
    HttpSendReturned {
        elapsed_ms: u64,
    },
    HttpSendDuration {
        elapsed_ms: u64,
        duration_ms: u64,
    },
    HttpCompleted(usize),
    ResponseBodyCompleted {
        elapsed_ms: u64,
        count: usize,
    },
    ResponseObservationDuration {
        elapsed_ms: u64,
        duration_ms: u64,
    },
    PipelineFinished {
        duration_ms: f64,
        success: bool,
    },
    ErrorSample {
        step_id: String,
        http_status: Option<u16>,
        error: String,
    },
    NetworkBytes {
        tx: u64,
        rx: u64,
    },
    DispatcherLaggedStarts {
        elapsed_ms: u64,
        count: usize,
    },
    RuntimeLaggedStart {
        elapsed_ms: u64,
    },
    SenderLaggedStarts {
        elapsed_ms: u64,
        count: usize,
    },
    SenderQueueDepth {
        depth: usize,
    },
    DependencyLimitedStarts(usize),
    Snapshot {
        wave: WaveMetricsSnapshot,
        runtime: Option<RunnerInfoResponse>,
        duration_ms: Option<u64>,
        scope: MetricsSnapshotScope,
    },
}

pub async fn run_wave_metrics_actor(
    mut event_rx: mpsc::UnboundedReceiver<WaveMetricEvent>,
    snapshot_tx: watch::Sender<LoadTestMetrics>,
) {
    let mut accumulator = MetricsAccumulator::new();
    let mut latest_wave: Option<WaveMetricsSnapshot> = None;
    let mut latest_runtime: Option<RunnerInfoResponse> = None;
    let mut latest_duration_ms: Option<u64> = None;
    let mut latest_scope = MetricsSnapshotScope::Full;

    while let Some(event) = event_rx.recv().await {
        let mut should_publish = false;
        match event {
            WaveMetricEvent::Scheduler(WaveSchedulerMetric::DispatchScheduled {
                elapsed_ms,
                count,
            }) => {
                accumulator.record_dispatch_submitted_count(count);
                accumulator.record_planned_at(elapsed_ms, count);
            }
            WaveMetricEvent::Scheduler(WaveSchedulerMetric::SlotEnqueued { .. }) => {}
            WaveMetricEvent::Scheduler(WaveSchedulerMetric::SchedulerLag {
                lag_ms,
                missed_starts,
                ..
            }) => {
                accumulator.record_scheduler_lag_ms(lag_ms);
                accumulator.record_scheduler_lagged_starts_count(missed_starts);
            }
            WaveMetricEvent::Scheduler(WaveSchedulerMetric::SlotBackpressure {
                dropped_starts,
                ..
            }) => {
                accumulator.record_scheduler_lagged_starts_count(dropped_starts);
            }
            WaveMetricEvent::PipelineStarted => accumulator.record_start(),
            WaveMetricEvent::DispatchStarted { elapsed_ms } => {
                accumulator.record_dispatch_started_at(elapsed_ms);
            }
            WaveMetricEvent::SlotEnqueued { elapsed_ms, count } => {
                accumulator.record_slot_enqueued_at(elapsed_ms, count);
            }
            WaveMetricEvent::RequestPrepared { elapsed_ms } => {
                accumulator.record_request_prepared_at(elapsed_ms)
            }
            WaveMetricEvent::RequestEnqueued { elapsed_ms } => {
                accumulator.record_request_enqueued_at(elapsed_ms)
            }
            WaveMetricEvent::SendTaskSpawned { elapsed_ms } => {
                accumulator.record_send_task_spawned_at(elapsed_ms)
            }
            WaveMetricEvent::SendStarted { elapsed_ms } => {
                accumulator.record_send_started_at(elapsed_ms)
            }
            WaveMetricEvent::SenderStartLag { elapsed_ms, lag_ms } => {
                accumulator.record_sender_start_lag_at(elapsed_ms, lag_ms)
            }
            WaveMetricEvent::HttpStarted { elapsed_ms } => {
                accumulator.record_http_start_at(elapsed_ms)
            }
            WaveMetricEvent::HttpSendReturned { elapsed_ms } => {
                accumulator.record_http_send_returned_at(elapsed_ms)
            }
            WaveMetricEvent::HttpSendDuration {
                elapsed_ms,
                duration_ms,
            } => accumulator.record_http_send_duration_at(elapsed_ms, duration_ms),
            WaveMetricEvent::HttpCompleted(count) => {
                accumulator.record_http_completed_count(count);
            }
            WaveMetricEvent::ResponseBodyCompleted { elapsed_ms, count } => {
                accumulator.record_response_body_completed_at(elapsed_ms, count);
            }
            WaveMetricEvent::ResponseObservationDuration {
                elapsed_ms,
                duration_ms,
            } => accumulator.record_response_observation_duration_at(elapsed_ms, duration_ms),
            WaveMetricEvent::PipelineFinished {
                duration_ms,
                success,
            } => {
                accumulator.update(duration_ms, success);
            }
            WaveMetricEvent::ErrorSample {
                step_id,
                http_status,
                error,
            } => accumulator.record_error_sample(&step_id, http_status, &error),
            WaveMetricEvent::NetworkBytes { tx, rx } => accumulator.add_network_bytes(tx, rx),
            WaveMetricEvent::DispatcherLaggedStarts { elapsed_ms, count } => {
                accumulator.record_dispatcher_lagged_starts_at(elapsed_ms, count);
            }
            WaveMetricEvent::RuntimeLaggedStart { elapsed_ms } => {
                accumulator.record_runtime_lagged_start_at(elapsed_ms);
            }
            WaveMetricEvent::SenderLaggedStarts { elapsed_ms, count } => {
                accumulator.record_sender_lagged_starts_at(elapsed_ms, count);
            }
            WaveMetricEvent::SenderQueueDepth { depth } => {
                accumulator.record_sender_queue_depth(depth);
            }
            WaveMetricEvent::DependencyLimitedStarts(count) => {
                accumulator.record_dependency_limited_starts_count(count);
            }
            WaveMetricEvent::Snapshot {
                wave,
                runtime,
                duration_ms,
                scope,
            } => {
                latest_wave = Some(wave);
                latest_runtime = runtime;
                latest_duration_ms = duration_ms;
                latest_scope = scope;
                should_publish = true;
            }
        }

        if should_publish {
            publish_snapshot(
                &accumulator,
                &snapshot_tx,
                latest_duration_ms,
                latest_runtime.clone(),
                latest_wave,
                latest_scope,
            );
        }
    }

    publish_snapshot(
        &accumulator,
        &snapshot_tx,
        latest_duration_ms,
        latest_runtime,
        latest_wave,
        MetricsSnapshotScope::Full,
    );
}

fn publish_snapshot(
    accumulator: &MetricsAccumulator,
    snapshot_tx: &watch::Sender<LoadTestMetrics>,
    duration_ms: Option<u64>,
    runtime: Option<RunnerInfoResponse>,
    wave: Option<WaveMetricsSnapshot>,
    scope: MetricsSnapshotScope,
) {
    let snapshot = accumulator.snapshot_with_wave_scope(duration_ms, runtime, wave, scope);
    let _ = snapshot_tx.send(snapshot);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn metrics_actor_applies_dispatch_and_scheduler_events() {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (snapshot_tx, snapshot_rx) = watch::channel(LoadTestMetrics::default());

        let actor = tokio::spawn(run_wave_metrics_actor(event_rx, snapshot_tx));

        event_tx
            .send(WaveMetricEvent::Scheduler(
                WaveSchedulerMetric::DispatchScheduled {
                    elapsed_ms: 1_000,
                    count: 3,
                },
            ))
            .unwrap();
        event_tx
            .send(WaveMetricEvent::DispatchStarted { elapsed_ms: 42_000 })
            .unwrap();
        event_tx
            .send(WaveMetricEvent::DispatchStarted { elapsed_ms: 42_000 })
            .unwrap();
        event_tx
            .send(WaveMetricEvent::SlotEnqueued {
                elapsed_ms: 1_000,
                count: 3,
            })
            .unwrap();
        event_tx
            .send(WaveMetricEvent::RequestPrepared { elapsed_ms: 1_000 })
            .unwrap();
        event_tx
            .send(WaveMetricEvent::RequestPrepared { elapsed_ms: 1_000 })
            .unwrap();
        event_tx
            .send(WaveMetricEvent::RequestEnqueued { elapsed_ms: 1_000 })
            .unwrap();
        event_tx
            .send(WaveMetricEvent::SendTaskSpawned { elapsed_ms: 1_000 })
            .unwrap();
        event_tx
            .send(WaveMetricEvent::SendStarted { elapsed_ms: 1_000 })
            .unwrap();
        event_tx
            .send(WaveMetricEvent::HttpStarted { elapsed_ms: 1_000 })
            .unwrap();
        event_tx
            .send(WaveMetricEvent::HttpSendReturned { elapsed_ms: 1_000 })
            .unwrap();
        event_tx
            .send(WaveMetricEvent::Scheduler(
                WaveSchedulerMetric::SchedulerLag {
                    elapsed_ms: 1_000,
                    lag_ms: 25,
                    missed_starts: 4,
                },
            ))
            .unwrap();
        event_tx
            .send(WaveMetricEvent::DispatcherLaggedStarts {
                elapsed_ms: 1_000,
                count: 6,
            })
            .unwrap();
        drop(event_tx);

        actor.await.unwrap();
        let snapshot = snapshot_rx.borrow().clone();

        assert_eq!(snapshot.dispatch_submitted, Some(3));
        assert_eq!(snapshot.dispatch_started, Some(2));
        assert_eq!(snapshot.dispatch_buckets.len(), 1);
        assert_eq!(snapshot.dispatch_buckets[0].elapsed_ms, 42_000);
        assert_eq!(snapshot.dispatch_buckets[0].count, 2);
        assert_eq!(snapshot.scheduler_lag_ms, Some(25));
        assert_eq!(snapshot.scheduler_lagged_starts, Some(4));
        assert_eq!(snapshot.dispatcher_lagged_starts, Some(6));
        assert_eq!(snapshot.slot_enqueued, Some(3));
        assert_eq!(snapshot.request_prepared, Some(2));
        assert_eq!(snapshot.request_enqueued, Some(1));
        assert_eq!(snapshot.send_task_spawned, Some(1));
        assert_eq!(snapshot.send_started, Some(1));
        assert_eq!(snapshot.http_started, 1);
        assert_eq!(snapshot.http_send_returned, Some(1));
        assert_eq!(snapshot.lifecycle_buckets.len(), 1);
        assert_eq!(snapshot.lifecycle_buckets[0].planned, 3);
        assert_eq!(snapshot.lifecycle_buckets[0].http_send_returned, 1);
    }

    #[tokio::test]
    async fn metrics_actor_publishes_scoped_live_snapshot() {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (snapshot_tx, mut snapshot_rx) = watch::channel(LoadTestMetrics::default());

        let actor = tokio::spawn(run_wave_metrics_actor(event_rx, snapshot_tx));

        event_tx
            .send(WaveMetricEvent::Scheduler(
                WaveSchedulerMetric::DispatchScheduled {
                    elapsed_ms: 0,
                    count: 10,
                },
            ))
            .unwrap();
        event_tx
            .send(WaveMetricEvent::Scheduler(
                WaveSchedulerMetric::DispatchScheduled {
                    elapsed_ms: 1_000,
                    count: 20,
                },
            ))
            .unwrap();
        event_tx
            .send(WaveMetricEvent::Snapshot {
                wave: WaveMetricsSnapshot {
                    target_intensity: 10.0,
                    target_rps_limit: 100.0,
                    in_flight: 0,
                    runner_max_rps: 1000.0,
                    tick_ms: 100,
                    scheduled_starts: 30,
                    missed_starts: 0,
                    ready_requests: 0,
                    active_pipelines: 0,
                    outstanding_requests: 0,
                },
                runtime: None,
                duration_ms: None,
                scope: MetricsSnapshotScope::LiveWindow {
                    from_elapsed_ms: 1_000,
                    through_elapsed_ms: 1_000,
                },
            })
            .unwrap();

        snapshot_rx.changed().await.unwrap();
        let snapshot = snapshot_rx.borrow().clone();

        assert_eq!(snapshot.snapshot_mode, Some(LoadMetricsSnapshotMode::Live));
        assert_eq!(snapshot.lifecycle_buckets.len(), 1);
        assert_eq!(snapshot.lifecycle_buckets[0].elapsed_ms, 1_000);
        assert_eq!(snapshot.lifecycle_buckets[0].planned, 20);

        drop(event_tx);
        actor.await.unwrap();
    }

    #[tokio::test]
    async fn metrics_actor_records_sender_saturation() {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (snapshot_tx, mut snapshot_rx) = watch::channel(LoadTestMetrics::default());

        let actor = tokio::spawn(run_wave_metrics_actor(event_rx, snapshot_tx));

        event_tx
            .send(WaveMetricEvent::SenderLaggedStarts {
                elapsed_ms: 2_000,
                count: 3,
            })
            .unwrap();
        event_tx
            .send(WaveMetricEvent::SenderQueueDepth { depth: 9 })
            .unwrap();
        event_tx
            .send(WaveMetricEvent::Snapshot {
                wave: WaveMetricsSnapshot {
                    target_intensity: 50.0,
                    target_rps_limit: 500.0,
                    in_flight: 0,
                    runner_max_rps: 1_000.0,
                    tick_ms: 100,
                    scheduled_starts: 10,
                    missed_starts: 3,
                    ready_requests: 9,
                    active_pipelines: 9,
                    outstanding_requests: 0,
                },
                runtime: None,
                duration_ms: None,
                scope: MetricsSnapshotScope::Full,
            })
            .unwrap();

        snapshot_rx.changed().await.unwrap();
        let snapshot = snapshot_rx.borrow().clone();

        assert_eq!(snapshot.sender_lagged_starts, Some(3));
        assert_eq!(snapshot.sender_queue_depth, Some(9));

        drop(event_tx);
        actor.await.unwrap();
    }
}
