# Kubernetes Runner Reservations Design

## Goal

Add an optional Kubernetes capacity plugin for Previa load tests. The plugin
provisions or reuses `previa-runner` instances on demand, with one runner per
Kubernetes node, so high-throughput tests can reserve predictable network
capacity without manually preparing runner hosts.

The plugin is additive. Previa must continue to support the current manual
runner registry and `RUNNER_ENDPOINTS` workflows when the plugin is disabled.

## Product Model

The public client does not manage runner reservations directly. A client asks
`previa-main` to run a load test, and `previa-main` decides whether to use
manual runners or dynamic Kubernetes capacity.

When dynamic capacity is enabled, `previa-main` calculates the required runner
count from a configured `rps_per_runner` value:

```text
runner_count = ceil(target_rps / rps_per_runner)
```

The client can call a capacity preview API before starting a test to see how
many runners the execution is expected to use.

## Capacity Modes

Previa supports two capacity modes:

- `manual`: default mode. Use runners registered through existing registry
  APIs, startup environment, or CLI workflows.
- `kubernetes`: optional mode. Ask the Kubernetes plugin to reserve enough
  runners for a load-test execution.

If the Kubernetes plugin is disabled or unavailable, executions configured for
dynamic capacity should fail explicitly instead of silently falling back to a
smaller manual runner pool.

## Public Main API

The public API remains execution-oriented. It should not expose reservation
tokens or runner IPs.

Capacity preview:

```http
POST /api/v1/tests/load/capacity-preview
```

Example response:

```json
{
  "target_rps": 50000,
  "rps_per_runner": 5000,
  "estimated_runner_count": 10,
  "capacity_mode": "kubernetes"
}
```

Load execution:

```http
POST /api/v1/tests/load
```

Example response while dynamic runners are being prepared:

```json
{
  "execution_id": "exec_123",
  "status": "provisioning",
  "capacity": {
    "target_rps": 50000,
    "rps_per_runner": 5000,
    "requested_runners": 10,
    "ready_runners": 0
  }
}
```

The client polls the existing execution status API. When the reservation is
ready, `previa-main` starts the load test automatically.

## Pipeline Queueing

Dynamic capacity should allow parallel executions for different pipelines, but
only one active execution per pipeline.

The pipeline is the lock key. The execution owns the concrete reservation.

```text
pipeline_id -> FIFO execution queue
execution_id -> optional reservation_id
```

When a new load-test execution is requested:

1. If the same pipeline has an active execution in `provisioning`, `running`,
   or `cancelling`, create the new execution as `queued`.
2. If no active execution exists for that pipeline, create the execution as
   `provisioning` and request a Kubernetes reservation.
3. Executions for different pipelines may create reservations and run in
   parallel.
4. When the active execution reaches a terminal state, promote the next queued
   execution for the same pipeline to `provisioning`.

Queued executions do not consume runner reservations.

## Cancellation

Cancellation is exposed through the execution API:

```http
POST /api/v1/executions/{execution_id}/cancel
```

Cancellation behavior depends on execution state:

- `queued`: cancel immediately. No reservation exists.
- `provisioning`: cancel the execution and cancel the plugin reservation.
- `running`: request runner cancellation, mark the execution as `cancelling`,
  and let runners drain safely.
- terminal states: return the current state without creating new side effects.

After an active execution finishes cancellation cleanup, `previa-main` promotes
the next queued execution for the same pipeline.

## Internal Plugin API

The Kubernetes plugin exposes an internal reservation API to `previa-main`.
This API is not intended for public clients.

Create reservation:

```http
POST /internal/runner-reservations
```

Request:

```json
{
  "execution_id": "exec_123",
  "pipeline_id": "pipe_123",
  "count": 10
}
```

Initial response:

```json
{
  "reservation_id": "rr_123",
  "status": "provisioning",
  "requested": 10,
  "ready": 0
}
```

Poll reservation:

```http
GET /internal/runner-reservations/{reservation_id}
```

Ready response:

```json
{
  "reservation_id": "rr_123",
  "status": "ready",
  "requested": 10,
  "ready": 10,
  "reservation_token": "opaque-secret",
  "expires_at": "2026-05-12T18:40:00Z",
  "runners": [
    {
      "id": "runner-1",
      "endpoint": "http://10.0.4.12:55880"
    }
  ]
}
```

Cancel reservation:

```http
POST /internal/runner-reservations/{reservation_id}/cancel
```

