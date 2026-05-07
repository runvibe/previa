use crate::server::db::DbPool;
use previa_runner::Pipeline;
use serde_json::Value;
use sqlx::{QueryBuilder, Row};

use crate::server::db::query_utils::{
    clamp_history_limit, clamp_history_offset, history_order_to_sql,
};
use crate::server::models::{
    ProjectListQuery, ProjectMetadataUpsertRequest, ProjectRecord, ProjectUpsertRequest,
};
use crate::server::utils::{new_uuid_v7, now_iso, now_ms};

fn tags_to_json(tags: &[String]) -> String {
    serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_owned())
}

fn tags_from_row(row: &sqlx::any::AnyRow) -> Vec<String> {
    row.try_get::<String, _>("tags_json")
        .ok()
        .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
        .unwrap_or_default()
}

pub async fn list_project_records(
    db: &DbPool,
    query: ProjectListQuery,
) -> Result<Vec<ProjectRecord>, sqlx::Error> {
    let limit = clamp_history_limit(query.limit);
    let offset = clamp_history_offset(query.offset);
    let order_sql = history_order_to_sql(query.order);

    let mut qb = QueryBuilder::<sqlx::Any>::new(
        "SELECT id, name, description, tags_json, created_at, updated_at FROM projects ORDER BY updated_at_ms ",
    );
    qb.push(order_sql)
        .push(" LIMIT ")
        .push_bind(limit as i64)
        .push(" OFFSET ")
        .push_bind(offset as i64);

    let rows = qb.build().fetch_all(db).await?;
    let mut projects = Vec::with_capacity(rows.len());
    for row in rows {
        let description = row
            .try_get::<Option<String>, _>("description")
            .ok()
            .flatten();
        projects.push(ProjectRecord {
            id: row.try_get("id").unwrap_or_default(),
            name: row.try_get("name").unwrap_or_default(),
            description,
            tags: tags_from_row(&row),
            created_at: row.try_get("created_at").unwrap_or_default(),
            updated_at: row.try_get("updated_at").unwrap_or_default(),
        });
    }

    Ok(projects)
}

pub async fn load_project_record(
    db: &DbPool,
    project_id: &str,
) -> Result<Option<ProjectRecord>, sqlx::Error> {
    let row = db
        .query("SELECT id, name, description, tags_json, created_at, updated_at FROM projects WHERE id = ?")
        .bind(project_id)
        .fetch_optional(db)
        .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let description = row
        .try_get::<Option<String>, _>("description")
        .ok()
        .flatten();

    Ok(Some(ProjectRecord {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or_default(),
        description,
        tags: tags_from_row(&row),
        created_at: row.try_get("created_at").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
    }))
}

pub async fn project_name_exists(db: &DbPool, project_name: &str) -> Result<bool, sqlx::Error> {
    let row = sqlx::query_scalar::<sqlx::Any, i64>(
        db.sql("SELECT 1 FROM projects WHERE name = ? LIMIT 1"),
    )
    .bind(project_name)
    .fetch_optional(db)
    .await?;
    Ok(row.is_some())
}

