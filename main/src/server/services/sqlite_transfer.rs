use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::server::db::{
    DbPool, import_project_bundle, load_e2e_history_for_export, load_load_history_for_export,
    load_project_export, project_name_exists,
};
use crate::server::models::{
    ProjectEnvGroupRecord, ProjectExportProject, ProjectImportResponse, ProjectSpecRecord,
};
use crate::server::utils::new_uuid_v7;
use serde::Serialize;
use sqlx::Row;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SqliteProjectImportItem {
    pub source_project_id: String,
    pub project_id: String,
    pub project_name: String,
    pub pipelines_imported: usize,
    pub specs_imported: usize,
    pub env_groups_imported: usize,
    pub e2e_history_imported: usize,
    pub load_history_imported: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SqliteProjectImportResponse {
    pub include_history: bool,
    pub projects_imported: usize,
    pub projects: Vec<SqliteProjectImportItem>,
}

pub async fn export_projects_to_sqlite(
    source: &DbPool,
    target_path: &Path,
    project_ids: &[String],
    include_history: bool,
) -> Result<(), sqlx::Error> {
    if target_path.exists() {
        std::fs::remove_file(target_path).map_err(|err| sqlx::Error::Io(err.into()))?;
    }

    let target = DbPool::connect(&sqlite_url(target_path), 1).await?;
    sqlx::migrate!("./migrations/sqlite")
        .run(target.pool())
        .await?;

    let project_ids = if project_ids.is_empty() {
        list_all_project_ids(source).await?
    } else {
        project_ids.to_vec()
    };

    for project_id in project_ids {
        let Some(mut project) = load_project_export(source, &project_id).await? else {
            return Err(sqlx::Error::RowNotFound);
        };
        if include_history {
            project.history.e2e = load_e2e_history_for_export(source, &project_id).await?;
            project.history.load = load_load_history_for_export(source, &project_id).await?;
        }
        import_project_bundle(&target, &project, include_history).await?;
    }

    Ok(())
}

pub async fn import_projects_from_sqlite(
    target: &DbPool,
    source_path: &Path,
    include_history: bool,
) -> Result<SqliteProjectImportResponse, sqlx::Error> {
    let source = DbPool::connect(&sqlite_url(source_path), 1).await?;
    let project_ids = list_all_project_ids(&source).await?;
    let mut reserved_names = HashSet::new();
    let mut projects = Vec::with_capacity(project_ids.len());

    for source_project_id in project_ids {
        let Some(mut project) = load_project_export(&source, &source_project_id).await? else {
            continue;
        };
        if include_history {
            project.history.e2e = load_e2e_history_for_export(&source, &source_project_id).await?;
            project.history.load =
                load_load_history_for_export(&source, &source_project_id).await?;
        }

        rewrite_imported_project(target, &mut project, &mut reserved_names).await?;
        let imported = import_project_bundle(target, &project, include_history).await?;
        projects.push(import_item(
            source_project_id,
            project.name.clone(),
            imported,
        ));
    }

    Ok(SqliteProjectImportResponse {
        include_history,
        projects_imported: projects.len(),
        projects,
    })
}

async fn list_all_project_ids(db: &DbPool) -> Result<Vec<String>, sqlx::Error> {
    let rows = db
        .query("SELECT id FROM projects ORDER BY updated_at_ms DESC")
        .fetch_all(db)
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("id").ok())
        .collect())
}

async fn rewrite_imported_project(
    target: &DbPool,
    project: &mut ProjectExportProject,
    reserved_names: &mut HashSet<String>,
) -> Result<(), sqlx::Error> {
    let new_project_id = new_uuid_v7();
    project.id = new_project_id.clone();
    project.name = unique_imported_name(target, &project.name, reserved_names).await?;

    let mut pipeline_ids = HashMap::new();
    for pipeline in &mut project.pipelines {
        let old_id = pipeline.id.clone();
        let new_id = new_uuid_v7();
        if let Some(old_id) = old_id {
            pipeline_ids.insert(old_id, new_id.clone());
        }
        pipeline.id = Some(new_id);
    }

    project.specs = project
        .specs
        .iter()
        .cloned()
        .map(|mut spec| {
            spec.id = new_uuid_v7();
            spec.project_id = new_project_id.clone();
            spec
        })
        .collect::<Vec<ProjectSpecRecord>>();

    project.env_groups = project
        .env_groups
        .iter()
        .cloned()
        .map(|mut group| {
            group.id = new_uuid_v7();
            group.project_id = new_project_id.clone();
            group
        })
        .collect::<Vec<ProjectEnvGroupRecord>>();

    for record in &mut project.history.e2e {
        record.id = new_uuid_v7();
        record.execution_id = new_uuid_v7();
        record.project_id = Some(new_project_id.clone());
        if let Some(old_pipeline_id) = record.pipeline_id.as_ref() {
            if let Some(new_pipeline_id) = pipeline_ids.get(old_pipeline_id) {
                record.pipeline_id = Some(new_pipeline_id.clone());
            }
        }
    }

    for record in &mut project.history.load {
        record.id = new_uuid_v7();
        record.execution_id = new_uuid_v7();
        record.project_id = Some(new_project_id.clone());
        if let Some(old_pipeline_id) = record.pipeline_id.as_ref() {
            if let Some(new_pipeline_id) = pipeline_ids.get(old_pipeline_id) {
                record.pipeline_id = Some(new_pipeline_id.clone());
            }
        }
    }
    Ok(())
}

