use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::config::RunnerQueueConfig;
use super::event_buffer::{EventBuffer, EventPublisher, QueueEvent};
use super::repository::{ClaimedJob, JobFencing, RunnerIdentity, RunnerQueueRepository};

#[derive(Debug, Clone)]
pub enum JobOutcome {
    Completed(Value),
    Failed {
        error: String,
        result: Value,
        retryable: bool,
    },
    Cancelled(Value),
}

#[derive(Clone)]
pub struct EventSink {
    sender: mpsc::Sender<QueueEvent>,
}

impl EventSink {
    pub async fn push(
        &self,
        event_type: impl Into<String>,
        elapsed_ms: i64,
        payload_json: Value,
    ) -> Result<(), String> {
        self.sender
            .send(QueueEvent {
                seq: 0,
                event_type: event_type.into(),
                elapsed_ms,
                payload_json,
            })
            .await
            .map_err(|_| "job event sink is closed".to_owned())
    }

    pub fn try_push(
        &self,
        event_type: impl Into<String>,
        elapsed_ms: i64,
        payload_json: Value,
    ) -> Result<(), String> {
        self.sender
            .try_send(QueueEvent {
                seq: 0,
                event_type: event_type.into(),
                elapsed_ms,
                payload_json,
            })
            .map_err(|error| format!("job event buffer rejected event: {error}"))
    }
}

#[async_trait]
pub trait JobExecutor: Send + Sync {
    async fn execute(
        &self,
        job: ClaimedJob,
        events: EventSink,
        cancel: CancellationToken,
    ) -> JobOutcome;
}

#[async_trait]
pub trait WorkerBackend: Send + Sync {
    async fn heartbeat(&self, identity: &RunnerIdentity, status: &str) -> Result<bool, String>;
    async fn claim_job(
        &self,
        identity: &RunnerIdentity,
        lease: Duration,
    ) -> Result<Option<ClaimedJob>, String>;
    async fn renew(
        &self,
        identity: &RunnerIdentity,
        fencing: JobFencing,
        lease: Duration,
    ) -> Result<bool, String>;
    async fn publish_events(
        &self,
        identity: &RunnerIdentity,
        fencing: JobFencing,
        events: &[QueueEvent],
    ) -> Result<(), String>;
    async fn complete(
        &self,
        identity: &RunnerIdentity,
        fencing: JobFencing,
        result: &Value,
    ) -> Result<(), String>;
    async fn fail(
        &self,
        identity: &RunnerIdentity,
        fencing: JobFencing,
        error: &str,
        result: &Value,
        retryable: bool,
        backoff_base: Duration,
        backoff_max: Duration,
    ) -> Result<(), String>;
    async fn acknowledge_cancellation(
        &self,
        identity: &RunnerIdentity,
        fencing: JobFencing,
        result: &Value,
    ) -> Result<(), String>;
    async fn read_control(
        &self,
        identity: &RunnerIdentity,
        fencing: JobFencing,
    ) -> Result<String, String>;
    async fn wait_for_wakeup(&self, timeout: Duration) -> Result<(), String>;
}

#[async_trait]
impl WorkerBackend for RunnerQueueRepository {
    async fn heartbeat(&self, identity: &RunnerIdentity, status: &str) -> Result<bool, String> {
        RunnerQueueRepository::heartbeat(self, identity, status).await
    }

    async fn claim_job(
        &self,
        identity: &RunnerIdentity,
        lease: Duration,
    ) -> Result<Option<ClaimedJob>, String> {
        RunnerQueueRepository::claim_job(self, identity, lease).await
    }

    async fn renew(
        &self,
        identity: &RunnerIdentity,
        fencing: JobFencing,
        lease: Duration,
    ) -> Result<bool, String> {
        RunnerQueueRepository::renew(self, identity, fencing, lease).await
    }

    async fn publish_events(
        &self,
        identity: &RunnerIdentity,
        fencing: JobFencing,
        events: &[QueueEvent],
    ) -> Result<(), String> {
        let value = serde_json::to_value(events).map_err(|error| error.to_string())?;
        RunnerQueueRepository::publish_events(self, identity, fencing, &value)
            .await
            .map(|_| ())
    }

