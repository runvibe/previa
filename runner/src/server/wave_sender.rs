use std::collections::HashMap;
#[cfg(test)]
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use reqwest::Client;
use tokio::sync::mpsc;
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
    pub scheduled_elapsed_ms: u64,
    pub expires_at_elapsed_ms: u64,
    pub sender_enqueued_elapsed_ms: u64,
}

pub struct WaveObserverEvent<C> {
    pub cursor: C,
    pub result: StepExecutionResult,
}

struct SenderWorkerCommand<C> {
    request: ReadyWaveRequest<C>,
}

struct ObserverCommand<C> {
    request: ReadyWaveRequest<C>,
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

pub struct WaveSenderHandle {
    token: tokio_util::sync::CancellationToken,
    join: std::thread::JoinHandle<()>,
}

impl WaveSenderHandle {
    pub fn stop(self) {
        self.token.cancel();
        if let Err(err) = self.join.join() {
            tracing::error!("wave sender thread panicked: {:?}", err);
        }
    }
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
        let worker_count = sender_worker_count();
        let mut workers = Vec::with_capacity(worker_count);
        let mut worker_txs = Vec::with_capacity(worker_count);
        let (observer_command_tx, observer_command_rx) = mpsc::unbounded_channel();
        let observer = tokio::spawn(run_observer_loop(
            Arc::clone(&self.client),
            self.started,
            self.metric_tx.clone(),
            Arc::clone(&self.response_in_flight),
            observer_command_rx,
            self.observer_tx.clone(),
            self.token.clone(),
        ));

        for _ in 0..worker_count {
            let (worker_tx, worker_rx) = mpsc::unbounded_channel();
            worker_txs.push(worker_tx);
            workers.push(tokio::spawn(run_sender_worker(
                self.started,
                self.metric_tx.clone(),
                Arc::clone(&self.response_in_flight),
                Arc::clone(&self.ready_to_send),
                worker_rx,
                observer_command_tx.clone(),
                self.token.clone(),
            )));
        }

        let mut cancelled = false;
        let mut next_worker = 0usize;
        while !self.token.is_cancelled() {
            tokio::select! {
                _ = self.token.cancelled() => {
                    cancelled = true;
                    break;
                }
                maybe_request = self.request_rx.recv() => {
                    let Some(request) = maybe_request else {
                        break;
                    };
                    if worker_txs.is_empty() {
                        break;
                    }
                    let target = next_worker % worker_txs.len();
                    next_worker = next_worker.wrapping_add(1);
                    if worker_txs[target]
                        .send(SenderWorkerCommand { request })
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }

        drop(worker_txs);
        for worker in workers {
            if cancelled {
                worker.abort();
            }
            let _ = worker.await;
        }
        drop(observer_command_tx);
        if cancelled {
            observer.abort();
        }
        let _ = observer.await;
    }
}

struct SenderDeadlineCheck<'a> {
    scheduled_elapsed_ms: u64,
    expires_at_elapsed_ms: u64,
    started: Instant,
    metric_tx: &'a mpsc::UnboundedSender<WaveMetricEvent>,
    ready_to_send: &'a Arc<AtomicUsize>,
    token: &'a tokio_util::sync::CancellationToken,
}

fn drop_if_expired(args: SenderDeadlineCheck<'_>) -> bool {
    let elapsed_ms = args.started.elapsed().as_millis() as u64;
    if args.token.is_cancelled() || elapsed_ms <= args.expires_at_elapsed_ms {
        return false;
    }

    args.ready_to_send.fetch_sub(1, Ordering::SeqCst);
    let _ = args.metric_tx.send(WaveMetricEvent::SenderLaggedStarts {
        elapsed_ms: args.scheduled_elapsed_ms,
        count: 1,
    });
    let _ = args.metric_tx.send(WaveMetricEvent::SenderQueueDepth {
        depth: args.ready_to_send.load(Ordering::SeqCst),
    });
    true
}

async fn observe_ready_request<C>(
    client: Arc<Client>,
    started: Instant,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    request: ReadyWaveRequest<C>,
    token: tokio_util::sync::CancellationToken,
) -> Option<WaveObserverEvent<C>>
where
    C: Send + 'static,
{
    let metrics_for_send = metric_tx.clone();
    let metrics_for_body = metric_tx.clone();
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
                let _ = metric_tx.send(WaveMetricEvent::HttpSendReturned {
                    elapsed_ms: started.elapsed().as_millis() as u64,
                });
            }
        },
        move || {
            let metric_tx = metrics_for_body.clone();
            async move {
                let _ = metric_tx.send(WaveMetricEvent::ResponseBodyCompleted {
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    count: 1,
                });
            }
        },
    )
    .await?;

    let (network_tx_bytes, network_rx_bytes) =
        estimate_results_network_bytes(std::slice::from_ref(&result));
    if result.request.is_some() {
        let _ = metric_tx.send(WaveMetricEvent::HttpCompleted(1));
    }
    let _ = metric_tx.send(WaveMetricEvent::NetworkBytes {
        tx: network_tx_bytes,
        rx: network_rx_bytes,
    });

    Some(WaveObserverEvent {
        cursor: request.cursor,
        result,
    })
}

