use std::collections::HashSet;

use crate::server::auth::Principal;
use crate::server::auth::permissions::Role;
use crate::server::db::DbPool;
use previa_runner::Pipeline;
use sqlx::{QueryBuilder, Row};

use crate::server::db::common::touch_project_updated_at;
use crate::server::utils::{new_uuid_v7, now_iso, now_ms};

pub async fn load_pipelines_for_project(
    db: &DbPool,
    project_id: &str,
) -> Result<Vec<Pipeline>, sqlx::Error> {
    let rows = db
        .query("SELECT pipeline_json FROM pipelines WHERE project_id = ? ORDER BY position ASC")
        .bind(project_id)
        .fetch_all(db)
        .await?;

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let raw = row
            .try_get::<String, _>("pipeline_json")
            .unwrap_or_else(|_| "{}".to_owned());
        if let Ok(pipeline) = serde_json::from_str::<Pipeline>(&raw) {
            items.push(pipeline);
        }
    }
    Ok(items)
}

pub async fn load_pipelines_for_project_accessible(
    db: &DbPool,
    project_id: &str,
    principal: &Principal,
) -> Result<Vec<Pipeline>, sqlx::Error> {
    let rows = if matches!(principal.role, Role::Root | Role::Admin) {
        db.query("SELECT pipeline_json FROM pipelines WHERE project_id = ? ORDER BY position ASC")
            .bind(project_id)
            .fetch_all(db)
            .await?
    } else {
        db.query(
            "SELECT pipeline_json FROM pipelines
            WHERE project_id = ?
              AND (
                EXISTS (
                    SELECT 1 FROM projects
                    WHERE projects.id = pipelines.project_id
                      AND (
                        projects.owner_user_id = ?
                        OR projects.visibility = 'public'
                        OR EXISTS (
                            SELECT 1 FROM project_shares
                            WHERE project_shares.project_id = projects.id
                              AND project_shares.user_id = ?
                        )
                      )
                )
                OR owner_user_id = ?
                OR visibility = 'public'
                OR EXISTS (
                    SELECT 1 FROM pipeline_shares
                    WHERE pipeline_shares.pipeline_id = pipelines.id
                      AND pipeline_shares.user_id = ?
                )
              )
            ORDER BY position ASC",
        )
        .bind(project_id)
        .bind(&principal.subject)
        .bind(&principal.subject)
        .bind(&principal.subject)
        .bind(&principal.subject)
        .fetch_all(db)
        .await?
    };

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let raw = row
            .try_get::<String, _>("pipeline_json")
            .unwrap_or_else(|_| "{}".to_owned());
        if let Ok(pipeline) = serde_json::from_str::<Pipeline>(&raw) {
            items.push(pipeline);
        }
    }
    Ok(items)
}

pub async fn load_project_pipeline_record(
    db: &DbPool,
    project_id: &str,
    pipeline_id: &str,
) -> Result<Option<Pipeline>, sqlx::Error> {
    let row = db
        .query("SELECT pipeline_json FROM pipelines WHERE project_id = ? AND id = ?")
        .bind(project_id)
        .bind(pipeline_id)
        .fetch_optional(db)
        .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let raw = row
        .try_get::<String, _>("pipeline_json")
        .unwrap_or_else(|_| "{}".to_owned());
    Ok(serde_json::from_str::<Pipeline>(&raw).ok())
}

pub async fn load_project_pipeline_for_execution(
    db: &DbPool,
    project_id: &str,
    pipeline_id: &str,
) -> Result<Option<(Pipeline, i64)>, sqlx::Error> {
    let row = db
        .query(
            "SELECT position, pipeline_json FROM pipelines WHERE project_id = ? AND id = ? LIMIT 1",
        )
        .bind(project_id)
        .bind(pipeline_id)
        .fetch_optional(db)
        .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let raw = row
        .try_get::<String, _>("pipeline_json")
        .unwrap_or_else(|_| "{}".to_owned());
    let position = row.try_get::<i64, _>("position").unwrap_or_default();
    Ok(serde_json::from_str::<Pipeline>(&raw)
        .ok()
        .map(|pipeline| (pipeline, position)))
}

pub async fn load_existing_project_pipeline_ids(
    db: &DbPool,
    project_id: &str,
    pipeline_ids: &[String],
) -> Result<HashSet<String>, sqlx::Error> {
    if pipeline_ids.is_empty() {
        return Ok(HashSet::new());
    }

    let unique_ids = pipeline_ids
        .iter()
        .map(|pipeline_id| pipeline_id.trim())
        .filter(|pipeline_id| !pipeline_id.is_empty())
        .collect::<Vec<_>>();
    if unique_ids.is_empty() {
        return Ok(HashSet::new());
    }

    let mut qb = QueryBuilder::<sqlx::Any>::new("SELECT id FROM pipelines WHERE project_id = ");
    qb.push_bind(project_id);
    qb.push(" AND id IN (");
    {
        let mut separated = qb.separated(", ");
        for pipeline_id in &unique_ids {
            separated.push_bind(*pipeline_id);
        }
    }
    qb.push(")");

    let rows = qb.build().fetch_all(db).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("id").ok())
        .collect())
}

