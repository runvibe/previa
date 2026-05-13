# Kubernetes Runner Reservations Stabilization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Previa Kubernetes runner reservations production-usable on AWS/Karpenter by replacing static endpoints with real Kubernetes-managed runner lifecycle, protecting active runners from disruption, and reporting load-test capacity failures accurately.

**Architecture:** Keep HTTP transport in `routes/` and handlers, move reusable logic into services, and keep data contracts in models. The Kubernetes plugin owns runner provisioning, reservation lifecycle, idle cleanup, and Kubernetes/Karpenter integration. `previa-main` owns public load-test orchestration, per-pipeline queueing, capacity preview, and internal plugin polling. `previa-runner` owns reservation gate validation, health/readiness semantics, execution busy state, and backpressure.

**Tech Stack:** Rust 2024, Axum, Tokio, SQLx, Reqwest, Serde, Utoipa, Kubernetes API via `kube`, Karpenter NodePools/NodeClaims, Kubernetes StatefulSet/Service/PDB manifests, React/TypeScript/Vitest for UI status where needed.

---

## Confirmed Problems

This plan addresses these confirmed problems:

- The Kubernetes plugin does not provision runners and only uses `PREVIA_STATIC_RUNNER_ENDPOINTS`.
- Plugin reservations can stay in `provisioning` forever when static capacity is insufficient.
- Runner/reservation lifecycle is incomplete for `reserved`, `ready`, `running`, `idle`, `draining`, and `terminating`.
- Idle runner cleanup is not implemented.
- The in-cluster plugin is configured with `127.0.0.1` runner endpoints, which is invalid inside Kubernetes.
- No `PodDisruptionBudget` protects runners or the plugin.
- Karpenter evicts runner/plugin pods as `Underutilized`.
- Runner probes use `timeoutSeconds: 1`, which is too aggressive under load.
- Load history `success` ignores whether target RPS was actually sustained.
- Runner backpressure is diagnostic but not protective enough.
- Metrics exist, but Previa does not convert them into clear capacity diagnoses.

## File Structure

Create or modify these files:

- `kubernetes-plugin/Cargo.toml`: add Kubernetes API and error dependencies.
- `kubernetes-plugin/src/models.rs`: reservation, runner, lifecycle, config, and error contracts.
- `kubernetes-plugin/src/main.rs`: construct services from config.
- `kubernetes-plugin/src/routes.rs`: keep HTTP routes thin.
- `kubernetes-plugin/src/services/config.rs`: parse plugin configuration from env.
- `kubernetes-plugin/src/services/kubernetes.rs`: Kubernetes client wrapper and resource CRUD.
- `kubernetes-plugin/src/services/runner_resources.rs`: build labels, StatefulSet, Service, PDB, and runner env.
- `kubernetes-plugin/src/services/reservations.rs`: reservation state machine and lifecycle transitions.
- `kubernetes-plugin/src/services/reconciler.rs`: provision, observe, idle cleanup, and termination loop.
- `kubernetes-plugin/src/services/runner_health.rs`: query runner `/info` and `/health`.
- `main/src/server/models.rs`: internal plugin status additions and load status contracts.
- `main/src/server/services/kubernetes_reservations.rs`: client updates for failure reasons and lifecycle states.
- `main/src/server/execution/load.rs`: consume richer reservation status and write load status.
- `main/src/server/execution/history_capture.rs`: classify load result beyond functional success.
- `main/src/server/execution/load_batch.rs`: aggregate new diagnosis fields.
- `runner/src/server/models.rs`: runner info/readiness and backpressure fields.
- `runner/src/server/reservation.rs`: finish lifecycle and consumed/busy state semantics.
- `runner/src/server/handlers/system.rs`: add `/ready` and richer `/info`.
- `runner/src/server/wave_sender.rs`: enforce in-flight and queue limits.
- `runner/src/server/wave_executor.rs`: surface backpressure and saturation metrics.
- `runner/src/server/wave_metrics_actor.rs`: record backpressure metrics.
- `runner/src/server/metrics.rs`: include diagnosis inputs in snapshots.
- `app/src/types/load-test.ts`: load status and diagnosis fields.
- `app/src/lib/remote-executor.ts`: parse new status/diagnosis fields.
- `app/src/components/LoadTestResultsPanel.tsx`: display load status/diagnosis.
- `deploy/kubernetes/previa-runner.yaml`: if deployment manifests exist there later, update runner probes/PDB; otherwise document the generated resources in plugin tests.
- `docs/previa/kubernetes-plugin.md`: operator configuration, lifecycle, and testing guide.

---

### Task 1: Plugin Configuration And Lifecycle Contracts

**Files:**
- Modify: `kubernetes-plugin/Cargo.toml`
- Modify: `kubernetes-plugin/src/models.rs`
- Create: `kubernetes-plugin/src/services/config.rs`
- Modify: `kubernetes-plugin/src/services/mod.rs`

- [ ] **Step 1: Add failing config tests**

