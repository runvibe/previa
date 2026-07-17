use std::str::FromStr;

use sqlx::Row;

use crate::server::auth::permissions::Role;
use crate::server::auth::{Principal, PrincipalSource};
use crate::server::db::DbPool;
use crate::server::models::ProjectShareAccessLevel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectAccess {
    Read,
    Run,
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

pub async fn load_project_share_access_level(
    db: &DbPool,
    project_id: &str,
    principal: &Principal,
) -> Result<Option<ProjectShareAccessLevel>, sqlx::Error> {
    let row = sqlx::query_scalar::<sqlx::Any, String>(db.sql(
        "SELECT access_level FROM project_shares
            WHERE project_id = ? AND user_id = ? LIMIT 1",
    ))
    .bind(project_id)
    .bind(&principal.subject)
    .fetch_optional(db)
    .await?;

    Ok(row.map(|value| {
        ProjectShareAccessLevel::from_str(&value).unwrap_or(ProjectShareAccessLevel::Editor)
    }))
}

fn project_share_allows(level: ProjectShareAccessLevel, access: ProjectAccess) -> bool {
    match access {
        ProjectAccess::Read => matches!(
            level,
            ProjectShareAccessLevel::Viewer
                | ProjectShareAccessLevel::Runner
                | ProjectShareAccessLevel::Editor
                | ProjectShareAccessLevel::Manager
        ),
        ProjectAccess::Run => matches!(
            level,
            ProjectShareAccessLevel::Runner
                | ProjectShareAccessLevel::Editor
                | ProjectShareAccessLevel::Manager
        ),
        ProjectAccess::Write => matches!(
            level,
            ProjectShareAccessLevel::Editor | ProjectShareAccessLevel::Manager
        ),
        ProjectAccess::Manage | ProjectAccess::Delete => {
            matches!(level, ProjectShareAccessLevel::Manager)
        }
    }
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

    if matches!(
        access,
        ProjectAccess::Read | ProjectAccess::Run | ProjectAccess::Write
    ) && is_public(&record)
    {
        return Ok(true);
    }

    if let Some(level) = load_project_share_access_level(db, project_id, principal).await? {
        return Ok(project_share_allows(level, access));
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use crate::server::auth::permissions::Role;
    use crate::server::auth::{Principal, PrincipalSource, anonymous_principal};
    use crate::server::db::{
        DbPool, delete_project_share_record, insert_project_pipeline_for_owner,
        list_project_records_accessible, update_project_visibility_record,
        upsert_pipeline_share_record, upsert_project_share_record,
        upsert_project_with_pipelines_for_owner,
    };
    use crate::server::models::{
        ProjectListQuery, ProjectShareAccessLevel, ProjectUpsertRequest, ProjectVisibility,
    };
    use previa_runner::Pipeline;

    use super::{ProjectAccess, can_access_project};

    async fn db() -> DbPool {
        let db = DbPool::connect_test_sqlite("sqlite::memory:", 1)
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
    async fn project_share_levels_gate_read_run_write_and_manage() {
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

        let teammate = user("user-2", "teammate");
        for (level, expected) in [
            (
                ProjectShareAccessLevel::Viewer,
                [true, false, false, false, false],
            ),
            (
                ProjectShareAccessLevel::Runner,
                [true, true, false, false, false],
            ),
            (
                ProjectShareAccessLevel::Editor,
                [true, true, true, false, false],
            ),
            (
                ProjectShareAccessLevel::Manager,
                [true, true, true, true, true],
            ),
        ] {
            upsert_project_share_record(&db, "project-1", "user-2", "teammate", level)
                .await
                .expect("share");

            for (access, allowed) in [
                (ProjectAccess::Read, expected[0]),
                (ProjectAccess::Run, expected[1]),
                (ProjectAccess::Write, expected[2]),
                (ProjectAccess::Manage, expected[3]),
                (ProjectAccess::Delete, expected[4]),
            ] {
                assert_eq!(
                    can_access_project(&db, "project-1", &teammate, access)
                        .await
                        .expect("access check"),
                    allowed,
                    "{level:?} should allow {access:?}={allowed}"
                );
            }
        }
    }

    #[tokio::test]
    async fn revoking_project_share_removes_pipeline_shares_for_that_stack() {
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
        insert_project_pipeline_for_owner(
            &db,
            "project-1",
            Pipeline {
                id: Some("pipe-1".to_owned()),
                name: "Pipeline".to_owned(),
                description: None,
                steps: Vec::new(),
            },
            "owner-1",
            "owner",
        )
        .await
        .expect("pipeline");

        let teammate = user("user-2", "teammate");
        upsert_project_share_record(
            &db,
            "project-1",
            "user-2",
            "teammate",
            ProjectShareAccessLevel::Editor,
        )
        .await
        .expect("project share");
        upsert_pipeline_share_record(
            &db,
            "pipe-1",
            "user-2",
            "teammate",
            crate::server::models::PipelineShareAccessLevel::Editor,
        )
        .await
        .expect("pipeline share");

        assert!(
            !list_project_records_accessible(&db, project_list_query(), &teammate)
                .await
                .expect("visible before revoke")
                .is_empty()
        );

        assert!(
            delete_project_share_record(&db, "project-1", "user-2")
                .await
                .expect("revoke")
        );

        assert!(
            list_project_records_accessible(&db, project_list_query(), &teammate)
                .await
                .expect("visible after revoke")
                .is_empty()
        );
    }

    fn project_list_query() -> ProjectListQuery {
        ProjectListQuery {
            limit: None,
            offset: None,
            order: None,
        }
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
