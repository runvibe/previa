use std::str::FromStr;

use sqlx::Row;

use crate::server::db::DbPool;
use crate::server::models::{
    ProjectShareAccessLevel, ProjectShareRecord, ProjectSharingRecord, ProjectVisibility,
};
use crate::server::utils::{new_uuid_v7, now_iso, now_ms};

pub async fn load_project_sharing_record(
    db: &DbPool,
    project_id: &str,
) -> Result<Option<ProjectSharingRecord>, sqlx::Error> {
    let row = db
        .query(
            "SELECT id, owner_user_id, owner_username, visibility
            FROM projects
            WHERE id = ?
            LIMIT 1",
        )
        .bind(project_id)
        .fetch_optional(db)
        .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let shares = list_project_share_records(db, project_id).await?;
    Ok(Some(ProjectSharingRecord {
        project_id: row.try_get("id").unwrap_or_default(),
        owner_user_id: row
            .try_get("owner_user_id")
            .unwrap_or_else(|_| "anonymous".to_owned()),
        owner_username: row
            .try_get("owner_username")
            .unwrap_or_else(|_| "anonymous".to_owned()),
        visibility: row
            .try_get::<String, _>("visibility")
            .ok()
            .and_then(|value| ProjectVisibility::from_str(&value).ok())
            .unwrap_or(ProjectVisibility::Private),
        shares,
    }))
}

pub async fn list_project_share_records(
    db: &DbPool,
    project_id: &str,
) -> Result<Vec<ProjectShareRecord>, sqlx::Error> {
    let rows = db
        .query(
            "SELECT id, project_id, user_id, username, access_level, created_at, updated_at
            FROM project_shares
            WHERE project_id = ?
            ORDER BY username ASC",
        )
        .bind(project_id)
        .fetch_all(db)
        .await?;
    Ok(rows.iter().map(share_from_row).collect())
}

pub async fn upsert_project_share_record(
    db: &DbPool,
    project_id: &str,
    user_id: &str,
    username: &str,
    access_level: ProjectShareAccessLevel,
) -> Result<ProjectShareRecord, sqlx::Error> {
    let now_iso = now_iso();
    let now_ms_i64 = now_ms() as i64;
    let id = new_uuid_v7();

    db.query(
        "INSERT INTO project_shares (
            id, project_id, user_id, username, access_level, created_at, updated_at, created_at_ms, updated_at_ms
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(project_id, user_id) DO UPDATE SET
            username = excluded.username,
            access_level = excluded.access_level,
            updated_at = excluded.updated_at,
            updated_at_ms = excluded.updated_at_ms",
    )
    .bind(&id)
    .bind(project_id)
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
            "SELECT id, project_id, user_id, username, access_level, created_at, updated_at
            FROM project_shares
            WHERE project_id = ? AND user_id = ?",
        )
        .bind(project_id)
        .bind(user_id)
        .fetch_one(db)
        .await?;
    Ok(share_from_row(&row))
}

pub async fn delete_project_share_record(
    db: &DbPool,
    project_id: &str,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    let mut tx = db.begin().await?;

    let project_result = db
        .query("DELETE FROM project_shares WHERE project_id = ? AND user_id = ?")
        .bind(project_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

    let pipeline_result = db
        .query(
            "DELETE FROM pipeline_shares
            WHERE user_id = ?
              AND pipeline_id IN (
                SELECT id FROM pipelines WHERE project_id = ?
              )",
        )
        .bind(user_id)
        .bind(project_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    Ok(project_result.rows_affected() > 0 || pipeline_result.rows_affected() > 0)
}

pub async fn update_project_visibility_record(
    db: &DbPool,
    project_id: &str,
    visibility: ProjectVisibility,
) -> Result<bool, sqlx::Error> {
    let now_iso = now_iso();
    let now_ms_i64 = now_ms() as i64;
    let result = db
        .query("UPDATE projects SET visibility = ?, updated_at = ?, updated_at_ms = ? WHERE id = ?")
        .bind(visibility.to_string())
        .bind(now_iso)
        .bind(now_ms_i64)
        .bind(project_id)
        .execute(db)
        .await?;
    Ok(result.rows_affected() > 0)
}

fn share_from_row(row: &sqlx::any::AnyRow) -> ProjectShareRecord {
    ProjectShareRecord {
        id: row.try_get("id").unwrap_or_default(),
        project_id: row.try_get("project_id").unwrap_or_default(),
        user_id: row.try_get("user_id").unwrap_or_default(),
        username: row.try_get("username").unwrap_or_default(),
        access_level: row
            .try_get::<String, _>("access_level")
            .ok()
            .and_then(|value| ProjectShareAccessLevel::from_str(&value).ok())
            .unwrap_or(ProjectShareAccessLevel::Editor),
        created_at: row.try_get("created_at").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
    }
}
