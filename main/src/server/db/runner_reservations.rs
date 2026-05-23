use sqlx::Row;

use crate::server::db::DbPool;
use crate::server::models::{RunnerReservationRecord, RunnerReservationUpsert};
use crate::server::utils::now_iso;

pub async fn upsert_runner_reservation(
    db: &DbPool,
    input: RunnerReservationUpsert,
) -> Result<RunnerReservationRecord, sqlx::Error> {
    let now = now_iso();
    let endpoints_json =
        serde_json::to_string(&input.runner_endpoints).unwrap_or_else(|_| "[]".to_owned());

    db.query(
        "INSERT INTO runner_reservations (
            execution_id, pipeline_id, capacity_mode, requested_runner_count, ready_runner_count,
            target_rps, node_profile, reservation_id, reservation_token, reservation_expires_at,
            reservation_status, runner_endpoints_json, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(execution_id) DO UPDATE SET
            pipeline_id = excluded.pipeline_id,
            capacity_mode = excluded.capacity_mode,
            requested_runner_count = excluded.requested_runner_count,
            ready_runner_count = excluded.ready_runner_count,
            target_rps = excluded.target_rps,
            node_profile = excluded.node_profile,
            reservation_id = excluded.reservation_id,
            reservation_token = excluded.reservation_token,
            reservation_expires_at = excluded.reservation_expires_at,
            reservation_status = excluded.reservation_status,
            runner_endpoints_json = excluded.runner_endpoints_json,
            updated_at = excluded.updated_at",
    )
    .bind(&input.execution_id)
    .bind(&input.pipeline_id)
    .bind(&input.capacity_mode)
    .bind(input.requested_runner_count as i64)
    .bind(input.ready_runner_count as i64)
    .bind(input.target_rps as i64)
    .bind(&input.node_profile)
    .bind(&input.reservation_id)
    .bind(&input.reservation_token)
    .bind(&input.reservation_expires_at)
    .bind(&input.reservation_status)
    .bind(&endpoints_json)
    .bind(&now)
    .bind(&now)
    .execute(db)
    .await?;

    load_runner_reservation(db, &input.execution_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn load_runner_reservation(
    db: &DbPool,
    execution_id: &str,
) -> Result<Option<RunnerReservationRecord>, sqlx::Error> {
    let row = db
        .query(
            "SELECT execution_id, pipeline_id, capacity_mode, requested_runner_count,
                ready_runner_count, target_rps, node_profile, reservation_id, reservation_token,
                reservation_expires_at, reservation_status, runner_endpoints_json, created_at,
                updated_at
            FROM runner_reservations
            WHERE execution_id = ?
            LIMIT 1",
        )
        .bind(execution_id)
        .fetch_optional(db)
        .await?;

    Ok(row.as_ref().map(record_from_row))
}

pub async fn load_latest_runner_reservation_for_pipeline(
    db: &DbPool,
    pipeline_id: &str,
) -> Result<Option<RunnerReservationRecord>, sqlx::Error> {
    let row = db
        .query(
            "SELECT execution_id, pipeline_id, capacity_mode, requested_runner_count,
                ready_runner_count, target_rps, node_profile, reservation_id, reservation_token,
                reservation_expires_at, reservation_status, runner_endpoints_json, created_at,
                updated_at
            FROM runner_reservations
            WHERE pipeline_id = ?
            ORDER BY updated_at DESC, created_at DESC
            LIMIT 1",
        )
        .bind(pipeline_id)
        .fetch_optional(db)
        .await?;

    Ok(row.as_ref().map(record_from_row))
}

fn record_from_row(row: &sqlx::any::AnyRow) -> RunnerReservationRecord {
    let endpoints_json = row
        .try_get::<String, _>("runner_endpoints_json")
        .unwrap_or_else(|_| "[]".to_owned());
    let runner_endpoints = serde_json::from_str::<Vec<String>>(&endpoints_json).unwrap_or_default();

    RunnerReservationRecord {
        execution_id: row.try_get("execution_id").unwrap_or_default(),
        pipeline_id: row.try_get("pipeline_id").ok().flatten(),
        capacity_mode: row.try_get("capacity_mode").unwrap_or_default(),
        requested_runner_count: row
            .try_get::<i64, _>("requested_runner_count")
            .unwrap_or_default()
            .max(0) as usize,
        ready_runner_count: row
            .try_get::<i64, _>("ready_runner_count")
            .unwrap_or_default()
            .max(0) as usize,
        target_rps: row
            .try_get::<i64, _>("target_rps")
            .unwrap_or_default()
            .max(0) as u64,
        node_profile: row.try_get("node_profile").ok().flatten(),
        reservation_id: row.try_get("reservation_id").ok().flatten(),
        reservation_token: row.try_get("reservation_token").ok().flatten(),
        reservation_expires_at: row.try_get("reservation_expires_at").ok().flatten(),
        reservation_status: row.try_get("reservation_status").unwrap_or_default(),
        runner_endpoints,
        created_at: row.try_get("created_at").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        load_latest_runner_reservation_for_pipeline, load_runner_reservation,
        upsert_runner_reservation,
    };
    use crate::server::models::RunnerReservationUpsert;

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
    async fn runner_reservation_roundtrips_secret_and_endpoints() {
        let db = db().await;

        let stored = upsert_runner_reservation(
            &db,
            RunnerReservationUpsert {
                execution_id: "exec-1".to_owned(),
                pipeline_id: Some("pipe-1".to_owned()),
                capacity_mode: "kubernetes".to_owned(),
                requested_runner_count: 2,
                ready_runner_count: 1,
                target_rps: 10_000,
                node_profile: Some("small".to_owned()),
                reservation_id: Some("rr-1".to_owned()),
                reservation_token: Some("secret".to_owned()),
                reservation_expires_at: Some("2026-05-13T00:00:00Z".to_owned()),
                reservation_status: "provisioning".to_owned(),
                runner_endpoints: vec!["http://10.0.0.1:55880".to_owned()],
            },
        )
        .await
        .expect("upsert reservation");

        assert_eq!(stored.execution_id, "exec-1");
        assert_eq!(stored.reservation_token.as_deref(), Some("secret"));
        assert_eq!(stored.runner_endpoints, vec!["http://10.0.0.1:55880"]);

        let loaded = load_runner_reservation(&db, "exec-1")
            .await
            .expect("load reservation")
            .expect("reservation exists");
        assert_eq!(loaded, stored);
    }

    #[tokio::test]
    async fn latest_runner_reservation_for_pipeline_returns_newest_record() {
        let db = db().await;

        upsert_runner_reservation(
            &db,
            RunnerReservationUpsert {
                execution_id: "exec-old".to_owned(),
                pipeline_id: Some("pipe-1".to_owned()),
                capacity_mode: "kubernetes".to_owned(),
                requested_runner_count: 2,
                ready_runner_count: 1,
                target_rps: 1_000,
                node_profile: Some("4gn.nano".to_owned()),
                reservation_id: Some("rr-old".to_owned()),
                reservation_token: Some("secret-old".to_owned()),
                reservation_expires_at: Some("2026-05-14T10:00:00Z".to_owned()),
                reservation_status: "provisioning".to_owned(),
                runner_endpoints: vec!["http://10.0.0.1:55880".to_owned()],
            },
        )
        .await
        .expect("insert old reservation");

        tokio::time::sleep(std::time::Duration::from_millis(2)).await;

        upsert_runner_reservation(
            &db,
            RunnerReservationUpsert {
                execution_id: "exec-new".to_owned(),
                pipeline_id: Some("pipe-1".to_owned()),
                capacity_mode: "kubernetes".to_owned(),
                requested_runner_count: 3,
                ready_runner_count: 2,
                target_rps: 2_500,
                node_profile: Some("4gn.nano".to_owned()),
                reservation_id: Some("rr-new".to_owned()),
                reservation_token: Some("secret-new".to_owned()),
                reservation_expires_at: Some("2026-05-14T10:05:00Z".to_owned()),
                reservation_status: "provisioning".to_owned(),
                runner_endpoints: vec![
                    "http://10.0.0.2:55880".to_owned(),
                    "http://10.0.0.3:55880".to_owned(),
                ],
            },
        )
        .await
        .expect("insert new reservation");

        let loaded = load_latest_runner_reservation_for_pipeline(&db, "pipe-1")
            .await
            .expect("load latest reservation")
            .expect("reservation exists");

        assert_eq!(loaded.execution_id, "exec-new");
        assert_eq!(loaded.reservation_id.as_deref(), Some("rr-new"));
        assert_eq!(loaded.requested_runner_count, 3);
        assert_eq!(loaded.ready_runner_count, 2);
        assert_eq!(loaded.reservation_token.as_deref(), Some("secret-new"));
    }

    #[tokio::test]
    async fn latest_runner_reservation_for_pipeline_returns_none_without_record() {
        let db = db().await;

        let loaded = load_latest_runner_reservation_for_pipeline(&db, "pipe-missing")
            .await
            .expect("load missing reservation");

        assert!(loaded.is_none());
    }
}
