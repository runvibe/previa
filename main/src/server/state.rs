use std::collections::HashMap;
use std::sync::Arc;

use crate::server::db::DbPool;
use reqwest::Client;
use serde_json::Value;
use tokio::sync::{RwLock, broadcast};
use tokio_util::sync::CancellationToken;

use crate::server::auth::AuthRuntime;
use crate::server::execution::scheduler::{ExecutionScheduler, SharedValue};
use crate::server::mcp::models::McpSession;
use crate::server::models::{E2eQueueRecord, SseMessage};

#[derive(Clone)]
pub struct AppState {
    pub client: Client,
    pub db: DbPool,
    pub context_name: String,
    pub runner_auth_key: Option<String>,
    pub auth: AuthRuntime,
    pub rps_per_node: u64,
    pub scheduler: ExecutionScheduler,
    pub executions: Arc<RwLock<HashMap<String, Arc<ExecutionCtx>>>>,
    pub e2e_queues: Arc<RwLock<HashMap<String, Arc<E2eQueueRuntime>>>>,
    pub mcp_sessions: Arc<RwLock<HashMap<String, McpSession>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionKind {
    E2e,
    Load,
}

#[derive(Debug, Clone)]
pub struct ExecutionCtx {
    pub cancel: CancellationToken,
    pub project_id: String,
    pub pipeline_id: Option<String>,
    pub kind: ExecutionKind,
    pub sse_tx: broadcast::Sender<SseMessage>,
    pub init_payload: SharedValue<Value>,
    pub snapshot_payload: SharedValue<Value>,
}

#[derive(Debug)]
pub struct E2eQueueRuntime {
    pub queue_id: String,
    pub project_id: String,
    pub cancel: CancellationToken,
    pub sse_tx: broadcast::Sender<SseMessage>,
    pub snapshot: Arc<RwLock<E2eQueueRecord>>,
    pub active_execution_id: Arc<RwLock<Option<String>>>,
    finished: std::sync::atomic::AtomicBool,
    finished_notify: tokio::sync::Notify,
}

impl E2eQueueRuntime {
    pub fn new(queue_id: String, project_id: String, snapshot: E2eQueueRecord) -> Arc<Self> {
        let (sse_tx, _) = broadcast::channel(EXECUTION_SSE_BUFFER_SIZE);
        Arc::new(Self {
            queue_id,
            project_id,
            cancel: CancellationToken::new(),
            sse_tx,
            snapshot: Arc::new(RwLock::new(snapshot)),
            active_execution_id: Arc::new(RwLock::new(None)),
            finished: std::sync::atomic::AtomicBool::new(false),
            finished_notify: tokio::sync::Notify::new(),
        })
    }

    pub async fn snapshot(&self) -> E2eQueueRecord {
        self.snapshot.read().await.clone()
    }

    pub async fn set_snapshot(&self, snapshot: E2eQueueRecord) {
        *self.snapshot.write().await = snapshot.clone();
        let _ = self.sse_tx.send(SseMessage {
            event: "queue:update".to_owned(),
            data: serde_json::to_value(snapshot).unwrap_or(Value::Null),
        });
    }

    pub async fn set_active_execution_id(&self, execution_id: Option<String>) {
        *self.active_execution_id.write().await = execution_id;
    }

    pub async fn active_execution_id(&self) -> Option<String> {
        self.active_execution_id.read().await.clone()
    }

    pub fn mark_finished(&self) {
        self.finished
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.finished_notify.notify_waiters();
    }

    pub async fn wait_finished(&self) {
        while !self.finished.load(std::sync::atomic::Ordering::SeqCst) {
            self.finished_notify.notified().await;
        }
    }
}

pub const TRANSACTION_ID_HEADER: &str = "x-transaction-id";
pub const LOAD_BATCH_WINDOW_MS: u64 = 50;
pub const DB_SCHEMA_VERSION: u32 = 1;
pub const EXECUTION_SSE_BUFFER_SIZE: usize = 1024;
