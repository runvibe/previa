use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::server::utils::now_ms;

#[derive(Clone, Default)]
pub struct LoadExecutionStore {
    records: Arc<RwLock<HashMap<String, LoadExecutionRecord>>>,
}

#[derive(Clone)]
struct LoadExecutionRecord {
    status: LoadExecutionStatus,
    started_at_ms: u64,
    finished_at_ms: Option<u64>,
    next_seq: u64,
    acked_through_seq: u64,
    buckets: Vec<LoadExecutionBucket>,
    token: CancellationToken,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LoadExecutionStatus {
    Running,
    Completed,
    Cancelled,
}

impl LoadExecutionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

#[derive(Clone)]
pub struct LoadExecutionBucket {
    pub seq: u64,
    pub event: String,
    pub elapsed_ms: u64,
    pub payload: Value,
}

pub struct LoadExecutionStart {
    pub execution_id: String,
    pub status: LoadExecutionStatus,
    pub next_seq: u64,
    pub started_at_ms: u64,
}

pub struct LoadExecutionPoll {
    pub execution_id: String,
    pub status: LoadExecutionStatus,
    pub from_seq: u64,
    pub through_seq: u64,
    pub next_seq: u64,
    pub buckets: Vec<LoadExecutionBucket>,
}

pub struct LoadExecutionAck {
    pub execution_id: String,
    pub acked_through_seq: u64,
    pub retained_from_seq: u64,
}

impl LoadExecutionStore {
    pub async fn start(
        &self,
        execution_id: String,
        token: CancellationToken,
    ) -> LoadExecutionStart {
        let started_at_ms = now_ms();
        let record = LoadExecutionRecord {
            status: LoadExecutionStatus::Running,
            started_at_ms,
            finished_at_ms: None,
            next_seq: 1,
            acked_through_seq: 0,
            buckets: Vec::new(),
            token,
        };

        let mut records = self.records.write().await;
        records.insert(execution_id.clone(), record);

        LoadExecutionStart {
            execution_id,
            status: LoadExecutionStatus::Running,
            next_seq: 1,
            started_at_ms,
        }
    }

    pub async fn push_event(&self, execution_id: &str, event: &str, payload: Value) -> Option<u64> {
        let mut records = self.records.write().await;
        let record = records.get_mut(execution_id)?;
        let seq = record.next_seq;
        record.next_seq += 1;
        record.buckets.push(LoadExecutionBucket {
            seq,
            event: event.to_owned(),
            elapsed_ms: now_ms().saturating_sub(record.started_at_ms),
            payload,
        });
        Some(seq)
    }

    pub async fn poll(
        &self,
        execution_id: &str,
        after_seq: u64,
        limit: usize,
    ) -> Option<LoadExecutionPoll> {
        let records = self.records.read().await;
        let record = records.get(execution_id)?;
        let from_seq = after_seq.max(record.acked_through_seq);
        let mut buckets = record
            .buckets
            .iter()
            .filter(|bucket| bucket.seq > from_seq)
            .take(limit.max(1))
            .cloned()
            .collect::<Vec<_>>();
        buckets.sort_by_key(|bucket| bucket.seq);
        let through_seq = buckets.last().map(|bucket| bucket.seq).unwrap_or(from_seq);

        Some(LoadExecutionPoll {
            execution_id: execution_id.to_owned(),
            status: record.status,
            from_seq,
            through_seq,
            next_seq: record.next_seq,
            buckets,
        })
    }

    pub async fn ack(&self, execution_id: &str, through_seq: u64) -> Option<LoadExecutionAck> {
        let mut records = self.records.write().await;
        let record = records.get_mut(execution_id)?;
        record.acked_through_seq = record.acked_through_seq.max(through_seq);
        record
            .buckets
            .retain(|bucket| bucket.seq > record.acked_through_seq);
        let retained_from_seq = record
            .buckets
            .first()
            .map(|bucket| bucket.seq)
            .unwrap_or(record.acked_through_seq.saturating_add(1));

        Some(LoadExecutionAck {
            execution_id: execution_id.to_owned(),
            acked_through_seq: record.acked_through_seq,
            retained_from_seq,
        })
    }

    pub async fn finish(&self, execution_id: &str, status: LoadExecutionStatus) {
        let mut records = self.records.write().await;
        if let Some(record) = records.get_mut(execution_id) {
            record.status = status;
            record.finished_at_ms = Some(now_ms());
        }
    }

    pub async fn cancel(&self, execution_id: &str) -> bool {
        let records = self.records.read().await;
        if let Some(record) = records.get(execution_id) {
            record.token.cancel();
            true
        } else {
            false
        }
    }
}