The plugin returns the reservation token only after all requested runners are
ready. `previa-main` stores the token with the execution reservation record and
uses it only when dispatching work to runners.

## Reservation Lifecycle

Reservations protect capacity only until each runner's first execution.

```text
provisioning -> ready -> consumed
```

Each reserved runner starts with reservation metadata:

```text
PREVIA_RESERVATION_ID
PREVIA_RESERVATION_TOKEN
PREVIA_RESERVATION_EXPIRES_AT
```

Rules:

- The reservation TTL is configured in the plugin, not supplied by the client.
- A runner reserved for an execution accepts its first load execution only when
  the reservation headers match its configured reservation.
- Once the first execution starts, the runner marks the reservation as consumed.
- After consumption, the reservation no longer controls that runner.
- After the runner becomes idle, the plugin's idle timeout controls whether the
  runner is reused or terminated.
- If the reservation expires before first use, the plugin may terminate the idle
  runner and the runner must reject execution attempts for the expired
  reservation.

## Main To Runner Authorization

Existing `RUNNER_AUTH_KEY` remains the general authentication mechanism between
`previa-main` and `previa-runner`.

Reserved runners add reservation authorization for the first execution:

```text
Authorization: <RUNNER_AUTH_KEY>
X-Previa-Reservation-Id: <reservation_id>
X-Previa-Reservation-Token: <reservation_token>
```

The runner rejects the execution when:

- reservation metadata is configured but headers are missing;
- the reservation id does not match;
- the token does not match;
- the reservation expired before first use.

The token should be a high-entropy opaque secret. A future implementation may
replace it with a signed token, but the initial contract should not depend on
clients interpreting token contents.

## Kubernetes Scheduling

The plugin must enforce one runner per Kubernetes node. This protects the
runner's access to node-level network capacity, regardless of whether the
cluster runs on AWS, GCP, Azure, another managed Kubernetes provider, or a
self-hosted Kubernetes environment.

The recommended implementation uses:

- a dedicated runner node class or node pool;
- pod labels identifying Previa runners and reservation ownership;
- pod anti-affinity or topology spread constraints keyed by hostname;
- taints and tolerations to keep unrelated workloads off runner nodes;
- a provider-specific provisioner integration to provision and remove nodes.

The plugin must expose portable node profile configuration rather than
hard-coding cloud-specific instance types. A node profile describes the capacity
shape Previa needs, while a provisioner driver maps that shape to the target
cluster implementation.

Examples:

- AWS may map a profile to Karpenter `NodePool` and EC2 instance families.
- GCP may map a profile to GKE node pools and machine types.
- Azure may map a profile to AKS node pools and VM sizes.
- Generic Kubernetes may map a profile to existing node labels, taints, and a
  Cluster Autoscaler-compatible node group.

The reservation and runner lifecycle should depend on the generic profile name,
not on provider-specific details.

## Node Provisioning Abstraction

The plugin should keep Kubernetes orchestration independent from any single
cloud by introducing a provisioner boundary:

```text
Reservation reconciler -> NodeProvisioner -> Kubernetes/cloud-specific mapping
```

The `NodeProvisioner` contract should cover:

- resolving a `node_profile` into labels, tolerations, resource requests, and
  scheduling constraints;
- ensuring enough node capacity exists for the requested runner count;
- tagging or labeling nodes and pods for reservation ownership and cleanup;
- reporting provisioning progress and capacity errors;
- releasing unused capacity when reservations expire or consumed runners become
  idle.

Provider-specific behavior belongs behind provisioner implementations such as
`karpenter`, `cluster-autoscaler`, `static-node-pool`, `gke-node-pool`, or
`aks-node-pool`. The first implementation may support one provider, but the
configuration and service boundary must make additional providers incremental
rather than a rewrite.

## Plugin Configuration

Example configuration:

```yaml
runner_capacity:
  mode: manual
  kubernetes:
    enabled: true
    endpoint: http://previa-kubernetes-plugin.default.svc.cluster.local
    rps_per_runner: 5000
    max_runners_per_execution: 100
    reservation_ttl_seconds: 300
    idle_shutdown_seconds: 300
    reservation_poll_interval_ms: 1000
    reservation_ready_timeout_seconds: 600
    node_profile: small-dedicated-runner
    provisioner:
      kind: karpenter
    node_profiles:
      small-dedicated-runner:
        runner_cpu_request: "500m"
        runner_memory_request: "512Mi"
        required_network_gbps: 5
        labels:
          previa.dev/runner-profile: small-dedicated-runner
        tolerations:
          - key: previa.dev/runner
            operator: Equal
            value: dedicated
            effect: NoSchedule
        provider:
          aws:
            instance_families: ["t4g", "c7g"]
            instance_sizes: ["nano", "micro"]
```

