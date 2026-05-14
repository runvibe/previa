# Previa Kubernetes Plugin

The Kubernetes plugin is an optional capacity provider for Previa load-test runners. In v0 it targets AWS EKS with Karpenter and keeps the external contract cloud-friendly enough to evolve later without changing the Previa main reservation flow.

## Contract

- Previa main calculates required runners from global target RPS and configured RPS per runner.
- Previa main creates one reservation per pipeline execution through the plugin API.
- The plugin creates one runner pod per Kubernetes node and returns stable runner endpoints when the reservation becomes ready.
- The reservation token is only returned to Previa main after all requested runners are ready.
- Runner reservation TTL is configured in the plugin, not by clients.
- A reservation protects runners until first execution. After first execution, idle cleanup uses the configured idle TTL.
- Expired unconsumed reservations are deleted.
- Busy runners are not deleted by idle cleanup.

## Required Settings

v0 provider:

```text
AWS EKS + Karpenter
```

Required operational behavior:

- runner pods must use required pod anti-affinity on `kubernetes.io/hostname`;
- every reservation gets a StatefulSet, a headless Service, and a PodDisruptionBudget;
- runner pods must be reachable from Previa main through the returned endpoint;
- Karpenter must be allowed to create nodes matching the configured runner NodePool;
- disruption policies must respect reservation PDBs for reserved or running runners.

## Environment

```text
PREVIA_CAPACITY_MODE=kubernetes
PREVIA_RUNNER_NAMESPACE=previa
PREVIA_RUNNER_IMAGE=ghcr.io/runvibe/previa-runner:latest
PREVIA_RUNNER_PORT=55880
PREVIA_RUNNER_SERVICE_NAME=previa-runner
PREVIA_KARPENTER_NODE_POOL=previa-runner-small
PREVIA_RESERVATION_TTL_SECONDS=300
PREVIA_IDLE_TTL_SECONDS=300
PREVIA_PROVISION_TIMEOUT_SECONDS=300
PREVIA_RUNNER_CPU_REQUEST=100m
PREVIA_RUNNER_MEMORY_REQUEST=128Mi
PREVIA_RUNNER_CPU_LIMIT=500m
PREVIA_RUNNER_MEMORY_LIMIT=512Mi
PREVIA_RECONCILE_INTERVAL_MS=1000
```

`PREVIA_STATIC_RUNNER_ENDPOINTS` is supported only for local/static development mode and should not be used for Kubernetes mode.

## Example NodePool

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

## Plugin RBAC

The plugin needs namespaced permissions to create, inspect, update, and delete
runner resources. Include `deletecollection` for pods so idle/expired
reservations can remove all runner pods selected by reservation label.

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: previa-kubernetes-plugin
  namespace: previa
rules:
  - apiGroups: [""]
    resources: ["services", "pods"]
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete", "deletecollection"]
  - apiGroups: ["apps"]
    resources: ["statefulsets"]
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
  - apiGroups: ["policy"]
    resources: ["poddisruptionbudgets"]
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
```

## Plugin PDB

Deploy the plugin itself with a PDB so Karpenter does not voluntarily evict the only plugin replica during reservation creation.

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

## Validation

```bash
cargo test -p previa-kubernetes-plugin
cargo build --release -p previa-kubernetes-plugin
docker build -f Dockerfile.kubernetes-plugin -t ghcr.io/runvibe/previa-kubernetes-plugin:validation .
```

Create and poll a reservation:

```bash
curl -sS -X POST http://127.0.0.1:55980/internal/runner-reservations \
  -H 'content-type: application/json' \
  -d '{"executionId":"validation-1","pipelineId":"pipe-1","count":3}'
curl -sS http://127.0.0.1:55980/internal/runner-reservations/<reservationId>
```

Expected Kubernetes checks:

```bash
kubectl -n previa get pods -l previa.runvibe.com/reservation-id=<reservationId> -o wide
kubectl -n previa get pdb
kubectl get nodeclaim -A
```

The reservation should have the requested number of ready runner pods, each on a different node, with a PDB for the reservation.
