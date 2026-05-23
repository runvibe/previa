use std::str::FromStr;

use sqlx::Row;

use crate::server::db::DbPool;
use crate::server::models::{
    PipelineShareAccessLevel, PipelineShareRecord, PipelineSharingRecord, PipelineVisibility,
};
use crate::server::utils::{new_uuid_v7, now_iso, now_ms};

pub async fn load_pipeline_sharing_record(
    db: &DbPool,
    project_id: &str,
    pipeline_id: &str,
) -> Result<Option<PipelineSharingRecord>, sqlx::Error> {
    let row = db
        .query(
            "SELECT id, owner_user_id, owner_username, visibility
            FROM pipelines
            WHERE project_id = ? AND id = ?
            LIMIT 1",
        )
        .bind(project_id)
        .bind(pipeline_id)
        .fetch_optional(db)
        .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let shares = list_pipeline_share_records(db, pipeline_id).await?;
    Ok(Some(PipelineSharingRecord {
        pipeline_id: row.try_get("id").unwrap_or_else(|_| pipeline_id.to_owned()),
        owner_user_id: row
            .try_get("owner_user_id")
            .unwrap_or_else(|_| "anonymous".to_owned()),
        owner_username: row
            .try_get("owner_username")
            .unwrap_or_else(|_| "anonymous".to_owned()),
        visibility: row
            .try_get::<String, _>("visibility")
            .ok()
            .and_then(|value| PipelineVisibility::from_str(&value).ok())
            .unwrap_or(PipelineVisibility::Private),
        shares,
    }))
}

pub async fn list_pipeline_share_records(
    db: &DbPool,
    pipeline_id: &str,
) -> Result<Vec<PipelineShareRecord>, sqlx::Error> {
    let rows = db
        .query(
            "SELECT id, pipeline_id, user_id, username, access_level, created_at, updated_at
            FROM pipeline_shares
            WHERE pipeline_id = ?
            ORDER BY username ASC",
        )
        .bind(pipeline_id)
        .fetch_all(db)
        .await?;

    Ok(rows.iter().map(share_from_row).collect())
}

pub async fn upsert_pipeline_share_record(
    db: &DbPool,
    pipeline_id: &str,
    user_id: &str,
    username: &str,
    access_level: PipelineShareAccessLevel,
) -> Result<PipelineShareRecord, sqlx::Error> {
    let now_iso = now_iso();
    let now_ms_i64 = now_ms() as i64;
    let id = new_uuid_v7();

    db.query(
        "INSERT INTO pipeline_shares (
            id, pipeline_id, user_id, username, access_level,
            created_at, updated_at, created_at_ms, updated_at_ms
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(pipeline_id, user_id) DO UPDATE SET
            username = excluded.username,
            access_level = excluded.access_level,
            updated_at = excluded.updated_at,
            updated_at_ms = excluded.updated_at_ms",
    )
    .bind(&id)
    .bind(pipeline_id)
    .bind(user_id)
    .bind(username)
    .bind(access_level.to_string())
    .bind(&now_iso)
    .bind(&now_iso)
    .bind(now_ms_i64)
    .bind(now_ms_i64)
    .execute(db)
    .await?;

    let row = db
        .query(
            "SELECT id, pipeline_id, user_id, username, access_level, created_at, updated_at
            FROM pipeline_shares
            WHERE pipeline_id = ? AND user_id = ?
            LIMIT 1",
        )
        .bind(pipeline_id)
        .bind(user_id)
        .fetch_one(db)
        .await?;

    Ok(share_from_row(&row))
}

pub async fn delete_pipeline_share_record(
    db: &DbPool,
    pipeline_id: &str,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    let result = db
        .query("DELETE FROM pipeline_shares WHERE pipeline_id = ? AND user_id = ?")
        .bind(pipeline_id)
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn update_pipeline_visibility_record(
    db: &DbPool,
    project_id: &str,
    pipeline_id: &str,
    visibility: PipelineVisibility,
) -> Result<bool, sqlx::Error> {
    let result = db
        .query(
            "UPDATE pipelines SET visibility = ?, updated_at = ? WHERE project_id = ? AND id = ?",
        )
        .bind(visibility.to_string())
        .bind(now_iso())
        .bind(project_id)
        .bind(pipeline_id)
        .execute(db)
        .await?;
    Ok(result.rows_affected() > 0)
}

fn share_from_row(row: &sqlx::any::AnyRow) -> PipelineShareRecord {
    PipelineShareRecord {
        id: row.try_get("id").unwrap_or_default(),
        pipeline_id: row.try_get("pipeline_id").unwrap_or_default(),
        user_id: row.try_get("user_id").unwrap_or_default(),
        username: row.try_get("username").unwrap_or_default(),
        access_level: row
            .try_get::<String, _>("access_level")
            .ok()
            .and_then(|value| PipelineShareAccessLevel::from_str(&value).ok())
            .unwrap_or(PipelineShareAccessLevel::Editor),
        created_at: row.try_get("created_at").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
    }
}
