use crate::server::db::{DatabaseKind, DbPool};
use chrono::DateTime;
use serde_json::Value;
use sqlx::Row;

use super::env_groups::list_project_env_group_records;
use super::pipelines::load_pipelines_for_project;
use super::specs::list_project_spec_records;
use crate::server::models::{
    E2eHistoryRecord, LoadHistoryRecord, ProjectExportProject, ProjectHistoryExport,
    ProjectImportResponse,
};
use crate::server::utils::{new_uuid_v7, now_ms};

fn tags_from_row(row: &sqlx::any::AnyRow) -> Vec<String> {
    row.try_get::<String, _>("tags_json")
        .ok()
        .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
        .unwrap_or_default()
}

pub async fn load_project_export(
    db: &DbPool,
    project_id: &str,
) -> Result<Option<ProjectExportProject>, sqlx::Error> {
    let row = db
        .query(
            "SELECT id, name, description, tags_json, created_at, updated_at, spec_json
        FROM projects
        WHERE id = ?",
        )
        .bind(project_id)
        .fetch_optional(db)
        .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let spec_json = row.try_get::<Option<String>, _>("spec_json").ok().flatten();
    let spec = spec_json.and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let pipelines = load_pipelines_for_project(db, project_id).await?;
    let specs = list_project_spec_records(db, project_id).await?;
    let env_groups = if table_exists(db, "project_env_groups").await? {
        list_project_env_group_records(db, project_id).await?
    } else {
        Vec::new()
    };

    Ok(Some(ProjectExportProject {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or_default(),
        description: row.try_get("description").ok(),
        tags: tags_from_row(&row),
        created_at: row.try_get("created_at").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
        spec,
        pipelines,
        specs,
        env_groups,
        history: ProjectHistoryExport {
            e2e: Vec::new(),
            load: Vec::new(),
        },
    }))
}

async fn table_exists(db: &DbPool, table_name: &str) -> Result<bool, sqlx::Error> {
    match db.kind() {
        DatabaseKind::Sqlite => {
            let row = db
                .query("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ? LIMIT 1")
                .bind(table_name)
                .fetch_optional(db)
                .await?;
            Ok(row.is_some())
        }
        DatabaseKind::Postgres => {
            let row = db
                .query("SELECT to_regclass(?) IS NOT NULL AS present")
                .bind(table_name)
                .fetch_one(db)
                .await?;
            Ok(row.try_get::<bool, _>("present").unwrap_or(false))
        }
    }
}

pub async fn load_e2e_history_for_export(
    db: &DbPool,
    project_id: &str,
) -> Result<Vec<E2eHistoryRecord>, sqlx::Error> {
    let rows = db
        .query(
            "SELECT id, execution_id, transaction_id, project_id, pipeline_index, pipeline_id,
            pipeline_name, selected_base_url_key, status, started_at_ms, finished_at_ms,
            duration_ms, summary_json, steps_json, errors_json, request_json
        FROM integration_history
        WHERE project_id = ?
        ORDER BY finished_at_ms DESC",
        )
        .bind(project_id)
        .fetch_all(db)
        .await?;

    let mut records = Vec::with_capacity(rows.len());
    for row in rows {
        let summary_json = row
            .try_get::<Option<String>, _>("summary_json")
            .ok()
            .flatten();
        let steps_json = row
            .try_get::<String, _>("steps_json")
            .unwrap_or_else(|_| "[]".to_owned());
        let errors_json = row
            .try_get::<String, _>("errors_json")
            .unwrap_or_else(|_| "[]".to_owned());
        let request_json = row
            .try_get::<String, _>("request_json")
            .unwrap_or_else(|_| "{}".to_owned());

        records.push(E2eHistoryRecord {
            id: row.try_get("id").unwrap_or_default(),
            execution_id: row.try_get("execution_id").unwrap_or_default(),
            transaction_id: row.try_get("transaction_id").ok(),
            project_id: row.try_get("project_id").ok(),
            pipeline_index: row.try_get("pipeline_index").ok(),
            pipeline_id: row.try_get("pipeline_id").ok(),
            pipeline_name: row.try_get("pipeline_name").unwrap_or_default(),
            selected_base_url_key: row.try_get("selected_base_url_key").ok(),
            status: row.try_get("status").unwrap_or_default(),
            started_at_ms: row.try_get("started_at_ms").unwrap_or_default(),
            finished_at_ms: row.try_get("finished_at_ms").unwrap_or_default(),
            duration_ms: row.try_get("duration_ms").unwrap_or_default(),
            summary: summary_json.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()),
            steps: serde_json::from_str::<Vec<Value>>(&steps_json).unwrap_or_default(),
            errors: serde_json::from_str::<Vec<String>>(&errors_json).unwrap_or_default(),
            request: serde_json::from_str::<Value>(&request_json).unwrap_or(Value::Null),
        });
    }

    Ok(records)
}

