# Kubernetes Runner Stable Selectors Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep runner DNS and Service endpoints stable after a reservation changes from `reserved` to `running`.

**Architecture:** The plugin should keep lifecycle state as metadata for observability, but Kubernetes selectors must use only immutable labels. `previa.runvibe.com/state` remains on runner pods and resources, while StatefulSet, Service, PDB, and anti-affinity selectors use a stable subset: app name, component, and reservation id.

**Tech Stack:** Rust, kube-rs, k8s-openapi, Kubernetes StatefulSet/Service/PDB, cargo tests, GHCR image deploy to EKS.

---

## Evidence And Root Cause

During the EKS validation run on May 14, 2026:

- The plugin created runner `previa-runner-rr5460eaa2f41643deb6-0`.
- The pod changed label `previa.runvibe.com/state` from `reserved` to `running`.
- The Service `previa-runner-rr5460eaa2f41643deb6` still selected `previa.runvibe.com/state=reserved`.
- EndpointSlice for that Service became empty.
- DNS lookup from an in-cluster curl pod failed for:

```text
previa-runner-rr5460eaa2f41643deb6-0.previa-runner-rr5460eaa2f41643deb6.previa.svc.cluster.local
```

This is a plugin bug. The `502` responses from `qcrud_open` are a separate target/gateway issue and should not be fixed in this plan.

## File Structure

- Modify: `kubernetes-plugin/src/services/runner_resources.rs`
  - Add stable selector helper.
  - Use stable selector labels in StatefulSet selector, pod anti-affinity, Service selector, and PDB selector.
  - Keep lifecycle labels on resource metadata and pod template.
  - Add unit tests proving mutable state is excluded from selectors.

- No change expected: `kubernetes-plugin/src/services/kubernetes.rs`
  - `update_runner_state_label` may continue patching pod lifecycle state.
  - It already finds pods by reservation id, not by state.

- Optional deploy-only change: Kubernetes deployment image tag in the cluster.
  - Do not commit cluster-only `kubectl set image` changes unless manifests are tracked and intentionally updated.

---

### Task 1: Add Failing Tests For Stable Selectors

**Files:**
- Modify: `kubernetes-plugin/src/services/runner_resources.rs`

- [ ] **Step 1: Add tests that describe the expected selector behavior**

Add these tests inside the existing `#[cfg(test)] mod tests` in `kubernetes-plugin/src/services/runner_resources.rs`:

```rust
    #[test]
    fn runner_service_selector_does_not_include_lifecycle_state() {
        let config = PluginConfig::test_default();
        let spec = RunnerReservationSpec::new("rr_test", "rt_secret", 1);
        let service = build_runner_service(&config, &spec);

        let selector = service.spec.unwrap().selector.unwrap();

        assert_eq!(
            selector.get("app.kubernetes.io/name").map(String::as_str),
            Some("previa")
        );
        assert_eq!(
            selector
                .get("app.kubernetes.io/component")
                .map(String::as_str),
            Some("runner")
        );
        assert_eq!(
            selector
                .get("previa.runvibe.com/reservation-id")
                .map(String::as_str),
            Some("rr_test")
        );
        assert!(
            !selector.contains_key("previa.runvibe.com/state"),
            "Service selector must not depend on mutable runner state"
        );
    }

    #[test]
    fn statefulset_selector_does_not_include_lifecycle_state() {
        let config = PluginConfig::test_default();
        let spec = RunnerReservationSpec::new("rr_test", "rt_secret", 1);
        let statefulset = build_runner_statefulset(&config, &spec);

        let selector = statefulset
            .spec
            .unwrap()
            .selector
            .match_labels
            .expect("statefulset selector");

        assert_eq!(
            selector
                .get("previa.runvibe.com/reservation-id")
                .map(String::as_str),
            Some("rr_test")
        );
        assert!(
            !selector.contains_key("previa.runvibe.com/state"),
            "StatefulSet selector must not depend on mutable runner state"
        );
    }

    #[test]
    fn pod_template_keeps_lifecycle_state_label_for_observability() {
        let config = PluginConfig::test_default();
        let spec = RunnerReservationSpec::new("rr_test", "rt_secret", 1);
        let statefulset = build_runner_statefulset(&config, &spec);

        let labels = statefulset
            .spec
            .unwrap()
            .template
            .metadata
            .unwrap()
            .labels
            .expect("pod template labels");

        assert_eq!(
            labels.get("previa.runvibe.com/state").map(String::as_str),
            Some("reserved")
        );
    }

    #[test]
    fn pdb_selector_does_not_include_lifecycle_state() {
        let config = PluginConfig::test_default();
        let spec = RunnerReservationSpec::new("rr_test", "rt_secret", 2);
        let pdb = build_runner_pdb(&config, &spec);

        let selector = pdb
            .spec
            .unwrap()
            .selector
            .unwrap()
            .match_labels
            .expect("pdb selector");

        assert_eq!(
            selector
                .get("previa.runvibe.com/reservation-id")
                .map(String::as_str),
            Some("rr_test")
        );
        assert!(
            !selector.contains_key("previa.runvibe.com/state"),
            "PDB selector must not depend on mutable runner state"
        );
    }
```

