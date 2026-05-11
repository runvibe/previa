use std::collections::BTreeMap;

use crate::server::models::{
    LoadDispatchBucket, LoadErrorSample, LoadLatencyBucket, LoadLifecycleBucket,
    LoadMetricsSnapshotMode, LoadTestMetrics, RunnerInfoResponse,
};
use crate::server::utils::{now_ms, round2};
use previa_runner::{StepExecutionResult, StepRequest, StepResponse};

#[derive(Debug, Clone, Copy)]
pub struct WaveMetricsSnapshot {
    pub target_intensity: f64,
    pub target_rps_limit: f64,
    pub in_flight: usize,
    pub runner_max_rps: f64,
    pub tick_ms: u64,
    pub scheduled_starts: usize,
    pub missed_starts: usize,
    pub ready_requests: usize,
    pub active_pipelines: usize,
    pub outstanding_requests: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricsSnapshotScope {
    Full,
    LiveWindow {
        from_elapsed_ms: u64,
        through_elapsed_ms: u64,
    },
}

#[derive(Debug)]
pub struct MetricsAccumulator {
    total_started: usize,
    total_sent: usize,
    total_success: usize,
    total_error: usize,
    http_started: usize,
    http_completed: usize,
    dispatch_submitted: usize,
    dispatch_started: usize,
    http_send_returned: usize,
    response_body_completed: usize,
    dependency_limited_starts: usize,
    dispatcher_lagged_starts: usize,
    runtime_lagged_starts: usize,
    scheduler_lag_ms: u64,
    scheduler_lagged_starts: usize,
    slot_enqueued: usize,
    request_prepared: usize,
    request_enqueued: usize,
    send_task_spawned: usize,
    send_started: usize,
    sender_lagged_starts: usize,
    sender_queue_depth: usize,
    sender_start_lag_histogram: BTreeMap<u64, usize>,
    http_send_duration_histogram: BTreeMap<u64, usize>,
    response_observation_duration_histogram: BTreeMap<u64, usize>,
    start_time: u64,
    network_tx_bytes: u64,
    network_rx_bytes: u64,
    latency_sample_count: usize,
    latency_total_duration_ms: u64,
    latency_histogram: BTreeMap<u64, usize>,
    dispatch_buckets: BTreeMap<u64, usize>,
    lifecycle_buckets: BTreeMap<u64, LoadLifecycleBucket>,
    error_samples: Vec<LoadErrorSample>,
}

impl MetricsAccumulator {
    pub fn new() -> Self {
        Self {
            total_started: 0,
            total_sent: 0,
            total_success: 0,
            total_error: 0,
            http_started: 0,
            http_completed: 0,
            dispatch_submitted: 0,
            dispatch_started: 0,
            http_send_returned: 0,
            response_body_completed: 0,
            dependency_limited_starts: 0,
            dispatcher_lagged_starts: 0,
            runtime_lagged_starts: 0,
            scheduler_lag_ms: 0,
            scheduler_lagged_starts: 0,
            slot_enqueued: 0,
            request_prepared: 0,
            request_enqueued: 0,
            send_task_spawned: 0,
            send_started: 0,
            sender_lagged_starts: 0,
            sender_queue_depth: 0,
            sender_start_lag_histogram: BTreeMap::new(),
            http_send_duration_histogram: BTreeMap::new(),
            response_observation_duration_histogram: BTreeMap::new(),
            start_time: now_ms(),
            network_tx_bytes: 0,
            network_rx_bytes: 0,
            latency_sample_count: 0,
            latency_total_duration_ms: 0,
            latency_histogram: BTreeMap::new(),
            dispatch_buckets: BTreeMap::new(),
            lifecycle_buckets: BTreeMap::new(),
            error_samples: Vec::new(),
        }
    }

    pub fn record_start(&mut self) {
        self.total_started += 1;
    }

    pub fn update(&mut self, duration: f64, success: bool) {
        let duration_ms = if duration.is_finite() && duration >= 0.0 {
            duration.round() as u64
        } else {
            0
        };

        self.total_sent = self.total_sent.saturating_add(1);
        self.latency_sample_count = self.latency_sample_count.saturating_add(1);
        self.latency_total_duration_ms = self.latency_total_duration_ms.saturating_add(duration_ms);
        *self.latency_histogram.entry(duration_ms).or_insert(0) += 1;

        if success {
            self.total_success = self.total_success.saturating_add(1);
        } else {
            self.total_error = self.total_error.saturating_add(1);
        }
    }

    pub fn record_http_start(&mut self) {
        self.http_started += 1;
    }

    pub fn record_http_completed_count(&mut self, count: usize) {
        self.http_completed = self.http_completed.saturating_add(count);
    }

    pub fn record_dispatch_submitted_count(&mut self, count: usize) {
        self.dispatch_submitted = self.dispatch_submitted.saturating_add(count);
    }

    pub fn record_dispatch_started_at(&mut self, elapsed_ms: u64) {
        self.dispatch_started = self.dispatch_started.saturating_add(1);
        let bucket_ms = (elapsed_ms / 1000).saturating_mul(1000);
        *self.dispatch_buckets.entry(bucket_ms).or_insert(0) += 1;
    }