The plugin owns reservation TTL, idle timeout, Kubernetes scheduling profile,
and runner reuse policy. `previa-main` owns execution state, pipeline queueing,
capacity calculation, and dispatch.

Only the `provider` section should be cloud-specific. The rest of the profile
should remain portable enough that a GCP, Azure, or generic Kubernetes mapping
can be added without changing the public reservation API or the `previa-main`
integration.

## Main Architecture

Follow the existing server boundaries:

- `handlers/`: HTTP request and response handling for capacity preview,
  execution creation, cancellation, and status.
- `services/`: capacity calculation, pipeline queue promotion, reservation
  polling, and Kubernetes plugin client logic.
- `models/`: public API contracts, internal reservation records, and execution
  capacity state.
- `db/`: persisted execution-to-reservation records and queue state.

The existing runner registry remains useful. When a Kubernetes reservation is
ready, `previa-main` may upsert the runner endpoints with a source such as
`kubernetes-reservation`, but dispatch for the reserved execution should use
the reservation's explicit endpoint list rather than the global enabled runner
list.

## Plugin Architecture

The plugin should keep transport, orchestration, and Kubernetes data separated:

- `routes/`: internal reservation API.
- `services/`: reservation reconciler, runner reuse policy, idle cleanup, and
  Kubernetes client operations.
- `models/`: reservation request/response contracts, runner lease state, and
  node profile configuration.

The reconciler observes runners directly through their health or info endpoints
to determine readiness, first-use consumption, busy state, and idleness.

Runner info should expose durable enough execution state for the reconciler to
detect first use without relying only on a transient `busy` poll. Preferred
fields include `started_execution_count`, `last_started_at`, `last_finished_at`,
and `busy`.

## Error Handling

`previa-main` should make dynamic-capacity failures visible in execution state.

Examples:

- plugin unreachable: execution becomes `failed` with a capacity error;
- reservation timeout: execution becomes `failed` and reservation is cancelled;
- some runners become unhealthy during provisioning: plugin keeps reconciling
  until ready timeout, then reports failure;
- runner rejects reservation headers: execution fails with a runner authorization
  error;
- cancellation during provisioning: execution becomes `cancelled` after the
  plugin acknowledges reservation cancellation or cleanup timeout expires.

The client should not need to inspect plugin-specific errors to understand the
execution result.

## Data Model

`previa-main` needs a persisted link from execution to reservation:

```text
execution_id
pipeline_id
capacity_mode
requested_runner_count
ready_runner_count
reservation_id
reservation_token
reservation_expires_at
reservation_status
runner_endpoints_json
created_at
updated_at
```

The reservation token should be treated as secret data. It should never appear
in public execution responses, logs, or API error messages.

## Testing

Main tests:

- capacity preview calculates `ceil(target_rps / rps_per_runner)`;
- same-pipeline execution requests are queued while one is active;
- different-pipeline execution requests can provision in parallel;
- queued cancellation does not contact the plugin;
- provisioning cancellation cancels the plugin reservation;
- ready reservation starts execution automatically;
- public responses never include reservation token or runner IPs.

Runner tests:

- reserved runner rejects missing reservation headers;
- reserved runner rejects wrong reservation id or token;
- reserved runner rejects expired reservation before first use;
- reserved runner accepts the first valid execution and marks reservation
  consumed;
- consumed runner follows normal idle behavior.

Plugin tests:

- creates one runner pod per node profile allocation;
- reuses eligible idle runners when policy allows it;
- never assigns the same reserved runner to two reservations;
- marks reservations ready only when all runners are healthy;
- expires unused reservations and terminates idle reserved runners;
- preserves running consumed runners until they become idle.

## Open Decisions

The first implementation still needs concrete choices for:

- whether the Kubernetes plugin lives in this repository or a separate package;
- the exact Kubernetes client library and deployment manifest format;
- whether runner reuse is enabled in the first release or deferred behind a
  configuration flag;
- the exact execution status names used in the existing load-test history and
  SSE flows.
