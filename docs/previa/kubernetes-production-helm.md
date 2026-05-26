# Kubernetes Production Helm

The production Helm chart installs `previa-main` and
`previa-kubernetes-plugin` as separate workloads in one release. The chart
composes the system; the plugin remains only the dynamic runner capacity
provider.

## Database Model

Production installs should use an external Postgres service such as RDS or
Aurora. The chart does not install Postgres by default and expects database URLs
from existing Kubernetes Secrets.

Use separate database URLs for `previa-main` and the Kubernetes plugin:

```bash
kubectl -n previa create secret generic previa-main-db \
  --from-literal=ORCHESTRATOR_DATABASE_URL='postgres://previa_main:secret@postgres.example:5432/previa_main'

kubectl -n previa create secret generic previa-plugin-db \
  --from-literal=PREVIA_PLUGIN_DATABASE_URL='postgres://previa_plugin:secret@postgres.example:5432/previa_plugin'
```

The plugin owns a small schema for reservation and physical runner state. It
reloads non-terminal reservations on startup, then reconciles Kubernetes pods
and runner health to recover after restarts.

## Install

Create a production values file:

```yaml
main:
  database:
    existingSecret: previa-main-db
    urlKey: ORCHESTRATOR_DATABASE_URL
  image:
    tag: "1.0.0-alpha.40"

kubernetesPlugin:
  database:
    existingSecret: previa-plugin-db
    urlKey: PREVIA_PLUGIN_DATABASE_URL
  image:
    tag: "1.0.0-alpha.40"
  runnerImage:
    tag: "1.0.0-alpha.40"
  env:
    PREVIA_KARPENTER_NODE_POOL: previa-runner-small
    PREVIA_RUNNER_CPU_REQUEST: 100m
    PREVIA_RUNNER_MEMORY_REQUEST: 128Mi
    PREVIA_RUNNER_CPU_LIMIT: 500m
    PREVIA_RUNNER_MEMORY_LIMIT: 512Mi
```

Render or install:

```bash
helm template previa ./charts/previa -n previa -f values-production.yaml
helm upgrade --install previa ./charts/previa -n previa --create-namespace -f values-production.yaml
```

## What The Chart Creates

- `previa-main` Deployment, Service, optional Ingress, and PDB.
- `previa-kubernetes-plugin` Deployment, Service, ServiceAccount, Role,
  RoleBinding, and PDB.
- Plugin RBAC for Services, Pods, StatefulSets, and PodDisruptionBudgets in the
  release namespace.
- `PREVIA_KUBERNETES_PLUGIN_URL` injected into `previa-main`.

Runner pods are not static chart resources. The plugin creates one StatefulSet,
headless Service, and PDB per runner reservation.

## Production Checks

After installing, verify:

```bash
kubectl -n previa get deploy,svc,pdb
kubectl -n previa auth can-i create statefulsets --as system:serviceaccount:previa:previa-kubernetes-plugin
kubectl -n previa logs deploy/previa-kubernetes-plugin
```

Create a reservation through `previa-main` or call the plugin directly for a
low-level check:

```bash
kubectl -n previa port-forward svc/previa-kubernetes-plugin 55980:55980
curl -sS -X POST http://127.0.0.1:55980/internal/runner-reservations \
  -H 'content-type: application/json' \
  -d '{"executionId":"validation-1","pipelineId":"pipe-1","count":1}'
```

Then inspect runner resources:

```bash
kubectl -n previa get pods -l app.kubernetes.io/component=runner -o wide
kubectl -n previa get pdb
kubectl get nodeclaim -A
```

## Restart Recovery

The plugin persists:

- logical runner reservations;
- reservation tokens and ready runner endpoints;
- physical runner records and lifecycle state.

If the plugin pod restarts, it reloads active reservations from Postgres before
starting the reconciliation loop. Ready reservations remain queryable, running
reservations continue to be protected by their PDBs, and idle reusable runners
can be rearmed for later reservations.
