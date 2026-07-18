use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_EVENT_PAYLOAD_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueueEvent {
    pub seq: i64,
    pub event_type: String,
    pub elapsed_ms: i64,
    pub payload_json: Value,
}

#[async_trait]
pub trait EventPublisher: Send + Sync {
    async fn publish(&self, events: &[QueueEvent]) -> Result<(), String>;
}

pub struct EventBuffer {
    publisher: Arc<dyn EventPublisher>,
    events: Vec<QueueEvent>,
    batch_size: usize,
    max_size: usize,
    flush_interval: Duration,
    last_flush: Instant,
    next_seq: i64,
}

impl EventBuffer {
    pub fn new(
        publisher: Arc<dyn EventPublisher>,
        batch_size: usize,
        max_size: usize,
        flush_interval: Duration,
    ) -> Self {
        Self {
            publisher,
            events: Vec::with_capacity(batch_size),
            batch_size,
            max_size,
            flush_interval,
            last_flush: Instant::now(),
            next_seq: 1,
        }
    }

    pub async fn push(&mut self, mut event: QueueEvent) -> Result<(), String> {
        let payload_size = serde_json::to_vec(&event.payload_json)
            .map_err(|error| error.to_string())?
            .len();
        if payload_size > MAX_EVENT_PAYLOAD_BYTES {
            return Err(format!(
                "event payload exceeds {MAX_EVENT_PAYLOAD_BYTES} bytes"
            ));
        }
        if self.events.len() >= self.max_size {
            return Err("event buffer capacity exceeded".to_owned());
        }
        event.seq = self.next_seq;
        self.next_seq += 1;
        self.events.push(event);
        if self.events.len() >= self.batch_size {
            self.flush().await?;
        }
        Ok(())
    }

    pub async fn flush_if_due(&mut self) -> Result<(), String> {
        if !self.events.is_empty() && self.last_flush.elapsed() >= self.flush_interval {
            self.flush().await?;
        }
        Ok(())
    }

    pub async fn flush(&mut self) -> Result<(), String> {
        if self.events.is_empty() {
            return Ok(());
        }
        self.publisher.publish(&self.events).await?;
        self.events.clear();
        self.last_flush = Instant::now();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct RecordingPublisher {
        batches: Mutex<Vec<Vec<QueueEvent>>>,
    }

    #[async_trait]
    impl EventPublisher for RecordingPublisher {
        async fn publish(&self, events: &[QueueEvent]) -> Result<(), String> {
            self.batches.lock().unwrap().push(events.to_vec());
            Ok(())
        }
    }

    fn event(value: i64) -> QueueEvent {
        QueueEvent {
            seq: 0,
            event_type: "progress".to_owned(),
            elapsed_ms: value,
            payload_json: json!({"value": value}),
        }
    }

    #[tokio::test]
    async fn flushes_at_batch_size_and_interval() {
        let publisher = Arc::new(RecordingPublisher::default());
        let mut buffer = EventBuffer::new(publisher.clone(), 2, 10, Duration::from_millis(1));
        buffer.push(event(1)).await.unwrap();
        assert_eq!(publisher.batches.lock().unwrap().len(), 0);
        buffer.push(event(2)).await.unwrap();
        assert_eq!(publisher.batches.lock().unwrap()[0].len(), 2);

        buffer.push(event(3)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
        buffer.flush_if_due().await.unwrap();
        assert_eq!(publisher.batches.lock().unwrap().len(), 2);
        assert_eq!(publisher.batches.lock().unwrap()[1][0].seq, 3);
    }
}
