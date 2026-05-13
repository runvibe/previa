use reqwest::Client;

use crate::server::models::{KubernetesReservationCreateRequest, KubernetesReservationStatus};

#[derive(Clone)]
#[allow(dead_code)]
pub struct KubernetesReservationClient {
    client: Client,
    base_url: String,
}

impl KubernetesReservationClient {
    #[allow(dead_code)]
    pub fn new(client: Client, base_url: impl Into<String>) -> Self {
        Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
        }
    }

    #[allow(dead_code)]
    pub async fn create(
        &self,
        request: &KubernetesReservationCreateRequest,
    ) -> Result<KubernetesReservationStatus, reqwest::Error> {
        self.client
            .post(format!("{}/internal/runner-reservations", self.base_url))
            .json(request)
            .send()
            .await?
            .error_for_status()?
            .json::<KubernetesReservationStatus>()
            .await
    }

    #[allow(dead_code)]
    pub async fn get(
        &self,
        reservation_id: &str,
    ) -> Result<KubernetesReservationStatus, reqwest::Error> {
        self.client
            .get(format!(
                "{}/internal/runner-reservations/{}",
                self.base_url, reservation_id
            ))
            .send()
            .await?
            .error_for_status()?
            .json::<KubernetesReservationStatus>()
            .await
    }

    #[allow(dead_code)]
    pub async fn cancel(&self, reservation_id: &str) -> Result<(), reqwest::Error> {
        self.client
            .post(format!(
                "{}/internal/runner-reservations/{}/cancel",
                self.base_url, reservation_id
            ))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::server::models::{KubernetesReservationCreateRequest, KubernetesReservationStatus};

    #[test]
    fn create_request_serializes_with_camel_case_fields() {
        let payload = KubernetesReservationCreateRequest {
            execution_id: "exec_123".to_owned(),
            pipeline_id: "pipe_123".to_owned(),
            count: 10,
        };

        let value = serde_json::to_value(payload).unwrap();

        assert_eq!(value["executionId"], "exec_123");
        assert_eq!(value["pipelineId"], "pipe_123");
        assert_eq!(value["count"], 10);
        assert!(value.get("execution_id").is_none());
        assert!(value.get("pipeline_id").is_none());
    }

    #[test]
    fn reservation_status_deserializes_ready_response() {
        let payload = serde_json::json!({
            "reservationId": "rr_123",
            "status": "ready",
            "requestedRunners": 2,
            "readyRunners": 2,
            "reservationToken": "secret",
            "expiresAt": "2026-05-12T18:40:00Z",
            "runners": [
                { "id": "runner-1", "endpoint": "http://10.0.4.12:55880" }
            ]
        });

        let status: KubernetesReservationStatus = serde_json::from_value(payload).unwrap();

        assert_eq!(status.reservation_id, "rr_123");
        assert_eq!(status.status, "ready");
        assert_eq!(status.requested_runners, 2);
        assert_eq!(status.ready_runners, 2);
        assert_eq!(status.reservation_token.as_deref(), Some("secret"));
        assert_eq!(status.runners.len(), 1);
    }
}
