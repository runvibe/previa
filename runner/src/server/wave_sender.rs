use std::collections::HashMap;
#[cfg(test)]
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use reqwest::Client;
use tokio::sync::mpsc;
#[cfg(test)]
use tokio::task::JoinSet;

use previa_runner::{
    PipelineStep, PreparedHttpStep, RuntimeEnvGroup, RuntimeSpec, StepExecutionResult,
    send_prepared_http_step_with_hooks,
};

use crate::server::metrics::estimate_results_network_bytes;
use crate::server::wave_metrics_actor::WaveMetricEvent;

pub struct ReadyWaveRequest<C> {
    pub step: PipelineStep,
    pub cursor: C,
    pub context: HashMap<String, StepExecutionResult>,
    pub prepared: PreparedHttpStep,
    pub specs: Arc<Vec<RuntimeSpec>>,
    pub env_groups: Arc<Vec<RuntimeEnvGroup>>,
    pub selected_env_group_slug: Option<String>,
}

pub struct WaveObserverEvent<C> {
    pub cursor: C,
    pub result: StepExecutionResult,
}

pub struct WaveSender<C> {
    client: Arc<Client>,
    started: Instant,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    response_in_flight: Arc<AtomicUsize>,
    ready_to_send: Arc<AtomicUsize>,
    request_rx: mpsc::UnboundedReceiver<ReadyWaveRequest<C>>,
    observer_tx: mpsc::UnboundedSender<WaveObserverEvent<C>>,
    token: tokio_util::sync::CancellationToken,
}

impl<C> WaveSender<C>
where
    C: Send + 'static,
{
    pub fn new(
        client: Arc<Client>,
        started: Instant,
        metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
        response_in_flight: Arc<AtomicUsize>,
        ready_to_send: Arc<AtomicUsize>,
        request_rx: mpsc::UnboundedReceiver<ReadyWaveRequest<C>>,
        observer_tx: mpsc::UnboundedSender<WaveObserverEvent<C>>,
        token: tokio_util::sync::CancellationToken,
    ) -> Self {
        Self {
            client,
            started,
            metric_tx,
            response_in_flight,
            ready_to_send,
            request_rx,
            observer_tx,
            token,
        }
    }

    pub async fn run(mut self) {
        while let Some(request) = self.request_rx.recv().await {
            self.ready_to_send.fetch_sub(1, Ordering::SeqCst);
            if self.token.is_cancelled() {
                break;
            }
            self.spawn_observer(request);
        }
    }

    fn spawn_observer(&self, request: ReadyWaveRequest<C>) {
        self.response_in_flight.fetch_add(1, Ordering::SeqCst);
        let dispatch_elapsed_ms = self.started.elapsed().as_millis() as u64;

        let client = Arc::clone(&self.client);
        let metric_tx = self.metric_tx.clone();
        let metrics_for_send = self.metric_tx.clone();
        let metrics_for_body = self.metric_tx.clone();
        let response_in_flight = Arc::clone(&self.response_in_flight);
        let observer_tx = self.observer_tx.clone();
        let token = self.token.clone();

        let _ = metric_tx.send(WaveMetricEvent::SendTaskSpawned);
        tokio::spawn(async move {
            let _ = metric_tx.send(WaveMetricEvent::SendStarted);
            let _ = metric_tx.send(WaveMetricEvent::DispatchStarted {
                elapsed_ms: dispatch_elapsed_ms,
            });
            let _ = metric_tx.send(WaveMetricEvent::HttpStarted);

            let result = send_prepared_http_step_with_hooks(
                client.as_ref(),
                request.prepared,
                &request.step,
                &request.context,
                Some(request.specs.as_slice()),
                Some(request.env_groups.as_slice()),
                request.selected_env_group_slug.as_deref(),
                || token.is_cancelled(),
                move || {
                    let metric_tx = metrics_for_send.clone();
                    async move {
                        let _ = metric_tx.send(WaveMetricEvent::HttpSendReturned);
                    }
                },
                move || {
                    let metric_tx = metrics_for_body.clone();
                    async move {
                        let _ = metric_tx.send(WaveMetricEvent::ResponseBodyCompleted(1));
                    }
                },
            )
            .await;

            response_in_flight.fetch_sub(1, Ordering::SeqCst);
            let Some(result) = result else {
                return;
            };

            let (network_tx_bytes, network_rx_bytes) =
                estimate_results_network_bytes(std::slice::from_ref(&result));
            if result.request.is_some() {
                let _ = metric_tx.send(WaveMetricEvent::HttpCompleted(1));
            }
            let _ = metric_tx.send(WaveMetricEvent::NetworkBytes {
                tx: network_tx_bytes,
                rx: network_rx_bytes,
            });

            let _ = observer_tx.send(WaveObserverEvent {
                cursor: request.cursor,
                result,
            });
        });
    }
}

