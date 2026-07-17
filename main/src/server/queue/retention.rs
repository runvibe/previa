use std::time::Duration;

use super::repository::QueueRepository;

pub async fn delete_expired_queue_data(
    repository: &QueueRepository,
    event_retention: Duration,
    runner_retention: Duration,
) -> Result<u64, sqlx::Error> {
    let event_hours = i64::try_from(event_retention.as_secs() / 3_600)
        .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
    let runner_hours = i64::try_from(runner_retention.as_secs() / 3_600)
        .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
    let events = sqlx::query(
        "DELETE FROM execution_events event
         USING executions execution, execution_snapshots snapshot
         WHERE execution.id = event.execution_id
           AND snapshot.execution_id = execution.id
           AND execution.status IN ('completed', 'failed', 'cancelled')
           AND snapshot.last_event_id >= event.id
           AND execution.finished_at <
               CURRENT_TIMESTAMP - ($1 * INTERVAL '1 hour')",
    )
    .bind(event_hours)
    .execute(repository.pool())
    .await?
    .rows_affected();
    let runners = sqlx::query(
        "DELETE FROM runner_instances runner
         WHERE runner.status IN ('stale', 'stopped')
           AND runner.updated_at < CURRENT_TIMESTAMP - ($1 * INTERVAL '1 hour')
           AND NOT EXISTS (
               SELECT 1 FROM execution_jobs job WHERE job.runner_id = runner.id
           )",
    )
    .bind(runner_hours)
    .execute(repository.pool())
    .await?
    .rows_affected();
    Ok(events + runners)
}
