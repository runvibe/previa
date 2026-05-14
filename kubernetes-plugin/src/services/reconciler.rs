use tokio_util::sync::CancellationToken;

use crate::services::reservations::ReservationStore;

#[derive(Clone)]
pub struct ReservationReconciler {
    store: ReservationStore,
    interval: std::time::Duration,
}

impl ReservationReconciler {
    pub fn new(store: ReservationStore, interval: std::time::Duration) -> Self {
        Self { store, interval }
    }

    pub async fn run(self, cancel: CancellationToken) {
        while !cancel.is_cancelled() {
            self.store.reconcile_all_once().await;
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(self.interval) => {}
            }
        }
    }
}

pub fn reconcile_interval_from_env() -> std::time::Duration {
    std::time::Duration::from_millis(
        std::env::var("PREVIA_RECONCILE_INTERVAL_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(1000),
    )
}