    pub fn record_planned_at(&mut self, elapsed_ms: u64, count: usize) {
        let bucket = self.lifecycle_bucket_mut(elapsed_ms);
        bucket.planned = bucket.planned.saturating_add(count);
    }

    pub fn record_slot_enqueued_at(&mut self, elapsed_ms: u64, count: usize) {
        self.record_slot_enqueued_count(count);
        let bucket = self.lifecycle_bucket_mut(elapsed_ms);
        bucket.slot_enqueued = bucket.slot_enqueued.saturating_add(count);
    }

    pub fn record_request_prepared_at(&mut self, elapsed_ms: u64) {
        self.record_request_prepared();
        let bucket = self.lifecycle_bucket_mut(elapsed_ms);
        bucket.request_prepared = bucket.request_prepared.saturating_add(1);
    }

    pub fn record_request_enqueued_at(&mut self, elapsed_ms: u64) {
        self.record_request_enqueued();
        let bucket = self.lifecycle_bucket_mut(elapsed_ms);
        bucket.request_enqueued = bucket.request_enqueued.saturating_add(1);
    }

    pub fn record_send_task_spawned_at(&mut self, elapsed_ms: u64) {
        self.record_send_task_spawned();
        let bucket = self.lifecycle_bucket_mut(elapsed_ms);
        bucket.send_task_spawned = bucket.send_task_spawned.saturating_add(1);
    }

    pub fn record_send_started_at(&mut self, elapsed_ms: u64) {
        self.record_send_started();
        let bucket = self.lifecycle_bucket_mut(elapsed_ms);
        bucket.send_started = bucket.send_started.saturating_add(1);
    }

    pub fn record_http_start_at(&mut self, elapsed_ms: u64) {
        self.record_http_start();
        let bucket = self.lifecycle_bucket_mut(elapsed_ms);
        bucket.http_started = bucket.http_started.saturating_add(1);
    }

    pub fn record_http_send_returned_at(&mut self, elapsed_ms: u64) {
        self.record_http_send_returned();
        let bucket = self.lifecycle_bucket_mut(elapsed_ms);
        bucket.http_send_returned = bucket.http_send_returned.saturating_add(1);
    }

    pub fn record_response_body_completed_at(&mut self, elapsed_ms: u64, count: usize) {
        self.record_response_body_completed_count(count);
        let bucket = self.lifecycle_bucket_mut(elapsed_ms);
        bucket.response_body_completed = bucket.response_body_completed.saturating_add(count);
    }

    pub fn record_dispatcher_lagged_starts_at(&mut self, elapsed_ms: u64, count: usize) {
        self.record_dispatcher_lagged_starts_count(count);
        let bucket = self.lifecycle_bucket_mut(elapsed_ms);
        bucket.dispatcher_lagged = bucket.dispatcher_lagged.saturating_add(count);
    }

    pub fn record_runtime_lagged_start_at(&mut self, elapsed_ms: u64) {
        self.record_runtime_lagged_start();
        let bucket = self.lifecycle_bucket_mut(elapsed_ms);
        bucket.runtime_lagged = bucket.runtime_lagged.saturating_add(1);
    }

    pub fn record_sender_lagged_starts_at(&mut self, elapsed_ms: u64, count: usize) {
        self.sender_lagged_starts = self.sender_lagged_starts.saturating_add(count);
        let bucket = self.lifecycle_bucket_mut(elapsed_ms);
        bucket.sender_lagged = bucket.sender_lagged.saturating_add(count);
    }

    pub fn record_sender_queue_depth(&mut self, depth: usize) {
        self.sender_queue_depth = depth;
    }

    pub fn record_sender_start_lag_at(&mut self, elapsed_ms: u64, lag_ms: u64) {
        *self.sender_start_lag_histogram.entry(lag_ms).or_insert(0) += 1;
        let bucket = self.lifecycle_bucket_mut(elapsed_ms);
        bucket.sender_start_lag_ms_max = bucket.sender_start_lag_ms_max.max(lag_ms);
    }

    pub fn record_http_send_duration_at(&mut self, elapsed_ms: u64, duration_ms: u64) {
        *self
            .http_send_duration_histogram
            .entry(duration_ms)
            .or_insert(0) += 1;
        let bucket = self.lifecycle_bucket_mut(elapsed_ms);
        bucket.http_send_duration_ms_max = bucket.http_send_duration_ms_max.max(duration_ms);
    }

    pub fn record_response_observation_duration_at(&mut self, elapsed_ms: u64, duration_ms: u64) {
        *self
            .response_observation_duration_histogram
            .entry(duration_ms)
            .or_insert(0) += 1;
        let bucket = self.lifecycle_bucket_mut(elapsed_ms);
        bucket.response_observation_duration_ms_max =
            bucket.response_observation_duration_ms_max.max(duration_ms);
    }

    pub fn record_http_send_returned(&mut self) {
        self.http_send_returned = self.http_send_returned.saturating_add(1);
    }

