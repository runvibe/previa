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

When dynamic capacity is enabled, load-test requests must include an explicit
target RPS. `previa-main` calculates the required runner count from that
requested target and a configured `rps_per_runner` value:

```text
runner_count = ceil(target_rps / rps_per_runner)
```

The client can call a capacity preview API before starting a test to see how
many runners the execution is expected to use.

This is allowed to be a breaking change for the current load-test API. V0 should
prefer a clear asynchronous execution contract over preserving the existing
streaming response shape.

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
tokens, reservation ids, or runner IPs.

V0 may break the current load-test response contract. Load-test creation should
return an execution resource immediately, and clients should follow execution
status or event endpoints instead of depending on the create request remaining
open as the runner SSE stream.

Capacity preview:

```http
POST /api/v1/tests/load/capacity-preview
```

Request:

```json
{
  "targetRps": 50000
}
```

Example response:

```json
{
  "targetRps": 50000,
  "rpsPerRunner": 5000,
  "estimatedRunnerCount": 10,
  "capacityMode": "kubernetes"
}
```

Load execution:

```http
POST /api/v1/tests/load
```

Request shape:

```json
{
  "pipelineId": "pipe_123",
  "targetRps": 50000,
  "load": {
    "points": [
      { "atMs": 0, "intensity": 10 },
      { "atMs": 60000, "intensity": 100 }
    ],
    "interpolation": "smooth"
  }
}
```

Example response while dynamic runners are being prepared:

```json
{
  "executionId": "exec_123",
  "status": "provisioning",
  "capacity": {
    "targetRps": 50000,
    "rpsPerRunner": 5000,
    "requestedRunners": 10,
    "readyRunners": 0
  }
}
```

The client polls the existing execution status API. When the reservation is
ready, `previa-main` starts the load test automatically.

The implementation may add or repurpose an execution event endpoint such as:

```http
GET /api/v1/executions/{executionId}/events
```

That endpoint, not the create request, should own SSE delivery for provisioning,
running, cancellation, and final status updates.

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

The queueing implementation must avoid head-of-line blocking across different
pipelines. A queued execution for pipeline A must not prevent pipeline B from
starting when B has no active execution and capacity can be provisioned for B.
This can be implemented as independent FIFO queues per pipeline or as a
scheduler that scans past blocked entries when locks do not conflict.

## Cancellation

Cancellation is exposed through the execution API:

```http
POST /api/v1/executions/{executionId}/cancel
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
  "executionId": "exec_123",
  "pipelineId": "pipe_123",
  "count": 10
}
```

Initial response:

```json
{
  "reservationId": "rr_123",
  "status": "provisioning",
  "requestedRunners": 10,
  "readyRunners": 0
}
```

Poll reservation:

```http
GET /internal/runner-reservations/{reservationId}
```

Ready response:

```json
{
  "reservationId": "rr_123",
  "status": "ready",
  "requestedRunners": 10,
  "readyRunners": 10,
  "reservationToken": "opaque-secret",
  "expiresAt": "2026-05-12T18:40:00Z",
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
POST /internal/runner-reservations/{reservationId}/cancel
```

The plugin returns the reservation token only after all requested runners are
ready. `previa-main` stores the token with the execution reservation record and
uses it only when dispatching work to runners.

`expiresAt` is calculated from the moment the reservation reaches `ready`, not
from the moment the reservation request is created. Reservations in
`provisioning` should use `reservation_ready_timeout_seconds`; ready-but-unused
reservations should use `reservation_ttl_seconds`.

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
- The reservation TTL starts when all runners are ready and the reservation
  token becomes available to `previa-main`.
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

- a dedicated Karpenter `NodePool` and provider-specific node class;
- pod labels identifying Previa runners and reservation ownership;
- pod anti-affinity or topology spread constraints keyed by hostname;
- taints and tolerations to keep unrelated workloads off runner nodes;
- Karpenter disruption, consolidation, and expiry settings to remove unused
  runner nodes.

The plugin must expose portable node profile configuration rather than
hard-coding cloud-specific instance types. A node profile describes the capacity
shape Previa needs, while a Karpenter provider mapping translates that shape to
the target cluster implementation.

Examples:

- AWS maps a profile to a Karpenter `NodePool`, EC2 node class, and EC2
  instance requirements.
- Azure may map a profile to AKS Node Autoprovisioning or an Azure Karpenter
  provider when available.
- Other clouds may be supported when they expose a Karpenter-compatible
  provider model.

The reservation and runner lifecycle should depend on the generic profile name,
not on provider-specific details.

## Node Provisioning Abstraction

The plugin should keep Kubernetes orchestration independent from any single
cloud while still requiring a common Karpenter-based provisioning model:

```text
Reservation reconciler -> KarpenterProvisioner -> provider-specific Karpenter mapping
```