pub async fn load_load_history_for_export(
    db: &DbPool,
    project_id: &str,
) -> Result<Vec<LoadHistoryRecord>, sqlx::Error> {
    let rows = db
        .query(
            "SELECT id, execution_id, transaction_id, project_id, pipeline_index, pipeline_id,
            pipeline_name, selected_base_url_key, status, started_at_ms, finished_at_ms,
            duration_ms, requested_config_json, final_consolidated_json, final_lines_json,
            errors_json, request_json, context_json
        FROM load_history
        WHERE project_id = ?
        ORDER BY finished_at_ms DESC",
        )
        .bind(project_id)
        .fetch_all(db)
        .await?;

    let mut records = Vec::with_capacity(rows.len());
    for row in rows {
        let requested_config_json = row
            .try_get::<String, _>("requested_config_json")
            .unwrap_or_else(|_| "{}".to_owned());
        let final_consolidated_json = row
            .try_get::<Option<String>, _>("final_consolidated_json")
            .ok()
            .flatten();
        let final_lines_json = row
            .try_get::<String, _>("final_lines_json")
            .unwrap_or_else(|_| "[]".to_owned());
        let errors_json = row
            .try_get::<String, _>("errors_json")
            .unwrap_or_else(|_| "[]".to_owned());
        let request_json = row
            .try_get::<String, _>("request_json")
            .unwrap_or_else(|_| "{}".to_owned());
        let context_json = row
            .try_get::<String, _>("context_json")
            .unwrap_or_else(|_| "{}".to_owned());

        records.push(LoadHistoryRecord {
            id: row.try_get("id").unwrap_or_default(),
            execution_id: row.try_get("execution_id").unwrap_or_default(),
            transaction_id: row.try_get("transaction_id").ok(),
            project_id: row.try_get("project_id").ok(),
            pipeline_index: row.try_get("pipeline_index").ok(),
            pipeline_id: row.try_get("pipeline_id").ok(),
            pipeline_name: row.try_get("pipeline_name").unwrap_or_default(),
            selected_base_url_key: row.try_get("selected_base_url_key").ok(),
            status: row.try_get("status").unwrap_or_default(),
            started_at_ms: row.try_get("started_at_ms").unwrap_or_default(),
            finished_at_ms: row.try_get("finished_at_ms").unwrap_or_default(),
            duration_ms: row.try_get("duration_ms").unwrap_or_default(),
            requested_config: serde_json::from_str::<Value>(&requested_config_json)
                .unwrap_or(Value::Null),
            final_consolidated: final_consolidated_json
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok()),
            final_lines: serde_json::from_str::<Vec<Value>>(&final_lines_json).unwrap_or_default(),
            errors: serde_json::from_str::<Vec<String>>(&errors_json).unwrap_or_default(),
            request: serde_json::from_str::<Value>(&request_json).unwrap_or(Value::Null),
            context: serde_json::from_str::<Value>(&context_json).unwrap_or(Value::Null),
        });
    }

    Ok(records)
}

