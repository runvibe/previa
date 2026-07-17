mod common;

use common::migrated_queue_repository;
use previa_main::server::queue::repository::{EnqueueExecution, EnqueueJob};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn e2e_enqueue_creates_one_job_without_a_registered_runner() {
    let Some(database_url) = std::env::var("PREVIA_TEST_POSTGRES_URL").ok() else {
        eprintln!("skipping: PREVIA_TEST_POSTGRES_URL is not configured");
        return;
    };
    let queue = migrated_queue_repository(&database_url, 4).await;
    let project_id = format!("queue-e2e-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO projects (
            id, name, created_at, updated_at, created_at_ms, updated_at_ms
         ) VALUES ($1, 'queue e2e', 'now', 'now', 0, 0)",
    )
    .bind(&project_id)
    .execute(queue.pool())
    .await
    .unwrap();
    let execution_id = Uuid::now_v7();
    queue
        .enqueue_execution(&EnqueueExecution {
            id: execution_id,
            project_id: project_id.clone(),
            pipeline_id: None,
            kind: "e2e".to_owned(),
            request_json: json!({"pipeline": {"steps": [{}]}}),
            created_by: "test".to_owned(),
            transaction_id: None,
            max_attempts: 3,
            jobs: vec![EnqueueJob {
                id: Uuid::now_v7(),
                shard_index: None,
                pool: "pool-without-runners".to_owned(),
                requirements_json: json!({}),
                payload_json: json!({"pipeline": {"steps": [{}]}}),
                priority: 0,
            }],
        })
        .await
        .unwrap();

    let job_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM execution_jobs WHERE execution_id = $1")
            .bind(execution_id)
            .fetch_one(queue.pool())
            .await
            .unwrap();
    let runner_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM runner_instances WHERE pool = 'pool-without-runners'",
    )
    .fetch_one(queue.pool())
    .await
    .unwrap();
    assert_eq!(job_count, 1);
    assert_eq!(runner_count, 0);

    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project_id)
        .execute(queue.pool())
        .await
        .unwrap();
}
