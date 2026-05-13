# Kubernetes Runner Reservations V0 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the AWS/Karpenter dynamic runner reservation foundation for Previa without exposing reservation details to public clients.

**Architecture:** Implement the feature in narrow, testable slices. The runner owns reservation validation and durable execution state. `previa-main` owns public capacity preview, async execution state, pipeline queueing, and internal plugin client contracts. The Kubernetes plugin is introduced behind an internal reservation API with AWS/Karpenter v0 configuration, while keeping the Previa boundary provider-neutral.

**Tech Stack:** Rust 2024, Axum, Tokio, SQLx, Reqwest, Utoipa, Kubernetes/Karpenter manifests/contracts.

---

### Task 1: Runner Reservation Guard

**Files:**
- Create: `runner/src/server/reservation.rs`
- Modify: `runner/src/server/mod.rs`
- Modify: `runner/src/server/handlers/load.rs`
- Modify: `runner/src/server/handlers/system.rs`
- Modify: `runner/src/server/models.rs`

- [ ] **Step 1: Write failing tests**

Add runner handler tests that start a runner app with reservation env/state and assert:

```rust
// runner/src/server/handlers/load.rs test module
// Missing X-Previa-Reservation-* headers returns 403 before load starts.
// Wrong reservation token returns 403.
// Correct headers allow first execution and mark reservation consumed.
```

Run:

```bash
cargo test -p previa-runner reservation
```

Expected: tests fail because reservation state and validation do not exist.

- [ ] **Step 2: Implement reservation state**

Add `ReservationState` with:

```rust
pub struct ReservationState {
    reservation_id: Option<String>,
    reservation_token: Option<String>,
    expires_at: Option<DateTime<Utc>>,
    consumed: AtomicBool,
    busy: AtomicBool,
    started_execution_count: AtomicU64,
    last_started_at: RwLock<Option<String>>,
    last_finished_at: RwLock<Option<String>>,
}
```

Read env:

```text
PREVIA_RESERVATION_ID
PREVIA_RESERVATION_TOKEN
PREVIA_RESERVATION_EXPIRES_AT
```

- [ ] **Step 3: Enforce headers on first load execution**

In `runner/src/server/handlers/load.rs`, validate:

```text
X-Previa-Reservation-Id
X-Previa-Reservation-Token
```

Only when reservation metadata exists and has not been consumed.

- [ ] **Step 4: Expose required `/info` fields**

Extend runner info with:

```json
{
  "busy": false,
  "startedExecutionCount": 0,
  "lastStartedAt": null,
  "lastFinishedAt": null
}
```

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p previa-runner reservation
cargo test -p previa-runner
```

Expected: all pass.

### Task 2: Main Capacity Preview

**Files:**
- Create: `main/src/server/services/runner_capacity.rs`
- Modify: `main/src/server/models.rs`
- Modify: `main/src/server/handlers/tests_load.rs`
- Modify: `main/src/server/handlers/mod.rs`
- Modify: `main/src/server/docs.rs`

- [ ] **Step 1: Write failing service tests**

Test:

```rust
assert_eq!(estimate_runner_count(50_000, 5_000), Ok(10));
assert_eq!(estimate_runner_count(50_001, 5_000), Ok(11));
assert!(estimate_runner_count(50_000, 0).is_err());
```

- [ ] **Step 2: Add public preview models**

Add camelCase contracts:

```rust
pub struct LoadCapacityPreviewRequest {
    pub target_rps: u64,
}

pub struct LoadCapacityPreviewResponse {
    pub target_rps: u64,
    pub rps_per_runner: u64,
    pub estimated_runner_count: usize,
    pub capacity_mode: String,
}
```

- [ ] **Step 3: Add route**

Add:

```http
POST /api/v1/tests/load/capacity-preview
```

Return dynamic/manual mode based on config available in `AppState`.

- [ ] **Step 4: Verify**

Run:

```bash
cargo test -p previa-main runner_capacity
cargo test -p previa-main
```

Expected: all pass.

### Task 3: Main Reservation Contracts

**Files:**
- Create: `main/src/server/services/kubernetes_reservations.rs`
- Modify: `main/src/server/models.rs`

- [ ] **Step 1: Write serialization tests**

Verify internal plugin request/response JSON uses camelCase and never appears in public preview response.

- [ ] **Step 2: Add client contracts**

Add:

```rust
KubernetesReservationCreateRequest
KubernetesReservationCreateResponse
KubernetesReservationReadyResponse
KubernetesReservationRunner
```

- [ ] **Step 3: Add client shell**

Implement a small `KubernetesReservationClient` using `reqwest::Client` with `create`, `get`, and `cancel` methods.

- [ ] **Step 4: Verify**

Run:

```bash
cargo test -p previa-main kubernetes_reservations
```

Expected: all pass.

### Task 4: Async Load Execution Boundary

**Files:**
- Modify: `main/src/server/handlers/tests_load.rs`
- Modify: `main/src/server/execution/load.rs`
- Modify: `main/src/server/handlers/executions.rs`

- [ ] **Step 1: Write failing handler tests**

Assert load creation can return JSON `executionId` and `status` without keeping the create response open.

- [ ] **Step 2: Add event endpoint**

Add:

```http
GET /api/v1/executions/{executionId}/events
```

Move SSE consumption to this endpoint while preserving execution snapshots.

- [ ] **Step 3: Verify**

Run:

```bash
cargo test -p previa-main load_execution_events
```

Expected: all pass.

### Task 5: Dynamic Capacity Queueing

**Files:**
- Modify: `main/src/server/execution/scheduler.rs`
- Modify: `main/src/server/execution/load.rs`

- [ ] **Step 1: Write failing scheduler tests**

Create a test where pipeline A is blocked and pipeline B can still acquire capacity.

- [ ] **Step 2: Fix scheduler**

Allow the scheduler to scan past blocked entries when lock keys do not conflict, or introduce per-pipeline FIFO queues for load executions.

- [ ] **Step 3: Verify**

Run:

```bash
cargo test -p previa-main scheduler
```

Expected: all pass.

### Task 6: AWS/Karpenter Plugin Skeleton

**Files:**
- Create: `kubernetes-plugin/Cargo.toml`
- Create: `kubernetes-plugin/src/main.rs`
- Create: `kubernetes-plugin/src/routes.rs`
- Create: `kubernetes-plugin/src/services/reservations.rs`
- Create: `kubernetes-plugin/src/services/karpenter.rs`
- Create: `kubernetes-plugin/src/models.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Write API contract tests**

Assert create reservation returns `reservationId`, provisioning status, requested runner count, and ready runner count.

- [ ] **Step 2: Add in-memory reservation reconciler**

Implement v0 API shape with in-memory state first. Karpenter operations remain behind the service trait.

- [ ] **Step 3: Add AWS/Karpenter model structs**

Represent `resourceMode`, `NodePool`, `EC2NodeClass`, labels, taints, tolerations, and instance requirements.

- [ ] **Step 4: Verify**

Run:

```bash
cargo test -p previa-kubernetes-plugin
```

Expected: all pass.

### Task 7: Release Verification

**Files:**
- Modify as needed from prior tasks.

- [ ] **Step 1: Run focused tests**

```bash
cargo test -p previa-runner
cargo test -p previa-main
cargo test -p previa-kubernetes-plugin
```

- [ ] **Step 2: Run release build**

```bash
cargo build --release
```

- [ ] **Step 3: Commit and push**

```bash
git add .
git commit -m "Implement Kubernetes runner reservation foundation"
git push
```
