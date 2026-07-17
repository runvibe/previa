use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use previa_runner::{Pipeline, RuntimeEnvGroup, RuntimeSpec};

use crate::server::handlers::load::run_classic_load;
use crate::server::load_wave::{sample_intensity, timeline_end_ms};
use crate::server::models::{LoadProfile, LoadTestConfig};
use crate::server::sse::SseMessage;
use crate::server::wave_executor::run_wave_load;

use super::e2e_executor::E2eQueueExecutor;
use super::repository::ClaimedJob;
use super::worker::{EventSink, JobExecutor, JobOutcome};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadShardJobPayload {
    pub execution_started_at_ms: u64,
    pub global_deadline_ms: u64,
    pub shard_index: usize,
    pub shard_count: usize,
    pub assigned_rps: f64,
    pub pipeline: Pipeline,
    pub selected_base_url_key: Option<String>,
    pub selected_env_group_slug: Option<String>,
    #[serde(default)]
    pub specs: Vec<RuntimeSpec>,
    #[serde(default)]
    pub env_groups: Vec<RuntimeEnvGroup>,
    pub config: Option<LoadTestConfig>,
    pub load: Option<LoadProfile>,
}

#[derive(Default)]
pub struct LoadQueueExecutor;

#[async_trait]
impl JobExecutor for LoadQueueExecutor {
    async fn execute(
        &self,
        job: ClaimedJob,
        events: EventSink,
        cancel: CancellationToken,
    ) -> JobOutcome {
        let mut payload: LoadShardJobPayload = match serde_json::from_value(job.payload_json) {
            Ok(payload) => payload,
            Err(error) => {
                return JobOutcome::Failed {
                    error: format!("invalid load shard payload: {error}"),
                    result: json!({"error": "invalid_payload"}),
                    retryable: false,
                };
            }
        };
        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
        if now_ms >= payload.global_deadline_ms {
            return JobOutcome::Completed(json!({
                "shardIndex": payload.shard_index,
                "skipped": true,
                "reason": "global_deadline_elapsed"
            }));
        }

        if let Some(profile) = payload.load.as_mut() {
            let elapsed_ms = now_ms.saturating_sub(payload.execution_started_at_ms);
            let current_intensity = sample_intensity(profile, elapsed_ms);
            let mut remaining = profile
                .points
                .iter()
                .filter(|point| point.at_ms > elapsed_ms)
                .cloned()
                .map(|mut point| {
                    point.at_ms -= elapsed_ms;
                    point
                })
                .collect::<Vec<_>>();
            remaining.insert(
                0,
                crate::server::models::LoadPoint {
                    at_ms: 0,
                    intensity: current_intensity,
                },
            );
            profile.points = remaining;
            profile.runner_max_rps = payload.assigned_rps;
        }

        let (tx, mut rx) = mpsc::unbounded_channel::<SseMessage>();
        let timeline_end = payload.load.as_ref().map(timeline_end_ms);
        let bridge_events = events.clone();
        let bridge = tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                if bridge_events
                    .push(message.event, 0, message.data)
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        if let Some(profile) = payload.load {
            run_wave_load(
                profile,
                payload.pipeline,
                payload.selected_base_url_key,
                payload.selected_env_group_slug,
                payload.specs,
                payload.env_groups,
                tx,
                cancel.clone(),
            )
            .await;
        } else if let Some(config) = payload.config {
            run_classic_load(
                config,
                payload.pipeline,
                payload.selected_base_url_key,
                payload.selected_env_group_slug,
                payload.specs,
                payload.env_groups,
                tx,
                cancel.clone(),
            )
            .await;
        } else {
            return JobOutcome::Failed {
                error: "load shard requires config or load profile".to_owned(),
                result: json!({"error": "missing_load_profile"}),
                retryable: false,
            };
        }
        let _ = bridge.await;
        if cancel.is_cancelled() {
            JobOutcome::Cancelled(json!({"shardIndex": payload.shard_index}))
        } else {
            JobOutcome::Completed(json!({
                "shardIndex": payload.shard_index,
                "shardCount": payload.shard_count,
                "timelineEndMs": timeline_end
            }))
        }
    }
}

#[derive(Default)]
pub struct QueueJobExecutor {
    e2e: E2eQueueExecutor,
    load: LoadQueueExecutor,
}

#[async_trait]
impl JobExecutor for QueueJobExecutor {
    async fn execute(
        &self,
        job: ClaimedJob,
        events: EventSink,
        cancel: CancellationToken,
    ) -> JobOutcome {
        match job.kind.as_str() {
            "e2e" => self.e2e.execute(job, events, cancel).await,
            "load" => self.load.execute(job, events, cancel).await,
            kind => JobOutcome::Failed {
                error: format!("unsupported job kind: {kind}"),
                result: json!({"error": "unsupported_job_kind"}),
                retryable: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::models::{LoadInterpolation, LoadPoint};

    #[test]
    fn retry_trimming_keeps_only_future_wave_points() {
        let profile = LoadProfile {
            points: vec![
                LoadPoint {
                    at_ms: 0,
                    intensity: 1.0,
                },
                LoadPoint {
                    at_ms: 1_000,
                    intensity: 2.0,
                },
                LoadPoint {
                    at_ms: 2_000,
                    intensity: 3.0,
                },
            ],
            interpolation: LoadInterpolation::Step,
            runner_max_rps: 10.0,
            grace_period_ms: 1_000,
        };
        assert_eq!(sample_intensity(&profile, 1_500), 2.0);
        assert_eq!(timeline_end_ms(&profile), 2_000);
    }
}
