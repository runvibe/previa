use std::time::Duration;

mod common;

use common::migrated_queue_repository;
use previa_main::server::queue::repository::{EnqueueExecution, EnqueueJob};
use serde_json::json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug)]
struct RunnerSession {
    id: Uuid,
    token: String,
}

#[derive(Debug)]
struct Claim {
    job_id: Uuid,
    attempt: i32,
    lease_epoch: i64,
    lease_token: Uuid,
}

async fn register_runner(pool: &PgPool, name: &str) -> RunnerSession {
    let row = sqlx::query(
        "SELECT * FROM queue_register_runner(
            $1, 'default', 1, 'test', ARRAY['load']::TEXT[],
            '{}'::jsonb, '{}'::jsonb, 0, 1, 5000
         )",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .expect("register runner");
    RunnerSession {
        id: row.get("runner_id"),
        token: row.get("runner_session_token"),
    }
}

async fn claim(pool: &PgPool, runner: &RunnerSession) -> Claim {
    let row = sqlx::query("SELECT * FROM queue_claim_job($1, $2, 30000)")
        .bind(runner.id)
        .bind(&runner.token)
        .fetch_one(pool)
        .await
        .expect("claim queued job");
    Claim {
        job_id: row.get("job_id"),
        attempt: row.get("attempt"),
        lease_epoch: row.get("lease_epoch"),
        lease_token: row.get("lease_token"),
    }
}

#[tokio::test]
async fn connects_only_to_postgres_queue_storage() {
    let Some(database_url) = std::env::var("PREVIA_TEST_POSTGRES_URL").ok() else {
        eprintln!("skipping: PREVIA_TEST_POSTGRES_URL is not configured");
        return;
    };

    let repository = migrated_queue_repository(&database_url, 2).await;
    assert_eq!(
        repository
            .protocol_version()
            .await
            .expect("read protocol version"),
        1
    );
}

#[tokio::test]
async fn claims_are_concurrent_idempotent_and_fenced() {
    let Some(database_url) = std::env::var("PREVIA_TEST_POSTGRES_URL").ok() else {
        eprintln!("skipping: PREVIA_TEST_POSTGRES_URL is not configured");
        return;
    };
    let repository = migrated_queue_repository(&database_url, 8).await;
    let pool = repository.pool();

    let project_id = format!("queue-test-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO projects (
            id, name, created_at, updated_at, created_at_ms, updated_at_ms
         ) VALUES ($1, 'queue integration test', 'now', 'now', 0, 0)",
    )
    .bind(&project_id)
    .execute(pool)
    .await
    .expect("seed project");

    let execution_id = Uuid::now_v7();
    repository
        .enqueue_execution(&EnqueueExecution {
            id: execution_id,
            project_id: project_id.clone(),
            pipeline_id: None,
            kind: "load".to_owned(),
            request_json: json!({"test": true}),
            created_by: "integration-test".to_owned(),
            transaction_id: None,
            max_attempts: 3,
            jobs: (0..2)
                .map(|shard_index| EnqueueJob {
                    id: Uuid::now_v7(),
                    shard_index: Some(shard_index),
                    pool: "default".to_owned(),
                    requirements_json: json!({}),
                    payload_json: json!({"shardIndex": shard_index}),
                    priority: 0,
                })
                .collect(),
        })
        .await
        .expect("enqueue load execution");

    let left_runner = register_runner(pool, "runner-left").await;
    let right_runner = register_runner(pool, "runner-right").await;
    let (left, right) = tokio::join!(claim(pool, &left_runner), claim(pool, &right_runner));
    assert_ne!(left.job_id, right.job_id);

    let active_jobs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM execution_jobs
         WHERE execution_id = $1 AND status IN ('leased', 'running')",
    )
    .bind(execution_id)
    .fetch_one(pool)
    .await
    .expect("count active jobs");
    assert_eq!(active_jobs, 2);

    let event = json!([{
        "seq": 1,
        "event_type": "load_bucket",
        "elapsed_ms": 100,
        "payload_json": {"requests": 10}
    }]);
    let inserted: i64 =
        sqlx::query_scalar("SELECT queue_publish_events($1, $2, $3, $4, $5, $6, $7)")
            .bind(left_runner.id)
            .bind(&left_runner.token)
            .bind(left.job_id)
            .bind(left.attempt)
            .bind(left.lease_epoch)
            .bind(left.lease_token)
            .bind(&event)
            .fetch_one(pool)
            .await
            .expect("publish event");
    assert_eq!(inserted, 1);
    let duplicate: i64 =
        sqlx::query_scalar("SELECT queue_publish_events($1, $2, $3, $4, $5, $6, $7)")
            .bind(left_runner.id)
            .bind(&left_runner.token)
            .bind(left.job_id)
            .bind(left.attempt)
            .bind(left.lease_epoch)
            .bind(left.lease_token)
            .bind(&event)
            .fetch_one(pool)
            .await
            .expect("deduplicate event");
    assert_eq!(duplicate, 0);

    sqlx::query(
        "UPDATE execution_jobs
         SET lease_expires_at = CURRENT_TIMESTAMP - INTERVAL '1 second'
         WHERE id = $1",
    )
    .bind(left.job_id)
    .execute(pool)
    .await
    .expect("expire first lease");
    repository
        .reap_expired_jobs(Duration::ZERO, Duration::ZERO)
        .await
        .expect("requeue expired lease");
    let reclaimed = claim(pool, &left_runner).await;
    assert_eq!(reclaimed.job_id, left.job_id);
    assert!(reclaimed.lease_epoch > left.lease_epoch);

    let stale_publish =
        sqlx::query_scalar::<_, i64>("SELECT queue_publish_events($1, $2, $3, $4, $5, $6, $7)")
            .bind(left_runner.id)
            .bind(&left_runner.token)
            .bind(left.job_id)
            .bind(left.attempt)
            .bind(left.lease_epoch)
            .bind(left.lease_token)
            .bind(json!([]))
            .fetch_one(pool)
            .await;
    assert!(stale_publish.is_err());
    let stale_finish =
        sqlx::query_scalar::<_, bool>("SELECT queue_complete_job($1, $2, $3, $4, $5, $6, $7)")
            .bind(left_runner.id)
            .bind(&left_runner.token)
            .bind(left.job_id)
            .bind(left.attempt)
            .bind(left.lease_epoch)
            .bind(left.lease_token)
            .bind(json!({}))
            .fetch_one(pool)
            .await;
    assert!(stale_finish.is_err());
    let completed: bool =
        sqlx::query_scalar("SELECT queue_complete_job($1, $2, $3, $4, $5, $6, $7)")
            .bind(left_runner.id)
            .bind(&left_runner.token)
            .bind(reclaimed.job_id)
            .bind(reclaimed.attempt)
            .bind(reclaimed.lease_epoch)
            .bind(reclaimed.lease_token)
            .bind(json!({"ok": true}))
            .fetch_one(pool)
            .await
            .expect("complete with current fencing");
    assert!(completed);

    let runner_can_select_jobs: bool = sqlx::query_scalar(
        "SELECT has_table_privilege('previa_runner_queue', 'execution_jobs', 'SELECT')",
    )
    .fetch_one(pool)
    .await
    .expect("inspect runner role");
    assert!(!runner_can_select_jobs);

    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project_id)
        .execute(pool)
        .await
        .expect("clean project");
}
