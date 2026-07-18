use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use super::repository::{ProjectionLease, QueueRepository};

pub struct QueueProjector {
    repository: QueueRepository,
    owner: Uuid,
}

impl QueueProjector {
    pub fn new(repository: QueueRepository, owner: Uuid) -> Self {
        Self { repository, owner }
    }

    pub async fn project_once(
        &self,
        lease_duration: std::time::Duration,
    ) -> Result<bool, sqlx::Error> {
        let Some(lease) = self
            .repository
            .claim_projection(self.owner, lease_duration)
            .await?
        else {
            return Ok(false);
        };
        self.project_lease(lease).await?;
        Ok(true)
    }

    async fn project_lease(&self, lease: ProjectionLease) -> Result<(), sqlx::Error> {
        let events = self
            .repository
            .read_events_after(lease.execution_id, lease.last_event_id, 1_000)
            .await?;
        let job_rows = sqlx::query(
            "SELECT id, status, shard_index, attempt, result_json, last_error,
                    started_at, finished_at
             FROM execution_jobs
             WHERE execution_id = $1
             ORDER BY shard_index NULLS FIRST, created_at",
        )
        .bind(lease.execution_id)
        .fetch_all(self.repository.pool())
        .await?;
        let desired_status: String =
            sqlx::query_scalar("SELECT desired_status FROM executions WHERE id = $1")
                .bind(lease.execution_id)
                .fetch_one(self.repository.pool())
                .await?;
        let statuses = job_rows
            .iter()
            .map(|row| row.get::<String, _>("status"))
            .collect::<Vec<_>>();
        let status = derive_execution_status(&statuses, &desired_status);
        let last_event_id = events
            .last()
            .map(|event| event.id)
            .unwrap_or(lease.last_event_id);
        let previous_events = lease
            .snapshot_json
            .get("events")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut event_values = previous_events;
        event_values.extend(events.iter().map(|event| {
            json!({
                "id": event.id,
                "jobId": event.job_id,
                "attempt": event.attempt,
                "seq": event.seq,
                "event": event.event_type,
                "elapsedMs": event.elapsed_ms,
                "payload": event.payload_json
            })
        }));
        if event_values.len() > 1_000 {
            event_values.drain(..event_values.len() - 1_000);
        }
        let jobs = job_rows
            .iter()
            .map(|row| {
                json!({
                    "id": row.get::<Uuid, _>("id"),
                    "status": row.get::<String, _>("status"),
                    "shardIndex": row.get::<Option<i32>, _>("shard_index"),
                    "attempt": row.get::<i32, _>("attempt"),
                    "result": row.get::<Option<Value>, _>("result_json"),
                    "lastError": row.get::<Option<String>, _>("last_error")
                })
            })
            .collect::<Vec<_>>();
        let snapshot = json!({
            "executionId": lease.execution_id,
            "status": status,
            "events": event_values,
            "jobs": jobs
        });

        let terminal = matches!(status.as_str(), "completed" | "failed" | "cancelled");
        sqlx::query(
            "UPDATE executions
             SET status = $2,
                 started_at = CASE
                     WHEN $2 = 'running' THEN COALESCE(started_at, CURRENT_TIMESTAMP)
                     ELSE started_at
                 END,
                 finished_at = CASE
                     WHEN $3 THEN COALESCE(finished_at, CURRENT_TIMESTAMP)
                     ELSE NULL
                 END,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = $1
               AND status NOT IN ('completed', 'failed', 'cancelled')",
        )
        .bind(lease.execution_id)
        .bind(&status)
        .bind(terminal)
        .execute(self.repository.pool())
        .await?;
        self.repository
            .store_snapshot(&lease, &status, &snapshot, last_event_id)
            .await?;
        if terminal {
            persist_terminal_history(&self.repository, lease.execution_id, &status, &snapshot)
                .await?;
        }
        Ok(())
    }
}

pub fn derive_execution_status(statuses: &[String], desired_status: &str) -> String {
    if statuses.is_empty() {
        return "failed".to_owned();
    }
    let all_terminal = statuses.iter().all(|status| {
        matches!(
            status.as_str(),
            "completed" | "failed" | "cancelled" | "dead_letter"
        )
    });
    if all_terminal {
        if desired_status == "cancelled" || statuses.iter().all(|status| status == "cancelled") {
            "cancelled".to_owned()
        } else if statuses
            .iter()
            .any(|status| matches!(status.as_str(), "failed" | "dead_letter"))
        {
            "failed".to_owned()
        } else {
            "completed".to_owned()
        }
    } else if desired_status == "cancelled" {
        "cancel_requested".to_owned()
    } else if statuses
        .iter()
        .any(|status| matches!(status.as_str(), "leased" | "running"))
    {
        "running".to_owned()
    } else {
        "queued".to_owned()
    }
}

