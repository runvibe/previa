use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::server::load_execution::LoadExecutionStore;
use crate::server::reservation::ReservationState;

#[derive(Clone, Default)]
pub struct AppState {
    pub executions: Arc<RwLock<HashMap<String, CancellationToken>>>,
    pub runner_auth_key: Option<String>,
    pub reservation: ReservationState,
    pub load_executions: LoadExecutionStore,
}
