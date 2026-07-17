use std::str::FromStr;

use sqlx::Row;

use crate::server::auth::permissions::Role;
use crate::server::auth::{Principal, PrincipalSource};
use crate::server::db::DbPool;
use crate::server::models::PipelineShareAccessLevel;
use crate::server::services::project_access::{ProjectAccess, can_access_project};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineAccess {
    Read,
    Run,
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

pub async fn load_pipeline_share_access_level(
    db: &DbPool,
    pipeline_id: &str,
    principal: &Principal,
) -> Result<Option<PipelineShareAccessLevel>, sqlx::Error> {
    let row = sqlx::query_scalar::<sqlx::Any, String>(db.sql(
        "SELECT access_level FROM pipeline_shares
            WHERE pipeline_id = ? AND user_id = ? LIMIT 1",
    ))
    .bind(pipeline_id)
    .bind(&principal.subject)
    .fetch_optional(db)
    .await?;

    Ok(row.map(|value| {
        PipelineShareAccessLevel::from_str(&value).unwrap_or(PipelineShareAccessLevel::Editor)
    }))
}

fn pipeline_share_allows(level: PipelineShareAccessLevel, access: PipelineAccess) -> bool {
    match access {
        PipelineAccess::Read => matches!(
            level,
            PipelineShareAccessLevel::Viewer
                | PipelineShareAccessLevel::Runner
                | PipelineShareAccessLevel::Editor
                | PipelineShareAccessLevel::Manager
        ),
        PipelineAccess::Run => matches!(
            level,
            PipelineShareAccessLevel::Runner
                | PipelineShareAccessLevel::Editor
                | PipelineShareAccessLevel::Manager
        ),
        PipelineAccess::Write => matches!(
            level,
            PipelineShareAccessLevel::Editor | PipelineShareAccessLevel::Manager
        ),
        PipelineAccess::Manage | PipelineAccess::Delete => {
            matches!(level, PipelineShareAccessLevel::Manager)
        }
    }
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

    let inherited_project_access = match access {
        PipelineAccess::Read => Some(ProjectAccess::Read),
        PipelineAccess::Run => Some(ProjectAccess::Run),
        PipelineAccess::Write => Some(ProjectAccess::Write),
        PipelineAccess::Manage => Some(ProjectAccess::Manage),
        PipelineAccess::Delete => None,
    };
    if let Some(project_access) = inherited_project_access {
        if can_access_project(db, project_id, principal, project_access).await? {
            return Ok(true);
        }
    }

    let Some(record) = load_pipeline_access_record(db, project_id, pipeline_id).await? else {
        return Ok(false);
    };

    if is_owner(&record, principal) {
        return Ok(true);
    }

    if matches!(
        access,
        PipelineAccess::Read | PipelineAccess::Run | PipelineAccess::Write
    ) && is_public(&record)
    {
        return Ok(true);
    }

    if let Some(level) = load_pipeline_share_access_level(db, pipeline_id, principal).await? {
        return Ok(pipeline_share_allows(level, access));
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
        let project_access = match access {
            PipelineAccess::Read => ProjectAccess::Read,
            PipelineAccess::Run => ProjectAccess::Run,
            PipelineAccess::Write => ProjectAccess::Write,
            PipelineAccess::Manage => ProjectAccess::Manage,
            PipelineAccess::Delete => ProjectAccess::Delete,
        };
        return can_access_project(db, project_id, principal, project_access).await;
    };
    can_access_pipeline(db, project_id, pipeline_id, principal, access).await
}

#[cfg(test)]
mod tests {
    use crate::server::auth::permissions::Role;
    use crate::server::auth::{Principal, PrincipalSource};
    use crate::server::db::{
        DbPool, insert_project_pipeline_for_owner, upsert_pipeline_share_record,
        upsert_project_share_record, upsert_project_with_pipelines_for_owner,
    };
    use crate::server::models::{
        PipelineShareAccessLevel, ProjectShareAccessLevel, ProjectUpsertRequest,
    };
    use previa_runner::Pipeline;

    use super::{PipelineAccess, can_access_pipeline};

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

    async fn seed_project_pipeline(db: &DbPool) {
        upsert_project_with_pipelines_for_owner(
            db,
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
            db,
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
    }

    #[tokio::test]
    async fn pipeline_share_levels_gate_read_run_write_and_manage() {
        let db = db().await;
        seed_project_pipeline(&db).await;
        let teammate = user("user-2", "teammate");

        for (level, expected) in [
            (
                PipelineShareAccessLevel::Viewer,
                [true, false, false, false, false],
            ),
            (
                PipelineShareAccessLevel::Runner,
                [true, true, false, false, false],
            ),
            (
                PipelineShareAccessLevel::Editor,
                [true, true, true, false, false],
            ),
            (
                PipelineShareAccessLevel::Manager,
                [true, true, true, true, true],
            ),
        ] {
            upsert_pipeline_share_record(&db, "pipe-1", "user-2", "teammate", level)
                .await
                .expect("share");

            for (access, allowed) in [
                (PipelineAccess::Read, expected[0]),
                (PipelineAccess::Run, expected[1]),
                (PipelineAccess::Write, expected[2]),
                (PipelineAccess::Manage, expected[3]),
                (PipelineAccess::Delete, expected[4]),
            ] {
                assert_eq!(
                    can_access_pipeline(&db, "project-1", "pipe-1", &teammate, access)
                        .await
                        .expect("access check"),
                    allowed,
                    "{level:?} should allow {access:?}={allowed}"
                );
            }
        }
    }

    #[tokio::test]
    async fn pipeline_access_inherits_stack_share_level() {
        let db = db().await;
        seed_project_pipeline(&db).await;
        let teammate = user("user-2", "teammate");

        upsert_project_share_record(
            &db,
            "project-1",
            "user-2",
            "teammate",
            ProjectShareAccessLevel::Runner,
        )
        .await
        .expect("share");

        assert!(
            can_access_pipeline(&db, "project-1", "pipe-1", &teammate, PipelineAccess::Run)
                .await
                .expect("run inherited")
        );
        assert!(
            !can_access_pipeline(&db, "project-1", "pipe-1", &teammate, PipelineAccess::Write)
                .await
                .expect("write denied")
        );
    }
}
