use sqlx::Row;

use crate::server::auth::permissions::Role;
use crate::server::db::DbPool;
use crate::server::models::UserRecord;
use crate::server::utils::{now_iso, now_ms};

#[derive(Debug, Clone)]
pub struct UserInsert {
    pub id: String,
    pub username: String,
    pub password_hash: String,
    pub role: Role,
    pub active: bool,
}

#[derive(Debug, Clone)]
pub struct UserUpdate {
    pub username: Option<String>,
    pub password_hash: Option<String>,
    pub role: Option<Role>,
    pub active: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct UserAuthRecord {
    pub id: String,
    pub username: String,
    pub password_hash: String,
    pub role: Role,
    pub active: bool,
}

pub async fn insert_user_record(db: &DbPool, input: UserInsert) -> Result<UserRecord, sqlx::Error> {
    let now_iso = now_iso();
    let now_ms = now_ms() as i64;
    db.query(
        "INSERT INTO users (
            id, username, password_hash, role, active, created_at, updated_at,
            created_at_ms, updated_at_ms
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&input.id)
    .bind(&input.username)
    .bind(&input.password_hash)
    .bind(input.role.to_string())
    .bind(if input.active { 1_i64 } else { 0_i64 })
    .bind(&now_iso)
    .bind(&now_iso)
    .bind(now_ms)
    .bind(now_ms)
    .execute(db)
    .await?;

    load_user_record(db, &input.id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn list_user_records(db: &DbPool) -> Result<Vec<UserRecord>, sqlx::Error> {
    let rows = db
        .query(
            "SELECT id, username, role, active, created_at, updated_at
            FROM users
            ORDER BY updated_at_ms DESC",
        )
        .fetch_all(db)
        .await?;
    Ok(rows.iter().map(user_record_from_row).collect())
}

pub async fn load_user_record(
    db: &DbPool,
    user_id: &str,
) -> Result<Option<UserRecord>, sqlx::Error> {
    let row = db
        .query(
            "SELECT id, username, role, active, created_at, updated_at
            FROM users
            WHERE id = ?",
        )
        .bind(user_id)
        .fetch_optional(db)
        .await?;
    Ok(row.as_ref().map(user_record_from_row))
}

pub async fn load_user_auth_record_by_username(
    db: &DbPool,
    username: &str,
) -> Result<Option<UserAuthRecord>, sqlx::Error> {
    let row = db
        .query(
            "SELECT id, username, password_hash, role, active
            FROM users
            WHERE username = ?",
        )
        .bind(username)
        .fetch_optional(db)
        .await?;
    Ok(row.as_ref().map(user_auth_record_from_row))
}

pub async fn update_user_record(
    db: &DbPool,
    user_id: &str,
    input: UserUpdate,
) -> Result<Option<UserRecord>, sqlx::Error> {
    let existing = db
        .query("SELECT username, password_hash, role, active FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(db)
        .await?;
    let Some(existing) = existing else {
        return Ok(None);
    };

    let username = input
        .username
        .unwrap_or_else(|| existing.try_get("username").unwrap_or_default());
    let password_hash = input
        .password_hash
        .unwrap_or_else(|| existing.try_get("password_hash").unwrap_or_default());
    let role = input.role.unwrap_or_else(|| {
        existing
            .try_get::<String, _>("role")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(Role::Viewer)
    });
    let active = input.active.unwrap_or_else(|| {
        existing
            .try_get::<i64, _>("active")
            .map(|value| value != 0)
            .unwrap_or(false)
    });

    db.query(
        "UPDATE users
        SET username = ?, password_hash = ?, role = ?, active = ?, updated_at = ?, updated_at_ms = ?
        WHERE id = ?",
    )
    .bind(username)
    .bind(password_hash)
    .bind(role.to_string())
    .bind(if active { 1_i64 } else { 0_i64 })
    .bind(now_iso())
    .bind(now_ms() as i64)
    .bind(user_id)
    .execute(db)
    .await?;

    load_user_record(db, user_id).await
}

pub async fn delete_user_record(db: &DbPool, user_id: &str) -> Result<bool, sqlx::Error> {
    let result = db
        .query("DELETE FROM users WHERE id = ?")
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(result.rows_affected() > 0)
}

fn user_record_from_row(row: &sqlx::any::AnyRow) -> UserRecord {
    UserRecord {
        id: row.try_get("id").unwrap_or_default(),
        username: row.try_get("username").unwrap_or_default(),
        role: row
            .try_get::<String, _>("role")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(Role::Viewer),
        active: row
            .try_get::<i64, _>("active")
            .map(|value| value != 0)
            .unwrap_or(false),
        created_at: row.try_get("created_at").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
    }
}

fn user_auth_record_from_row(row: &sqlx::any::AnyRow) -> UserAuthRecord {
    UserAuthRecord {
        id: row.try_get("id").unwrap_or_default(),
        username: row.try_get("username").unwrap_or_default(),
        password_hash: row.try_get("password_hash").unwrap_or_default(),
        role: row
            .try_get::<String, _>("role")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(Role::Viewer),
        active: row
            .try_get::<i64, _>("active")
            .map(|value| value != 0)
            .unwrap_or(false),
    }
}
