use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::repository::{RunnerIdentity, RunnerQueueRepository};

pub async fn run_heartbeat(
    repository: RunnerQueueRepository,
    identity: RunnerIdentity,
    interval: Duration,
    cancel: CancellationToken,
) -> Result<(), String> {
    let mut ticker = tokio::time::interval(interval);
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = repository.heartbeat(&identity, "stopped").await;
                return Ok(());
            }
            _ = ticker.tick() => {
                repository.heartbeat(&identity, "ready").await?;
            }
        }
    }
}
