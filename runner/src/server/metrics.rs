use crate::server::models::{LoadTestMetrics, RunnerInfoResponse};
use crate::server::utils::{now_ms, round2};
use previa_runner::{StepExecutionResult, StepRequest, StepResponse};

#[derive(Debug)]
pub struct MetricsAccumulator {
    total_sent: usize,
    total_success: usize,
    total_error: usize,
    start_time: u64,
    network_tx_bytes: u64,
    network_rx_bytes: u64,
}

impl MetricsAccumulator {
    pub fn new() -> Self {
        Self {
            total_sent: 0,
            total_success: 0,
            total_error: 0,
            start_time: now_ms(),
            network_tx_bytes: 0,
            network_rx_bytes: 0,
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

    pub fn add_network_bytes(&mut self, tx_bytes: u64, rx_bytes: u64) {
        self.network_tx_bytes = self.network_tx_bytes.saturating_add(tx_bytes);
        self.network_rx_bytes = self.network_rx_bytes.saturating_add(rx_bytes);
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

        let runtime = runtime.map(|mut runtime| {
            runtime.network_tx_bytes = self.network_tx_bytes;
            runtime.network_rx_bytes = self.network_rx_bytes;
            runtime.network_total_bytes =
                self.network_tx_bytes.saturating_add(self.network_rx_bytes);
            runtime
        });

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