Add tests in `kubernetes-plugin/src/services/config.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe_for_aws_karpenter_v0() {
        let config = PluginConfig::from_pairs([
            ("PREVIA_RUNNER_NAMESPACE", "previa"),
            ("PREVIA_RUNNER_IMAGE", "gcr.io/distroless/cc-debian12:nonroot"),
        ]);

        assert_eq!(config.namespace, "previa");
        assert_eq!(config.reservation_ttl_seconds, 300);
        assert_eq!(config.idle_ttl_seconds, 300);
        assert_eq!(config.runner_port, 7373);
        assert_eq!(config.provision_timeout_seconds, 300);
        assert_eq!(config.capacity_mode, CapacityMode::Kubernetes);
    }

    #[test]
    fn static_endpoints_are_dev_only() {
        let config = PluginConfig::from_pairs([
            ("PREVIA_RUNNER_NAMESPACE", "previa"),
            ("PREVIA_RUNNER_IMAGE", "runner:dev"),
            ("PREVIA_STATIC_RUNNER_ENDPOINTS", "http://127.0.0.1:17373"),
        ]);

        assert_eq!(config.capacity_mode, CapacityMode::StaticDev);
        assert_eq!(config.static_runner_endpoints, vec!["http://127.0.0.1:17373"]);
    }
}
```

Run:

```bash
cargo test -p previa-kubernetes-plugin services::config
```

Expected: fail because `PluginConfig`, `CapacityMode`, and parser do not exist.

- [ ] **Step 2: Add dependencies**

In `kubernetes-plugin/Cargo.toml`, add:

```toml
futures = "0.3"
k8s-openapi = { version = "0.24", features = ["v1_32"] }
kube = { version = "0.98", features = ["client", "runtime", "derive"] }
thiserror = "2"
```

If the workspace already pins a compatible version later, move these into workspace dependencies instead.

- [ ] **Step 3: Define config**

Create `kubernetes-plugin/src/services/config.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapacityMode {
    Kubernetes,
    StaticDev,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginConfig {
    pub namespace: String,
    pub runner_image: String,
    pub runner_port: u16,
    pub service_name: String,
    pub reservation_ttl_seconds: i64,
    pub idle_ttl_seconds: i64,
    pub provision_timeout_seconds: i64,
    pub runner_cpu_request: String,
    pub runner_memory_request: String,
    pub runner_cpu_limit: String,
    pub runner_memory_limit: String,
    pub node_pool: Option<String>,
    pub capacity_mode: CapacityMode,
    pub static_runner_endpoints: Vec<String>,
}
```

Use env names:

```text
PREVIA_RUNNER_NAMESPACE
PREVIA_RUNNER_IMAGE
PREVIA_RUNNER_PORT
PREVIA_RUNNER_SERVICE_NAME
PREVIA_RESERVATION_TTL_SECONDS
PREVIA_IDLE_TTL_SECONDS
PREVIA_PROVISION_TIMEOUT_SECONDS
PREVIA_RUNNER_CPU_REQUEST
PREVIA_RUNNER_MEMORY_REQUEST
PREVIA_RUNNER_CPU_LIMIT
PREVIA_RUNNER_MEMORY_LIMIT
PREVIA_KARPENTER_NODE_POOL
PREVIA_STATIC_RUNNER_ENDPOINTS
```

- [ ] **Step 4: Define lifecycle models**

In `kubernetes-plugin/src/models.rs`, replace stringly status with serializable enums while preserving camelCase JSON:

```rust
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReservationStatusKind {
    Provisioning,
    Ready,
    Running,
    Idle,
    Draining,
    Terminating,
    Failed,
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReservationFailureReason {
    InsufficientCapacity,
    ProvisionTimeout,
    KubernetesError,
    RunnerHealthTimeout,
    Cancelled,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RunnerLifecycleState {
    Provisioning,
    Reserved,
    Ready,
    Running,
    Idle,
    Draining,
    Terminating,
    Failed,
}
```

Add optional fields to `ReservationStatus`:

```rust
pub reason: Option<ReservationFailureReason>,
pub message: Option<String>,
pub created_at: String,
pub updated_at: String,
pub first_execution_started_at: Option<String>,
pub idle_since: Option<String>,
```

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p previa-kubernetes-plugin services::config models
```

Expected: pass.

### Task 2: Kubernetes Resource Builders

**Files:**
- Create: `kubernetes-plugin/src/services/runner_resources.rs`
- Modify: `kubernetes-plugin/src/services/mod.rs`

- [ ] **Step 1: Write failing resource builder tests**

Add tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::config::PluginConfig;

    #[test]
    fn builds_statefulset_with_one_runner_per_node_anti_affinity() {
        let config = PluginConfig::test_default();
        let spec = RunnerReservationSpec::new("rr_test", "rt_secret", 3);
        let statefulset = build_runner_statefulset(&config, &spec);

        assert_eq!(statefulset.metadata.name.as_deref(), Some("previa-runner-rr-test"));
        assert_eq!(statefulset.spec.as_ref().unwrap().replicas, Some(3));
        let template = &statefulset.spec.as_ref().unwrap().template;
        assert!(template.spec.as_ref().unwrap().affinity.is_some());
    }

    #[test]
    fn builds_headless_service_for_stable_runner_dns() {
        let config = PluginConfig::test_default();
        let spec = RunnerReservationSpec::new("rr_test", "rt_secret", 2);
        let service = build_runner_service(&config, &spec);

        assert_eq!(service.spec.as_ref().unwrap().cluster_ip.as_deref(), Some("None"));
        assert_eq!(runner_dns_name(&config, &spec, 0), "previa-runner-rr-test-0.previa-runner-rr-test.previa.svc.cluster.local");
    }

    #[test]
    fn builds_pdb_for_reserved_runners() {
        let config = PluginConfig::test_default();
        let spec = RunnerReservationSpec::new("rr_test", "rt_secret", 2);
        let pdb = build_runner_pdb(&config, &spec);

        assert!(pdb.spec.as_ref().unwrap().min_available.is_some());
    }
}
```

