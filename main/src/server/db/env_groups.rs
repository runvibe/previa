use std::collections::HashMap;

use previa_runner::RuntimeEnvGroup;
use sqlx::Row;

use crate::server::db::DbPool;
use crate::server::db::common::touch_project_updated_at;
use crate::server::models::{EnvGroupEntry, ProjectEnvGroupRecord, ProjectEnvGroupUpsertRequest};
use crate::server::utils::{new_uuid_v7, now_iso, now_ms};

pub fn project_env_group_from_row(row: &sqlx::any::AnyRow) -> ProjectEnvGroupRecord {
    let entries_json = row
        .try_get::<String, _>("entries_json")
        .unwrap_or_else(|_| "[]".to_owned());
    let entries = serde_json::from_str::<Vec<EnvGroupEntry>>(&entries_json).unwrap_or_default();

    ProjectEnvGroupRecord {
        id: row.try_get("id").unwrap_or_default(),
        project_id: row.try_get("project_id").unwrap_or_default(),
        slug: row.try_get("slug").unwrap_or_default(),
        name: row.try_get("name").unwrap_or_default(),
        entries,
        created_at: row.try_get("created_at").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
    }
}

pub async fn list_project_env_group_records(
    db: &DbPool,
    project_id: &str,
) -> Result<Vec<ProjectEnvGroupRecord>, sqlx::Error> {
    let rows = db
        .query(
            "SELECT id, project_id, slug, name, entries_json, created_at, updated_at
            FROM project_env_groups
            WHERE project_id = ?
            ORDER BY updated_at_ms DESC, id ASC",
        )
        .bind(project_id)
        .fetch_all(db)
        .await?;

    Ok(rows.iter().map(project_env_group_from_row).collect())
}

pub async fn load_project_env_group_record_by_id(
    db: &DbPool,
    project_id: &str,
    env_group_id: &str,
) -> Result<Option<ProjectEnvGroupRecord>, sqlx::Error> {
    let row = db
        .query(
            "SELECT id, project_id, slug, name, entries_json, created_at, updated_at
            FROM project_env_groups
            WHERE project_id = ? AND id = ?
            LIMIT 1",
        )
        .bind(project_id)
        .bind(env_group_id)
        .fetch_optional(db)
        .await?;

    Ok(row.as_ref().map(project_env_group_from_row))
}

pub async fn insert_project_env_group_record(
    db: &DbPool,
    project_id: &str,
    payload: ProjectEnvGroupUpsertRequest,
) -> Result<ProjectEnvGroupRecord, sqlx::Error> {
    let now_iso = now_iso();
    let now_ms_i64 = now_ms() as i64;
    let env_group_id = new_uuid_v7();
    let entries_json = serde_json::to_string(&payload.entries).unwrap_or_else(|_| "[]".to_owned());
    let mut tx = db.begin().await?;

    db.query(
        "INSERT INTO project_env_groups (
            id, project_id, slug, name, entries_json, created_at, updated_at, created_at_ms, updated_at_ms
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&env_group_id)
    .bind(project_id)
    .bind(&payload.slug)
    .bind(&payload.name)
    .bind(&entries_json)
    .bind(&now_iso)
    .bind(&now_iso)
    .bind(now_ms_i64)
    .bind(now_ms_i64)
    .execute(&mut *tx)
    .await?;

    touch_project_updated_at(db, &mut tx, project_id, &now_iso, now_ms_i64).await?;
    tx.commit().await?;

    load_project_env_group_record_by_id(db, project_id, &env_group_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn update_project_env_group_record(
    db: &DbPool,
    project_id: &str,
    env_group_id: &str,
    payload: ProjectEnvGroupUpsertRequest,
) -> Result<Option<ProjectEnvGroupRecord>, sqlx::Error> {
    let now_iso = now_iso();
    let now_ms_i64 = now_ms() as i64;
    let entries_json = serde_json::to_string(&payload.entries).unwrap_or_else(|_| "[]".to_owned());
    let mut tx = db.begin().await?;

    let result = db
        .query(
            "UPDATE project_env_groups SET
                slug = ?,
                name = ?,
                entries_json = ?,
                updated_at = ?,
                updated_at_ms = ?
            WHERE project_id = ? AND id = ?",
        )
        .bind(&payload.slug)
        .bind(&payload.name)
        .bind(&entries_json)
        .bind(&now_iso)
        .bind(now_ms_i64)
        .bind(project_id)
        .bind(env_group_id)
        .execute(&mut *tx)
        .await?;

    if result.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(None);
    }

    touch_project_updated_at(db, &mut tx, project_id, &now_iso, now_ms_i64).await?;
    tx.commit().await?;

    load_project_env_group_record_by_id(db, project_id, env_group_id).await
}

pub async fn delete_project_env_group_record(
    db: &DbPool,
    project_id: &str,
    env_group_id: &str,
) -> Result<bool, sqlx::Error> {
    let now_iso = now_iso();
    let now_ms_i64 = now_ms() as i64;
    let mut tx = db.begin().await?;
    let result = db
        .query("DELETE FROM project_env_groups WHERE project_id = ? AND id = ?")
        .bind(project_id)
        .bind(env_group_id)
        .execute(&mut *tx)
        .await?;

    if result.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(false);
    }

    touch_project_updated_at(db, &mut tx, project_id, &now_iso, now_ms_i64).await?;
    tx.commit().await?;
    Ok(true)
}

