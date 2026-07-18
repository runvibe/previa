use previa_main::server::execution::split_even;
mod common;

use common::migrated_queue_repository;
use previa_main::server::queue::repository::{EnqueueExecution, EnqueueJob};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn load_enqueue_preserves_exact_split_across_shards() {
    assert_eq!(split_even(10, 3), vec![4, 3, 3]);

    let Some(database_url) = std::env::var("PREVIA_TEST_POSTGRES_URL").ok() else {
        eprintln!("skipping: PREVIA_TEST_POSTGRES_URL is not configured");
        return;
    };
    let queue = migrated_queue_repository(&database_url, 4).await;
    let project_id = format!("queue-load-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO projects (
            id, name, created_at, updated_at, created_at_ms, updated_at_ms
         ) VALUES ($1, 'queue load', 'now', 'now', 0, 0)",
    )
    .bind(&project_id)
    .execute(queue.pool())
    .await
    .unwrap();
    let execution_id = Uuid::now_v7();
    let split = split_even(10, 3);
    queue
        .enqueue_execution(&EnqueueExecution {
            id: execution_id,
            project_id: project_id.clone(),
            pipeline_id: None,
            kind: "load".to_owned(),
            request_json: json!({"targetRps": 10, "shardCount": 3}),
            created_by: "test".to_owned(),
            transaction_id: None,
            max_attempts: 3,
            jobs: split
                .iter()
                .enumerate()
                .map(|(index, assigned_rps)| EnqueueJob {
                    id: Uuid::now_v7(),
                    shard_index: Some(index as i32),
                    pool: "load".to_owned(),
                    requirements_json: json!({}),
                    payload_json: json!({
                        "shardIndex": index,
                        "shardCount": 3,
                        "assignedRps": assigned_rps
                    }),
                    priority: 0,
                })
                .collect(),
        })
        .await
        .unwrap();

    let assigned: Vec<i64> = sqlx::query_scalar(
        "SELECT (payload_json->>'assignedRps')::BIGINT
         FROM execution_jobs WHERE execution_id = $1 ORDER BY shard_index",
    )
    .bind(execution_id)
    .fetch_all(queue.pool())
    .await
    .unwrap();
    assert_eq!(assigned, vec![4, 3, 3]);

    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project_id)
        .execute(queue.pool())
        .await
        .unwrap();
}