#[cfg(test)]
#[derive(Debug)]
pub struct TestReadyWaveRequest<T> {
    pub payload: T,
}

#[cfg(test)]
pub async fn run_test_sender<T, F, Fut>(
    mut rx: mpsc::UnboundedReceiver<TestReadyWaveRequest<T>>,
    started: Arc<AtomicUsize>,
    mut send: F,
) where
    T: Send + 'static,
    F: FnMut(T) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut tasks = JoinSet::new();
    while let Some(request) = rx.recv().await {
        started.fetch_add(1, Ordering::SeqCst);
        tasks.spawn(send(request.payload));
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

#[cfg(test)]
pub async fn run_test_sender_with_metric_events<T, F, Fut>(
    mut rx: mpsc::UnboundedReceiver<TestReadyWaveRequest<T>>,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    started: Arc<AtomicUsize>,
    mut send: F,
) where
    T: Send + 'static,
    F: FnMut(T) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut tasks = JoinSet::new();
    while let Some(request) = rx.recv().await {
        started.fetch_add(1, Ordering::SeqCst);
        let _ = metric_tx.send(WaveMetricEvent::DispatchStarted { elapsed_ms: 0 });
        tasks.spawn(send(request.payload));
    }
    while tasks.join_next().await.is_some() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::Notify;
    use tokio::time::{Duration, timeout};

    #[tokio::test]
    async fn sender_starts_later_requests_without_waiting_for_prior_response_task() {
        let (tx, rx) = mpsc::unbounded_channel();
        let started = Arc::new(AtomicUsize::new(0));
        let blocker = Arc::new(Notify::new());

        let sender_started = Arc::clone(&started);
        let sender_blocker = Arc::clone(&blocker);
        let sender = tokio::spawn(run_test_sender(
            rx,
            sender_started,
            move |_payload: usize| {
                let blocker = Arc::clone(&sender_blocker);
                async move {
                    blocker.notified().await;
                }
            },
        ));

        tx.send(TestReadyWaveRequest { payload: 1 }).unwrap();
        tx.send(TestReadyWaveRequest { payload: 2 }).unwrap();
        tx.send(TestReadyWaveRequest { payload: 3 }).unwrap();

        timeout(Duration::from_millis(100), async {
            loop {
                if started.load(Ordering::SeqCst) == 3 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("sender should start all requests while response tasks are blocked");

        drop(tx);
        sender.abort();
    }

    #[tokio::test]
    async fn sender_accepts_many_requests_even_when_observers_never_finish() {
        let (tx, rx) = mpsc::unbounded_channel();
        let started = Arc::new(AtomicUsize::new(0));
        let blocker = Arc::new(Notify::new());

        let sender_started = Arc::clone(&started);
        let sender_blocker = Arc::clone(&blocker);
        let sender = tokio::spawn(run_test_sender(
            rx,
            sender_started,
            move |_payload: usize| {
                let blocker = Arc::clone(&sender_blocker);
                async move {
                    blocker.notified().await;
                }
            },
        ));

        for payload in 0..10_000 {
            tx.send(TestReadyWaveRequest { payload }).unwrap();
        }

        timeout(Duration::from_secs(1), async {
            loop {
                if started.load(Ordering::SeqCst) == 10_000 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("sender hot path should accept requests even with blocked observers");

        drop(tx);
        sender.abort();
    }

    #[tokio::test]
    async fn sender_emits_dispatch_events_for_accepted_requests() {
        let (tx, rx) = mpsc::unbounded_channel();
        let (metric_tx, mut metric_rx) = mpsc::unbounded_channel();
        let started = Arc::new(AtomicUsize::new(0));

        let sender_started = Arc::clone(&started);
        let sender = tokio::spawn(run_test_sender_with_metric_events(
            rx,
            metric_tx,
            sender_started,
            |_payload: usize| async move {},
        ));

        tx.send(TestReadyWaveRequest { payload: 1 }).unwrap();
        tx.send(TestReadyWaveRequest { payload: 2 }).unwrap();
        drop(tx);

        sender.await.unwrap();

        let mut dispatch_started = 0;
        while let Ok(event) = metric_rx.try_recv() {
            if matches!(
                event,
                crate::server::wave_metrics_actor::WaveMetricEvent::DispatchStarted { .. }
            ) {
                dispatch_started += 1;
            }
        }

        assert_eq!(started.load(Ordering::SeqCst), 2);
        assert_eq!(dispatch_started, 2);
    }
}
