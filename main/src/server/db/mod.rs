pub mod api_tokens;
pub mod common;
pub mod e2e_queues;
pub mod env_groups;
pub mod history;
pub mod pipelines;
pub mod pool;
pub mod projects;
pub mod query_utils;
pub mod runner_reservations;
pub mod runners;
pub mod specs;
pub mod transfers;
pub mod users;

pub use common::project_exists;
pub use e2e_queues::{
    cancel_non_terminal_e2e_queue, cancel_stale_e2e_queues, insert_e2e_queue,
    load_e2e_queue_record, queue_request_json, update_e2e_queue_item_status,
    update_e2e_queue_status,
};
pub use env_groups::{
    delete_project_env_group_record, insert_project_env_group_record,
    list_project_env_group_records, load_project_env_group_record_by_id,
    runtime_env_group_from_record, update_project_env_group_record,
};
pub use history::{
    list_e2e_history_records, list_load_history_records, load_e2e_history_record_by_id,
    load_load_history_record_by_id, save_e2e_history, save_load_history, upsert_e2e_history,
    upsert_load_history,
};
pub use pipelines::{
    delete_pipeline_record, insert_project_pipeline, load_existing_pipeline_ids,
    load_existing_project_pipeline_ids, load_pipelines_for_project,
    load_project_pipeline_for_execution, load_project_pipeline_record, update_project_pipeline,
};
pub use pool::{DatabaseKind, DbPool};
pub use projects::{
    create_project_with_pipelines, list_project_records, load_project_record, project_name_exists,
    upsert_project_metadata, upsert_project_with_pipelines,
};
pub use query_utils::{clamp_history_limit, clamp_history_offset, history_order_to_sql};
pub use runner_reservations::upsert_runner_reservation;
pub use runners::{
    delete_runner_record, list_enabled_runner_endpoints, list_runner_records, load_runner_record,
    mark_runner_observed, seed_env_runner_records, update_runner_record, upsert_runner_record,
};
pub use specs::{
    backfill_project_spec_md5_hashes, delete_project_spec_record, insert_project_spec_record,
    list_project_spec_records, load_project_spec_record_by_id, update_project_spec_record,
};
pub use transfers::{
    import_project_bundle, load_e2e_history_for_export, load_load_history_for_export,
    load_project_export,
};

#[cfg(test)]
mod runner_registry_tests {
    use crate::server::models::RunnerUpsertRequest;

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
    async fn env_seed_upserts_and_enables_runners() {
        let db = db().await;

        crate::server::db::seed_env_runner_records(
            &db,
            &[
                "http://runner-a:55880/".to_owned(),
                "runner-b:55880".to_owned(),
            ],
        )
        .await
        .expect("seed env runners");

        crate::server::db::update_runner_record(
            &db,
            "http://runner-a:55880",
            crate::server::models::RunnerUpdateRequest {
                name: None,
                enabled: Some(false),
            },
        )
        .await
        .expect("disable runner");
        crate::server::db::seed_env_runner_records(&db, &["http://runner-a:55880".to_owned()])
            .await
            .expect("seed env runner again");

        let endpoints = crate::server::db::list_enabled_runner_endpoints(&db)
            .await
            .expect("enabled runners");
        assert_eq!(
            endpoints,
            vec![
                "http://runner-a:55880".to_owned(),
                "http://runner-b:55880".to_owned()
            ]
        );

        let runners = crate::server::db::list_runner_records(&db)
            .await
            .expect("list runners");
        assert_eq!(runners.len(), 2);
        assert!(runners.iter().all(|runner| runner.enabled));
        assert!(runners.iter().all(|runner| runner.source == "env"));
    }

    #[tokio::test]
    async fn cli_runner_crud_updates_registry() {
        let db = db().await;

        let runner = crate::server::db::upsert_runner_record(
            &db,
            RunnerUpsertRequest {
                endpoint: "runner-c:55880/".to_owned(),
                name: Some("runner-c".to_owned()),
                enabled: Some(true),
            },
            "cli",
        )
        .await
        .expect("upsert runner");
        assert_eq!(runner.endpoint, "http://runner-c:55880");
        assert_eq!(runner.source, "cli");

        crate::server::db::mark_runner_observed(
            &db,
            &runner.endpoint,
            false,
            Some("connection refused"),
            None,
        )
        .await
        .expect("mark observed");
        let disabled = crate::server::db::update_runner_record(
            &db,
            &runner.endpoint,
            crate::server::models::RunnerUpdateRequest {
                name: None,
                enabled: Some(false),
            },
        )
        .await
        .expect("disable runner")
        .expect("runner exists");
        assert!(!disabled.enabled);
        assert_eq!(disabled.health_status, "unhealthy");
        assert_eq!(disabled.last_error.as_deref(), Some("connection refused"));

        let endpoints = crate::server::db::list_enabled_runner_endpoints(&db)
            .await
            .expect("enabled runners");
        assert!(endpoints.is_empty());

        assert!(
            crate::server::db::delete_runner_record(&db, &runner.endpoint)
                .await
                .expect("delete runner")
        );
    }
}
pub use api_tokens::{
    ApiTokenInsert, delete_api_token_record, insert_api_token_record, list_api_token_records,
    load_api_token_auth_record_by_hash, set_api_token_active, update_api_token_last_used,
};
pub use users::{
    UserInsert, UserUpdate, delete_user_record, insert_user_record, list_user_records,
    load_user_auth_record_by_username, update_user_record,
};