- [ ] **Step 2: Run tests and confirm they fail**

Run:

```bash
cargo test -p previa-kubernetes-plugin runner_service_selector_does_not_include_lifecycle_state statefulset_selector_does_not_include_lifecycle_state pod_template_keeps_lifecycle_state_label_for_observability pdb_selector_does_not_include_lifecycle_state
```

Expected:

```text
runner_service_selector_does_not_include_lifecycle_state ... FAILED
statefulset_selector_does_not_include_lifecycle_state ... FAILED
pod_template_keeps_lifecycle_state_label_for_observability ... ok
pdb_selector_does_not_include_lifecycle_state ... FAILED
```

The failures should show that `previa.runvibe.com/state` is currently present in selectors.

---

### Task 2: Implement Stable Selector Labels

**Files:**
- Modify: `kubernetes-plugin/src/services/runner_resources.rs`

- [ ] **Step 1: Add a stable selector helper**

Immediately after `reservation_labels`, add:

```rust
pub fn reservation_selector_labels(reservation_id: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (LABEL_APP_NAME.to_owned(), APP_NAME.to_owned()),
        (LABEL_COMPONENT.to_owned(), RUNNER_COMPONENT.to_owned()),
        (LABEL_RESERVATION_ID.to_owned(), reservation_id.to_owned()),
    ])
}
```

- [ ] **Step 2: Use the stable selector in StatefulSet and anti-affinity**

In `build_runner_statefulset`, replace:

```rust
    let labels = reservation_labels(&spec.reservation_id, RunnerLifecycleState::Reserved);
    let selector = LabelSelector {
        match_labels: Some(labels.clone()),
        ..Default::default()
    };
```

with:

```rust
    let labels = reservation_labels(&spec.reservation_id, RunnerLifecycleState::Reserved);
    let selector_labels = reservation_selector_labels(&spec.reservation_id);
    let selector = LabelSelector {
        match_labels: Some(selector_labels),
        ..Default::default()
    };
```

Keep this existing anti-affinity line unchanged because it will now receive the stable selector:

```rust
label_selector: Some(selector),
```

- [ ] **Step 3: Use the stable selector in the Service**

In `build_runner_service`, replace:

```rust
    let labels = reservation_labels(&spec.reservation_id, RunnerLifecycleState::Reserved);
```

with:

```rust
    let labels = reservation_labels(&spec.reservation_id, RunnerLifecycleState::Reserved);
    let selector_labels = reservation_selector_labels(&spec.reservation_id);
```

Then replace:

```rust
            selector: Some(labels),
```

with:

```rust
            selector: Some(selector_labels),
```

- [ ] **Step 4: Use the stable selector in the PDB**

In `build_runner_pdb`, replace:

```rust
    let labels = reservation_labels(&spec.reservation_id, RunnerLifecycleState::Reserved);
```

with:

```rust
    let labels = reservation_labels(&spec.reservation_id, RunnerLifecycleState::Reserved);
    let selector_labels = reservation_selector_labels(&spec.reservation_id);
```

Then replace:

```rust
            selector: Some(LabelSelector {
                match_labels: Some(labels),
                ..Default::default()
            }),
```

with:

```rust
            selector: Some(LabelSelector {
                match_labels: Some(selector_labels),
                ..Default::default()
            }),
```

- [ ] **Step 5: Run the focused tests**

Run:

```bash
cargo test -p previa-kubernetes-plugin runner_service_selector_does_not_include_lifecycle_state statefulset_selector_does_not_include_lifecycle_state pod_template_keeps_lifecycle_state_label_for_observability pdb_selector_does_not_include_lifecycle_state
```

Expected:

```text
test result: ok
```

- [ ] **Step 6: Commit**

Run:

```bash
git add kubernetes-plugin/src/services/runner_resources.rs
git commit -m "fix: keep kubernetes runner selectors stable"
```

---

### Task 3: Add A Regression Test For Service Survivability Across State Changes

**Files:**
- Modify: `kubernetes-plugin/src/services/runner_resources.rs`

- [ ] **Step 1: Add a focused regression test**

Add this test in the same test module:

```rust
    #[test]
    fn service_selector_still_matches_running_runner_labels() {
        let config = PluginConfig::test_default();
        let spec = RunnerReservationSpec::new("rr_test", "rt_secret", 1);
        let service = build_runner_service(&config, &spec);
        let selector = service.spec.unwrap().selector.unwrap();
        let running_labels = reservation_labels(&spec.reservation_id, RunnerLifecycleState::Running);

        for (key, value) in selector {
            assert_eq!(
                running_labels.get(&key),
                Some(&value),
                "running runner labels must satisfy Service selector key {key}"
            );
        }
    }
```

- [ ] **Step 2: Run the regression test**

Run:

```bash
cargo test -p previa-kubernetes-plugin service_selector_still_matches_running_runner_labels
```

Expected:

```text
test service_selector_still_matches_running_runner_labels ... ok
```

- [ ] **Step 3: Commit**

Run:

```bash
git add kubernetes-plugin/src/services/runner_resources.rs
git commit -m "test: cover running runner service selector"
```