    pub fn record_response_body_completed_count(&mut self, count: usize) {
        self.response_body_completed = self.response_body_completed.saturating_add(count);
    }

    pub fn record_dependency_limited_starts_count(&mut self, count: usize) {
        self.dependency_limited_starts = self.dependency_limited_starts.saturating_add(count);
    }

    pub fn record_dispatcher_lagged_starts_count(&mut self, count: usize) {
        self.dispatcher_lagged_starts = self.dispatcher_lagged_starts.saturating_add(count);
    }

    pub fn record_runtime_lagged_start(&mut self) {
        self.runtime_lagged_starts = self.runtime_lagged_starts.saturating_add(1);
    }

    pub fn record_scheduler_lag_ms(&mut self, lag_ms: u64) {
        self.scheduler_lag_ms = self.scheduler_lag_ms.saturating_add(lag_ms);
    }

    pub fn record_scheduler_lagged_starts_count(&mut self, count: usize) {
        self.scheduler_lagged_starts = self.scheduler_lagged_starts.saturating_add(count);
    }

    pub fn record_slot_enqueued_count(&mut self, count: usize) {
        self.slot_enqueued = self.slot_enqueued.saturating_add(count);
    }

    pub fn record_request_prepared(&mut self) {
        self.request_prepared = self.request_prepared.saturating_add(1);
    }

    pub fn record_request_enqueued(&mut self) {
        self.request_enqueued = self.request_enqueued.saturating_add(1);
    }

    pub fn record_send_task_spawned(&mut self) {
        self.send_task_spawned = self.send_task_spawned.saturating_add(1);
    }

    pub fn record_send_started(&mut self) {
        self.send_started = self.send_started.saturating_add(1);
    }

    pub fn add_network_bytes(&mut self, tx_bytes: u64, rx_bytes: u64) {
        self.network_tx_bytes = self.network_tx_bytes.saturating_add(tx_bytes);
        self.network_rx_bytes = self.network_rx_bytes.saturating_add(rx_bytes);
    }

    pub fn record_error_sample(&mut self, step_id: &str, http_status: Option<u16>, error: &str) {
        if let Some(existing) = self.error_samples.iter_mut().find(|sample| {
            sample.step_id == step_id && sample.http_status == http_status && sample.error == error
        }) {
            existing.count = existing.count.saturating_add(1);
            return;
        }

        if self.error_samples.len() >= 10 {
            return;
        }

        self.error_samples.push(LoadErrorSample {
            step_id: step_id.to_owned(),
            http_status,
            error: error.to_owned(),
            count: 1,
        });
    }

    pub fn snapshot(
        &self,
        duration_ms: Option<u64>,
        runtime: Option<RunnerInfoResponse>,
    ) -> LoadTestMetrics {
        self.snapshot_with_wave(duration_ms, runtime, None)
    }

    pub fn snapshot_with_wave(
        &self,
        duration_ms: Option<u64>,
        runtime: Option<RunnerInfoResponse>,
        wave: Option<WaveMetricsSnapshot>,
    ) -> LoadTestMetrics {
        self.snapshot_with_wave_scope(duration_ms, runtime, wave, MetricsSnapshotScope::Full)
    }