pub fn runtime_env_group_from_record(record: &ProjectEnvGroupRecord) -> Option<RuntimeEnvGroup> {
    let slug = record.slug.trim();
    if slug.is_empty() || slug == "current" {
        return None;
    }

    let mut urls = HashMap::new();
    for entry in &record.entries {
        let name = entry.name.trim();
        let url = entry.url.trim();
        if !name.is_empty() && !url.is_empty() {
            urls.insert(name.to_owned(), url.to_owned());
        }
    }

    if urls.is_empty() {
        return None;
    }

    Some(RuntimeEnvGroup {
        slug: slug.to_owned(),
        urls,
    })
}

#[cfg(test)]
mod tests {
    use crate::server::db::{
        DbPool, delete_project_env_group_record, insert_project_env_group_record,
        list_project_env_group_records, update_project_env_group_record, upsert_project_metadata,
    };
    use crate::server::models::{
        EnvGroupEntry, ProjectEnvGroupUpsertRequest, ProjectMetadataUpsertRequest,
    };

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

    async fn create_project(db: &DbPool) {
        upsert_project_metadata(
            db,
            "project-1".to_owned(),
            ProjectMetadataUpsertRequest {
                name: "Project".to_owned(),
                description: None,
                tags: Vec::new(),
            },
        )
        .await
        .expect("project");
    }

    fn payload(slug: &str, url: &str) -> ProjectEnvGroupUpsertRequest {
        ProjectEnvGroupUpsertRequest {
            slug: slug.to_owned(),
            name: slug.to_owned(),
            entries: vec![EnvGroupEntry {
                name: "api".to_owned(),
                url: url.to_owned(),
                description: None,
            }],
        }
    }

    #[tokio::test]
    async fn env_group_crud_roundtrip() {
        let db = db().await;
        create_project(&db).await;

        let created = insert_project_env_group_record(
            &db,
            "project-1",
            payload("local", "http://localhost:3000"),
        )
        .await
        .expect("insert");
        assert_eq!(created.slug, "local");

        let listed = list_project_env_group_records(&db, "project-1")
            .await
            .expect("list");
        assert_eq!(listed.len(), 1);

        let updated = update_project_env_group_record(
            &db,
            "project-1",
            &created.id,
            payload("hml", "https://api-hml.example.com"),
        )
        .await
        .expect("update")
        .expect("updated");
        assert_eq!(updated.slug, "hml");
        assert_eq!(updated.entries[0].url, "https://api-hml.example.com");

        let deleted = delete_project_env_group_record(&db, "project-1", &created.id)
            .await
            .expect("delete");
        assert!(deleted);
        let listed = list_project_env_group_records(&db, "project-1")
            .await
            .expect("list after delete");
        assert!(listed.is_empty());
    }
}