pub async fn upsert_project_metadata(
    db: &DbPool,
    project_id: String,
    payload: ProjectMetadataUpsertRequest,
) -> Result<ProjectRecord, sqlx::Error> {
    let now_iso = now_iso();
    let now_ms_i64 = now_ms() as i64;
    let mut tx = db.begin().await?;

    let existing = db
        .query("SELECT created_at, created_at_ms FROM projects WHERE id = ?")
        .bind(&project_id)
        .fetch_optional(&mut *tx)
        .await?;
    let created_at = existing
        .as_ref()
        .and_then(|row| row.try_get::<String, _>("created_at").ok())
        .unwrap_or_else(|| now_iso.clone());
    let created_at_ms = existing
        .as_ref()
        .and_then(|row| row.try_get::<i64, _>("created_at_ms").ok())
        .unwrap_or(now_ms_i64);

    db.query(
        "INSERT INTO projects (
            id, name, description, tags_json, created_at, updated_at, created_at_ms, updated_at_ms
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            description = excluded.description,
            tags_json = excluded.tags_json,
            updated_at = excluded.updated_at,
            updated_at_ms = excluded.updated_at_ms",
    )
    .bind(&project_id)
    .bind(&payload.name)
    .bind(&payload.description)
    .bind(tags_to_json(&payload.tags))
    .bind(&created_at)
    .bind(&now_iso)
    .bind(created_at_ms)
    .bind(now_ms_i64)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    load_project_record(db, &project_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn upsert_project_with_pipelines(
    db: &DbPool,
    project_id: String,
    payload: ProjectUpsertRequest,
) -> Result<ProjectRecord, sqlx::Error> {
    let now_iso = now_iso();
    let now_ms_i64 = now_ms() as i64;
    let mut tx = db.begin().await?;

    let existing = db
        .query("SELECT created_at, created_at_ms FROM projects WHERE id = ?")
        .bind(&project_id)
        .fetch_optional(&mut *tx)
        .await?;
    let created_at = payload.created_at.clone().unwrap_or_else(|| {
        existing
            .as_ref()
            .and_then(|row| row.try_get::<String, _>("created_at").ok())
            .unwrap_or_else(|| now_iso.clone())
    });
    let created_at_ms = existing
        .as_ref()
        .and_then(|row| row.try_get::<i64, _>("created_at_ms").ok())
        .unwrap_or(now_ms_i64);
    let updated_at = payload
        .updated_at
        .clone()
        .unwrap_or_else(|| now_iso.clone());

    db.query(
        "INSERT INTO projects (
            id, name, description, tags_json, created_at, updated_at, created_at_ms, updated_at_ms, spec_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            description = excluded.description,
            tags_json = excluded.tags_json,
            updated_at = excluded.updated_at,
            updated_at_ms = excluded.updated_at_ms,
            spec_json = excluded.spec_json",
    )
    .bind(&project_id)
    .bind(&payload.name)
    .bind(&payload.description)
    .bind(tags_to_json(&payload.tags))
    .bind(&created_at)
    .bind(&updated_at)
    .bind(created_at_ms)
    .bind(now_ms_i64)
    .bind(payload.spec.as_ref().map(Value::to_string))
    .execute(&mut *tx)
    .await?;

    db.query("DELETE FROM pipelines WHERE project_id = ?")
        .bind(&project_id)
        .execute(&mut *tx)
        .await?;

    for (index, pipeline_input) in payload.pipelines.into_iter().enumerate() {
        let pipeline_id = new_uuid_v7();
        let pipeline = Pipeline {
            id: Some(pipeline_id.clone()),
            name: pipeline_input.name,
            description: pipeline_input.description,
            steps: pipeline_input.steps,
        };

        db.query(
            "INSERT INTO pipelines (
                id, project_id, position, name, description, created_at, updated_at, pipeline_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(pipeline_id)
        .bind(&project_id)
        .bind(index as i64)
        .bind(&pipeline.name)
        .bind(&pipeline.description)
        .bind(&now_iso)
        .bind(&updated_at)
        .bind(serde_json::to_string(&pipeline).unwrap_or_else(|_| "{}".to_owned()))
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    load_project_record(db, &project_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn create_project_with_pipelines(
    db: &DbPool,
    project_name: String,
    pipelines: Vec<Pipeline>,
) -> Result<ProjectRecord, sqlx::Error> {
    let project_id = new_uuid_v7();
    let now_iso = now_iso();
    let now_ms_i64 = now_ms() as i64;
    let mut tx = db.begin().await?;

    db.query(
        "INSERT INTO projects (
            id, name, description, tags_json, created_at, updated_at, created_at_ms, updated_at_ms, spec_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&project_id)
    .bind(&project_name)
    .bind(Option::<String>::None)
    .bind("[]")
    .bind(&now_iso)
    .bind(&now_iso)
    .bind(now_ms_i64)
    .bind(now_ms_i64)
    .bind(Option::<String>::None)
    .execute(&mut *tx)
    .await?;

    for (index, mut pipeline) in pipelines.into_iter().enumerate() {
        let pipeline_id = pipeline.id.clone().unwrap_or_else(new_uuid_v7);
        pipeline.id = Some(pipeline_id.clone());

        db.query(
            "INSERT INTO pipelines (
                id, project_id, position, name, description, created_at, updated_at, pipeline_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(pipeline_id)
        .bind(&project_id)
        .bind(index as i64)
        .bind(&pipeline.name)
        .bind(&pipeline.description)
        .bind(&now_iso)
        .bind(&now_iso)
        .bind(serde_json::to_string(&pipeline).unwrap_or_else(|_| "{}".to_owned()))
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    load_project_record(db, &project_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> crate::server::db::DbPool {
        let db = crate::server::db::DbPool::connect("sqlite::memory:", 1)
            .await
            .expect("sqlite memory db");
        sqlx::migrate!("./migrations/sqlite")
            .run(db.pool())
            .await
            .expect("migrations");
        db
    }

    #[tokio::test]
    async fn project_records_round_trip_tags() {
        let db = db().await;

        let project = upsert_project_metadata(
            &db,
            "project-1".to_owned(),
            ProjectMetadataUpsertRequest {
                name: "Payments".to_owned(),
                description: Some("Checkout".to_owned()),
                tags: vec!["billing".to_owned(), "critical".to_owned()],
            },
        )
        .await
        .expect("upsert project");

        assert_eq!(project.tags, vec!["billing", "critical"]);

        let loaded = load_project_record(&db, "project-1")
            .await
            .expect("load project")
            .expect("project exists");
        assert_eq!(loaded.tags, vec!["billing", "critical"]);
    }
}
