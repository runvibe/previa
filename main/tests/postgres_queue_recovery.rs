use std::time::Duration;

use previa_main::server::queue::projector::QueueProjector;
mod common;

use common::migrated_queue_repository;
use previa_main::server::queue::repository::{EnqueueExecution, EnqueueJob};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

#[tokio::test]
async fn projector_resumes_from_checkpoint_and_writes_history_once() {
    let Some(database_url) = std::env::var("PREVIA_TEST_POSTGRES_URL").ok() else {
        eprintln!("skipping: PREVIA_TEST_POSTGRES_URL is not configured");
        return;
    };
    let queue = migrated_queue_repository(&database_url, 6).await;
    let project_id = format!("queue-recovery-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO projects (
            id, name, created_at, updated_at, created_at_ms, updated_at_ms
         ) VALUES ($1, 'queue recovery', 'now', 'now', 0, 0)",
    )
    .bind(&project_id)
    .execute(queue.pool())
    .await
    .unwrap();
    let execution_id = Uuid::now_v7();
    let job_id = Uuid::now_v7();
    queue
        .enqueue_execution(&EnqueueExecution {
            id: execution_id,
            project_id: project_id.clone(),
            pipeline_id: None,
            kind: "e2e".to_owned(),
            request_json: json!({"pipeline": {"name": "Recovery", "steps": [{}]}}),
            created_by: "test".to_owned(),
            transaction_id: None,
            max_attempts: 3,
            jobs: vec![EnqueueJob {
                id: job_id,
                shard_index: None,
                pool: "recovery".to_owned(),
                requirements_json: json!({}),
                payload_json: json!({"pipeline": {"name": "Recovery", "steps": [{}]}}),
                priority: 0,
            }],
        })
        .await
        .unwrap();

    let runner = sqlx::query(
        "SELECT * FROM queue_register_runner(
            'recovery-runner', 'recovery', 1, 'test', ARRAY['e2e']::TEXT[],
            '{}'::jsonb, '{}'::jsonb, 1, 0, 5000
        )",
    )
    .fetch_one(queue.pool())
    .await
    .unwrap();
    let runner_id: Uuid = runner.get("runner_id");
    let token: String = runner.get("runner_session_token");
    let claim = sqlx::query("SELECT * FROM queue_claim_job($1, $2, 30000)")
        .bind(runner_id)
        .bind(&token)
        .fetch_one(queue.pool())
        .await
        .unwrap();
    let attempt: i32 = claim.get("attempt");
    let lease_epoch: i64 = claim.get("lease_epoch");
    let lease_token: Uuid = claim.get("lease_token");
    sqlx::query_scalar::<_, i64>("SELECT queue_publish_events($1, $2, $3, $4, $5, $6, $7)")
        .bind(runner_id)
        .bind(&token)
        .bind(job_id)
        .bind(attempt)
        .bind(lease_epoch)
        .bind(lease_token)
        .bind(json!([{
            "seq": 1,
            "event_type": "step:result",
            "elapsed_ms": 5,
            "payload_json": {"status": "success"}
        }]))
        .fetch_one(queue.pool())
        .await
        .unwrap();
    sqlx::query_scalar::<_, bool>("SELECT queue_complete_job($1, $2, $3, $4, $5, $6, $7)")
        .bind(runner_id)
        .bind(&token)
        .bind(job_id)
        .bind(attempt)
        .bind(lease_epoch)
        .bind(lease_token)
        .bind(json!({
            "results": [{"status": "success"}],
            "summary": {"totalSteps": 1, "passed": 1, "failed": 0}
        }))
        .fetch_one(queue.pool())
        .await
        .unwrap();

    let projector = QueueProjector::new(queue.clone(), Uuid::now_v7());
    assert!(
        projector
            .project_once(Duration::from_secs(30))
            .await
            .unwrap()
    );
    let snapshot = sqlx::query(
        "SELECT status, last_event_id FROM execution_snapshots WHERE execution_id = $1",
    )
    .bind(execution_id)
    .fetch_one(queue.pool())
    .await
    .unwrap();
    assert_eq!(snapshot.get::<String, _>("status"), "completed");
    assert!(snapshot.get::<i64, _>("last_event_id") > 0);
    let history_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM integration_history WHERE execution_id = $1")
            .bind(execution_id.to_string())
            .fetch_one(queue.pool())
            .await
            .unwrap();
    assert_eq!(history_count, 1);

    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project_id)
        .execute(queue.pool())
        .await
        .unwrap();
}
