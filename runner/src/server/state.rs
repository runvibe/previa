use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::server::reservation::ReservationState;

#[derive(Clone, Default)]
pub struct AppState {
    pub runner_auth_key: Option<String>,
    pub reservation: ReservationState,
    pub queue_ready: Option<Arc<AtomicBool>>,
}