    async fn complete(
        &self,
        identity: &RunnerIdentity,
        fencing: JobFencing,
        result: &Value,
    ) -> Result<(), String> {
        RunnerQueueRepository::complete(self, identity, fencing, result).await
    }

    async fn fail(
        &self,
        identity: &RunnerIdentity,
        fencing: JobFencing,
        error: &str,
        result: &Value,
        retryable: bool,
        backoff_base: Duration,
        backoff_max: Duration,
    ) -> Result<(), String> {
        RunnerQueueRepository::fail(
            self,
            identity,
            fencing,
            error,
            result,
            retryable,
            backoff_base,
            backoff_max,
        )
        .await
        .map(|_| ())
    }

    async fn acknowledge_cancellation(
        &self,
        identity: &RunnerIdentity,
        fencing: JobFencing,
        result: &Value,
    ) -> Result<(), String> {
        RunnerQueueRepository::acknowledge_cancellation(self, identity, fencing, result).await
    }

    async fn read_control(
        &self,
        identity: &RunnerIdentity,
        fencing: JobFencing,
    ) -> Result<String, String> {
        RunnerQueueRepository::read_control(self, identity, fencing).await
    }

    async fn wait_for_wakeup(&self, timeout: Duration) -> Result<(), String> {
        RunnerQueueRepository::wait_for_wakeup(self, timeout).await
    }
}

struct JobEventPublisher {
    backend: Arc<dyn WorkerBackend>,
    identity: RunnerIdentity,
    fencing: JobFencing,
}

#[async_trait]
impl EventPublisher for JobEventPublisher {
    async fn publish(&self, events: &[QueueEvent]) -> Result<(), String> {
        self.backend
            .publish_events(&self.identity, self.fencing, events)
            .await
    }
}

pub struct RunnerWorker {
    backend: Arc<dyn WorkerBackend>,
    executor: Arc<dyn JobExecutor>,
    identity: RunnerIdentity,
    config: RunnerQueueConfig,
    job_lease: Duration,
    retry_backoff_base: Duration,
    retry_backoff_max: Duration,
}

impl RunnerWorker {
    pub fn new(
        backend: Arc<dyn WorkerBackend>,
        executor: Arc<dyn JobExecutor>,
        identity: RunnerIdentity,
        config: RunnerQueueConfig,
        job_lease: Duration,
    ) -> Result<Self, String> {
        config.validate_lease_duration(job_lease)?;
        Ok(Self {
            backend,
            executor,
            identity,
            config,
            job_lease,
            retry_backoff_base: Duration::from_secs(1),
            retry_backoff_max: Duration::from_secs(30),
        })
    }

