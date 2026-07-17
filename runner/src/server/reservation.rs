use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use chrono::Utc;
use tokio::sync::RwLock;

#[derive(Clone, Default)]
pub struct ReservationState {
    inner: Arc<ReservationInner>,
}

#[derive(Default)]
struct ReservationInner {
    busy: AtomicBool,
    started_execution_count: AtomicU64,
    last_started_at: RwLock<Option<String>>,
    last_finished_at: RwLock<Option<String>>,
}

#[derive(Debug, Clone)]
pub struct ReservationSnapshot {
    pub busy: bool,
    pub started_execution_count: u64,
    pub last_started_at: Option<String>,
    pub last_finished_at: Option<String>,
}

impl ReservationState {
    pub fn from_env() -> Self {
        Self::default()
    }

    pub async fn mark_execution_started(&self) {
        self.inner.busy.store(true, Ordering::SeqCst);
        self.inner
            .started_execution_count
            .fetch_add(1, Ordering::SeqCst);
        *self.inner.last_started_at.write().await = Some(Utc::now().to_rfc3339());
    }

    pub async fn mark_execution_finished(&self) {
        self.inner.busy.store(false, Ordering::SeqCst);
        *self.inner.last_finished_at.write().await = Some(Utc::now().to_rfc3339());
    }

    pub async fn snapshot(&self) -> ReservationSnapshot {
        ReservationSnapshot {
            busy: self.inner.busy.load(Ordering::SeqCst),
            started_execution_count: self.inner.started_execution_count.load(Ordering::SeqCst),
            last_started_at: self.inner.last_started_at.read().await.clone(),
            last_finished_at: self.inner.last_finished_at.read().await.clone(),
        }
    }

    pub fn is_ready(&self) -> bool {
        !self.inner.busy.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tracks_operational_execution_state() {
        let state = ReservationState::default();
        state.mark_execution_started().await;
        assert!(!state.is_ready());
        state.mark_execution_finished().await;

        assert!(state.is_ready());
        assert_eq!(state.snapshot().await.started_execution_count, 1);
    }
}
