use tokio::sync::{mpsc, watch};

use crate::server::metrics::{MetricsAccumulator, WaveMetricsSnapshot};
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
    HttpStarted {
        elapsed_ms: u64,
    },
    HttpSendReturned {
        elapsed_ms: u64,
    },
    HttpCompleted(usize),
    ResponseBodyCompleted {
        elapsed_ms: u64,
        count: usize,
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
    DependencyLimitedStarts(usize),
    Snapshot {
        wave: WaveMetricsSnapshot,
        runtime: Option<RunnerInfoResponse>,
        duration_ms: Option<u64>,
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
            WaveMetricEvent::HttpStarted { elapsed_ms } => {
                accumulator.record_http_start_at(elapsed_ms)
            }
            WaveMetricEvent::HttpSendReturned { elapsed_ms } => {
                accumulator.record_http_send_returned_at(elapsed_ms)
            }
            WaveMetricEvent::HttpCompleted(count) => {
                accumulator.record_http_completed_count(count);
            }
            WaveMetricEvent::ResponseBodyCompleted { elapsed_ms, count } => {
                accumulator.record_response_body_completed_at(elapsed_ms, count);
            }
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
            WaveMetricEvent::DependencyLimitedStarts(count) => {
                accumulator.record_dependency_limited_starts_count(count);
            }
            WaveMetricEvent::Snapshot {
                wave,
                runtime,
                duration_ms,
            } => {
                latest_wave = Some(wave);
                latest_runtime = runtime;
                latest_duration_ms = duration_ms;
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
            );
        }
    }

    publish_snapshot(
        &accumulator,
        &snapshot_tx,
        latest_duration_ms,
        latest_runtime,
        latest_wave,
    );
}

fn publish_snapshot(
    accumulator: &MetricsAccumulator,
    snapshot_tx: &watch::Sender<LoadTestMetrics>,
    duration_ms: Option<u64>,
    runtime: Option<RunnerInfoResponse>,
    wave: Option<WaveMetricsSnapshot>,
) {
    let snapshot = accumulator.snapshot_with_wave(duration_ms, runtime, wave);
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
}
