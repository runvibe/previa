use std::collections::HashMap;
#[cfg(test)]
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use reqwest::Client;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tracing::error;

use previa_runner::{
    PipelineStep, PreparedHttpStep, RuntimeEnvGroup, RuntimeSpec, StepExecutionResult,
    send_prepared_http_step_with_hooks,
};

use crate::server::metrics::{MetricsAccumulator, estimate_results_network_bytes};

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
    metrics: Arc<tokio::sync::Mutex<MetricsAccumulator>>,
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
        metrics: Arc<tokio::sync::Mutex<MetricsAccumulator>>,
        response_in_flight: Arc<AtomicUsize>,
        ready_to_send: Arc<AtomicUsize>,
        request_rx: mpsc::UnboundedReceiver<ReadyWaveRequest<C>>,
        observer_tx: mpsc::UnboundedSender<WaveObserverEvent<C>>,
        token: tokio_util::sync::CancellationToken,
    ) -> Self {
        Self {
            client,
            metrics,
            response_in_flight,
            ready_to_send,
            request_rx,
            observer_tx,
            token,
        }
    }

    pub async fn run(mut self) {
        let mut tasks = JoinSet::new();
        loop {
            tokio::select! {
                maybe_request = self.request_rx.recv() => {
                    let Some(request) = maybe_request else {
                        break;
                    };
                    self.ready_to_send.fetch_sub(1, Ordering::SeqCst);
                    if self.token.is_cancelled() {
                        break;
                    }
                    self.spawn_observer(&mut tasks, request);
                }
                Some(joined) = tasks.join_next(), if !tasks.is_empty() => {
                    log_observer_join(joined);
                }
            }
        }

        while let Some(joined) = tasks.join_next().await {
            log_observer_join(joined);
        }
    }

    fn spawn_observer(&self, tasks: &mut JoinSet<()>, request: ReadyWaveRequest<C>) {
        self.response_in_flight.fetch_add(1, Ordering::SeqCst);

        let client = Arc::clone(&self.client);
        let metrics = Arc::clone(&self.metrics);
        let metrics_for_send = Arc::clone(&self.metrics);
        let metrics_for_body = Arc::clone(&self.metrics);
        let response_in_flight = Arc::clone(&self.response_in_flight);
        let observer_tx = self.observer_tx.clone();
        let token = self.token.clone();

        tasks.spawn(async move {
            {
                let mut lock = metrics.lock().await;
                lock.record_http_start();
            }

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
                    let metrics = Arc::clone(&metrics_for_send);
                    async move {
                        let mut lock = metrics.lock().await;
                        lock.record_http_send_returned();
                    }
                },
                move || {
                    let metrics = Arc::clone(&metrics_for_body);
                    async move {
                        let mut lock = metrics.lock().await;
                        lock.record_response_body_completed_count(1);
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
            {
                let mut lock = metrics.lock().await;
                if result.request.is_some() {
                    lock.record_http_completed_count(1);
                }
                lock.add_network_bytes(network_tx_bytes, network_rx_bytes);
            }

            let _ = observer_tx.send(WaveObserverEvent {
                cursor: request.cursor,
                result,
            });
        });
    }
}

fn log_observer_join(joined: Result<(), tokio::task::JoinError>) {
    if let Err(err) = joined {
        if !err.is_cancelled() {
            error!("wave response observer task failed: {err}");
        }
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
}