Run:

```bash
cargo test -p previa-kubernetes-plugin runner_resources
```

Expected: fail because builders do not exist.

- [ ] **Step 2: Define reservation resource spec**

Add:

```rust
pub struct RunnerReservationSpec {
    pub reservation_id: String,
    pub reservation_token: String,
    pub count: usize,
}
```

Normalize Kubernetes names by replacing `_` with `-` and trimming invalid characters:

```rust
pub fn reservation_resource_name(reservation_id: &str) -> String {
    format!("previa-runner-{}", reservation_id.replace('_', "-").to_ascii_lowercase())
}
```

- [ ] **Step 3: Build StatefulSet**

Build an `apps/v1::StatefulSet` with:

```text
replicas = requested count
serviceName = reservation resource name
labels:
  app.kubernetes.io/name=previa
  app.kubernetes.io/component=runner
  previa.runvibe.com/reservation-id=<reservation_id>
  previa.runvibe.com/state=reserved
env:
  ADDRESS=0.0.0.0
  PORT=<runner_port>
  RUST_LOG=info
  PREVIA_RESERVATION_ID=<reservation_id>
  PREVIA_RESERVATION_TOKEN=<reservation_token>
  PREVIA_RESERVATION_EXPIRES_AT=<expires_at>
```

Anti-affinity must require separate nodes:

```text
podAntiAffinity.requiredDuringSchedulingIgnoredDuringExecution
topologyKey = kubernetes.io/hostname
```

If `PREVIA_KARPENTER_NODE_POOL` is set, add:

```text
nodeSelector:
  karpenter.sh/nodepool=<node_pool>
```

- [ ] **Step 4: Build Service and PDB**

Build a headless Service:

```text
clusterIP = None
port = runner_port
selector = reservation labels
```

Build PDB:

```text
minAvailable = requested count while reserved/running
selector = reservation labels
```

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p previa-kubernetes-plugin runner_resources
```

Expected: pass.

### Task 3: Kubernetes Client Wrapper

**Files:**
- Create: `kubernetes-plugin/src/services/kubernetes.rs`
- Modify: `kubernetes-plugin/src/services/mod.rs`

- [ ] **Step 1: Write unit tests for resource names and dry-run operations**

Use a trait boundary so reservation logic can be tested without a real cluster:

```rust
#[async_trait::async_trait]
pub trait KubernetesRunnerApi: Send + Sync {
    async fn apply_reservation_resources(&self, spec: &RunnerReservationSpec) -> Result<(), KubernetesError>;
    async fn list_ready_runner_pods(&self, reservation_id: &str) -> Result<Vec<RunnerPod>, KubernetesError>;
    async fn update_runner_state_label(&self, reservation_id: &str, state: RunnerLifecycleState) -> Result<(), KubernetesError>;
    async fn delete_reservation_resources(&self, reservation_id: &str) -> Result<(), KubernetesError>;
}
```

Run:

```bash
cargo test -p previa-kubernetes-plugin kubernetes
```

Expected: fail because trait and structs do not exist.

- [ ] **Step 2: Implement production client**

Use `kube::Client::try_default().await`.

Implement apply using server-side apply:

```rust
let params = PatchParams::apply("previa-kubernetes-plugin").force();
api.patch(&name, &params, &Patch::Apply(resource)).await?;
```

Resources:

```text
Api<StatefulSet>
Api<Service>
Api<PodDisruptionBudget>
Api<Pod>
```

- [ ] **Step 3: Implement ready pod discovery**

Ready runner pod criteria:

```text
label previa.runvibe.com/reservation-id=<reservation_id>
condition Ready=True
podIP exists
deletionTimestamp is none
```

Return:

```rust
pub struct RunnerPod {
    pub name: String,
    pub ordinal: usize,
    pub pod_ip: String,
    pub endpoint: String,
}
```

Endpoint must use DNS, not Pod IP:

```text
http://<statefulset-name>-<ordinal>.<service-name>.<namespace>.svc.cluster.local:<port>
```

- [ ] **Step 4: Verify**

Run:

```bash
cargo test -p previa-kubernetes-plugin kubernetes runner_resources
```

Expected: pass without cluster access.

### Task 4: Plugin Reservation State Machine

**Files:**
- Modify: `kubernetes-plugin/src/services/reservations.rs`
- Modify: `kubernetes-plugin/src/routes.rs`
- Modify: `kubernetes-plugin/src/main.rs`
- Modify: `kubernetes-plugin/src/models.rs`

- [ ] **Step 1: Write failing lifecycle tests**

Add tests:

```rust
#[tokio::test]
async fn create_reservation_applies_resources_and_starts_provisioning() {
    let api = FakeKubernetesRunnerApi::default();
    let store = ReservationStore::for_test(api.clone(), PluginConfig::test_default());

    let status = store.create(ReservationCreateRequest {
        execution_id: "exec-1".to_owned(),
        pipeline_id: "pipe-1".to_owned(),
        count: 3,
    }).await.unwrap();

    assert_eq!(status.status, ReservationStatusKind::Provisioning);
    assert_eq!(status.requested_runners, 3);
    assert_eq!(api.applied_count(), 1);
}

