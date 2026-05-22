use async_trait::async_trait;
use serde::{Deserialize, Serialize};
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
        endpoint: &str,
        reservation_id: &str,
        reservation_token: &str,
        expires_at: Option<&str>,
    ) -> Result<(), RunnerHealthError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct RearmRequest<'a> {
            reservation_id: &'a str,
            reservation_token: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            expires_at: Option<&'a str>,
        }

        self.client
            .post(format!(
                "{}/internal/reservation/rearm",
                endpoint.trim_end_matches('/')
            ))
            .json(&RearmRequest {
                reservation_id,
                reservation_token,
                expires_at,
            })
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    async fn release_runner(&self, endpoint: &str) -> Result<(), RunnerHealthError> {
        self.client
            .post(format!(
                "{}/internal/reservation/release",
                endpoint.trim_end_matches('/')
            ))
            .send()
            .await?
            .error_for_status()?;
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
