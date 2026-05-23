use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

const RESERVATION_ID_HEADER: &str = "x-previa-reservation-id";
const RESERVATION_TOKEN_HEADER: &str = "x-previa-reservation-token";

#[derive(Clone, Default)]
pub struct ReservationState {
    inner: Arc<ReservationInner>,
}

#[derive(Default)]
struct ReservationInner {
    gate: StdRwLock<ReservationGate>,
    consumed: AtomicBool,
    busy: AtomicBool,
    started_execution_count: AtomicU64,
    last_started_at: RwLock<Option<String>>,
    last_finished_at: RwLock<Option<String>>,
}

#[derive(Default)]
struct ReservationGate {
    reservation_id: Option<String>,
    reservation_token: Option<String>,
    expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct ReservationSnapshot {
    pub busy: bool,
    pub started_execution_count: u64,
    pub last_started_at: Option<String>,
    pub last_finished_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReservationError {
    MissingHeaders,
    InvalidReservation,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReservationRearmError {
    Busy,
    ActiveReservation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReservationReleaseError {
    Busy,
}

impl ReservationState {
    pub fn from_env() -> Self {
        let reservation_id = optional_env("PREVIA_RESERVATION_ID");
        let reservation_token = optional_env("PREVIA_RESERVATION_TOKEN");
        let expires_at = optional_env("PREVIA_RESERVATION_EXPIRES_AT")
            .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
            .map(|value| value.with_timezone(&Utc));

        Self::new(reservation_id, reservation_token, expires_at)
    }

    pub fn new(
        reservation_id: Option<String>,
        reservation_token: Option<String>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            inner: Arc::new(ReservationInner {
                gate: StdRwLock::new(ReservationGate {
                    reservation_id,
                    reservation_token,
                    expires_at,
                }),
                consumed: AtomicBool::new(false),
                busy: AtomicBool::new(false),
                started_execution_count: AtomicU64::new(0),
                last_started_at: RwLock::new(None),
                last_finished_at: RwLock::new(None),
            }),
        }
    }

    #[cfg(test)]
    pub fn reserved_for_test(
        reservation_id: &str,
        reservation_token: &str,
        expires_at: &str,
    ) -> Self {
        let expires_at = DateTime::parse_from_rfc3339(expires_at)
            .expect("valid test reservation expiry")
            .with_timezone(&Utc);
        Self::new(
            Some(reservation_id.to_owned()),
            Some(reservation_token.to_owned()),
            Some(expires_at),
        )
    }

    pub fn validate_first_execution_headers(
        &self,
        headers: &HeaderMap,
    ) -> Result<(), ReservationError> {
        let gate = self.inner.gate.read().expect("reservation gate lock");
        if gate.reservation_id.is_none()
            || gate.reservation_token.is_none()
            || self.inner.consumed.load(Ordering::SeqCst)
        {
            return Ok(());
        }
        if gate
            .expires_at
            .is_some_and(|expires_at| Utc::now() >= expires_at)
        {
            return Err(ReservationError::Expired);
        }

        let Some(expected_id) = gate.reservation_id.as_deref() else {
            return Ok(());
        };
        let Some(expected_token) = gate.reservation_token.as_deref() else {
            return Ok(());
        };
        let Some(actual_id) = header_value(headers, RESERVATION_ID_HEADER) else {
            return Err(ReservationError::MissingHeaders);
        };
        let Some(actual_token) = header_value(headers, RESERVATION_TOKEN_HEADER) else {
            return Err(ReservationError::MissingHeaders);
        };

        if actual_id == expected_id && actual_token == expected_token {
            Ok(())
        } else {
            Err(ReservationError::InvalidReservation)
        }
    }

    pub async fn mark_execution_started(&self) {
        self.inner.consumed.store(true, Ordering::SeqCst);
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

    pub async fn rearm(
        &self,
        reservation_id: String,
        reservation_token: String,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<(), ReservationRearmError> {
        if self.inner.busy.load(Ordering::SeqCst) {
            return Err(ReservationRearmError::Busy);
        }
        if self.has_active_reservation_gate() {
            return Err(ReservationRearmError::ActiveReservation);
        }

        {
            let mut gate = self.inner.gate.write().expect("reservation gate lock");
            gate.reservation_id = Some(reservation_id);
            gate.reservation_token = Some(reservation_token);
            gate.expires_at = expires_at;
        }
        self.inner.consumed.store(false, Ordering::SeqCst);
        self.inner
            .started_execution_count
            .store(0, Ordering::SeqCst);
        *self.inner.last_started_at.write().await = None;
        *self.inner.last_finished_at.write().await = None;
        Ok(())
    }

    pub fn release(&self) -> Result<(), ReservationReleaseError> {
        if self.inner.busy.load(Ordering::SeqCst) {
            return Err(ReservationReleaseError::Busy);
        }

        let mut gate = self.inner.gate.write().expect("reservation gate lock");
        gate.reservation_id = None;
        gate.reservation_token = None;
        gate.expires_at = None;
        self.inner.consumed.store(true, Ordering::SeqCst);
        Ok(())
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

    fn has_active_reservation_gate(&self) -> bool {
        let gate = self.inner.gate.read().expect("reservation gate lock");
        gate.reservation_id.is_some()
            && gate.reservation_token.is_some()
            && !self.inner.consumed.load(Ordering::SeqCst)
    }
}

fn optional_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn header_value(headers: &HeaderMap, name: &'static str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reservation_gate_is_disabled_after_first_execution() {
        let state =
            ReservationState::reserved_for_test("rr_test", "rt_test", "2999-01-01T00:00:00Z");
        let headers = HeaderMap::new();

        assert_eq!(
            state.validate_first_execution_headers(&headers),
            Err(ReservationError::MissingHeaders)
        );

        state.mark_execution_started().await;
        state.mark_execution_finished().await;

        assert_eq!(state.validate_first_execution_headers(&headers), Ok(()));
        assert!(state.is_ready());
    }
}
