use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::config::MainQueueConfig;
use super::projector::QueueProjector;
use super::repository::QueueRepository;
use super::retention::delete_expired_queue_data;

pub struct QueueRuntime {
    cancel: CancellationToken,
}

impl QueueRuntime {
    pub fn start(repository: QueueRepository, config: MainQueueConfig) -> Self {
        let cancel = CancellationToken::new();
        let projector_cancel = cancel.child_token();
        let projector_repository = repository.clone();
        let projector_config = config.clone();
        tokio::spawn(async move {
            let projector = QueueProjector::new(projector_repository, Uuid::now_v7());
            let mut interval = tokio::time::interval(projector_config.projection_poll_interval);
            loop {
                tokio::select! {
                    _ = projector_cancel.cancelled() => return,
                    _ = interval.tick() => {
                        if let Err(error) = projector
                            .project_once(projector_config.projection_lease)
                            .await
                        {
                            tracing::error!("queue projection failed: {error}");
                        }
                    }
                }
            }
        });

        let maintenance_cancel = cancel.child_token();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(config.maintenance_interval);
            loop {
                tokio::select! {
                    _ = maintenance_cancel.cancelled() => return,
                    _ = interval.tick() => {
                        if let Err(error) = repository
                            .reap_expired_jobs(
                                config.retry_backoff_base,
                                config.retry_backoff_max,
                            )
                            .await
                        {
                            tracing::error!("queue reaper failed: {error}");
                        }
                        if let Err(error) = delete_expired_queue_data(
                            &repository,
                            config.event_retention,
                            config.runner_retention,
                        )
                        .await
                        {
                            tracing::error!("queue retention failed: {error}");
                        }
                    }
                }
            }
        });
        Self { cancel }
    }
}

impl Drop for QueueRuntime {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
