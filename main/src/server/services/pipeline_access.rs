use sqlx::Row;

use crate::server::auth::permissions::Role;
use crate::server::auth::{Principal, PrincipalSource};
use crate::server::db::DbPool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineAccess {
    Read,
    Write,
    Manage,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineAccessRecord {
    pub owner_user_id: String,
    pub owner_username: String,
    pub visibility: String,
}

pub fn is_admin(principal: &Principal) -> bool {
    matches!(principal.role, Role::Root | Role::Admin)
}

pub fn is_owner(record: &PipelineAccessRecord, principal: &Principal) -> bool {
    record.owner_user_id == principal.subject
}

pub fn is_public(record: &PipelineAccessRecord) -> bool {
    record.visibility == "public"
}

pub async fn load_pipeline_access_record(
    db: &DbPool,
    project_id: &str,
    pipeline_id: &str,
) -> Result<Option<PipelineAccessRecord>, sqlx::Error> {
    let row = db
        .query(
            "SELECT owner_user_id, owner_username, visibility
            FROM pipelines
            WHERE project_id = ? AND id = ?
            LIMIT 1",
        )
        .bind(project_id)
        .bind(pipeline_id)
        .fetch_optional(db)
        .await?;

    Ok(row.map(|row| PipelineAccessRecord {
        owner_user_id: row
            .try_get("owner_user_id")
            .unwrap_or_else(|_| "anonymous".to_owned()),
        owner_username: row
            .try_get("owner_username")
            .unwrap_or_else(|_| "anonymous".to_owned()),
        visibility: row
            .try_get("visibility")
            .unwrap_or_else(|_| "private".to_owned()),
    }))
}

pub async fn pipeline_is_shared_with(
    db: &DbPool,
    pipeline_id: &str,
    principal: &Principal,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query_scalar::<sqlx::Any, i64>(
        db.sql("SELECT 1 FROM pipeline_shares WHERE pipeline_id = ? AND user_id = ? LIMIT 1"),
    )
    .bind(pipeline_id)
    .bind(&principal.subject)
    .fetch_optional(db)
    .await?;
    Ok(row.is_some())
}

pub async fn can_access_pipeline(
    db: &DbPool,
    project_id: &str,
    pipeline_id: &str,
    principal: &Principal,
    access: PipelineAccess,
) -> Result<bool, sqlx::Error> {
    if is_admin(principal)
        || (matches!(principal.role, Role::Anonymous)
            && !matches!(principal.source, PrincipalSource::Anonymous))
    {
        return Ok(true);
    }

    let Some(record) = load_pipeline_access_record(db, project_id, pipeline_id).await? else {
        return Ok(false);
    };

    if is_owner(&record, principal) {
        return Ok(true);
    }

    if matches!(access, PipelineAccess::Read | PipelineAccess::Write) && is_public(&record) {
        return Ok(true);
    }

    if matches!(access, PipelineAccess::Read | PipelineAccess::Write)
        && pipeline_is_shared_with(db, pipeline_id, principal).await?
    {
        return Ok(true);
    }

    Ok(false)
}

pub async fn can_access_optional_pipeline(
    db: &DbPool,
    project_id: &str,
    pipeline_id: Option<&str>,
    principal: &Principal,
    access: PipelineAccess,
) -> Result<bool, sqlx::Error> {
    let Some(pipeline_id) = pipeline_id.filter(|value| !value.trim().is_empty()) else {
        return Ok(is_admin(principal)
            || (matches!(principal.role, Role::Anonymous)
                && !matches!(principal.source, PrincipalSource::Anonymous)));
    };
    can_access_pipeline(db, project_id, pipeline_id, principal, access).await
}
