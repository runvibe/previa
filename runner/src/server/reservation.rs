use std::sync::Arc;
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
    reservation_id: Option<String>,
    reservation_token: Option<String>,
    expires_at: Option<DateTime<Utc>>,
    consumed: AtomicBool,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReservationError {
    MissingHeaders,
    InvalidReservation,
    Expired,
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
                reservation_id,
                reservation_token,
                expires_at,
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
        if !self.has_active_reservation_gate() {
            return Ok(());
        }
        if self
            .inner
            .expires_at
            .is_some_and(|expires_at| Utc::now() >= expires_at)
        {
            return Err(ReservationError::Expired);
        }

        let Some(expected_id) = self.inner.reservation_id.as_deref() else {
            return Ok(());
        };
        let Some(expected_token) = self.inner.reservation_token.as_deref() else {
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

    pub async fn snapshot(&self) -> ReservationSnapshot {
        ReservationSnapshot {
            busy: self.inner.busy.load(Ordering::SeqCst),
            started_execution_count: self.inner.started_execution_count.load(Ordering::SeqCst),
            last_started_at: self.inner.last_started_at.read().await.clone(),
            last_finished_at: self.inner.last_finished_at.read().await.clone(),
        }
    }

    fn has_active_reservation_gate(&self) -> bool {
        self.inner.reservation_id.is_some()
            && self.inner.reservation_token.is_some()
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
