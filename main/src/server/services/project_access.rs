use sqlx::Row;

use crate::server::auth::permissions::Role;
use crate::server::auth::{Principal, PrincipalSource};
use crate::server::db::DbPool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectAccess {
    Read,
    Write,
    Manage,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectAccessRecord {
    pub owner_user_id: String,
    pub owner_username: String,
    pub visibility: String,
}

pub fn is_admin(principal: &Principal) -> bool {
    matches!(principal.role, Role::Root | Role::Admin)
}

pub fn has_full_access_anonymous(principal: &Principal) -> bool {
    matches!(principal.role, Role::Anonymous)
        && !matches!(principal.source, PrincipalSource::Anonymous)
}

pub fn is_owner(record: &ProjectAccessRecord, principal: &Principal) -> bool {
    record.owner_user_id == principal.subject
}

pub fn is_public(record: &ProjectAccessRecord) -> bool {
    record.visibility == "public"
}

pub async fn load_project_access_record(
    db: &DbPool,
    project_id: &str,
) -> Result<Option<ProjectAccessRecord>, sqlx::Error> {
    let row = db
        .query(
            "SELECT owner_user_id, owner_username, visibility
            FROM projects
            WHERE id = ?
            LIMIT 1",
        )
        .bind(project_id)
        .fetch_optional(db)
        .await?;

    Ok(row.map(|row| ProjectAccessRecord {
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

pub async fn project_is_shared_with(
    db: &DbPool,
    project_id: &str,
    principal: &Principal,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query_scalar::<sqlx::Any, i64>(
        db.sql("SELECT 1 FROM project_shares WHERE project_id = ? AND user_id = ? LIMIT 1"),
    )
    .bind(project_id)
    .bind(&principal.subject)
    .fetch_optional(db)
    .await?;
    Ok(row.is_some())
}

pub async fn can_access_project(
    db: &DbPool,
    project_id: &str,
    principal: &Principal,
    access: ProjectAccess,
) -> Result<bool, sqlx::Error> {
    if is_admin(principal) || has_full_access_anonymous(principal) {
        return Ok(true);
    }

    let Some(record) = load_project_access_record(db, project_id).await? else {
        return Ok(false);
    };

    if is_owner(&record, principal) {
        return Ok(true);
    }

    if matches!(access, ProjectAccess::Read | ProjectAccess::Write) && is_public(&record) {
        return Ok(true);
    }

    if matches!(access, ProjectAccess::Read | ProjectAccess::Write)
        && project_is_shared_with(db, project_id, principal).await?
    {
        return Ok(true);
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use crate::server::auth::permissions::Role;
    use crate::server::auth::{Principal, PrincipalSource, anonymous_principal};
    use crate::server::db::{
        DbPool, update_project_visibility_record, upsert_project_share_record,
        upsert_project_with_pipelines_for_owner,
    };
    use crate::server::models::{ProjectShareAccessLevel, ProjectUpsertRequest, ProjectVisibility};

    use super::{ProjectAccess, can_access_project};

    async fn db() -> DbPool {
        let db = DbPool::connect("sqlite::memory:", 1)
            .await
            .expect("sqlite memory db");
        sqlx::migrate!("./migrations/sqlite")
            .run(db.pool())
            .await
            .expect("migrations");
        db
    }

    fn user(subject: &str, username: &str) -> Principal {
        Principal {
            subject: subject.to_owned(),
            username: username.to_owned(),
            role: Role::Editor,
            source: PrincipalSource::Database,
        }
    }

    #[tokio::test]
    async fn private_project_is_only_visible_to_owner_until_shared() {
        let db = db().await;
        upsert_project_with_pipelines_for_owner(
            &db,
            "project-1".to_owned(),
            ProjectUpsertRequest {
                name: "Stack".to_owned(),
                description: None,
                tags: Vec::new(),
                created_at: None,
                updated_at: None,
                spec: None,
                pipelines: Vec::new(),
            },
            "owner-1",
            "owner",
        )
        .await
        .expect("project");

        let owner = user("owner-1", "owner");
        let teammate = user("user-2", "teammate");
        assert!(
            can_access_project(&db, "project-1", &owner, ProjectAccess::Write)
                .await
                .expect("owner access")
        );
        assert!(
            !can_access_project(&db, "project-1", &teammate, ProjectAccess::Read)
                .await
                .expect("private denied")
        );

        upsert_project_share_record(
            &db,
            "project-1",
            "user-2",
            "teammate",
            ProjectShareAccessLevel::Editor,
        )
        .await
        .expect("share");
        assert!(
            can_access_project(&db, "project-1", &teammate, ProjectAccess::Write)
                .await
                .expect("shared access")
        );
        assert!(
            !can_access_project(&db, "project-1", &teammate, ProjectAccess::Delete)
                .await
                .expect("shared delete denied")
        );
    }

    #[tokio::test]
    async fn public_project_allows_anonymous_write_but_not_delete_unless_owner() {
        let db = db().await;
        upsert_project_with_pipelines_for_owner(
            &db,
            "project-1".to_owned(),
            ProjectUpsertRequest {
                name: "Stack".to_owned(),
                description: None,
                tags: Vec::new(),
                created_at: None,
                updated_at: None,
                spec: None,
                pipelines: Vec::new(),
            },
            "owner-1",
            "owner",
        )
        .await
        .expect("project");
        update_project_visibility_record(&db, "project-1", ProjectVisibility::Public)
            .await
            .expect("public");

        let anonymous = anonymous_principal();
        assert!(
            can_access_project(&db, "project-1", &anonymous, ProjectAccess::Write)
                .await
                .expect("anonymous write")
        );
        assert!(
            !can_access_project(&db, "project-1", &anonymous, ProjectAccess::Delete)
                .await
                .expect("anonymous delete denied")
        );
    }
}