---

### Task 4: Verify The Plugin Package Locally

**Files:**
- No file changes expected.

- [ ] **Step 1: Run all plugin tests**

Run:

```bash
cargo test -p previa-kubernetes-plugin
```

Expected:

```text
test result: ok
```

- [ ] **Step 2: Run the required release build**

Run:

```bash
cargo build --release
```

Expected:

```text
Finished `release` profile
```

- [ ] **Step 3: Push the branch if release build succeeds**

Run:

```bash
git push
```

Expected:

```text
To github.com:runvibe/previa.git
```

---

### Task 5: Build And Deploy A New Plugin Image

**Files:**
- No source changes expected.
- Cluster target: namespace `previa`, deployment `previa-kubernetes-plugin`.

- [ ] **Step 1: Confirm the Kubernetes Plugin workflow starts from the pushed commit**

Run:

```bash
gh run list --workflow "Kubernetes Plugin" --limit 5 --json databaseId,status,conclusion,headSha,url
```

Expected:

```text
The newest run uses the pushed commit SHA and status is queued, in_progress, or completed.
```

- [ ] **Step 2: Wait for CI success**

Run:

```bash
gh run watch <run-id>
```

Expected:

```text
completed with conclusion success
```

- [ ] **Step 3: Deploy the image tag produced by CI**

Use the full commit SHA from the successful workflow:

```bash
kubectl -n previa set image deploy/previa-kubernetes-plugin \
  previa-kubernetes-plugin=ghcr.io/runvibe/previa-kubernetes-plugin:<commit-sha>
```

Expected:

```text
deployment.apps/previa-kubernetes-plugin image updated
```

- [ ] **Step 4: Wait for plugin rollout**

Run:

```bash
kubectl -n previa rollout status deploy/previa-kubernetes-plugin --timeout=180s
```

Expected:

```text
deployment "previa-kubernetes-plugin" successfully rolled out
```

---

### Task 6: Validate In EKS With A Real Reservation

**Files:**
- No source changes expected.

- [ ] **Step 1: Create a one-runner reservation**

Run:

```bash
curl -sS -X POST http://127.0.0.1:55980/internal/runner-reservations \
  -H 'content-type: application/json' \
  -d '{"executionId":"selector-regression-check","pipelineId":"selector-regression-check","count":1}' | jq .
```

Expected:

```json
{
  "status": "provisioning",
  "requestedRunners": 1
}
```

- [ ] **Step 2: Wait until the reservation is ready**

Run repeatedly with the returned reservation id:

```bash
curl -sS http://127.0.0.1:55980/internal/runner-reservations/<reservation-id> | jq .
```

Expected:

```json
{
  "status": "ready",
  "readyRunners": 1
}
```

- [ ] **Step 3: Confirm the Service selector is stable**

Run:

```bash
kubectl -n previa get svc -l previa.runvibe.com/reservation-id=<reservation-id> -o yaml
```

Expected:

```yaml
spec:
  selector:
    app.kubernetes.io/component: runner
    app.kubernetes.io/name: previa
    previa.runvibe.com/reservation-id: <reservation-id>
```

There must be no `previa.runvibe.com/state` under `spec.selector`.

- [ ] **Step 4: Mark the reservation as running through a main execution**

Start a load test from the Previa UI or API that uses the reservation path.

Expected:

```text
The runner pod label changes to previa.runvibe.com/state=running.
```

- [ ] **Step 5: Confirm EndpointSlice remains populated while running**

Run:

```bash
kubectl -n previa get endpointslice \
  -l kubernetes.io/service-name=<runner-service-name> \
  -o jsonpath='{.items[0].endpoints[0].addresses[0]}{"\n"}'
```

Expected:

```text
10.x.x.x
```

This validates the root fix: the Service still selects the pod after lifecycle state changes.

- [ ] **Step 6: Confirm DNS resolves inside the cluster**

Run:

```bash
kubectl -n previa run curl-runner-dns-check --rm -i --restart=Never \
  --image=curlimages/curl:8.10.1 --command -- sh -lc \
  'curl -sS -i --max-time 5 http://<runner-pod-dns>:7373/health'
```

Expected:

```text
HTTP/1.1 200 OK
```

- [ ] **Step 7: Confirm cleanup still works**

After the reservation expires or is cancelled:

```bash
kubectl -n previa get pods -l previa.runvibe.com/reservation-id=<reservation-id>
```

Expected:

```text
No resources found in previa namespace.
```

---

## Out Of Scope

- Fixing `HTTP 502 Bad Gateway` from `qcrud_open`.
- Changing the main-to-plugin API contract.
- Implementing plugin proxy exposure for local main access.
- Making `/data` persistent for the cluster `previa-main`.

## Self-Review

- Spec coverage: The plan fixes the confirmed selector/state mismatch, keeps lifecycle labels for observability, and includes local tests plus real EKS validation.
- Placeholder scan: No implementation step depends on unresolved placeholders or unspecified behavior.
- Type consistency: Function names are `reservation_labels` and `reservation_selector_labels`; both return `BTreeMap<String, String>` and are used directly in Kubernetes selectors.