#[tokio::test]
async fn reservation_fails_after_provision_timeout() {
    let config = PluginConfig::test_default().with_provision_timeout_seconds(1);
    let api = FakeKubernetesRunnerApi::default();
    let store = ReservationStore::for_test(api, config);

    let status = store.create(test_request(2)).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let status = store.reconcile_once(&status.reservation_id).await.unwrap();

    assert_eq!(status.status, ReservationStatusKind::Failed);
    assert_eq!(status.reason, Some(ReservationFailureReason::ProvisionTimeout));
}
```

Run:

```bash
cargo test -p previa-kubernetes-plugin reservations
```

Expected: fail.

- [ ] **Step 2: Replace simple HashMap status with reservation records**

Use:

```rust
pub struct ReservationRecord {
    pub request: ReservationCreateRequest,
    pub status: ReservationStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub token: String,
    pub first_execution_started_at: Option<DateTime<Utc>>,
    pub idle_since: Option<DateTime<Utc>>,
}
```

Keep in-memory storage for v0, but isolate it behind methods so persistence can be added later.

- [ ] **Step 3: Create reservations through Kubernetes mode**

In Kubernetes mode:

1. Generate reservation id and token.
2. Apply StatefulSet, Service, and PDB.
3. Store status `provisioning`.
4. Return `202` with `reservationId`, `status`, `requestedRunners`, `readyRunners=0`.

In `StaticDev` mode, preserve existing behavior for local development only.

- [ ] **Step 4: Reconcile ready state**

On `GET /internal/runner-reservations/{reservationId}`:

1. List ready runner pods.
2. If ready count equals requested count, return `ready`.
3. Include `reservationToken`, `expiresAt`, and runner endpoints.
4. If provision timeout exceeded, return `failed`.

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p previa-kubernetes-plugin reservations routes
```

Expected: pass.

### Task 5: Plugin Reconciler And Idle Cleanup

**Files:**
- Create: `kubernetes-plugin/src/services/reconciler.rs`
- Create: `kubernetes-plugin/src/services/runner_health.rs`
- Modify: `kubernetes-plugin/src/main.rs`
- Modify: `kubernetes-plugin/src/services/reservations.rs`

- [ ] **Step 1: Write failing idle cleanup tests**

Add fake runner health and fake Kubernetes API tests:

```rust
#[tokio::test]
async fn expired_unconsumed_reservation_is_deleted() {
    let store = test_store_with_ttls(1, 300);
    let reservation = store.create(test_request(1)).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    store.reconcile_all_once().await;

    let status = store.get(&reservation.reservation_id).await.unwrap();
    assert_eq!(status.status, ReservationStatusKind::Expired);
}

#[tokio::test]
async fn busy_runner_is_not_deleted_after_idle_ttl() {
    let store = test_store_with_runner_info(RunnerInfo { busy: true, started_execution_count: 1, ..Default::default() });
    let reservation = store.create_ready_for_test(1).await;

    store.reconcile_all_once().await;

    assert!(!store.kubernetes_api().deleted(&reservation.reservation_id));
}

#[tokio::test]
async fn idle_runner_after_first_execution_is_deleted_after_idle_ttl() {
    let store = test_store_with_ttls(300, 1);
    let reservation = store.create_ready_for_test(1).await;
    store.mark_running_for_test(&reservation.reservation_id).await;
    store.set_runner_info_for_test(RunnerInfo { busy: false, started_execution_count: 1, ..Default::default() });

    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    store.reconcile_all_once().await;

    assert!(store.kubernetes_api().deleted(&reservation.reservation_id));
}
```

Run:

```bash
cargo test -p previa-kubernetes-plugin reconciler
```

Expected: fail.

- [ ] **Step 2: Add runner health client**

Implement:

```rust
pub struct RunnerInfo {
    pub busy: bool,
    pub started_execution_count: u64,
    pub last_started_at: Option<String>,
    pub last_finished_at: Option<String>,
}

pub async fn fetch_runner_info(client: &reqwest::Client, endpoint: &str) -> Result<RunnerInfo, RunnerHealthError>;
```

Use:

```text
GET <runner-endpoint>/info
```

- [ ] **Step 3: Add reconciler loop**

In `main.rs`, spawn:

```rust
tokio::spawn(reconciler.run());
```

Loop interval env:

```text
PREVIA_RECONCILE_INTERVAL_MS
```

Default:

```text
1000
```

- [ ] **Step 4: Lifecycle rules**

Implement:

