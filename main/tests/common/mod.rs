use previa_main::server::queue::repository::QueueRepository;

pub async fn migrated_queue_repository(
    database_url: &str,
    max_connections: u32,
) -> QueueRepository {
    let repository = QueueRepository::connect(database_url, max_connections)
        .await
        .expect("connect queue repository");
    sqlx::migrate!("./migrations/postgres")
        .run(repository.pool())
        .await
        .expect("migrate queue database");
    repository
}