pub async fn import_project_bundle(
    db: &DbPool,
    project: &ProjectExportProject,
    include_history: bool,
) -> Result<ProjectImportResponse, sqlx::Error> {
    let mut tx = db.begin().await?;
    let now_ms_i64 = now_ms() as i64;
    let created_at_ms = parse_iso_to_ms(&project.created_at).unwrap_or(now_ms_i64);
    let updated_at_ms = parse_iso_to_ms(&project.updated_at).unwrap_or(now_ms_i64);

    db.query(
        "INSERT INTO projects (
            id, name, description, tags_json, created_at, updated_at, created_at_ms, updated_at_ms, spec_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&project.id)
    .bind(&project.name)
    .bind(&project.description)
    .bind(serde_json::to_string(&project.tags).unwrap_or_else(|_| "[]".to_owned()))
    .bind(&project.created_at)
    .bind(&project.updated_at)
    .bind(created_at_ms)
    .bind(updated_at_ms)
    .bind(project.spec.as_ref().map(Value::to_string))
    .execute(&mut *tx)
    .await?;

    for (index, pipeline) in project.pipelines.iter().enumerate() {
        let mut pipeline_to_store = pipeline.clone();
        if pipeline_to_store
            .id
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            pipeline_to_store.id = Some(new_uuid_v7());
        }
        let pipeline_id = pipeline_to_store.id.clone().unwrap_or_else(new_uuid_v7);

        db.query(
            "INSERT INTO pipelines (
                id, project_id, position, name, description, created_at, updated_at, pipeline_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(pipeline_id)
        .bind(&project.id)
        .bind(index as i64)
        .bind(&pipeline_to_store.name)
        .bind(&pipeline_to_store.description)
        .bind(&project.created_at)
        .bind(&project.updated_at)
        .bind(serde_json::to_string(&pipeline_to_store).unwrap_or_else(|_| "{}".to_owned()))
        .execute(&mut *tx)
        .await?;
    }

    for spec in &project.specs {
        let spec_id = if spec.id.trim().is_empty() {
            new_uuid_v7()
        } else {
            spec.id.clone()
        };
        let spec_json = spec.spec.to_string();
        let spec_md5 = format!("{:x}", md5::compute(spec_json.as_bytes()));
        let urls_json = serde_json::to_string(&spec.urls).unwrap_or_else(|_| "[]".to_owned());
        let spec_created_at_ms = parse_iso_to_ms(&spec.created_at).unwrap_or(now_ms_i64);
        let spec_updated_at_ms = parse_iso_to_ms(&spec.updated_at).unwrap_or(now_ms_i64);

        db.query(
            "INSERT INTO project_openapi_specs (
                id, project_id, spec_json, spec_md5, url, slug, urls_json, sync, live, created_at,
                updated_at, created_at_ms, updated_at_ms
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(spec_id)
        .bind(&project.id)
        .bind(spec_json)
        .bind(spec_md5)
        .bind(&spec.url)
        .bind(&spec.slug)
        .bind(urls_json)
        .bind(if spec.sync { 1i64 } else { 0i64 })
        .bind(if spec.live { 1i64 } else { 0i64 })
        .bind(&spec.created_at)
        .bind(&spec.updated_at)
        .bind(spec_created_at_ms)
        .bind(spec_updated_at_ms)
        .execute(&mut *tx)
        .await?;
    }

    for group in &project.env_groups {
        let group_id = if group.id.trim().is_empty() {
            new_uuid_v7()
        } else {
            group.id.clone()
        };
        let entries_json =
            serde_json::to_string(&group.entries).unwrap_or_else(|_| "[]".to_owned());
        let group_created_at_ms = parse_iso_to_ms(&group.created_at).unwrap_or(now_ms_i64);
        let group_updated_at_ms = parse_iso_to_ms(&group.updated_at).unwrap_or(now_ms_i64);

        db.query(
            "INSERT INTO project_env_groups (
                id, project_id, slug, name, entries_json, created_at, updated_at,
                created_at_ms, updated_at_ms
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(group_id)
        .bind(&project.id)
        .bind(&group.slug)
        .bind(&group.name)
        .bind(entries_json)
        .bind(&group.created_at)
        .bind(&group.updated_at)
        .bind(group_created_at_ms)
        .bind(group_updated_at_ms)
        .execute(&mut *tx)
        .await?;
    }

    let mut e2e_history_imported = 0usize;
    let mut load_history_imported = 0usize;

    if include_history {
        for record in &project.history.e2e {
            let record_id = if record.id.trim().is_empty() {
                new_uuid_v7()
            } else {
                record.id.clone()
            };
            db.query(
                "INSERT INTO integration_history (
                    id, execution_id, transaction_id, project_id, pipeline_index, pipeline_id, pipeline_name,
                    selected_base_url_key, status, started_at_ms, finished_at_ms, duration_ms,
                    summary_json, steps_json, errors_json, request_json
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(record_id)
            .bind(&record.execution_id)
            .bind(&record.transaction_id)
            .bind(&project.id)
            .bind(record.pipeline_index)
            .bind(&record.pipeline_id)
            .bind(&record.pipeline_name)
            .bind(&record.selected_base_url_key)
            .bind(&record.status)
            .bind(record.started_at_ms)
            .bind(record.finished_at_ms)
            .bind(record.duration_ms)
            .bind(record.summary.as_ref().map(Value::to_string))
            .bind(serde_json::to_string(&record.steps).unwrap_or_else(|_| "[]".to_owned()))
            .bind(serde_json::to_string(&record.errors).unwrap_or_else(|_| "[]".to_owned()))
            .bind(record.request.to_string())
            .execute(&mut *tx)
            .await?;
            e2e_history_imported += 1;
        }

        for record in &project.history.load {
            let record_id = if record.id.trim().is_empty() {
                new_uuid_v7()
            } else {
                record.id.clone()
            };
            db.query(
                "INSERT INTO load_history (
                    id, execution_id, transaction_id, project_id, pipeline_index, pipeline_id, pipeline_name,
                    selected_base_url_key, status, started_at_ms, finished_at_ms, duration_ms,
                    requested_config_json, final_consolidated_json, final_lines_json, errors_json,
                    request_json, context_json
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(record_id)
            .bind(&record.execution_id)
            .bind(&record.transaction_id)
            .bind(&project.id)
            .bind(record.pipeline_index)
            .bind(&record.pipeline_id)
            .bind(&record.pipeline_name)
            .bind(&record.selected_base_url_key)
            .bind(&record.status)
            .bind(record.started_at_ms)
            .bind(record.finished_at_ms)
            .bind(record.duration_ms)
            .bind(record.requested_config.to_string())
            .bind(record.final_consolidated.as_ref().map(Value::to_string))
            .bind(serde_json::to_string(&record.final_lines).unwrap_or_else(|_| "[]".to_owned()))
            .bind(serde_json::to_string(&record.errors).unwrap_or_else(|_| "[]".to_owned()))
            .bind(record.request.to_string())
            .bind(record.context.to_string())
            .execute(&mut *tx)
            .await?;
            load_history_imported += 1;
        }
    }

    tx.commit().await?;

    Ok(ProjectImportResponse {
        project_id: project.id.clone(),
        include_history,
        pipelines_imported: project.pipelines.len(),
        specs_imported: project.specs.len(),
        env_groups_imported: project.env_groups.len(),
        e2e_history_imported,
        load_history_imported,
    })
}

fn parse_iso_to_ms(raw: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|value| value.timestamp_millis())
}
