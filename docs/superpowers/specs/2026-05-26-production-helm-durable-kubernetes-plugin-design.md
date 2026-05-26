# Production Helm and Durable Kubernetes Plugin Design

## Decision

Previa production Kubernetes installs use one Helm release with separate
`previa-main` and `previa-kubernetes-plugin` Deployments. Helm composes the
components and injects the plugin service URL into `previa-main`; the plugin
does not own or supervise the main API.

## Database

Production Postgres is an external dependency. The chart reads connection
strings from Kubernetes Secrets instead of installing Postgres by default.
`previa-main` and the Kubernetes plugin use separate database URLs so the
plugin can own a small schema and run with narrower database permissions.

## Durable Plugin State

The Kubernetes plugin persists reservations and physical runner records before
returning reservation responses. On startup it reloads non-terminal
reservations and runner records, then normal reconciliation polls Kubernetes and
runner health to repair stale status. A plugin restart must not orphan active
reservations, lose ready runner endpoints, or forget idle reusable runners.

## Helm Shape

The chart includes:

- `previa-main` Deployment, Service, optional Ingress, and PDB.
- `previa-kubernetes-plugin` Deployment, Service, ServiceAccount, Role,
  RoleBinding, and PDB.
- Values for runner image, runner resources, Karpenter NodePool selection,
  reservation TTLs, and database Secrets.

Postgres subcharts are not enabled for production. A future sandbox values file
may opt into a bundled database explicitly.