async fn unique_imported_name(
    target: &DbPool,
    source_name: &str,
    reserved_names: &mut HashSet<String>,
) -> Result<String, sqlx::Error> {
    let base_name = source_name.trim();
    let base_name = if base_name.is_empty() {
        "Imported project"
    } else {
        base_name
    };

    if !reserved_names.contains(base_name) && !project_name_exists(target, base_name).await? {
        reserved_names.insert(base_name.to_owned());
        return Ok(base_name.to_owned());
    }

    let mut index = 1usize;
    loop {
        let candidate = if index == 1 {
            format!("{base_name}-imported")
        } else {
            format!("{base_name}-imported-{index}")
        };
        if !reserved_names.contains(&candidate) && !project_name_exists(target, &candidate).await? {
            reserved_names.insert(candidate.clone());
            return Ok(candidate);
        }
        index += 1;
    }
}

fn import_item(
    source_project_id: String,
    project_name: String,
    response: ProjectImportResponse,
) -> SqliteProjectImportItem {
    SqliteProjectImportItem {
        source_project_id,
        project_id: response.project_id,
        project_name,
        pipelines_imported: response.pipelines_imported,
        specs_imported: response.specs_imported,
        env_groups_imported: response.env_groups_imported,
        e2e_history_imported: response.e2e_history_imported,
        load_history_imported: response.load_history_imported,
    }
}

fn sqlite_url(path: &Path) -> String {
    format!("sqlite://{}", path.display())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::server::db::{DbPool, list_project_records, upsert_project_metadata};
    use crate::server::models::{ProjectListQuery, ProjectMetadataUpsertRequest};

    async fn migrated_db(url: &str) -> DbPool {
        let db = DbPool::connect(url, 1).await.expect("connect db");
        sqlx::migrate!("./migrations/sqlite")
            .run(db.pool())
            .await
            .expect("migrate db");
        db
    }

    async fn add_project(db: &DbPool, id: &str, name: &str) {
        add_project_with_tags(db, id, name, Vec::new()).await;
    }

    async fn add_project_with_tags(db: &DbPool, id: &str, name: &str, tags: Vec<String>) {
        upsert_project_metadata(
            db,
            id.to_owned(),
            ProjectMetadataUpsertRequest {
                name: name.to_owned(),
                description: None,
                tags,
            },
        )
        .await
        .expect("add project");
    }

    fn sqlite_url(path: &Path) -> String {
        format!("sqlite://{}", path.display())
    }

    #[tokio::test]
    async fn exports_selected_projects_to_sqlite() {
        let source = migrated_db("sqlite::memory:").await;
        add_project(&source, "project-a", "Project A").await;
        add_project_with_tags(
            &source,
            "project-b",
            "Project B",
            vec!["billing".to_owned(), "critical".to_owned()],
        )
        .await;

        let path = std::env::temp_dir().join(format!(
            "previa-export-selected-{}.sqlite3",
            crate::server::utils::new_uuid_v7()
        ));
        let _cleanup = TempFileCleanup(path.clone());

        super::export_projects_to_sqlite(&source, &path, &["project-b".to_owned()], true)
            .await
            .expect("export selected projects");

        let exported = migrated_db(&sqlite_url(&path)).await;
        let projects = list_project_records(
            &exported,
            ProjectListQuery {
                limit: None,
                offset: None,
                order: None,
            },
        )
        .await
        .expect("list exported projects");

        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "Project B");
        assert_eq!(projects[0].tags, vec!["billing", "critical"]);
    }

    #[tokio::test]
    async fn imports_sqlite_projects_with_imported_suffix_on_name_conflict() {
        let source_path = std::env::temp_dir().join(format!(
            "previa-import-source-{}.sqlite3",
            crate::server::utils::new_uuid_v7()
        ));
        let _source_cleanup = TempFileCleanup(source_path.clone());
        let source = migrated_db(&sqlite_url(&source_path)).await;
        add_project(&source, "source-project", "Project A").await;

        let target = migrated_db("sqlite::memory:").await;
        add_project(&target, "target-project", "Project A").await;

        let result = super::import_projects_from_sqlite(&target, &source_path, true)
            .await
            .expect("import sqlite");

        assert_eq!(result.projects_imported, 1);
        assert_eq!(result.projects[0].project_name, "Project A-imported");

        let projects = list_project_records(
            &target,
            ProjectListQuery {
                limit: None,
                offset: None,
                order: None,
            },
        )
        .await
        .expect("list target projects");
        let names = projects
            .into_iter()
            .map(|project| project.name)
            .collect::<Vec<_>>();
        assert!(names.contains(&"Project A".to_owned()));
        assert!(names.contains(&"Project A-imported".to_owned()));
    }

    #[tokio::test]
    async fn imports_legacy_sqlite_projects_without_env_groups_table() {
        let source_path = std::env::temp_dir().join(format!(
            "previa-import-legacy-source-{}.sqlite3",
            crate::server::utils::new_uuid_v7()
        ));
        let _source_cleanup = TempFileCleanup(source_path.clone());
        let source = migrated_db(&sqlite_url(&source_path)).await;
        add_project(&source, "source-project", "Legacy Project").await;
        source
            .query("DROP TABLE project_env_groups")
            .execute(&source)
            .await
            .expect("drop env groups table");
        drop(source);

        let target = migrated_db("sqlite::memory:").await;
        let result = super::import_projects_from_sqlite(&target, &source_path, true)
            .await
            .expect("import legacy sqlite");

        assert_eq!(result.projects_imported, 1);
        assert_eq!(result.projects[0].project_name, "Legacy Project");
        assert_eq!(result.projects[0].env_groups_imported, 0);
    }

    struct TempFileCleanup(std::path::PathBuf);

    impl Drop for TempFileCleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}
