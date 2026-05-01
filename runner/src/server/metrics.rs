use crate::server::models::{LoadTestMetrics, RunnerInfoResponse};
use crate::server::utils::{now_ms, round2};

#[derive(Debug)]
pub struct MetricsAccumulator {
    total_sent: usize,
    total_success: usize,
    total_error: usize,
    start_time: u64,
}

impl MetricsAccumulator {
    pub fn new() -> Self {
        Self {
            total_sent: 0,
            total_success: 0,
            total_error: 0,
            start_time: now_ms(),
        }
    }

    pub fn update(&mut self, _duration: f64, success: bool) {
        self.total_sent += 1;
        if success {
            self.total_success += 1;
        } else {
            self.total_error += 1;
        }
    }

    pub fn snapshot(
        &self,
        duration_ms: Option<u64>,
        runtime: Option<RunnerInfoResponse>,
    ) -> LoadTestMetrics {
        let now = now_ms();
        let elapsed_ms = now.saturating_sub(self.start_time);

        let elapsed = (elapsed_ms as f64) / 1000.0;
        let rps = if elapsed > 0.0 {
            round2((self.total_sent as f64) / elapsed)
        } else {
            0.0
        };

        LoadTestMetrics {
            total_sent: self.total_sent,
            total_success: self.total_success,
            total_error: self.total_error,
            rps,
            start_time: self.start_time,
            elapsed_ms,
            duration_ms,
            runtime,
        }
    }
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
    fn snapshot_includes_raw_duration_when_provided() {
        let mut metrics = MetricsAccumulator::new();
        metrics.update(150.0, true);

        let snapshot = metrics.snapshot(Some(150), None);

        assert_eq!(snapshot.total_sent, 1);
        assert_eq!(snapshot.duration_ms, Some(150));
    }

    #[test]
    fn snapshot_includes_runner_runtime_when_provided() {
        let mut metrics = MetricsAccumulator::new();
        metrics.update(150.0, true);

        let snapshot = metrics.snapshot(
            Some(150),
            Some(RunnerInfoResponse {
                pid: 42,
                memory_bytes: 1024,
                virtual_memory_bytes: 4096,
                cpu_usage_percent: 12.5,
            }),
        );

        let runtime = snapshot.runtime.expect("runtime snapshot");
        assert_eq!(runtime.pid, 42);
        assert_eq!(runtime.memory_bytes, 1024);
        assert_eq!(runtime.virtual_memory_bytes, 4096);
        assert_eq!(runtime.cpu_usage_percent, 12.5);
    }
}