async fn run_observer_request<C>(
    client: Arc<Client>,
    started: Instant,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    response_in_flight: Arc<AtomicUsize>,
    observer_tx: mpsc::UnboundedSender<WaveObserverEvent<C>>,
    request: ReadyWaveRequest<C>,
    token: tokio_util::sync::CancellationToken,
) where
    C: Send + 'static,
{
    let result = observe_ready_request(client, started, metric_tx, request, token).await;

    response_in_flight.fetch_sub(1, Ordering::SeqCst);
    if let Some(event) = result {
        let _ = observer_tx.send(event);
    }
}

async fn run_observer_loop<C>(
    client: Arc<Client>,
    started: Instant,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    response_in_flight: Arc<AtomicUsize>,
    mut observer_rx: mpsc::UnboundedReceiver<ObserverCommand<C>>,
    observer_tx: mpsc::UnboundedSender<WaveObserverEvent<C>>,
    token: tokio_util::sync::CancellationToken,
) where
    C: Send + 'static,
{
    let mut join_set = JoinSet::new();
    let mut observer_closed = false;

    loop {
        if observer_closed && join_set.is_empty() {
            break;
        }

        tokio::select! {
            _ = token.cancelled() => {
                join_set.abort_all();
                while join_set.join_next().await.is_some() {}
                response_in_flight.store(0, Ordering::SeqCst);
                break;
            }
            maybe_command = observer_rx.recv(), if !observer_closed => {
                if let Some(command) = maybe_command {
                    join_set.spawn(run_observer_request(
                        Arc::clone(&client),
                        started,
                        metric_tx.clone(),
                        Arc::clone(&response_in_flight),
                        observer_tx.clone(),
                        command.request,
                        token.clone(),
                    ));
                } else {
                    observer_closed = true;
                }
            }
            Some(_) = join_set.join_next(), if !join_set.is_empty() => {}
        }
    }

    while join_set.join_next().await.is_some() {}
}