    pub fn snapshot_with_wave_scope(
        &self,
        duration_ms: Option<u64>,
        runtime: Option<RunnerInfoResponse>,
        wave: Option<WaveMetricsSnapshot>,
        scope: MetricsSnapshotScope,
    ) -> LoadTestMetrics {
        let now = now_ms();
        let elapsed_ms = now.saturating_sub(self.start_time);

        let elapsed = (elapsed_ms as f64) / 1000.0;
        let rps_total = if self.http_started > 0 {
            self.http_started
        } else {
            self.total_sent
        };
        let rps = if elapsed > 0.0 {
            round2((rps_total as f64) / elapsed)
        } else {
            0.0
        };

        let runtime = runtime.map(|mut runtime| {
            runtime.network_tx_bytes = self.network_tx_bytes;
            runtime.network_rx_bytes = self.network_rx_bytes;
            runtime.network_total_bytes =
                self.network_tx_bytes.saturating_add(self.network_rx_bytes);
            runtime
        });
        let scheduled_starts = wave
            .as_ref()
            .map(|value| self.dispatch_submitted.max(value.scheduled_starts));
        let sender_start_lag = summarize_duration_histogram(&self.sender_start_lag_histogram);
        let http_send_duration = summarize_duration_histogram(&self.http_send_duration_histogram);
        let response_observation_duration =
            summarize_duration_histogram(&self.response_observation_duration_histogram);
        let curve_adherence = wave.as_ref().map(|value| {
            let scheduled_starts = self.dispatch_submitted.max(value.scheduled_starts);
            if scheduled_starts == 0 {
                100.0
            } else {
                round2(
                    ((scheduled_starts.saturating_sub(value.missed_starts)) as f64
                        / scheduled_starts as f64)
                        * 100.0,
                )
            }
        });

        LoadTestMetrics {
            snapshot_mode: Some(match scope {
                MetricsSnapshotScope::Full => LoadMetricsSnapshotMode::Final,
                MetricsSnapshotScope::LiveWindow { .. } => LoadMetricsSnapshotMode::Live,
            }),
            total_started: self.total_started,
            total_sent: self.total_sent,
            total_success: self.total_success,
            total_error: self.total_error,
            http_started: self.http_started,
            http_completed: self.http_completed,
            dispatch_submitted: (self.dispatch_submitted > 0).then_some(self.dispatch_submitted),
            dispatch_started: (self.dispatch_started > 0).then_some(self.dispatch_started),
            http_send_returned: (self.http_send_returned > 0).then_some(self.http_send_returned),
            response_body_completed: (self.response_body_completed > 0)
                .then_some(self.response_body_completed),
            dependency_limited_starts: (self.dependency_limited_starts > 0)
                .then_some(self.dependency_limited_starts),
            dispatcher_lagged_starts: (self.dispatcher_lagged_starts > 0)
                .then_some(self.dispatcher_lagged_starts),
            runtime_lagged_starts: (self.runtime_lagged_starts > 0)
                .then_some(self.runtime_lagged_starts),
            scheduler_lag_ms: (self.scheduler_lag_ms > 0).then_some(self.scheduler_lag_ms),
            scheduler_lagged_starts: (self.scheduler_lagged_starts > 0)
                .then_some(self.scheduler_lagged_starts),
            slot_enqueued: (self.slot_enqueued > 0).then_some(self.slot_enqueued),
            request_prepared: (self.request_prepared > 0).then_some(self.request_prepared),
            request_enqueued: (self.request_enqueued > 0).then_some(self.request_enqueued),
            send_task_spawned: (self.send_task_spawned > 0).then_some(self.send_task_spawned),
            send_started: (self.send_started > 0).then_some(self.send_started),
            sender_lagged_starts: (self.sender_lagged_starts > 0)
                .then_some(self.sender_lagged_starts),
            sender_queue_depth: (self.sender_queue_depth > 0).then_some(self.sender_queue_depth),
            sender_start_lag_avg_ms: sender_start_lag.as_ref().map(|summary| summary.avg_ms),
            sender_start_lag_p95_ms: sender_start_lag.as_ref().map(|summary| summary.p95_ms),
            sender_start_lag_p99_ms: sender_start_lag.as_ref().map(|summary| summary.p99_ms),
            sender_start_lag_max_ms: sender_start_lag.as_ref().map(|summary| summary.max_ms),
            http_send_duration_avg_ms: http_send_duration.as_ref().map(|summary| summary.avg_ms),
            http_send_duration_p95_ms: http_send_duration.as_ref().map(|summary| summary.p95_ms),
            http_send_duration_p99_ms: http_send_duration.as_ref().map(|summary| summary.p99_ms),
            response_observation_duration_avg_ms: response_observation_duration
                .as_ref()
                .map(|summary| summary.avg_ms),
            response_observation_duration_p95_ms: response_observation_duration
                .as_ref()
                .map(|summary| summary.p95_ms),
            response_observation_duration_p99_ms: response_observation_duration
                .as_ref()
                .map(|summary| summary.p99_ms),
            rps,
            start_time: self.start_time,
            elapsed_ms,
            target_intensity: wave.as_ref().map(|value| round2(value.target_intensity)),
            target_rps_limit: wave.as_ref().map(|value| round2(value.target_rps_limit)),
            in_flight: wave.as_ref().map(|value| value.in_flight),
            runner_max_rps: wave.as_ref().map(|value| round2(value.runner_max_rps)),
            tick_ms: wave.as_ref().map(|value| value.tick_ms),
            scheduled_starts,
            missed_starts: wave.as_ref().map(|value| value.missed_starts),
            ready_requests: wave.as_ref().map(|value| value.ready_requests),
            active_pipelines: wave.as_ref().map(|value| value.active_pipelines),
            outstanding_requests: wave.as_ref().map(|value| value.outstanding_requests),
            curve_adherence,
            duration_ms,
            latency_buckets: match scope {
                MetricsSnapshotScope::Full => self
                    .latency_histogram
                    .iter()
                    .map(|(duration_ms, count)| LoadLatencyBucket {
                        duration_ms: *duration_ms,
                        count: *count,
                    })
                    .collect(),
                MetricsSnapshotScope::LiveWindow { .. } => Vec::new(),
            },
            dispatch_buckets: filtered_dispatch_buckets(&self.dispatch_buckets, scope),
            lifecycle_buckets: filtered_lifecycle_buckets(&self.lifecycle_buckets, scope),
            latency_sample_count: (self.latency_sample_count > 0)
                .then_some(self.latency_sample_count),
            latency_total_duration_ms: (self.latency_sample_count > 0)
                .then_some(self.latency_total_duration_ms),
            error_samples: self.error_samples.clone(),
            runtime,
        }
    }