    pub async fn run(&self, cancel: CancellationToken) -> Result<(), String> {
        loop {
            if let Some(job) = self
                .backend
                .claim_job(&self.identity, self.job_lease)
                .await?
            {
                self.run_job(job, cancel.child_token()).await?;
                continue;
            }

            tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = self.backend.heartbeat(&self.identity, "stopped").await;
                    return Ok(());
                }
                wakeup = self.backend.wait_for_wakeup(self.config.poll_interval) => {
                    wakeup?;
                }
            }
        }
    }

    async fn run_job(&self, job: ClaimedJob, cancel: CancellationToken) -> Result<(), String> {
        let fencing = job.fencing();
        let (sender, mut receiver) = mpsc::channel(self.config.event_buffer_max);
        let sink = EventSink { sender };
        let publisher = Arc::new(JobEventPublisher {
            backend: self.backend.clone(),
            identity: self.identity.clone(),
            fencing,
        });
        let mut buffer = EventBuffer::new(
            publisher,
            self.config.event_batch_size,
            self.config.event_buffer_max,
            self.config.event_flush_interval,
        );
        let execution_cancel = cancel.child_token();
        let executor = self.executor.execute(job, sink, execution_cancel.clone());
        tokio::pin!(executor);
        let mut renew = tokio::time::interval(self.config.lease_renew_interval);
        renew.tick().await;
        let mut control = tokio::time::interval(self.config.poll_interval);
        control.tick().await;
        let mut flush = tokio::time::interval(self.config.event_flush_interval);
        flush.tick().await;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    execution_cancel.cancel();
                    return Ok(());
                }
                outcome = &mut executor => {
                    while let Ok(event) = receiver.try_recv() {
                        buffer.push(event).await?;
                    }
                    buffer.flush().await?;
                    return self.write_outcome(fencing, outcome).await;
                }
                _ = renew.tick() => {
                    let renewed = self.backend
                        .renew(&self.identity, fencing, self.job_lease)
                        .await?;
                    if !renewed {
                        execution_cancel.cancel();
                        let _ = tokio::time::timeout(Duration::from_secs(1), &mut executor).await;
                        return Err("job lease renewal was rejected".to_owned());
                    }
                }
                _ = control.tick() => {
                    if self.backend.read_control(&self.identity, fencing).await? == "cancelled" {
                        execution_cancel.cancel();
                        self.backend
                            .acknowledge_cancellation(
                                &self.identity,
                                fencing,
                                &json!({"cancelled": true}),
                            )
                            .await?;
                        return Ok(());
                    }
                }
                event = receiver.recv() => {
                    if let Some(event) = event {
                        buffer.push(event).await?;
                    }
                }
                _ = flush.tick() => {
                    buffer.flush_if_due().await?;
                }
            }
        }
    }

    async fn write_outcome(&self, fencing: JobFencing, outcome: JobOutcome) -> Result<(), String> {
        match outcome {
            JobOutcome::Completed(result) => {
                self.backend
                    .complete(&self.identity, fencing, &result)
                    .await
            }
            JobOutcome::Failed {
                error,
                result,
                retryable,
            } => {
                self.backend
                    .fail(
                        &self.identity,
                        fencing,
                        &error,
                        &result,
                        retryable,
                        self.retry_backoff_base,
                        self.retry_backoff_max,
                    )
                    .await
            }
            JobOutcome::Cancelled(result) => {
                self.backend
                    .acknowledge_cancellation(&self.identity, fencing, &result)
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    use super::*;

    struct FakeBackend {
        claims: Mutex<Vec<Option<ClaimedJob>>>,
        completed: AtomicUsize,
        renew: AtomicBool,
    }

    impl FakeBackend {
        fn new(claims: Vec<Option<ClaimedJob>>) -> Self {
            Self {
                claims: Mutex::new(claims.into_iter().rev().collect()),
                completed: AtomicUsize::new(0),
                renew: AtomicBool::new(true),
            }
        }
    }

    #[async_trait]
    impl WorkerBackend for FakeBackend {
        async fn heartbeat(
            &self,
            _identity: &RunnerIdentity,
            _status: &str,
        ) -> Result<bool, String> {
            Ok(true)
        }

        async fn claim_job(
            &self,
            _identity: &RunnerIdentity,
            _lease: Duration,
        ) -> Result<Option<ClaimedJob>, String> {
            Ok(self.claims.lock().unwrap().pop().flatten())
        }

        async fn renew(
            &self,
            _identity: &RunnerIdentity,
            _fencing: JobFencing,
            _lease: Duration,
        ) -> Result<bool, String> {
            Ok(self.renew.load(Ordering::SeqCst))
        }

        async fn publish_events(
            &self,
            _identity: &RunnerIdentity,
            _fencing: JobFencing,
            _events: &[QueueEvent],
        ) -> Result<(), String> {
            Ok(())
        }

        async fn complete(
            &self,
            _identity: &RunnerIdentity,
            _fencing: JobFencing,
            _result: &Value,
        ) -> Result<(), String> {
            self.completed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn fail(
            &self,
            _identity: &RunnerIdentity,
            _fencing: JobFencing,
            _error: &str,
            _result: &Value,
            _retryable: bool,
            _backoff_base: Duration,
            _backoff_max: Duration,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn acknowledge_cancellation(
            &self,
            _identity: &RunnerIdentity,
            _fencing: JobFencing,
            _result: &Value,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn read_control(
            &self,
            _identity: &RunnerIdentity,
            _fencing: JobFencing,
        ) -> Result<String, String> {
            Ok("running".to_owned())
        }

        async fn wait_for_wakeup(&self, timeout: Duration) -> Result<(), String> {
            tokio::time::sleep(timeout).await;
            Ok(())
        }
    }

    struct CompletingExecutor;

    #[async_trait]
    impl JobExecutor for CompletingExecutor {
        async fn execute(
            &self,
            _job: ClaimedJob,
            events: EventSink,
            _cancel: CancellationToken,
        ) -> JobOutcome {
            events
                .push("progress", 1, json!({"ok": true}))
                .await
                .unwrap();
            JobOutcome::Completed(json!({"ok": true}))
        }
    }

    struct CancellationExecutor {
        cancelled: Arc<AtomicBool>,
    }

    #[async_trait]
    impl JobExecutor for CancellationExecutor {
        async fn execute(
            &self,
            _job: ClaimedJob,
            _events: EventSink,
            cancel: CancellationToken,
        ) -> JobOutcome {
            cancel.cancelled().await;
            self.cancelled.store(true, Ordering::SeqCst);
            JobOutcome::Cancelled(json!({}))
        }
    }

    fn job() -> ClaimedJob {
        ClaimedJob {
            job_id: Uuid::now_v7(),
            execution_id: Uuid::now_v7(),
            kind: "e2e".to_owned(),
            shard_index: None,
            payload_json: json!({}),
            attempt: 1,
            lease_epoch: 1,
            lease_token: Uuid::new_v4(),
            lease_expires_at: Utc::now() + chrono::Duration::seconds(1),
        }
    }

    fn config() -> RunnerQueueConfig {
        RunnerQueueConfig::from_env_values(&[
            (
                "PREVIA_QUEUE_DATABASE_URL",
                "postgres://runner@localhost/previa",
            ),
            ("PREVIA_QUEUE_HEARTBEAT_INTERVAL_MS", "1000"),
            ("PREVIA_QUEUE_LEASE_RENEW_INTERVAL_MS", "1000"),
            ("PREVIA_QUEUE_POLL_INTERVAL_MS", "100"),
            ("PREVIA_QUEUE_EVENT_FLUSH_INTERVAL_MS", "10"),
            ("PREVIA_QUEUE_EVENT_BATCH_SIZE", "1"),
            ("PREVIA_QUEUE_EVENT_BUFFER_MAX", "200"),
        ])
        .unwrap()
    }

    #[tokio::test]
    async fn lost_notify_is_recovered_by_polling() {
        let backend = Arc::new(FakeBackend::new(vec![None, Some(job())]));
        let worker = RunnerWorker::new(
            backend.clone(),
            Arc::new(CompletingExecutor),
            RunnerIdentity {
                runner_id: Uuid::new_v4(),
                session_token: "secret".to_owned(),
            },
            config(),
            Duration::from_secs(3),
        )
        .unwrap();
        let cancel = CancellationToken::new();
        let run_cancel = cancel.clone();
        let handle = tokio::spawn(async move { worker.run(run_cancel).await });
        tokio::time::timeout(Duration::from_secs(1), async {
            while backend.completed.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("polling must find job");
        cancel.cancel();
        handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn lease_renew_failure_cancels_executor() {
        let backend = Arc::new(FakeBackend::new(vec![Some(job())]));
        backend.renew.store(false, Ordering::SeqCst);
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker = RunnerWorker::new(
            backend,
            Arc::new(CancellationExecutor {
                cancelled: cancelled.clone(),
            }),
            RunnerIdentity {
                runner_id: Uuid::new_v4(),
                session_token: "secret".to_owned(),
            },
            config(),
            Duration::from_secs(3),
        )
        .unwrap();
        let error = worker
            .run_job(job(), CancellationToken::new())
            .await
            .expect_err("lost lease must stop job");
        assert!(error.contains("renewal"));
        tokio::task::yield_now().await;
        assert!(cancelled.load(Ordering::SeqCst));
    }
}