async fn persist_terminal_history(
    repository: &QueueRepository,
    execution_id: Uuid,
    status: &str,
    snapshot: &Value,
) -> Result<(), sqlx::Error> {
    let row = sqlx::query(
        "SELECT kind, project_id, pipeline_id, transaction_id, request_json,
                (EXTRACT(EPOCH FROM COALESCE(started_at, queued_at)) * 1000)::DOUBLE PRECISION AS started_ms,
                (EXTRACT(EPOCH FROM COALESCE(finished_at, CURRENT_TIMESTAMP)) * 1000)::DOUBLE PRECISION AS finished_ms
         FROM executions WHERE id = $1",
    )
    .bind(execution_id)
    .fetch_one(repository.pool())
    .await?;
    let kind: String = row.get("kind");
    let project_id: String = row.get("project_id");
    let pipeline_id: Option<String> = row.get("pipeline_id");
    let transaction_id: Option<String> = row.get("transaction_id");
    let request: Value = row.get("request_json");
    let started_ms = row.get::<f64, _>("started_ms").max(0.0) as i64;
    let finished_ms = row.get::<f64, _>("finished_ms").max(0.0) as i64;
    let duration_ms = finished_ms.saturating_sub(started_ms);
    let pipeline_name = request
        .pointer("/pipeline/name")
        .and_then(Value::as_str)
        .unwrap_or("Queued execution");

    if kind == "e2e" {
        let result = snapshot
            .get("jobs")
            .and_then(Value::as_array)
            .and_then(|jobs| jobs.first())
            .and_then(|job| job.get("result"))
            .cloned()
            .unwrap_or(Value::Null);
        let steps = result.get("results").cloned().unwrap_or_else(|| json!([]));
        let summary = result.get("summary").cloned();
        sqlx::query(
            "INSERT INTO integration_history (
                id, execution_id, transaction_id, project_id, pipeline_id,
                pipeline_name, status, started_at_ms, finished_at_ms, duration_ms,
                summary_json, steps_json, errors_json, request_json
             ) VALUES (
                $1, $1, $2, $3, $4, $5, $6, $7, $8, $9,
                $10, $11, '[]', $12
             ) ON CONFLICT (execution_id) DO NOTHING",
        )
        .bind(execution_id.to_string())
        .bind(transaction_id)
        .bind(project_id)
        .bind(pipeline_id)
        .bind(pipeline_name)
        .bind(status)
        .bind(started_ms)
        .bind(finished_ms)
        .bind(duration_ms)
        .bind(summary.map(|value| value.to_string()))
        .bind(steps.to_string())
        .bind(request.to_string())
        .execute(repository.pool())
        .await?;
    } else {
        let lines = snapshot.get("events").cloned().unwrap_or_else(|| json!([]));
        sqlx::query(
            "INSERT INTO load_history (
                id, execution_id, transaction_id, project_id, pipeline_id,
                pipeline_name, status, started_at_ms, finished_at_ms, duration_ms,
                requested_config_json, final_consolidated_json, final_lines_json,
                errors_json, request_json, context_json
             ) VALUES (
                $1, $1, $2, $3, $4, $5, $6, $7, $8, $9,
                $10, $11, $12, '[]', $10, '{}'
             ) ON CONFLICT (execution_id) DO NOTHING",
        )
        .bind(execution_id.to_string())
        .bind(transaction_id)
        .bind(project_id)
        .bind(pipeline_id)
        .bind(pipeline_name)
        .bind(status)
        .bind(started_ms)
        .bind(finished_ms)
        .bind(duration_ms)
        .bind(request.to_string())
        .bind(snapshot.to_string())
        .bind(lines.to_string())
        .execute(repository.pool())
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::derive_execution_status;

    #[test]
    fn derives_terminal_state_from_all_jobs() {
        assert_eq!(
            derive_execution_status(&["completed".to_owned(), "completed".to_owned()], "running"),
            "completed"
        );
        assert_eq!(
            derive_execution_status(
                &["completed".to_owned(), "dead_letter".to_owned()],
                "running"
            ),
            "failed"
        );
    }
}