    fn lifecycle_bucket_mut(&mut self, elapsed_ms: u64) -> &mut LoadLifecycleBucket {
        let bucket_ms = lifecycle_bucket_ms(elapsed_ms);
        self.lifecycle_buckets
            .entry(bucket_ms)
            .or_insert_with(|| LoadLifecycleBucket {
                elapsed_ms: bucket_ms,
                ..LoadLifecycleBucket::default()
            })
    }
}

fn lifecycle_bucket_ms(elapsed_ms: u64) -> u64 {
    (elapsed_ms / 1000).saturating_mul(1000)
}

struct DurationHistogramSummary {
    avg_ms: f64,
    p95_ms: u64,
    p99_ms: u64,
    max_ms: u64,
}

fn summarize_duration_histogram(
    histogram: &BTreeMap<u64, usize>,
) -> Option<DurationHistogramSummary> {
    let sample_count = histogram.values().copied().sum::<usize>();
    if sample_count == 0 {
        return None;
    }

    let total_ms = histogram
        .iter()
        .fold(0_u128, |total, (duration_ms, count)| {
            total.saturating_add((*duration_ms as u128).saturating_mul(*count as u128))
        });
    let max_ms = histogram.keys().next_back().copied().unwrap_or(0);

    Some(DurationHistogramSummary {
        avg_ms: round2(total_ms as f64 / sample_count as f64),
        p95_ms: histogram_percentile(histogram, sample_count, 0.95),
        p99_ms: histogram_percentile(histogram, sample_count, 0.99),
        max_ms,
    })
}

fn histogram_percentile(
    histogram: &BTreeMap<u64, usize>,
    sample_count: usize,
    percentile: f64,
) -> u64 {
    if sample_count == 0 {
        return 0;
    }

    let rank = ((sample_count as f64) * percentile).ceil().max(1.0) as usize;
    let mut seen = 0usize;
    for (duration_ms, count) in histogram {
        seen = seen.saturating_add(*count);
        if seen >= rank {
            return *duration_ms;
        }
    }

    histogram.keys().next_back().copied().unwrap_or(0)
}

fn bucket_in_live_window(
    bucket_elapsed_ms: u64,
    from_elapsed_ms: u64,
    through_elapsed_ms: u64,
) -> bool {
    bucket_elapsed_ms >= from_elapsed_ms && bucket_elapsed_ms <= through_elapsed_ms
}

fn filtered_dispatch_buckets(
    buckets: &BTreeMap<u64, usize>,
    scope: MetricsSnapshotScope,
) -> Vec<LoadDispatchBucket> {
    buckets
        .iter()
        .filter(|(elapsed_ms, _)| match scope {
            MetricsSnapshotScope::Full => true,
            MetricsSnapshotScope::LiveWindow {
                from_elapsed_ms,
                through_elapsed_ms,
            } => bucket_in_live_window(**elapsed_ms, from_elapsed_ms, through_elapsed_ms),
        })
        .map(|(elapsed_ms, count)| LoadDispatchBucket {
            elapsed_ms: *elapsed_ms,
            count: *count,
        })
        .collect()
}

fn filtered_lifecycle_buckets(
    buckets: &BTreeMap<u64, LoadLifecycleBucket>,
    scope: MetricsSnapshotScope,
) -> Vec<LoadLifecycleBucket> {
    buckets
        .iter()
        .filter(|(elapsed_ms, _)| match scope {
            MetricsSnapshotScope::Full => true,
            MetricsSnapshotScope::LiveWindow {
                from_elapsed_ms,
                through_elapsed_ms,
            } => bucket_in_live_window(**elapsed_ms, from_elapsed_ms, through_elapsed_ms),
        })
        .map(|(_, bucket)| bucket.clone())
        .collect()
}

pub fn estimate_results_network_bytes(results: &[StepExecutionResult]) -> (u64, u64) {
    results.iter().fold((0_u64, 0_u64), |(tx, rx), result| {
        (
            tx.saturating_add(
                result
                    .request
                    .as_ref()
                    .map(estimate_request_bytes)
                    .unwrap_or(0),
            ),
            rx.saturating_add(
                result
                    .response
                    .as_ref()
                    .map(estimate_response_bytes)
                    .unwrap_or(0),
            ),
        )
    })
}

fn estimate_request_bytes(request: &StepRequest) -> u64 {
    let mut bytes = request.method.len() + 1 + request.url.len() + "\r\n".len();
    for (key, value) in &request.headers {
        bytes += key.len() + ": ".len() + value.len() + "\r\n".len();
    }
    bytes += "\r\n".len();
    if let Some(body) = request.body.as_ref() {
        bytes += serde_json::to_vec(body).map(|body| body.len()).unwrap_or(0);
    }
    bytes as u64
}

fn estimate_response_bytes(response: &StepResponse) -> u64 {
    let mut bytes = "HTTP/1.1 ".len()
        + response.status.to_string().len()
        + 1
        + response.status_text.len()
        + "\r\n".len();
    for (key, value) in &response.headers {
        bytes += key.len() + ": ".len() + value.len() + "\r\n".len();
    }
    bytes += "\r\n".len();
    bytes += match &response.body {
        serde_json::Value::String(body) => body.len(),
        body => serde_json::to_vec(body).map(|body| body.len()).unwrap_or(0),
    };
    bytes as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_has_core_metrics_without_percentiles_or_history() {
        let mut metrics = MetricsAccumulator::new();
        metrics.update(100.0, true);
        metrics.update(200.0, true);

        let snapshot = metrics.snapshot(None, None);

        assert_eq!(snapshot.total_sent, 2);
        assert_eq!(snapshot.total_success, 2);
        assert_eq!(snapshot.total_error, 0);
        assert_eq!(snapshot.duration_ms, None);
    }

    #[test]
    fn snapshot_includes_cumulative_latency_histogram() {
        let mut metrics = MetricsAccumulator::new();

        metrics.update(20.0, true);
        metrics.update(30.4, false);
        metrics.update(30.6, false);

        let snapshot = metrics.snapshot(None, None);

        assert_eq!(snapshot.total_sent, 3);
        assert_eq!(snapshot.total_success, 1);
        assert_eq!(snapshot.total_error, 2);
        assert_eq!(snapshot.latency_sample_count, Some(3));
        assert_eq!(snapshot.latency_total_duration_ms, Some(81));
        assert_eq!(snapshot.latency_buckets.len(), 3);
        assert_eq!(snapshot.latency_buckets[0].duration_ms, 20);
        assert_eq!(snapshot.latency_buckets[0].count, 1);
        assert_eq!(snapshot.latency_buckets[1].duration_ms, 30);
        assert_eq!(snapshot.latency_buckets[1].count, 1);
        assert_eq!(snapshot.latency_buckets[2].duration_ms, 31);
        assert_eq!(snapshot.latency_buckets[2].count, 1);
    }

    #[test]
    fn snapshot_includes_dispatch_buckets() {
        let mut metrics = MetricsAccumulator::new();

        metrics.record_dispatch_started_at(0);
        metrics.record_dispatch_started_at(0);

        let snapshot = metrics.snapshot(None, None);

        assert_eq!(snapshot.dispatch_started, Some(2));
        assert_eq!(
            snapshot
                .dispatch_buckets
                .iter()
                .map(|bucket| bucket.count)
                .sum::<usize>(),
            2
        );
        assert!(
            snapshot
                .dispatch_buckets
                .iter()
                .all(|bucket| bucket.elapsed_ms % 1000 == 0)
        );
    }

    #[test]
    fn dispatch_bucket_uses_elapsed_time_from_event() {
        let mut metrics = MetricsAccumulator::new();

        metrics.record_dispatch_started_at(74_999);
        metrics.record_dispatch_started_at(75_000);
        metrics.record_dispatch_started_at(75_999);

        let snapshot = metrics.snapshot(None, None);

        assert_eq!(snapshot.dispatch_buckets.len(), 2);
        assert_eq!(snapshot.dispatch_buckets[0].elapsed_ms, 74_000);
        assert_eq!(snapshot.dispatch_buckets[0].count, 1);
        assert_eq!(snapshot.dispatch_buckets[1].elapsed_ms, 75_000);
        assert_eq!(snapshot.dispatch_buckets[1].count, 2);
    }

    #[test]
    fn snapshot_includes_lifecycle_buckets() {
        let mut metrics = MetricsAccumulator::new();

        metrics.record_planned_at(1_050, 10);
        metrics.record_slot_enqueued_at(1_050, 10);
        metrics.record_request_prepared_at(1_110);
        metrics.record_request_enqueued_at(1_120);
        metrics.record_send_task_spawned_at(1_130);
        metrics.record_send_started_at(1_140);
        metrics.record_http_start_at(1_150);
        metrics.record_http_send_returned_at(1_160);
        metrics.record_response_body_completed_at(1_170, 1);
        metrics.record_dispatcher_lagged_starts_at(1_180, 2);
        metrics.record_runtime_lagged_start_at(1_190);

        let snapshot = metrics.snapshot(None, None);

        assert_eq!(snapshot.lifecycle_buckets.len(), 1);
        let bucket = &snapshot.lifecycle_buckets[0];
        assert_eq!(bucket.elapsed_ms, 1_000);
        assert_eq!(bucket.planned, 10);
        assert_eq!(bucket.slot_enqueued, 10);
        assert_eq!(bucket.request_prepared, 1);
        assert_eq!(bucket.request_enqueued, 1);
        assert_eq!(bucket.send_task_spawned, 1);
        assert_eq!(bucket.send_started, 1);
        assert_eq!(bucket.http_started, 1);
        assert_eq!(bucket.http_send_returned, 1);
        assert_eq!(bucket.response_body_completed, 1);
        assert_eq!(bucket.dispatcher_lagged, 2);
        assert_eq!(bucket.runtime_lagged, 1);
    }

    #[test]
    fn snapshot_includes_sender_lagged_starts_and_bucket() {
        let mut metrics = MetricsAccumulator::new();

        metrics.record_planned_at(10_000, 5);
        metrics.record_sender_lagged_starts_at(10_000, 2);
        metrics.record_sender_queue_depth(17);

        let snapshot = metrics.snapshot_with_wave(None, None, None);

        assert_eq!(snapshot.sender_lagged_starts, Some(2));
        assert_eq!(snapshot.sender_queue_depth, Some(17));
        assert_eq!(snapshot.lifecycle_buckets.len(), 1);
        assert_eq!(snapshot.lifecycle_buckets[0].elapsed_ms, 10_000);
        assert_eq!(snapshot.lifecycle_buckets[0].sender_lagged, 2);
    }

    #[test]
    fn snapshot_includes_wave_lag_summaries_and_bucket_maxima() {
        let mut metrics = MetricsAccumulator::new();

        metrics.record_sender_start_lag_at(1_050, 10);
        metrics.record_sender_start_lag_at(1_080, 30);
        metrics.record_http_send_duration_at(1_090, 40);
        metrics.record_http_send_duration_at(1_100, 80);
        metrics.record_response_observation_duration_at(1_110, 100);
        metrics.record_response_observation_duration_at(1_120, 120);

        let snapshot = metrics.snapshot(None, None);

        assert_eq!(snapshot.sender_start_lag_avg_ms, Some(20.0));
        assert_eq!(snapshot.sender_start_lag_p95_ms, Some(30));
        assert_eq!(snapshot.sender_start_lag_p99_ms, Some(30));
        assert_eq!(snapshot.sender_start_lag_max_ms, Some(30));
        assert_eq!(snapshot.http_send_duration_avg_ms, Some(60.0));
        assert_eq!(snapshot.http_send_duration_p95_ms, Some(80));
        assert_eq!(snapshot.http_send_duration_p99_ms, Some(80));
        assert_eq!(snapshot.response_observation_duration_avg_ms, Some(110.0));
        assert_eq!(snapshot.response_observation_duration_p95_ms, Some(120));
        assert_eq!(snapshot.response_observation_duration_p99_ms, Some(120));
        assert_eq!(snapshot.lifecycle_buckets.len(), 1);
        assert_eq!(snapshot.lifecycle_buckets[0].sender_start_lag_ms_max, 30);
        assert_eq!(snapshot.lifecycle_buckets[0].http_send_duration_ms_max, 80);
        assert_eq!(
            snapshot.lifecycle_buckets[0].response_observation_duration_ms_max,
            120
        );
    }

    #[test]
    fn live_snapshot_includes_only_requested_lifecycle_window() {
        let mut metrics = MetricsAccumulator::new();

        metrics.record_planned_at(0, 10);
        metrics.record_http_start_at(0);
        metrics.record_planned_at(1_000, 20);
        metrics.record_http_start_at(1_000);
        metrics.record_planned_at(2_000, 30);
        metrics.record_http_start_at(2_000);

        let snapshot = metrics.snapshot_with_wave_scope(
            None,
            None,
            None,
            MetricsSnapshotScope::LiveWindow {
                from_elapsed_ms: 1_000,
                through_elapsed_ms: 1_000,
            },
        );

        assert_eq!(snapshot.snapshot_mode, Some(LoadMetricsSnapshotMode::Live));
        assert_eq!(snapshot.lifecycle_buckets.len(), 1);
        assert_eq!(snapshot.lifecycle_buckets[0].elapsed_ms, 1_000);
        assert_eq!(snapshot.lifecycle_buckets[0].planned, 20);
        assert_eq!(snapshot.lifecycle_buckets[0].http_started, 1);
    }

    #[test]
    fn snapshot_includes_deduped_error_samples() {
        let mut metrics = MetricsAccumulator::new();

        metrics.record_error_sample("create_user", Some(409), "HTTP 409 Conflict");
        metrics.record_error_sample("create_user", Some(409), "HTTP 409 Conflict");
        metrics.record_error_sample("get_created_user", Some(404), "HTTP 404 Not Found");

        let snapshot = metrics.snapshot(None, None);

        assert_eq!(snapshot.error_samples.len(), 2);
        assert_eq!(snapshot.error_samples[0].step_id, "create_user");
        assert_eq!(snapshot.error_samples[0].http_status, Some(409));
        assert_eq!(snapshot.error_samples[0].count, 2);
        assert_eq!(snapshot.error_samples[1].step_id, "get_created_user");
        assert_eq!(snapshot.error_samples[1].count, 1);
    }

    #[test]
    fn snapshot_includes_raw_duration_when_provided() {
        let mut metrics = MetricsAccumulator::new();
        metrics.update(150.0, true);

        let snapshot = metrics.snapshot(Some(150), None);

        assert_eq!(snapshot.total_sent, 1);
        assert_eq!(snapshot.duration_ms, Some(150));
    }

    #[test]
    fn snapshot_includes_dispatch_adherence() {
        let metrics = MetricsAccumulator::new();
        let snapshot = metrics.snapshot_with_wave(
            None,
            None,
            Some(WaveMetricsSnapshot {
                target_intensity: 80.0,
                target_rps_limit: 800.0,
                in_flight: 50,
                runner_max_rps: 1000.0,
                tick_ms: 100,
                scheduled_starts: 80,
                missed_starts: 4,
                ready_requests: 20,
                active_pipelines: 200,
                outstanding_requests: 150,
            }),
        );

        assert_eq!(snapshot.scheduled_starts, Some(80));
        assert_eq!(snapshot.missed_starts, Some(4));
        assert_eq!(snapshot.curve_adherence, Some(95.0));
    }

    #[test]
    fn snapshot_keeps_cumulative_scheduled_starts_after_wave_phase_ends() {
        let mut metrics = MetricsAccumulator::new();
        metrics.record_dispatch_submitted_count(1_000);

        let snapshot = metrics.snapshot_with_wave(
            None,
            None,
            Some(WaveMetricsSnapshot {
                target_intensity: 0.0,
                target_rps_limit: 0.0,
                in_flight: 0,
                runner_max_rps: 1000.0,
                tick_ms: 100,
                scheduled_starts: 0,
                missed_starts: 0,
                ready_requests: 0,
                active_pipelines: 0,
                outstanding_requests: 0,
            }),
        );

        assert_eq!(snapshot.scheduled_starts, Some(1_000));
        assert_eq!(snapshot.curve_adherence, Some(100.0));
    }

    #[test]
    fn snapshot_includes_scheduler_lag_metrics() {
        let mut metrics = MetricsAccumulator::new();

        metrics.record_scheduler_lag_ms(400);
        metrics.record_scheduler_lagged_starts_count(12);

        let snapshot = metrics.snapshot(None, None);

        assert_eq!(snapshot.scheduler_lag_ms, Some(400));
        assert_eq!(snapshot.scheduler_lagged_starts, Some(12));
    }

    #[test]
    fn snapshot_includes_dispatcher_lagged_starts() {
        let mut metrics = MetricsAccumulator::new();

        metrics.record_dispatcher_lagged_starts_count(12);
        metrics.record_dispatcher_lagged_starts_count(5);

        let snapshot = metrics.snapshot(None, None);

        assert_eq!(snapshot.dispatcher_lagged_starts, Some(17));
    }

    #[test]
    fn snapshot_includes_dispatch_started_counter() {
        let mut metrics = MetricsAccumulator::new();

        metrics.record_dispatch_started_at(0);
        metrics.record_dispatch_started_at(0);

        let snapshot = metrics.snapshot(None, None);

        assert_eq!(snapshot.dispatch_started, Some(2));
    }

    #[test]
    fn snapshot_includes_started_count_before_completion() {
        let mut metrics = MetricsAccumulator::new();
        metrics.record_start();
        metrics.record_start();

        let snapshot = metrics.snapshot(None, None);

        assert_eq!(snapshot.total_started, 2);
        assert_eq!(snapshot.total_sent, 0);
    }

    #[test]
    fn snapshot_includes_http_lifecycle_counters() {
        let mut metrics = MetricsAccumulator::new();

        metrics.record_dispatch_submitted_count(3);
        metrics.record_http_start();
        metrics.record_http_send_returned();
        metrics.record_http_completed_count(1);
        metrics.record_response_body_completed_count(1);

        let snapshot = metrics.snapshot(None, None);

        assert_eq!(snapshot.dispatch_submitted, Some(3));
        assert_eq!(snapshot.http_started, 1);
        assert_eq!(snapshot.http_send_returned, Some(1));
        assert_eq!(snapshot.http_completed, 1);
        assert_eq!(snapshot.response_body_completed, Some(1));
    }

    #[test]
    fn snapshot_includes_wave_lifecycle_boundary_counters() {
        let mut metrics = MetricsAccumulator::new();

        metrics.record_slot_enqueued_count(3);
        metrics.record_request_prepared();
        metrics.record_request_prepared();
        metrics.record_request_enqueued();
        metrics.record_send_task_spawned();
        metrics.record_send_started();

        let snapshot = metrics.snapshot(None, None);

        assert_eq!(snapshot.slot_enqueued, Some(3));
        assert_eq!(snapshot.request_prepared, Some(2));
        assert_eq!(snapshot.request_enqueued, Some(1));
        assert_eq!(snapshot.send_task_spawned, Some(1));
        assert_eq!(snapshot.send_started, Some(1));
    }

    #[test]
    fn snapshot_includes_runner_runtime_when_provided() {
        let mut metrics = MetricsAccumulator::new();
        metrics.update(150.0, true);
        metrics.add_network_bytes(1_024, 4_096);

        let snapshot = metrics.snapshot(
            Some(150),
            Some(RunnerInfoResponse {
                pid: 42,
                memory_bytes: 1024,
                virtual_memory_bytes: 4096,
                cpu_usage_percent: 12.5,
                network_tx_bytes: 0,
                network_rx_bytes: 0,
                network_total_bytes: 0,
            }),
        );

        let runtime = snapshot.runtime.expect("runtime snapshot");
        assert_eq!(runtime.pid, 42);
        assert_eq!(runtime.memory_bytes, 1024);
        assert_eq!(runtime.virtual_memory_bytes, 4096);
        assert_eq!(runtime.cpu_usage_percent, 12.5);
        assert_eq!(runtime.network_tx_bytes, 1_024);
        assert_eq!(runtime.network_rx_bytes, 4_096);
        assert_eq!(runtime.network_total_bytes, 5_120);
    }
}
