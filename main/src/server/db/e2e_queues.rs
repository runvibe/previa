use crate::server::db::DbPool;
use serde_json::{Value, json};
use sqlx::Row;

use crate::server::models::{E2eQueuePipelineRecord, E2eQueueRecord, E2eQueueStatus};
use crate::server::utils::new_uuid_v7;

fn parse_queue_status(raw: &str) -> E2eQueueStatus {
    match raw {
        "pending" => E2eQueueStatus::Pending,
        "running" => E2eQueueStatus::Running,
        "failed" => E2eQueueStatus::Failed,
        "completed" => E2eQueueStatus::Completed,
        "cancelled" => E2eQueueStatus::Cancelled,
        _ => E2eQueueStatus::Failed,
    }
}

pub async fn insert_e2e_queue(
    db: &DbPool,
    project_id: &str,
    queue_id: &str,
    selected_base_url_key: Option<&str>,
    request: &Value,
    pipeline_ids: &[String],
    created_at: &str,
) -> Result<E2eQueueRecord, sqlx::Error> {
    let mut tx = db.begin().await?;

    db.query(
        "INSERT INTO e2e_queues (
            id, project_id, status, selected_base_url_key, request_json, active_execution_id, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(queue_id)
    .bind(project_id)
    .bind(E2eQueueStatus::Pending.as_str())
    .bind(selected_base_url_key)
    .bind(serde_json::to_string(request).unwrap_or_else(|_| "{}".to_owned()))
    .bind(Option::<String>::None)
    .bind(created_at)
    .bind(created_at)
    .execute(&mut *tx)
    .await?;

    for (position, pipeline_id) in pipeline_ids.iter().enumerate() {
        db.query(
            "INSERT INTO e2e_queue_items (
                id, queue_id, project_id, position, pipeline_id, status, updated_at, execution_id
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(new_uuid_v7())
        .bind(queue_id)
        .bind(project_id)
        .bind(position as i64)
        .bind(pipeline_id)
        .bind(E2eQueueStatus::Pending.as_str())
        .bind(created_at)
        .bind(Option::<String>::None)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    Ok(E2eQueueRecord {
        id: queue_id.to_owned(),
        status: E2eQueueStatus::Pending,
        pipelines: pipeline_ids
            .iter()
            .map(|pipeline_id| E2eQueuePipelineRecord {
                id: pipeline_id.clone(),
                status: E2eQueueStatus::Pending,
                updated_at: created_at.to_owned(),
            })
            .collect(),
        updated_at: created_at.to_owned(),
    })
}

pub async fn load_e2e_queue_record(
    db: &DbPool,
    project_id: &str,
    queue_id: &str,
) -> Result<Option<E2eQueueRecord>, sqlx::Error> {
    let row = db
        .query(
            "SELECT id, status, updated_at FROM e2e_queues WHERE project_id = ? AND id = ? LIMIT 1",
        )
        .bind(project_id)
        .bind(queue_id)
        .fetch_optional(db)
        .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let item_rows = db
        .query(
            "SELECT pipeline_id, status, updated_at
        FROM e2e_queue_items
        WHERE project_id = ? AND queue_id = ?
        ORDER BY position ASC",
        )
        .bind(project_id)
        .bind(queue_id)
        .fetch_all(db)
        .await?;

    Ok(Some(E2eQueueRecord {
        id: row.try_get("id").unwrap_or_else(|_| queue_id.to_owned()),
        status: parse_queue_status(
            &row.try_get::<String, _>("status")
                .unwrap_or_else(|_| "failed".to_owned()),
        ),
        pipelines: item_rows
            .into_iter()
            .map(|item| E2eQueuePipelineRecord {
                id: item.try_get("pipeline_id").unwrap_or_default(),
                status: parse_queue_status(
                    &item
                        .try_get::<String, _>("status")
                        .unwrap_or_else(|_| "failed".to_owned()),
                ),
                updated_at: item.try_get("updated_at").unwrap_or_default(),
            })
            .collect(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
    }))
}

pub async fn update_e2e_queue_status(
    db: &DbPool,
    queue_id: &str,
    status: E2eQueueStatus,
    updated_at: &str,
    active_execution_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    db.query(
        "UPDATE e2e_queues
        SET status = ?, updated_at = ?, active_execution_id = ?
        WHERE id = ?",
    )
    .bind(status.as_str())
    .bind(updated_at)
    .bind(active_execution_id)
    .bind(queue_id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn update_e2e_queue_item_status(
    db: &DbPool,
    queue_id: &str,
    position: usize,
    status: E2eQueueStatus,
    updated_at: &str,
    execution_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    db.query(
        "UPDATE e2e_queue_items
        SET status = ?, updated_at = ?, execution_id = COALESCE(?, execution_id)
        WHERE queue_id = ? AND position = ?",
    )
    .bind(status.as_str())
    .bind(updated_at)
    .bind(execution_id)
    .bind(queue_id)
    .bind(position as i64)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn cancel_non_terminal_e2e_queue(
    db: &DbPool,
    queue_id: &str,
    updated_at: &str,
) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    db.query(
        "UPDATE e2e_queues
        SET status = ?, updated_at = ?, active_execution_id = NULL
        WHERE id = ? AND status IN ('pending', 'running')",
    )
    .bind(E2eQueueStatus::Cancelled.as_str())
    .bind(updated_at)
    .bind(queue_id)
    .execute(&mut *tx)
    .await?;

    db.query(
        "UPDATE e2e_queue_items
        SET status = ?, updated_at = ?
        WHERE queue_id = ? AND status IN ('pending', 'running')",
    )
    .bind(E2eQueueStatus::Cancelled.as_str())
    .bind(updated_at)
    .bind(queue_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn cancel_stale_e2e_queues(db: &DbPool, updated_at: &str) -> Result<u64, sqlx::Error> {
    let mut tx = db.begin().await?;
    let result = db
        .query(
            "UPDATE e2e_queues
        SET status = ?, updated_at = ?, active_execution_id = NULL
        WHERE status IN ('pending', 'running')",
        )
        .bind(E2eQueueStatus::Cancelled.as_str())
        .bind(updated_at)
        .execute(&mut *tx)
        .await?;

    db.query(
        "UPDATE e2e_queue_items
        SET status = ?, updated_at = ?
        WHERE status IN ('pending', 'running')",
    )
    .bind(E2eQueueStatus::Cancelled.as_str())
    .bind(updated_at)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(result.rows_affected())
}

pub fn queue_request_json(
    pipeline_ids: &[String],
    selected_base_url_key: Option<&str>,
    selected_env_group_slug: Option<&str>,
    specs: &[previa_runner::RuntimeSpec],
    env_groups: &[previa_runner::RuntimeEnvGroup],
) -> Value {
    json!({
        "pipelineIds": pipeline_ids,
        "selectedBaseUrlKey": selected_base_url_key,
        "selectedEnvGroupSlug": selected_env_group_slug,
        "specs": specs,
        "envGroups": env_groups
    })
}

#[cfg(test)]
mod tests {

    use super::{cancel_stale_e2e_queues, insert_e2e_queue, load_e2e_queue_record};
    use crate::server::models::E2eQueueStatus;

    async fn db() -> crate::server::db::DbPool {
        let db = crate::server::db::DbPool::connect_test_sqlite("sqlite::memory:", 1)
            .await
            .expect("sqlite memory db");
        sqlx::migrate!("./migrations/sqlite")
            .run(db.pool())
            .await
            .expect("migrations");
        db.query(
            "INSERT INTO projects (
                id, name, description, created_at, updated_at, created_at_ms, updated_at_ms, spec_json, execution_backend_url
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("project-1")
        .bind("Project")
        .bind(Option::<String>::None)
        .bind("2026-03-13T00:00:00.000Z")
        .bind("2026-03-13T00:00:00.000Z")
        .bind(0_i64)
        .bind(0_i64)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .execute(&db)
        .await
        .expect("insert project");
        db
    }

    #[tokio::test]
    async fn insert_queue_preserves_pipeline_order_and_duplicates() {
        let db = db().await;
        let snapshot = insert_e2e_queue(
            &db,
            "project-1",
            "queue-1",
            None,
            &serde_json::json!({}),
            &["a".to_owned(), "b".to_owned(), "a".to_owned()],
            "2026-03-13T00:00:00.000Z",
        )
        .await
        .expect("insert queue");

        assert_eq!(snapshot.pipelines.len(), 3);
        assert_eq!(snapshot.pipelines[0].id, "a");
        assert_eq!(snapshot.pipelines[1].id, "b");
        assert_eq!(snapshot.pipelines[2].id, "a");
    }

    #[tokio::test]
    async fn cancel_stale_queues_marks_non_terminal_rows_cancelled() {
        let db = db().await;
        insert_e2e_queue(
            &db,
            "project-1",
            "queue-1",
            None,
            &serde_json::json!({}),
            &["a".to_owned(), "b".to_owned()],
            "2026-03-13T00:00:00.000Z",
        )
        .await
        .expect("insert queue");

        let affected = cancel_stale_e2e_queues(&db, "2026-03-13T00:01:00.000Z")
            .await
            .expect("cancel stale queues");
        assert_eq!(affected, 1);

        let snapshot = load_e2e_queue_record(&db, "project-1", "queue-1")
            .await
            .expect("load queue")
            .expect("queue exists");
        assert_eq!(snapshot.status, E2eQueueStatus::Cancelled);
        assert!(
            snapshot
                .pipelines
                .iter()
                .all(|item| item.status == E2eQueueStatus::Cancelled)
        );
    }
}
