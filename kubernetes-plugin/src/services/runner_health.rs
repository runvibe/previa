use async_trait::async_trait;
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunnerInfo {
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub started_execution_count: u64,
    #[serde(default)]
    pub last_started_at: Option<String>,
    #[serde(default)]
    pub last_finished_at: Option<String>,
}

#[derive(Debug, Error)]
pub enum RunnerHealthError {
    #[error("runner info request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[cfg(test)]
    #[error("runner unavailable: {0}")]
    Unavailable(String),
}

#[async_trait]
pub trait RunnerHealthApi: Send + Sync {
    async fn fetch_runner_info(&self, endpoint: &str) -> Result<RunnerInfo, RunnerHealthError>;
    async fn rearm_runner(
        &self,
        endpoint: &str,
        reservation_id: &str,
        reservation_token: &str,
        expires_at: Option<&str>,
    ) -> Result<(), RunnerHealthError>;
    async fn release_runner(&self, endpoint: &str) -> Result<(), RunnerHealthError>;
}

#[derive(Clone)]
pub struct ReqwestRunnerHealth {
    client: reqwest::Client,
}

impl ReqwestRunnerHealth {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl RunnerHealthApi for ReqwestRunnerHealth {
    async fn fetch_runner_info(&self, endpoint: &str) -> Result<RunnerInfo, RunnerHealthError> {
        Ok(self
            .client
            .get(format!("{}/info", endpoint.trim_end_matches('/')))
            .send()
            .await?
            .error_for_status()?
            .json::<RunnerInfo>()
            .await?)
    }

    async fn rearm_runner(
        &self,
        _endpoint: &str,
        _reservation_id: &str,
        _reservation_token: &str,
        _expires_at: Option<&str>,
    ) -> Result<(), RunnerHealthError> {
        // Runner eligibility is now represented by its Postgres registration
        // and StatefulSet lifecycle. No HTTP control call is necessary.
        Ok(())
    }

    async fn release_runner(&self, _endpoint: &str) -> Result<(), RunnerHealthError> {
        // Releasing the Kubernetes reservation removes/scales the runner
        // resources; the runner marks itself stopped on graceful shutdown.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::RunnerInfo;

    #[test]
    fn runner_info_defaults_missing_lifecycle_fields() {
        let info: RunnerInfo = serde_json::from_value(serde_json::json!({
            "pid": 1,
            "memoryBytes": 1
        }))
        .unwrap();

        assert!(!info.busy);
        assert_eq!(info.started_execution_count, 0);
    }
}