```text
provisioning + all pods ready -> ready
provisioning + timeout -> failed
ready + expiresAt elapsed + no first execution -> expired -> delete resources
ready/running + runner startedExecutionCount > 0 -> running
running + all runners busy=false -> idle
idle + idleTtl elapsed -> draining -> delete resources -> terminating
cancelled -> delete resources
failed -> delete resources unless debug retention enabled
```

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p previa-kubernetes-plugin reconciler reservations
```

Expected: pass.

### Task 6: Main Reservation Client And Failure Handling

**Files:**
- Modify: `main/src/server/models.rs`
- Modify: `main/src/server/services/kubernetes_reservations.rs`
- Modify: `main/src/server/execution/load.rs`
- Modify: `main/src/server/db/runner_reservations.rs`

- [ ] **Step 1: Write failing deserialization tests**

Add tests:

```rust
#[test]
fn reservation_status_deserializes_failed_reason() {
    let payload = serde_json::json!({
        "reservationId": "rr_1",
        "status": "failed",
        "requestedRunners": 3,
        "readyRunners": 0,
        "reason": "provisionTimeout",
        "message": "timed out waiting for runner pods",
        "runners": []
    });

    let status: KubernetesReservationStatus = serde_json::from_value(payload).unwrap();
    assert_eq!(status.status, "failed");
    assert_eq!(status.reason.as_deref(), Some("provisionTimeout"));
}
```

Run:

```bash
cargo test -p previa-main kubernetes_reservations
```

Expected: fail until fields exist.

- [ ] **Step 2: Add status fields to main models**

Extend `KubernetesReservationStatus`:

```rust
pub reason: Option<String>,
pub message: Option<String>,
pub created_at: Option<String>,
pub updated_at: Option<String>,
pub first_execution_started_at: Option<String>,
pub idle_since: Option<String>,
```

Extend `RunnerReservationRecord` and migrations only if the database must persist reason/message. If persistence is added, create:

```text
main/migrations/sqlite/202605130002_runner_reservation_status_details.sql
main/migrations/postgres/202605130002_runner_reservation_status_details.sql
```

Columns:

```sql
reservation_reason TEXT;
reservation_message TEXT;
```

- [ ] **Step 3: Improve provisioning error messages**

In `wait_for_ready_reservation`, when status is `failed`, `cancelled`, or `expired`, return:

```rust
format!(
    "runner reservation ended as {}{}{}",
    status.status,
    status.reason.as_deref().map(|r| format!(" ({r})")).unwrap_or_default(),
    status.message.as_deref().map(|m| format!(": {m}")).unwrap_or_default(),
)
```

- [ ] **Step 4: Verify**

Run:

```bash
cargo test -p previa-main kubernetes_reservations
cargo test -p previa-main execution::load
```

Expected: pass.

### Task 7: Runner Readiness, Info, And Probe Semantics

**Files:**
- Modify: `runner/src/server/handlers/system.rs`
- Modify: `runner/src/server/models.rs`
- Modify: `runner/src/server/mod.rs`
- Modify: `runner/src/server/reservation.rs`

- [ ] **Step 1: Write failing `/ready` and `/info` tests**

Add tests:

```rust
#[tokio::test]
async fn ready_reports_unavailable_while_runner_busy() {
    let app = test_app_with_busy_reservation(true);
    let response = app.oneshot(Request::builder().uri("/ready").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn info_includes_busy_and_execution_counters() {
    let app = test_app_with_started_execution_count(1);
    let response = app.oneshot(Request::builder().uri("/info").body(Body::empty()).unwrap()).await.unwrap();
    let body: serde_json::Value = read_json(response).await;
    assert_eq!(body["startedExecutionCount"], 1);
    assert_eq!(body["busy"], false);
}
```

Run:

```bash
cargo test -p previa-runner handlers::system reservation
```

Expected: fail until `/ready` exists and `/info` includes fields.

- [ ] **Step 2: Add `/ready`**

Rules:

```text
200 if process alive and not busy
503 if busy or shutting down
```

Keep `/health` cheap:

```text
200 if process is alive
```

- [ ] **Step 3: Keep reservation consumed after first execution**

Do not require reservation headers after `started_execution_count > 0`.

Ensure:

```rust
has_active_reservation_gate() == reservation_id.is_some()
    && reservation_token.is_some()
    && !consumed
```

- [ ] **Step 4: Verify**

Run:

```bash
cargo test -p previa-runner reservation handlers::system
cargo test -p previa-runner
```

Expected: pass.

### Task 8: Runner Backpressure Enforcement

**Files:**
- Modify: `runner/src/server/models.rs`
- Modify: `runner/src/server/wave_sender.rs`
- Modify: `runner/src/server/wave_executor.rs`
- Modify: `runner/src/server/wave_metrics_actor.rs`
- Modify: `runner/src/server/metrics.rs`

- [ ] **Step 1: Write failing sender limit tests**

Add tests in `runner/src/server/wave_sender.rs`:

```rust
#[tokio::test]
async fn sender_does_not_start_above_max_in_flight() {
    let config = SenderBackpressureConfig { max_in_flight: 2, max_ready_queue: 10 };
    let result = run_sender_backpressure_test(config, 5).await;

    assert!(result.max_observed_in_flight <= 2);
    assert!(result.backpressure_events > 0);
}

#[tokio::test]
async fn sender_drops_or_skips_when_ready_queue_limit_is_exceeded() {
    let config = SenderBackpressureConfig { max_in_flight: 1, max_ready_queue: 2 };
    let result = run_sender_backpressure_test(config, 20).await;

    assert!(result.queue_limited_starts > 0);
}
```

Run:

```bash
cargo test -p previa-runner wave_sender::tests::sender_does_not_start_above_max_in_flight
```

Expected: fail.

- [ ] **Step 2: Add config**

Read env:

```text
RUNNER_WAVE_MAX_IN_FLIGHT
RUNNER_WAVE_MAX_READY_QUEUE
```

Defaults:

```text
RUNNER_WAVE_MAX_IN_FLIGHT=1000
RUNNER_WAVE_MAX_READY_QUEUE=5000
```

- [ ] **Step 3: Enforce max in-flight**

Before spawning `start_ready_request`, check:

```rust
if response_in_flight.load(Ordering::SeqCst) >= config.max_in_flight {
    record backpressure;
    keep request queued until next poll if not expired;
}
```

If request expires while waiting, count it as:

```text
backpressureLimitedStarts
missedStarts
```

- [ ] **Step 4: Enforce queue limit**

When `ready_to_send` exceeds max queue:

```text
do not enqueue additional ready requests
record queueLimitedStarts
```

- [ ] **Step 5: Add metrics fields**

Add to runner `LoadTestMetrics`:

```rust
pub backpressure_active: Option<bool>,
pub backpressure_limited_starts: Option<usize>,
pub max_in_flight: Option<usize>,
pub max_ready_queue: Option<usize>,
```

Mirror fields in main `ConsolidatedLoadMetrics` and frontend types.

- [ ] **Step 6: Verify**

Run:

```bash
cargo test -p previa-runner wave_sender wave_metrics_actor metrics
cargo test -p previa-runner
```

Expected: pass.

### Task 9: Load Result Semantics And Diagnosis

**Files:**
- Modify: `main/src/server/models.rs`
- Modify: `main/src/server/execution/history_capture.rs`
- Modify: `main/src/server/execution/load.rs`
- Modify: `main/src/server/execution/load_batch.rs`
- Modify: `app/src/types/load-test.ts`
- Modify: `app/src/lib/remote-executor.ts`
- Modify: `app/src/components/LoadTestResultsPanel.tsx`

- [ ] **Step 1: Write failing load status tests**

Add tests:

```rust
#[test]
fn load_status_is_under_target_when_curve_adherence_is_low() {
    let metrics = ConsolidatedLoadMetrics {
        total_error: 0,
        total_success: 1000,
        total_sent: 1000,
        rps: 400.0,
        curve_adherence: Some(50.0),
        missed_starts: Some(10_000),
        ..test_consolidated_metrics()
    };

    let status = determine_load_history_status(false, Some(&metrics), true);
    assert_eq!(status, "under_target");
}

#[test]
fn load_status_is_saturated_when_backpressure_is_active() {
    let metrics = test_consolidated_metrics_with_backpressure();
    let status = determine_load_history_status(false, Some(&metrics), true);
    assert_eq!(status, "saturated");
}
```

Run:

```bash
cargo test -p previa-main history_capture
```

Expected: fail because only `success`, `error`, and `cancelled` are returned today.

- [ ] **Step 2: Add status dimensions**

Add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum LoadExecutionStatus {
    Complete,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum LoadAssertionStatus {
    Pass,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum LoadCapacityStatus {
    Sustained,
    UnderTarget,
    Saturated,
    Unknown,
}
```

Keep existing history `status` compatible by mapping:

```text
error if assertion failure or execution failure
cancelled if cancelled
saturated if capacity saturated
under_target if target not sustained
success if sustained
```

- [ ] **Step 3: Add diagnosis reasons**

Add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum LoadDiagnosisReason {
    RunnerSaturated,
    TargetSlow,
    SchedulerLag,
    SenderBacklog,
    KubernetesEviction,
    CapacityUnderprovisioned,
    Unknown,
}
```

Classify:

```text
senderLaggedStarts > 0 or backpressureLimitedStarts > 0 -> RunnerSaturated
missedStarts > 0 and targetRpsLimit > rps -> CapacityUnderprovisioned
httpSendDurationP95Ms high and responseObservationDurationP95Ms low -> TargetSlow or network send path
schedulerLaggedStarts > 0 -> SchedulerLag
runner restart/eviction signal present -> KubernetesEviction
```

- [ ] **Step 4: Update frontend parsing and display**

In `app/src/types/load-test.ts`, add:

```ts
export type LoadCapacityStatus = "sustained" | "underTarget" | "saturated" | "unknown";
export type LoadDiagnosisReason =
  | "runnerSaturated"
  | "targetSlow"
  | "schedulerLag"
  | "senderBacklog"
  | "kubernetesEviction"
  | "capacityUnderprovisioned"
  | "unknown";
```

Show compact status in `LoadTestResultsPanel`, near existing metrics:

```text
Load status: Sustained / Under target / Saturated
Diagnosis: Runner saturated / Target slow / ...
```

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p previa-main history_capture load_batch
cd app && npm test -- LoadTestResultsPanel remote-executor
```

Expected: pass.

### Task 10: Kubernetes Manifests And Operational Defaults

**Files:**
- Create: `docs/previa/kubernetes-plugin.md`
- Modify: `Dockerfile.kubernetes-plugin`
- Modify: deployment manifests if present in repo
- Modify: `kubernetes-plugin/src/services/runner_resources.rs`

- [ ] **Step 1: Write manifest snapshot tests**

In `runner_resources` tests, assert generated probe defaults:

```rust
#[test]
fn runner_probes_use_tolerant_timeouts() {
    let statefulset = build_runner_statefulset(&PluginConfig::test_default(), &test_spec(1));
    let container = &statefulset.spec.unwrap().template.spec.unwrap().containers[0];

    assert_eq!(container.readiness_probe.as_ref().unwrap().timeout_seconds, Some(2));
    assert_eq!(container.liveness_probe.as_ref().unwrap().timeout_seconds, Some(3));
    assert_eq!(container.liveness_probe.as_ref().unwrap().failure_threshold, Some(5));
}
```

Run:

```bash
cargo test -p previa-kubernetes-plugin runner_resources::tests::runner_probes_use_tolerant_timeouts
```

Expected: fail until builder uses correct probes.

- [ ] **Step 2: Set generated probe defaults**

Use:

```yaml
readinessProbe:
  httpGet:
    path: /ready
    port: http
  initialDelaySeconds: 5
  periodSeconds: 10
  timeoutSeconds: 2
  failureThreshold: 3
livenessProbe:
  httpGet:
    path: /health
    port: http
  initialDelaySeconds: 15
  periodSeconds: 20
  timeoutSeconds: 3
  failureThreshold: 5
```

- [ ] **Step 3: Add plugin PDB**

Generated or static plugin PDB:

```yaml
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: previa-kubernetes-plugin
  namespace: previa
spec:
  minAvailable: 1
  selector:
    matchLabels:
      app: previa-kubernetes-plugin
```

- [ ] **Step 4: Document required Karpenter settings**

In `docs/previa/kubernetes-plugin.md`, document:

```text
v0 provider: AWS/Karpenter
required: one runner pod per node
required: Karpenter NodePool reachable by plugin-created runner pods
recommended: disruption budgets that do not consolidate reserved/running runner pods
```

Add an example NodePool:

```yaml
apiVersion: karpenter.sh/v1
kind: NodePool
metadata:
  name: previa-runner-small
spec:
  disruption:
    consolidationPolicy: WhenEmptyOrUnderutilized
    consolidateAfter: 5m
  template:
    metadata:
      labels:
        previa.runvibe.com/runner-pool: small
    spec:
      requirements:
        - key: kubernetes.io/arch
          operator: In
          values: ["arm64", "amd64"]
        - key: karpenter.k8s.aws/instance-size
          operator: In
          values: ["nano", "micro", "small", "medium"]
```

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p previa-kubernetes-plugin runner_resources
```

Expected: pass.

### Task 11: End-To-End Sandbox Validation

**Files:**
- Create: `docs/previa/kubernetes-plugin.md`

- [ ] **Step 1: Build release images locally**

Run:

```bash
cargo build --release
docker build -f Dockerfile.kubernetes-plugin -t ghcr.io/runvibe/previa-kubernetes-plugin:validation .
```

Expected: release build and image build pass.

- [ ] **Step 2: Push validation image**

Run:

```bash
docker push ghcr.io/runvibe/previa-kubernetes-plugin:validation
```

Expected: image pushed.

- [ ] **Step 3: Deploy plugin with Kubernetes mode**

Set:

```text
PREVIA_STATIC_RUNNER_ENDPOINTS unset
PREVIA_RUNNER_NAMESPACE=previa
PREVIA_RUNNER_IMAGE=<published previa-runner image or init-container install image>
PREVIA_KARPENTER_NODE_POOL=previa-runner-small
PREVIA_RESERVATION_TTL_SECONDS=300
PREVIA_IDLE_TTL_SECONDS=300
```

Run:

```bash
kubectl -n previa rollout restart deployment/previa-kubernetes-plugin
kubectl -n previa rollout status deployment/previa-kubernetes-plugin --timeout=180s
```

Expected: plugin ready.

- [ ] **Step 4: Validate reservation creation**

Run:

```bash
kubectl -n previa port-forward svc/previa-kubernetes-plugin 55980:80
curl -sS -X POST http://127.0.0.1:55980/internal/runner-reservations \
  -H 'content-type: application/json' \
  -d '{"executionId":"validation-1","pipelineId":"pipe-1","count":3}'
```

Expected:

```json
{
  "status": "provisioning",
  "requestedRunners": 3,
  "readyRunners": 0
}
```

Poll:

```bash
curl -sS http://127.0.0.1:55980/internal/runner-reservations/<reservationId>
```

Expected within timeout:

```json
{
  "status": "ready",
  "readyRunners": 3,
  "reservationToken": "...",
  "runners": [
    {"endpoint": "http://previa-runner-...svc.cluster.local:7373"}
  ]
}
```

- [ ] **Step 5: Validate Kubernetes resources**

Run:

```bash
kubectl -n previa get pods -l previa.runvibe.com/reservation-id=<reservationId> -o wide
kubectl -n previa get pdb
kubectl get nodeclaim -A
```

Expected:

```text
3 runner pods Ready
3 different node names
PDB exists for reservation
NodeClaims Ready
```

- [ ] **Step 6: Validate TTL before first execution**

Use short TTL in a temporary validation deployment:

```text
PREVIA_RESERVATION_TTL_SECONDS=30
```

Create a reservation and do not execute. After expiry:

```bash
curl -sS http://127.0.0.1:55980/internal/runner-reservations/<reservationId>
kubectl -n previa get pods -l previa.runvibe.com/reservation-id=<reservationId>
```

Expected:

```text
status expired
runner resources deleted or terminating
```

- [ ] **Step 7: Validate busy runners are not deleted**

Run a 60s load test and poll:

```bash
curl -sS <runner-endpoint>/info
kubectl -n previa get pods -l previa.runvibe.com/reservation-id=<reservationId>
```

Expected while running:

```text
busy=true
pods remain Running
no eviction of reservation pods
```

- [ ] **Step 8: Validate idle cleanup after execution**

After test completion and idle TTL:

```bash
kubectl -n previa get pods -l previa.runvibe.com/reservation-id=<reservationId>
```

Expected:

```text
No resources found
```

### Task 12: CI For Kubernetes Plugin

**Files:**
- Create: `.github/workflows/kubernetes-plugin.yml`

- [ ] **Step 1: Add plugin CI workflow**

Create workflow:

```yaml
name: Kubernetes Plugin

on:
  pull_request:
    paths:
      - "kubernetes-plugin/**"
      - "Dockerfile.kubernetes-plugin"
      - "Cargo.toml"
      - "Cargo.lock"
      - ".github/workflows/kubernetes-plugin.yml"
  push:
    branches: ["main"]
    paths:
      - "kubernetes-plugin/**"
      - "Dockerfile.kubernetes-plugin"
      - "Cargo.toml"
      - "Cargo.lock"
      - ".github/workflows/kubernetes-plugin.yml"

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test -p previa-kubernetes-plugin
      - run: cargo build -p previa-kubernetes-plugin --release
      - run: docker build -f Dockerfile.kubernetes-plugin -t ghcr.io/runvibe/previa-kubernetes-plugin:ci .
```

- [ ] **Step 2: Add publish job for main**

Add:

```yaml
  publish:
    if: github.ref == 'refs/heads/main'
    needs: test
    permissions:
      contents: read
      packages: write
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}
      - uses: docker/build-push-action@v6
        with:
          context: .
          file: Dockerfile.kubernetes-plugin
          push: true
          tags: |
            ghcr.io/runvibe/previa-kubernetes-plugin:latest
            ghcr.io/runvibe/previa-kubernetes-plugin:${{ github.sha }}
```

- [ ] **Step 3: Verify workflow syntax locally**

Run:

```bash
rg "ghcr.io/runvibe/previa-kubernetes-plugin" .github/workflows/kubernetes-plugin.yml
cargo test -p previa-kubernetes-plugin
```

Expected: command finds tags and tests pass.

---

## Final Verification Checklist

Run these before claiming the branch is complete:

```bash
cargo test -p previa-kubernetes-plugin
cargo test -p previa-runner
cargo test -p previa-main
cd app && npm test -- LoadTestResultsPanel remote-executor
cargo build --release
```

Sandbox verification:

```bash
kubectl -n previa get deploy previa-kubernetes-plugin
kubectl -n previa get pdb
kubectl -n previa get pods -l app.kubernetes.io/component=runner -o wide
kubectl -n previa get events --sort-by=.lastTimestamp | tail -80
```

Expected:

- Plugin is not using `PREVIA_STATIC_RUNNER_ENDPOINTS` in Kubernetes mode.
- Reservations create real Kubernetes runner resources.
- Runner endpoints returned by the plugin are Kubernetes DNS names.
- Runner pods land on individual nodes.
- PDBs exist for active reservations and plugin.
- Karpenter does not evict reserved/running runners during validation.
- Expired unconsumed reservations are deleted.
- Busy runners survive TTL.
- Idle runners are cleaned up after idle TTL.
- Load tests that miss target RPS do not appear as plain `success`.
- Saturation/backpressure is visible in metrics and diagnosis.

## Execution Order

Implement in this order:

1. Task 1: Plugin configuration and lifecycle contracts.
2. Task 2: Kubernetes resource builders.
3. Task 3: Kubernetes client wrapper.
4. Task 4: Plugin reservation state machine.
5. Task 5: Plugin reconciler and idle cleanup.
6. Task 7: Runner readiness/info/probe semantics.
7. Task 10: Kubernetes manifests and operational defaults.
8. Task 6: Main reservation client and failure handling.
9. Task 9: Load result semantics and diagnosis.
10. Task 8: Runner backpressure enforcement.
11. Task 11: Sandbox validation.
12. Task 12: CI for Kubernetes plugin.

This order gets real capacity management working before tuning high-load behavior.