The `KarpenterProvisioner` contract should cover:

- resolving a `node_profile` into labels, tolerations, resource requests, and
  Karpenter scheduling requirements;
- ensuring enough node capacity exists for the requested runner count;
- creating or selecting the Karpenter `NodePool` and provider node class needed
  for the profile;
- tagging or labeling Karpenter resources, nodes, and pods for reservation
  ownership and cleanup;
- reporting provisioning progress and capacity errors;
- releasing unused capacity when reservations expire or consumed runners become
  idle.

Dynamic runner capacity requires Karpenter or a Karpenter-compatible provider.
Cluster Autoscaler-only and static node pool modes are out of scope for the
dynamic provisioning path because they do not provide the same standard control
surface for per-runner node claims, constraints, expiry, and consolidation.

## Version Scope

Version 0 supports AWS Karpenter only. This keeps the first implementation
small and gives Previa one concrete, production-grade provisioning path.

Even with AWS as the only supported provider in v0, the stable contract should
remain provider-neutral at the Previa boundary:

- `previa-main` talks only to the internal reservation API;
- public execution APIs do not expose AWS, EC2, Karpenter, node claims, or
  provider-specific resource names;
- reservation records store generic `node_profile` and runner endpoint data,
  not cloud-specific instance details;
- plugin configuration keeps provider-specific fields under
  `node_profiles.<name>.provider.aws`;
- the provisioner service boundary is named around Karpenter behavior, not AWS
  APIs.

Adding another Karpenter-compatible provider later should require a new
provider mapping inside the plugin, not a change to public APIs, runner
authorization headers, execution queueing, or the `previa-main` reservation
client.

V0 requires Karpenter to be installed in the target EKS cluster before the
Previa plugin is deployed. The plugin does not install, upgrade, or own the
cluster-wide Karpenter controller, CRDs, IAM roles, IRSA configuration, subnet
discovery, or security group discovery.

The plugin owns Previa-specific AWS Karpenter resources:

- runner `NodePool` definitions;
- runner `EC2NodeClass` definitions or references;
- labels, taints, tolerations, and scheduling requirements for Previa runners;
- runner pods and services;
- owner labels and cleanup for resources tied to reservations.

V0 should support two Karpenter resource modes:

- `managed`: default. The plugin creates and reconciles Previa-owned
  `NodePool` and `EC2NodeClass` resources.
- `reference`: the operator provides existing `NodePool` and `EC2NodeClass`
  names, and the plugin only schedules runner pods against them.

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
      provider: aws
      resource_mode: managed
    node_profiles:
      small-dedicated-runner:
        runner_cpu_request: "500m"
        runner_memory_request: "512Mi"
        required_network_gbps: 5
        karpenter:
          node_pool: previa-runner-small
          expire_after: 10m
          consolidate_after: 30s
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
should remain portable enough that another Karpenter provider can be added
without changing the public reservation API or the `previa-main` integration.

## Main Architecture

Follow the existing server boundaries:

- `handlers/`: HTTP request and response handling for capacity preview,
  execution creation, cancellation, and status.
- `services/`: capacity calculation, pipeline queue promotion, reservation
  polling, and Kubernetes plugin client logic.
- `models/`: public API contracts, internal reservation records, and execution
  capacity state.
- `db/`: persisted execution-to-reservation records and queue state.

The existing public runner registry remains useful for manual capacity, but
reserved Kubernetes runners should not be exposed through public runner listing
APIs. Dynamic runners should be stored in execution-reservation records or an
internal runner lease table. Dispatch for the reserved execution must use the
reservation's explicit endpoint list rather than the global enabled runner
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

Runner info must expose durable execution state for the reconciler to detect
first use without relying only on a transient `busy` poll. Required fields
include `started_execution_count`, `last_started_at`, `last_finished_at`, and
`busy`.

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
target_rps
node_profile
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

- capacity preview calculates `ceil(targetRps / rpsPerRunner)`;
- dynamic capacity rejects requests without explicit `targetRps`;
- same-pipeline execution requests are queued while one is active;
- different-pipeline execution requests can provision in parallel;
- a blocked queued execution for one pipeline does not block another pipeline;
- queued cancellation does not contact the plugin;
- provisioning cancellation cancels the plugin reservation;
- ready reservation starts execution automatically;
- public responses never include reservation ids, reservation token, or runner
  IPs.

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
- starts reservation TTL only when the reservation reaches `ready`;
- expires unused reservations and terminates idle reserved runners;
- preserves running consumed runners until they become idle.

## Open Decisions

The first implementation still needs concrete choices for:

- whether the Kubernetes plugin lives in this repository or a separate package;
- the exact Kubernetes client library and deployment manifest format;
- whether runner reuse is enabled in the first release or deferred behind a
  configuration flag;
- the exact execution status names used in load-test history and execution
  event flows.