async fn run_sender_worker<C>(
    started: Instant,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    response_in_flight: Arc<AtomicUsize>,
    ready_to_send: Arc<AtomicUsize>,
    mut worker_rx: mpsc::UnboundedReceiver<SenderWorkerCommand<C>>,
    observer_tx: mpsc::UnboundedSender<ObserverCommand<C>>,
    token: tokio_util::sync::CancellationToken,
) where
    C: Send + 'static,
{
    let mut worker_closed = false;

    loop {
        if worker_closed {
            break;
        }

        tokio::select! {
            _ = token.cancelled() => break,
            maybe_command = worker_rx.recv(), if !worker_closed => {
                let Some(command) = maybe_command else {
                    worker_closed = true;
                    continue;
                };
                let request = command.request;
                if drop_if_expired(SenderDeadlineCheck {
                    scheduled_elapsed_ms: request.scheduled_elapsed_ms,
                    expires_at_elapsed_ms: request.expires_at_elapsed_ms,
                    started,
                    metric_tx: &metric_tx,
                    ready_to_send: &ready_to_send,
                    token: &token,
                }) {
                    continue;
                }

                let dispatch_elapsed_ms = started.elapsed().as_millis() as u64;
                let _sender_queue_wait_ms =
                    dispatch_elapsed_ms.saturating_sub(request.sender_enqueued_elapsed_ms);
                ready_to_send.fetch_sub(1, Ordering::SeqCst);
                response_in_flight.fetch_add(1, Ordering::SeqCst);
                let _ = metric_tx.send(WaveMetricEvent::SenderQueueDepth {
                    depth: ready_to_send.load(Ordering::SeqCst),
                });
                let _ = metric_tx.send(WaveMetricEvent::SendTaskSpawned {
                    elapsed_ms: dispatch_elapsed_ms,
                });
                let _ = metric_tx.send(WaveMetricEvent::SendStarted {
                    elapsed_ms: dispatch_elapsed_ms,
                });
                let _ = metric_tx.send(WaveMetricEvent::DispatchStarted {
                    elapsed_ms: dispatch_elapsed_ms,
                });
                let _ = metric_tx.send(WaveMetricEvent::HttpStarted {
                    elapsed_ms: dispatch_elapsed_ms,
                });

                if observer_tx.send(ObserverCommand { request }).is_err() {
                    response_in_flight.fetch_sub(1, Ordering::SeqCst);
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
async fn run_fire_only_sender_for_test<C>(
    started: Instant,
    metric_tx: mpsc::UnboundedSender<WaveMetricEvent>,
    response_in_flight: Arc<AtomicUsize>,
    ready_to_send: Arc<AtomicUsize>,
    mut request_rx: mpsc::UnboundedReceiver<ReadyWaveRequest<C>>,
    observer_tx: mpsc::UnboundedSender<ObserverCommand<C>>,
    token: tokio_util::sync::CancellationToken,
) where
    C: Send + 'static,
{
    let (worker_tx, worker_rx) = mpsc::unbounded_channel();
    let worker = tokio::spawn(run_sender_worker(
        started,
        metric_tx,
        response_in_flight,
        ready_to_send,
        worker_rx,
        observer_tx,
        token,
    ));

    while let Some(request) = request_rx.recv().await {
        worker_tx
            .send(SenderWorkerCommand { request })
            .expect("worker should receive request");
    }
    drop(worker_tx);
    worker.await.expect("worker should finish");
}

pub fn spawn_wave_sender_thread<C>(sender: WaveSender<C>) -> WaveSenderHandle
where
    C: Send + 'static,
{
    let sender_token = sender.token.clone();
    let join = std::thread::Builder::new()
        .name("previa-wave-sender".to_owned())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(sender_worker_count())
                .thread_name("previa-wave-http")
                .enable_all()
                .build()
                .expect("failed to build previa wave sender runtime");

            runtime.block_on(sender.run());
        })
        .expect("failed to spawn previa wave sender thread");

    WaveSenderHandle {
        token: sender_token,
        join,
    }
}

fn sender_worker_count() -> usize {
    std::env::var("RUNNER_WAVE_SENDER_WORKERS")
        .or_else(|_| std::env::var("RUNNER_WAVE_SENDER_THREADS"))
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|value| value.get())
                .unwrap_or(2)
                .clamp(2, 16)
        })
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

    static SENDER_WORKERS_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn test_ready_wave_request(
        cursor: usize,
        _started: Instant,
        scheduled_elapsed_ms: u64,
        expires_at_elapsed_ms: u64,
    ) -> ReadyWaveRequest<usize> {
        let step = PipelineStep {
            id: format!("step-{cursor}"),
            name: "GET".to_owned(),
            description: None,
            method: "GET".to_owned(),
            url: "http://127.0.0.1/test".to_owned(),
            headers: HashMap::new(),
            body: None,
            operation_id: None,
            delay: None,
            retry: None,
            asserts: Vec::new(),
        };
        let context = HashMap::new();
        let prepared = previa_runner::prepare_http_step(&step, &context, None, None, None, 1, 1)
            .expect("test request should prepare");

        ReadyWaveRequest {
            step,
            cursor,
            context,
            prepared,
            specs: Arc::new(Vec::new()),
            env_groups: Arc::new(Vec::new()),
            selected_env_group_slug: None,
            scheduled_elapsed_ms,
            expires_at_elapsed_ms,
            sender_enqueued_elapsed_ms: scheduled_elapsed_ms,
        }
    }

    fn restore_sender_workers_env(
        previous_workers: Option<String>,
        previous_threads: Option<String>,
    ) {
        unsafe {
            if let Some(value) = previous_workers {
                std::env::set_var("RUNNER_WAVE_SENDER_WORKERS", value);
            } else {
                std::env::remove_var("RUNNER_WAVE_SENDER_WORKERS");
            }
            if let Some(value) = previous_threads {
                std::env::set_var("RUNNER_WAVE_SENDER_THREADS", value);
            } else {
                std::env::remove_var("RUNNER_WAVE_SENDER_THREADS");
            }
        }
    }

    #[test]
    fn sender_worker_count_uses_positive_env_value() {
        let _guard = SENDER_WORKERS_ENV_LOCK.lock().unwrap();
        let previous_workers = std::env::var("RUNNER_WAVE_SENDER_WORKERS").ok();
        let previous_threads = std::env::var("RUNNER_WAVE_SENDER_THREADS").ok();
        unsafe {
            std::env::set_var("RUNNER_WAVE_SENDER_WORKERS", "3");
            std::env::remove_var("RUNNER_WAVE_SENDER_THREADS");
        }

        assert_eq!(sender_worker_count(), 3);

        restore_sender_workers_env(previous_workers, previous_threads);
    }

    #[test]
    fn sender_worker_count_ignores_zero_env_value() {
        let _guard = SENDER_WORKERS_ENV_LOCK.lock().unwrap();
        let previous_workers = std::env::var("RUNNER_WAVE_SENDER_WORKERS").ok();
        let previous_threads = std::env::var("RUNNER_WAVE_SENDER_THREADS").ok();
        unsafe {
            std::env::set_var("RUNNER_WAVE_SENDER_WORKERS", "0");
            std::env::remove_var("RUNNER_WAVE_SENDER_THREADS");
        }

        assert!(sender_worker_count() >= 2);

        restore_sender_workers_env(previous_workers, previous_threads);
    }

    #[tokio::test]
    async fn sender_drops_expired_request_instead_of_late_catchup() {
        let started = Instant::now() - Duration::from_millis(500);
        let (metric_tx, mut metric_rx) = mpsc::unbounded_channel();
        let ready_to_send = Arc::new(AtomicUsize::new(1));
        let token = tokio_util::sync::CancellationToken::new();

        let dropped = drop_if_expired(SenderDeadlineCheck {
            scheduled_elapsed_ms: 100,
            expires_at_elapsed_ms: 200,
            started,
            metric_tx: &metric_tx,
            ready_to_send: &ready_to_send,
            token: &token,
        });

        assert!(dropped);
        assert_eq!(ready_to_send.load(Ordering::SeqCst), 0);
        assert!(matches!(
            metric_rx.try_recv(),
            Ok(WaveMetricEvent::SenderLaggedStarts {
                elapsed_ms: 100,
                count: 1
            })
        ));
    }

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
    async fn dedicated_sender_thread_stops_when_cancelled() {
        let (_request_tx, request_rx) = mpsc::unbounded_channel::<ReadyWaveRequest<()>>();
        let (observer_tx, _observer_rx) = mpsc::unbounded_channel::<WaveObserverEvent<()>>();
        let (metric_tx, _metric_rx) = mpsc::unbounded_channel();
        let response_in_flight = Arc::new(AtomicUsize::new(0));
        let ready_to_send = Arc::new(AtomicUsize::new(0));
        let token = tokio_util::sync::CancellationToken::new();

        let sender = WaveSender::new(
            Arc::new(Client::new()),
            Instant::now(),
            metric_tx,
            Arc::clone(&response_in_flight),
            Arc::clone(&ready_to_send),
            request_rx,
            observer_tx,
            token,
        );

        let handle = spawn_wave_sender_thread(sender);
        handle.stop();

        assert_eq!(response_in_flight.load(Ordering::SeqCst), 0);
        assert_eq!(ready_to_send.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn sender_fire_path_accepts_requests_without_polling_responses() {
        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let (metric_tx, mut metric_rx) = mpsc::unbounded_channel();
        let (observer_tx, mut observer_rx) = mpsc::unbounded_channel();
        let ready_to_send = Arc::new(AtomicUsize::new(0));
        let response_in_flight = Arc::new(AtomicUsize::new(0));
        let token = tokio_util::sync::CancellationToken::new();
        let started = Instant::now();

        for index in 0..128usize {
            ready_to_send.fetch_add(1, Ordering::SeqCst);
            request_tx
                .send(test_ready_wave_request(index, started, 0, 60_000))
                .expect("request should enqueue");
        }
        drop(request_tx);

        run_fire_only_sender_for_test(
            started,
            metric_tx.clone(),
            Arc::clone(&response_in_flight),
            Arc::clone(&ready_to_send),
            request_rx,
            observer_tx,
            token.clone(),
        )
        .await;

        let mut observer_commands = 0usize;
        while observer_rx.try_recv().is_ok() {
            observer_commands += 1;
        }

        let mut http_started = 0usize;
        while let Ok(event) = metric_rx.try_recv() {
            if matches!(event, WaveMetricEvent::HttpStarted { .. }) {
                http_started += 1;
            }
        }

        assert_eq!(observer_commands, 128);
        assert_eq!(http_started, 128);
        assert_eq!(ready_to_send.load(Ordering::SeqCst), 0);
        assert_eq!(response_in_flight.load(Ordering::SeqCst), 128);
    }

    #[tokio::test]
    async fn observer_decrements_in_flight_after_response_completion() {
        let server = httpmock::MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET).path("/ok");
                then.status(200)
                    .header("content-type", "application/json")
                    .json_body(serde_json::json!({"ok": true}));
            })
            .await;

        let client = Arc::new(Client::new());
        let started = Instant::now();
        let (metric_tx, _metric_rx) = mpsc::unbounded_channel();
        let (observer_command_tx, observer_command_rx) = mpsc::unbounded_channel();
        let (observer_tx, mut observer_result_rx) = mpsc::unbounded_channel();
        let response_in_flight = Arc::new(AtomicUsize::new(1));
        let token = tokio_util::sync::CancellationToken::new();

        let mut request = test_ready_wave_request(7, started, 0, 60_000);
        request.step.url = format!("{}/ok", server.base_url());
        request.prepared = previa_runner::prepare_http_step(
            &request.step,
            &request.context,
            None,
            None,
            None,
            1,
            1,
        )
        .expect("mock request should prepare");

        observer_command_tx
            .send(ObserverCommand { request })
            .expect("observer command should enqueue");
        drop(observer_command_tx);

        run_observer_loop(
            client,
            started,
            metric_tx,
            Arc::clone(&response_in_flight),
            observer_command_rx,
            observer_tx,
            token,
        )
        .await;

        assert_eq!(response_in_flight.load(Ordering::SeqCst), 0);
        let event = observer_result_rx
            .try_recv()
            .expect("observer should emit completed event");
        assert_eq!(event.cursor, 7);
        assert_eq!(event.result.status, "success");
    }

    #[tokio::test]
    async fn sender_records_dispatch_start_inside_send_task() {
        let (tx, rx) = mpsc::unbounded_channel();
        let (metric_tx, mut metric_rx) = mpsc::unbounded_channel();
        let started = Arc::new(AtomicUsize::new(0));

        let sender_started = Arc::clone(&started);
        let sender = tokio::spawn(run_test_sender_with_metric_events(
            rx,
            metric_tx,
            sender_started,
            move |_payload: usize| async move {},
        ));

        tx.send(TestReadyWaveRequest { payload: 1 }).unwrap();
        drop(tx);
        sender.await.unwrap();

        let mut dispatch_started = 0usize;
        while let Ok(event) = metric_rx.try_recv() {
            if matches!(event, WaveMetricEvent::DispatchStarted { .. }) {
                dispatch_started += 1;
            }
        }

        assert_eq!(started.load(Ordering::SeqCst), 1);
        assert_eq!(dispatch_started, 1);
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
        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let (metric_tx, mut metric_rx) = mpsc::unbounded_channel();
        let (observer_tx, mut observer_rx) = mpsc::unbounded_channel();
        let ready_to_send = Arc::new(AtomicUsize::new(0));
        let response_in_flight = Arc::new(AtomicUsize::new(0));
        let token = tokio_util::sync::CancellationToken::new();
        let started = Instant::now();

        for index in 0..2usize {
            ready_to_send.fetch_add(1, Ordering::SeqCst);
            request_tx
                .send(test_ready_wave_request(index, started, 0, 60_000))
                .expect("request should enqueue");
        }
        drop(request_tx);

        run_fire_only_sender_for_test(
            started,
            metric_tx,
            Arc::clone(&response_in_flight),
            Arc::clone(&ready_to_send),
            request_rx,
            observer_tx,
            token,
        )
        .await;

        let mut observer_commands = 0usize;
        while observer_rx.try_recv().is_ok() {
            observer_commands += 1;
        }

        let mut dispatch_started = 0usize;
        let mut http_started = 0usize;
        let mut send_started = 0usize;
        while let Ok(event) = metric_rx.try_recv() {
            if matches!(
                event,
                crate::server::wave_metrics_actor::WaveMetricEvent::DispatchStarted { .. }
            ) {
                dispatch_started += 1;
            }
            if matches!(event, WaveMetricEvent::HttpStarted { .. }) {
                http_started += 1;
            }
            if matches!(event, WaveMetricEvent::SendStarted { .. }) {
                send_started += 1;
            }
        }

        assert_eq!(observer_commands, 2);
        assert_eq!(dispatch_started, 2);
        assert_eq!(http_started, 2);
        assert_eq!(send_started, 2);
    }
}
