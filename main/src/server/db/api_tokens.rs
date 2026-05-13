use sqlx::Row;

use crate::server::auth::permissions::Role;
use crate::server::db::DbPool;
use crate::server::models::ApiTokenRecord;
use crate::server::utils::{now_iso, now_ms};

#[derive(Debug, Clone)]
pub struct ApiTokenInsert {
    pub id: String,
    pub name: String,
    pub token_prefix: String,
    pub token_hash: String,
    pub role: Role,
    pub created_by_user_id: Option<String>,
    pub created_by_username: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ApiTokenAuthRecord {
    pub id: String,
    pub name: String,
    pub token_hash: String,
    pub role: Role,
    pub active: bool,
    pub expires_at: Option<String>,
}

pub async fn insert_api_token_record(
    db: &DbPool,
    input: ApiTokenInsert,
) -> Result<ApiTokenRecord, sqlx::Error> {
    let now_iso = now_iso();
    let now_ms = now_ms() as i64;
    db.query(
        "INSERT INTO api_tokens (
            id, name, token_prefix, token_hash, role, created_by_user_id,
            created_by_username, active, last_used_at, expires_at, created_at,
            updated_at, created_at_ms, updated_at_ms
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&input.id)
    .bind(&input.name)
    .bind(&input.token_prefix)
    .bind(&input.token_hash)
    .bind(input.role.to_string())
    .bind(&input.created_by_user_id)
    .bind(&input.created_by_username)
    .bind(1_i64)
    .bind(Option::<String>::None)
    .bind(&input.expires_at)
    .bind(&now_iso)
    .bind(&now_iso)
    .bind(now_ms)
    .bind(now_ms)
    .execute(db)
    .await?;

    load_api_token_record(db, &input.id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn list_api_token_records(db: &DbPool) -> Result<Vec<ApiTokenRecord>, sqlx::Error> {
    let rows = db
        .query(
            "SELECT id, name, token_prefix, role, active, expires_at, created_by_username,
                last_used_at, created_at, updated_at
            FROM api_tokens
            ORDER BY updated_at_ms DESC",
        )
        .fetch_all(db)
        .await?;
    Ok(rows.iter().map(api_token_record_from_row).collect())
}

pub async fn load_api_token_record(
    db: &DbPool,
    token_id: &str,
) -> Result<Option<ApiTokenRecord>, sqlx::Error> {
    let row = db
        .query(
            "SELECT id, name, token_prefix, role, active, expires_at, created_by_username,
                last_used_at, created_at, updated_at
            FROM api_tokens
            WHERE id = ?",
        )
        .bind(token_id)
        .fetch_optional(db)
        .await?;
    Ok(row.as_ref().map(api_token_record_from_row))
}

pub async fn load_api_token_auth_record_by_hash(
    db: &DbPool,
    token_hash: &str,
) -> Result<Option<ApiTokenAuthRecord>, sqlx::Error> {
    let row = db
        .query(
            "SELECT id, name, token_hash, role, active, expires_at
            FROM api_tokens
            WHERE token_hash = ?",
        )
        .bind(token_hash)
        .fetch_optional(db)
        .await?;
    Ok(row.as_ref().map(api_token_auth_record_from_row))
}

pub async fn update_api_token_last_used(db: &DbPool, token_id: &str) -> Result<(), sqlx::Error> {
    db.query(
        "UPDATE api_tokens SET last_used_at = ?, updated_at = ?, updated_at_ms = ? WHERE id = ?",
    )
    .bind(now_iso())
    .bind(now_iso())
    .bind(now_ms() as i64)
    .bind(token_id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn set_api_token_active(
    db: &DbPool,
    token_id: &str,
    active: bool,
) -> Result<Option<ApiTokenRecord>, sqlx::Error> {
    db.query("UPDATE api_tokens SET active = ?, updated_at = ?, updated_at_ms = ? WHERE id = ?")
        .bind(if active { 1_i64 } else { 0_i64 })
        .bind(now_iso())
        .bind(now_ms() as i64)
        .bind(token_id)
        .execute(db)
        .await?;
    load_api_token_record(db, token_id).await
}

pub async fn delete_api_token_record(db: &DbPool, token_id: &str) -> Result<bool, sqlx::Error> {
    let result = db
        .query("DELETE FROM api_tokens WHERE id = ?")
        .bind(token_id)
        .execute(db)
        .await?;
    Ok(result.rows_affected() > 0)
}

fn api_token_record_from_row(row: &sqlx::any::AnyRow) -> ApiTokenRecord {
    ApiTokenRecord {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or_default(),
        token_prefix: row.try_get("token_prefix").unwrap_or_default(),
        role: row
            .try_get::<String, _>("role")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(Role::Viewer),
        active: row
            .try_get::<i64, _>("active")
            .map(|value| value != 0)
            .unwrap_or(false),
        expires_at: row.try_get("expires_at").ok().flatten(),
        created_by_username: row.try_get("created_by_username").unwrap_or_default(),
        last_used_at: row.try_get("last_used_at").ok().flatten(),
        created_at: row.try_get("created_at").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
    }
}

fn api_token_auth_record_from_row(row: &sqlx::any::AnyRow) -> ApiTokenAuthRecord {
    ApiTokenAuthRecord {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or_default(),
        token_hash: row.try_get("token_hash").unwrap_or_default(),
        role: row
            .try_get::<String, _>("role")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(Role::Viewer),
        active: row
            .try_get::<i64, _>("active")
            .map(|value| value != 0)
            .unwrap_or(false),
        expires_at: row.try_get("expires_at").ok().flatten(),
    }
}