pub async fn load_existing_pipeline_ids(
    db: &DbPool,
    pipeline_ids: &[String],
) -> Result<HashSet<String>, sqlx::Error> {
    if pipeline_ids.is_empty() {
        return Ok(HashSet::new());
    }

    let unique_ids = pipeline_ids
        .iter()
        .map(|pipeline_id| pipeline_id.trim())
        .filter(|pipeline_id| !pipeline_id.is_empty())
        .collect::<Vec<_>>();
    if unique_ids.is_empty() {
        return Ok(HashSet::new());
    }

    let mut qb = QueryBuilder::<sqlx::Any>::new("SELECT id FROM pipelines WHERE id IN (");
    {
        let mut separated = qb.separated(", ");
        for pipeline_id in &unique_ids {
            separated.push_bind(*pipeline_id);
        }
    }
    qb.push(")");

    let rows = qb.build().fetch_all(db).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("id").ok())
        .collect())
}

pub async fn insert_project_pipeline(
    db: &DbPool,
    project_id: &str,
    pipeline: Pipeline,
) -> Result<Pipeline, sqlx::Error> {
    insert_project_pipeline_for_owner(db, project_id, pipeline, "anonymous", "anonymous").await
}

pub async fn insert_project_pipeline_for_owner(
    db: &DbPool,
    project_id: &str,
    mut pipeline: Pipeline,
    owner_user_id: &str,
    owner_username: &str,
) -> Result<Pipeline, sqlx::Error> {
    let now_iso = now_iso();
    let now_ms_i64 = now_ms() as i64;
    let pipeline_id = pipeline.id.clone().unwrap_or_else(new_uuid_v7);
    pipeline.id = Some(pipeline_id.clone());

    let mut tx = db.begin().await?;
    let next_position = sqlx::query_scalar::<sqlx::Any, i64>(
        db.sql("SELECT COALESCE(MAX(position) + 1, 0) FROM pipelines WHERE project_id = ?"),
    )
    .bind(project_id)
    .fetch_one(&mut *tx)
    .await?
    .max(0);

    db.query(
        "INSERT INTO pipelines (
            id, project_id, position, name, description, created_at, updated_at,
            pipeline_json, owner_user_id, owner_username, visibility
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'private')",
    )
    .bind(&pipeline_id)
    .bind(project_id)
    .bind(next_position)
    .bind(&pipeline.name)
    .bind(&pipeline.description)
    .bind(&now_iso)
    .bind(&now_iso)
    .bind(serde_json::to_string(&pipeline).unwrap_or_else(|_| "{}".to_owned()))
    .bind(owner_user_id)
    .bind(owner_username)
    .execute(&mut *tx)
    .await?;

    touch_project_updated_at(db, &mut tx, project_id, &now_iso, now_ms_i64).await?;
    tx.commit().await?;
    Ok(pipeline)
}

pub async fn update_project_pipeline(
    db: &DbPool,
    project_id: &str,
    pipeline_id: &str,
    mut pipeline: Pipeline,
) -> Result<Option<Pipeline>, sqlx::Error> {
    let now_iso = now_iso();
    let now_ms_i64 = now_ms() as i64;
    let mut tx = db.begin().await?;

    let row = db
        .query("SELECT created_at, position FROM pipelines WHERE project_id = ? AND id = ? LIMIT 1")
        .bind(project_id)
        .bind(pipeline_id)
        .fetch_optional(&mut *tx)
        .await?;

    if row.is_none() {
        tx.rollback().await?;
        return Ok(None);
    }

    pipeline.id = Some(pipeline_id.to_owned());
    db.query(
        "UPDATE pipelines SET
            name = ?,
            description = ?,
            updated_at = ?,
            pipeline_json = ?
        WHERE project_id = ? AND id = ?",
    )
    .bind(&pipeline.name)
    .bind(&pipeline.description)
    .bind(&now_iso)
    .bind(serde_json::to_string(&pipeline).unwrap_or_else(|_| "{}".to_owned()))
    .bind(project_id)
    .bind(pipeline_id)
    .execute(&mut *tx)
    .await?;

    touch_project_updated_at(db, &mut tx, project_id, &now_iso, now_ms_i64).await?;
    tx.commit().await?;
    Ok(Some(pipeline))
}

pub async fn delete_pipeline_record(
    db: &DbPool,
    project_id: &str,
    pipeline_id: &str,
) -> Result<bool, sqlx::Error> {
    let now_iso = now_iso();
    let now_ms_i64 = now_ms() as i64;
    let mut tx = db.begin().await?;

    let position = sqlx::query_scalar::<sqlx::Any, i64>(
        db.sql("SELECT position FROM pipelines WHERE project_id = ? AND id = ? LIMIT 1"),
    )
    .bind(project_id)
    .bind(pipeline_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(position) = position else {
        tx.rollback().await?;
        return Ok(false);
    };

    db.query("DELETE FROM pipelines WHERE project_id = ? AND id = ?")
        .bind(project_id)
        .bind(pipeline_id)
        .execute(&mut *tx)
        .await?;

    db.query("UPDATE pipelines SET position = position - 1 WHERE project_id = ? AND position > ?")
        .bind(project_id)
        .bind(position)
        .execute(&mut *tx)
        .await?;

    touch_project_updated_at(db, &mut tx, project_id, &now_iso, now_ms_i64).await?;
    tx.commit().await?;
    Ok(true)
}
