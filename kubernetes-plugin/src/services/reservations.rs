use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::warn;
use uuid::Uuid;

use crate::models::{
    ReservationCreateRequest, ReservationFailureReason, ReservationRunner, ReservationStatus,
    ReservationStatusKind, RunnerLifecycleState,
};
use crate::services::config::{CapacityMode, PluginConfig};
use crate::services::kubernetes::{KubernetesError, KubernetesRunnerApi};
use crate::services::persistence::{
    PersistedPhysicalRunner, PersistedReservation, PersistedReservationState, PersistenceError,
    ReservationPersistence,
};
use crate::services::runner_health::RunnerHealthApi;
use crate::services::runner_resources::RunnerReservationSpec;

#[derive(Debug, Error)]
pub enum ReservationStoreError {
    #[error("kubernetes error: {0}")]
    Kubernetes(#[from] KubernetesError),
    #[error("persistence error: {0}")]
    Persistence(#[from] PersistenceError),
}

#[derive(Clone)]
pub struct ReservationStore {
    inner: Arc<RwLock<HashMap<String, ReservationRecord>>>,
    runners: Arc<RwLock<HashMap<String, PhysicalRunnerRecord>>>,
    config: PluginConfig,
    kubernetes: Option<Arc<dyn KubernetesRunnerApi>>,
    runner_health: Option<Arc<dyn RunnerHealthApi>>,
    persistence: Option<Arc<dyn ReservationPersistence>>,
}

#[derive(Clone)]
struct ReservationRecord {
    request: ReservationCreateRequest,
    status: ReservationStatus,
    created_at: DateTime<Utc>,
    token: String,
    physical_runner_count: usize,
    resources_applied: bool,
}

#[derive(Clone)]
struct PhysicalRunnerRecord {
    id: String,
    endpoint: String,
    physical_reservation_id: String,
    logical_reservation_id: Option<String>,
    state: RunnerLifecycleState,
    idle_since: Option<DateTime<Utc>>,
}

impl Default for ReservationStore {
    fn default() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            runners: Arc::new(RwLock::new(HashMap::new())),
            config: PluginConfig::from_pairs(std::iter::empty::<(&str, &str)>()),
            kubernetes: None,
            runner_health: None,
            persistence: None,
        }
    }
}

impl ReservationStore {
    pub fn from_config(
        config: PluginConfig,
        kubernetes: Option<Arc<dyn KubernetesRunnerApi>>,
    ) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            runners: Arc::new(RwLock::new(HashMap::new())),
            config,
            kubernetes,
            runner_health: None,
            persistence: None,
        }
    }

    pub fn with_runner_health(mut self, runner_health: Arc<dyn RunnerHealthApi>) -> Self {
        self.runner_health = Some(runner_health);
        self
    }

    pub async fn with_persistence(
        mut self,
        persistence: Arc<dyn ReservationPersistence>,
    ) -> Result<Self, ReservationStoreError> {
        let persisted = persistence.load_state().await?;
        {
            let mut reservations = self.inner.write().await;
            reservations.clear();
            for reservation in persisted.reservations {
                let created_at = DateTime::parse_from_rfc3339(&reservation.created_at)
                    .map(|value| value.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                reservations.insert(
                    reservation.status.reservation_id.clone(),
                    ReservationRecord {
                        request: reservation.request,
                        status: reservation.status,
                        created_at,
                        token: reservation.token,
                        physical_runner_count: reservation.physical_runner_count,
                        resources_applied: false,
                    },
                );
            }
        }
        {
            let mut runners = self.runners.write().await;
            runners.clear();
            for runner in persisted.runners {
                let idle_since = runner
                    .idle_since
                    .as_deref()
                    .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                    .map(|value| value.with_timezone(&Utc));
                runners.insert(
                    runner.endpoint.clone(),
                    PhysicalRunnerRecord {
                        id: runner.id,
                        endpoint: runner.endpoint,
                        physical_reservation_id: runner.physical_reservation_id,
                        logical_reservation_id: runner.logical_reservation_id,
                        state: runner.state,
                        idle_since,
                    },
                );
            }
        }
        self.persistence = Some(persistence);
        Ok(self)
    }

    #[cfg(test)]
    pub fn for_test(api: Arc<dyn KubernetesRunnerApi>, config: PluginConfig) -> Self {
        Self::from_config(config, Some(api))
    }

    pub async fn create(
        &self,
        request: ReservationCreateRequest,
    ) -> Result<ReservationStatus, ReservationStoreError> {
        let reservation_id = format!("rr_{}", Uuid::new_v4());
        let token = format!("rt_{}", Uuid::new_v4());
        let now = Utc::now();
        let expires_at =
            Some((now + Duration::seconds(self.config.reservation_ttl_seconds)).to_rfc3339());
        let mut physical_runner_count = 0;
        let mut spec_to_apply = None;
        let mut status = ReservationStatus {
            reservation_id: reservation_id.clone(),
            status: ReservationStatusKind::Provisioning,
            requested_runners: request.count,
            ready_runners: 0,
            reservation_token: None,
            expires_at: None,
            reason: None,
            message: None,
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
            first_execution_started_at: None,
            idle_since: None,
            runners: Vec::new(),
        };

        match self.config.capacity_mode {
            CapacityMode::StaticDev => {
                if self.config.static_runner_endpoints.len() >= request.count {
                    status.status = ReservationStatusKind::Ready;
                    status.ready_runners = request.count;
                    status.reservation_token = Some(token.clone());
                    status.expires_at = expires_at.clone();
                    status.runners = self
                        .config
                        .static_runner_endpoints
                        .iter()
                        .take(request.count)
                        .enumerate()
                        .map(|(index, endpoint)| ReservationRunner {
                            id: format!("runner-{}", index + 1),
                            endpoint: endpoint.clone(),
                        })
                        .collect();
                }
            }
            CapacityMode::Kubernetes => {
                let reused = self
                    .claim_idle_runners(
                        &reservation_id,
                        &token,
                        expires_at.as_deref(),
                        request.count,
                    )
                    .await;
                status.ready_runners = reused.len();
                status.runners = reused;
                let remaining = request.count.saturating_sub(status.ready_runners);
                if remaining == 0 && request.count > 0 {
                    status.status = ReservationStatusKind::Ready;
                    status.ready_runners = request.count;
                    status.reservation_token = Some(token.clone());
                    status.expires_at = expires_at.clone();
                }
                if self.kubernetes.is_some() {
                    if remaining > 0 {
                        physical_runner_count = remaining;
                        spec_to_apply = Some(
                            RunnerReservationSpec::new(
                                reservation_id.clone(),
                                token.clone(),
                                remaining,
                            )
                            .with_expires_at(expires_at.clone()),
                        );
                    }
                }
            }
        }

        self.inner.write().await.insert(
            reservation_id,
            ReservationRecord {
                request,
                status: status.clone(),
                created_at: now,
                token,
                physical_runner_count,
                resources_applied: false,
            },
        );
        self.persist_snapshot().await?;
        if let (Some(api), Some(spec)) = (self.kubernetes.as_ref(), spec_to_apply.as_ref()) {
            if let Err(error) = api.apply_reservation_resources(spec).await {
                {
                    let mut lock = self.inner.write().await;
                    if let Some(record) = lock.get_mut(&status.reservation_id) {
                        record.status.status = ReservationStatusKind::Failed;
                        record.status.reason = Some(ReservationFailureReason::KubernetesError);
                        record.status.message = Some(error.to_string());
                        record.status.updated_at = Utc::now().to_rfc3339();
                    }
                }
                self.persist_snapshot().await?;
                return Err(error.into());
            }
            let mut lock = self.inner.write().await;
            if let Some(record) = lock.get_mut(&status.reservation_id) {
                record.resources_applied = true;
            }
        }
        Ok(status)
    }

    pub async fn get(&self, reservation_id: &str) -> Option<ReservationStatus> {
        self.reconcile_once(reservation_id).await.ok().flatten()
    }

    pub async fn reconcile_once(
        &self,
        reservation_id: &str,
    ) -> Result<Option<ReservationStatus>, ReservationStoreError> {
        let mut lock = self.inner.write().await;
        let Some(record) = lock.get_mut(reservation_id) else {
            return Ok(None);
        };

        if matches!(
            record.status.status,
            ReservationStatusKind::Failed
                | ReservationStatusKind::Cancelled
                | ReservationStatusKind::Expired
                | ReservationStatusKind::Terminating
        ) {
            return Ok(Some(record.status.clone()));
        }

        if record.status.status == ReservationStatusKind::Provisioning
            && Utc::now()
                >= record.created_at + Duration::seconds(self.config.provision_timeout_seconds)
        {
            record.status.status = ReservationStatusKind::Failed;
            record.status.reason = Some(ReservationFailureReason::ProvisionTimeout);
            record.status.message = Some("timed out waiting for runner pods".to_owned());
            record.status.updated_at = Utc::now().to_rfc3339();
            let status = record.status.clone();
            drop(lock);
            self.persist_snapshot().await?;
            return Ok(Some(status));
        }

        if self.config.capacity_mode == CapacityMode::Kubernetes {
            if let Some(api) = self.kubernetes.as_ref() {
                if record.status.status == ReservationStatusKind::Provisioning
                    && record.physical_runner_count > 0
                    && !record.resources_applied
                {
                    api.apply_reservation_resources(
                        &RunnerReservationSpec::new(
                            reservation_id,
                            record.token.clone(),
                            record.physical_runner_count,
                        )
                        .with_expires_at(Some(
                            (record.created_at
                                + Duration::seconds(self.config.reservation_ttl_seconds))
                            .to_rfc3339(),
                        )),
                    )
                    .await?;
                    record.resources_applied = true;
                }
                let pods = api.list_ready_runner_pods(reservation_id).await?;
                self.register_physical_runner_pods(reservation_id, &pods)
                    .await;
                let mut runners = record.status.runners.clone();
                for pod in pods {
                    if runners
                        .iter()
                        .any(|runner| runner.endpoint == pod.endpoint || runner.id == pod.name)
                    {
                        continue;
                    }
                    runners.push(ReservationRunner {
                        id: pod.name,
                        endpoint: pod.endpoint,
                    });
                }
                if runners.len() >= record.request.count {
                    record.status.status = ReservationStatusKind::Ready;
                    record.status.ready_runners = record.request.count;
                    record.status.reservation_token = Some(record.token.clone());
                    record.status.expires_at = Some(
                        (record.created_at
                            + Duration::seconds(self.config.reservation_ttl_seconds))
                        .to_rfc3339(),
                    );
                    record.status.runners =
                        runners.into_iter().take(record.request.count).collect();
                    record.status.updated_at = Utc::now().to_rfc3339();
                } else if !runners.is_empty() {
                    record.status.ready_runners = runners.len();
                    record.status.runners = runners;
                    record.status.updated_at = Utc::now().to_rfc3339();
                }
            }
        }

        let status = record.status.clone();
        drop(lock);
        self.reconcile_runner_lifecycle(reservation_id).await?;
        self.persist_snapshot().await?;
        Ok(self
            .inner
            .read()
            .await
            .get(reservation_id)
            .map(|record| record.status.clone())
            .or(Some(status)))
    }

    #[cfg(test)]
    pub async fn mark_ready_for_test(
        &self,
        reservation_id: &str,
        endpoints: Vec<String>,
    ) -> Option<ReservationStatus> {
        let mut lock = self.inner.write().await;
        let record = lock.get_mut(reservation_id)?;
        record.status.status = ReservationStatusKind::Ready;
        record.status.ready_runners = endpoints.len();
        record.status.reservation_token = Some(format!("rt_{}", Uuid::new_v4()));
        record.status.expires_at = Some(
            (Utc::now() + Duration::seconds(self.config.reservation_ttl_seconds)).to_rfc3339(),
        );
        record.status.updated_at = Utc::now().to_rfc3339();
        record.status.runners = endpoints
            .into_iter()
            .enumerate()
            .map(|(index, endpoint)| ReservationRunner {
                id: format!("runner-{}", index + 1),
                endpoint,
            })
            .collect();
        record.physical_runner_count = record.status.runners.len();
        record.resources_applied = true;
        Some(record.status.clone())
    }

    pub async fn cancel(&self, reservation_id: &str) -> bool {
        let mut lock = self.inner.write().await;
        let Some(mut record) = lock.remove(reservation_id) else {
            return false;
        };
        record.status.status = ReservationStatusKind::Cancelled;
        drop(lock);
        self.release_runners_for_reservation(reservation_id).await;
        if let Some(api) = self.kubernetes.as_ref() {
            if let Err(error) = api.delete_reservation_resources(reservation_id).await {
                warn!(%reservation_id, %error, "failed to delete cancelled reservation resources");
            }
        }
        if let Err(error) = self.persist_snapshot().await {
            warn!(%reservation_id, %error, "failed to persist cancelled reservation");
        }
        true
    }

    pub async fn reconcile_all_once(&self) {
        let ids = self.inner.read().await.keys().cloned().collect::<Vec<_>>();
        for id in ids {
            let _ = self.reconcile_once(&id).await;
        }
    }

    async fn reconcile_runner_lifecycle(
        &self,
        reservation_id: &str,
    ) -> Result<(), ReservationStoreError> {
        let status = {
            let lock = self.inner.read().await;
            let Some(record) = lock.get(reservation_id) else {
                return Ok(());
            };
            record.status.clone()
        };
        if !matches!(
            status.status,
            ReservationStatusKind::Ready
                | ReservationStatusKind::Running
                | ReservationStatusKind::Idle
                | ReservationStatusKind::IdleReusable
        ) {
            return Ok(());
        }

        if status.status == ReservationStatusKind::Ready {
            if let Some(expires_at) = status
                .expires_at
                .as_deref()
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc))
            {
                if Utc::now() >= expires_at && status.first_execution_started_at.is_none() {
                    self.expire_and_delete(reservation_id).await;
                    return Ok(());
                }
            }
        }

        let Some(runner_health) = self.runner_health.as_ref() else {
            return Ok(());
        };

        if status.runners.is_empty() {
            return Ok(());
        }

        let mut infos = Vec::new();
        for runner in &status.runners {
            if let Ok(info) = runner_health.fetch_runner_info(&runner.endpoint).await {
                infos.push(info);
            }
        }
        if infos.is_empty() {
            return Ok(());
        }

        let has_started = infos.iter().any(|info| info.started_execution_count > 0);
        let any_busy = infos.iter().any(|info| info.busy);
        let all_idle_after_start = has_started && infos.iter().all(|info| !info.busy);
        let mut delete = false;
        let mut desired_state_label = None;
        {
            let mut lock = self.inner.write().await;
            let Some(record) = lock.get_mut(reservation_id) else {
                return Ok(());
            };
            if has_started && record.status.first_execution_started_at.is_none() {
                record.status.first_execution_started_at = Some(Utc::now().to_rfc3339());
            }
            if any_busy {
                record.status.status = ReservationStatusKind::Running;
                record.status.idle_since = None;
                desired_state_label = Some(RunnerLifecycleState::Running);
            } else if all_idle_after_start {
                record.status.status = ReservationStatusKind::IdleReusable;
                desired_state_label = Some(RunnerLifecycleState::IdleReusable);
                if record.status.idle_since.is_none() {
                    record.status.idle_since = Some(Utc::now().to_rfc3339());
                }
                if let Some(idle_since) = record
                    .status
                    .idle_since
                    .as_deref()
                    .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                    .map(|value| value.with_timezone(&Utc))
                {
                    delete =
                        Utc::now() >= idle_since + Duration::seconds(self.config.idle_ttl_seconds);
                }
            }
            record.status.updated_at = Utc::now().to_rfc3339();
        }

        if let Some(state) = desired_state_label.clone() {
            self.update_runner_records_for_lifecycle(reservation_id, state)
                .await;
        }

        if delete {
            self.delete_idle_reservation_if_unleased(reservation_id)
                .await;
        } else if let (Some(api), Some(state)) = (self.kubernetes.as_ref(), desired_state_label) {
            let _ = api.update_runner_state_label(reservation_id, state).await;
        }
        Ok(())
    }

    async fn expire_and_delete(&self, reservation_id: &str) {
        self.release_runners_for_reservation(reservation_id).await;
        self.delete_and_mark(reservation_id, ReservationStatusKind::Expired)
            .await;
    }

    async fn claim_idle_runners(
        &self,
        reservation_id: &str,
        reservation_token: &str,
        expires_at: Option<&str>,
        count: usize,
    ) -> Vec<ReservationRunner> {
        let Some(runner_health) = self.runner_health.as_ref() else {
            return Vec::new();
        };
        let mut claimed = Vec::new();
        while claimed.len() < count {
            let candidate = {
                let mut runners = self.runners.write().await;
                let Some((key, runner)) = runners
                    .iter_mut()
                    .find(|(_, runner)| {
                        runner.state == RunnerLifecycleState::IdleReusable
                            && runner.logical_reservation_id.is_none()
                    })
                    .map(|(key, runner)| {
                        runner.state = RunnerLifecycleState::Reserved;
                        runner.logical_reservation_id = Some(reservation_id.to_owned());
                        runner.idle_since = None;
                        (key.clone(), runner.clone())
                    })
                else {
                    break;
                };
                (key, runner)
            };

            match runner_health
                .rearm_runner(
                    &candidate.1.endpoint,
                    reservation_id,
                    reservation_token,
                    expires_at,
                )
                .await
            {
                Ok(()) => claimed.push(ReservationRunner {
                    id: candidate.1.id,
                    endpoint: candidate.1.endpoint,
                }),
                Err(error) => {
                    warn!(
                        reservation_id,
                        endpoint = %candidate.1.endpoint,
                        %error,
                        "failed to rearm idle runner"
                    );
                    let mut runners = self.runners.write().await;
                    runners.remove(&candidate.0);
                }
            }
        }
        claimed
    }

    async fn register_physical_runner_pods(
        &self,
        reservation_id: &str,
        pods: &[crate::services::kubernetes::RunnerPod],
    ) {
        let mut runners = self.runners.write().await;
        for pod in pods {
            runners
                .entry(pod.endpoint.clone())
                .or_insert_with(|| PhysicalRunnerRecord {
                    id: pod.name.clone(),
                    endpoint: pod.endpoint.clone(),
                    physical_reservation_id: reservation_id.to_owned(),
                    logical_reservation_id: Some(reservation_id.to_owned()),
                    state: RunnerLifecycleState::Reserved,
                    idle_since: None,
                });
        }
    }

    async fn update_runner_records_for_lifecycle(
        &self,
        reservation_id: &str,
        state: RunnerLifecycleState,
    ) {
        let mut runners = self.runners.write().await;
        for runner in runners.values_mut() {
            if runner.logical_reservation_id.as_deref() != Some(reservation_id) {
                continue;
            }
            runner.state = state.clone();
            if state == RunnerLifecycleState::IdleReusable {
                runner.logical_reservation_id = None;
                runner.idle_since = Some(Utc::now());
            } else {
                runner.idle_since = None;
            }
        }
    }

    async fn release_runners_for_reservation(&self, reservation_id: &str) {
        let runners = {
            self.runners
                .read()
                .await
                .values()
                .filter(|runner| runner.logical_reservation_id.as_deref() == Some(reservation_id))
                .cloned()
                .collect::<Vec<_>>()
        };
        let Some(runner_health) = self.runner_health.as_ref() else {
            return;
        };
        for runner in runners {
            if let Err(error) = runner_health.release_runner(&runner.endpoint).await {
                warn!(
                    reservation_id,
                    endpoint = %runner.endpoint,
                    %error,
                    "failed to release runner reservation"
                );
            }
            let mut lock = self.runners.write().await;
            if let Some(record) = lock.get_mut(&runner.endpoint) {
                record.logical_reservation_id = None;
                record.state = RunnerLifecycleState::IdleReusable;
                record.idle_since = Some(Utc::now());
            }
        }
    }

    async fn delete_idle_reservation_if_unleased(&self, reservation_id: &str) {
        if !self
            .physical_reservation_idle_ttl_expired(reservation_id)
            .await
        {
            return;
        }
        self.delete_and_mark(reservation_id, ReservationStatusKind::Terminating)
            .await;
    }

    async fn physical_reservation_idle_ttl_expired(&self, reservation_id: &str) -> bool {
        let runners = self.runners.read().await;
        let physical = runners
            .values()
            .filter(|runner| runner.physical_reservation_id == reservation_id)
            .collect::<Vec<_>>();
        if physical.is_empty() {
            return false;
        }
        physical.iter().all(|runner| {
            runner.logical_reservation_id.is_none()
                && runner.state == RunnerLifecycleState::IdleReusable
                && runner.idle_since.is_some_and(|idle_since| {
                    Utc::now() >= idle_since + Duration::seconds(self.config.idle_ttl_seconds)
                })
        })
    }

    async fn delete_and_mark(&self, reservation_id: &str, status: ReservationStatusKind) {
        {
            let mut lock = self.inner.write().await;
            if let Some(record) = lock.get_mut(reservation_id) {
                record.status.status = status;
                record.status.updated_at = Utc::now().to_rfc3339();
            }
        }
        if let Some(api) = self.kubernetes.as_ref() {
            if let Err(error) = api
                .update_runner_state_label(reservation_id, RunnerLifecycleState::Terminating)
                .await
            {
                warn!(%reservation_id, %error, "failed to mark runner resources as terminating");
            }
            if let Err(error) = api.delete_reservation_resources(reservation_id).await {
                warn!(%reservation_id, %error, "failed to delete reservation resources");
            }
        }
        self.runners
            .write()
            .await
            .retain(|_, runner| runner.physical_reservation_id != reservation_id);
        if let Err(error) = self.persist_snapshot().await {
            warn!(%reservation_id, %error, "failed to persist reservation deletion");
        }
    }

    async fn persist_snapshot(&self) -> Result<(), ReservationStoreError> {
        let Some(persistence) = self.persistence.as_ref() else {
            return Ok(());
        };
        let reservations = self
            .inner
            .read()
            .await
            .values()
            .map(|record| PersistedReservation {
                request: record.request.clone(),
                status: record.status.clone(),
                created_at: record.created_at.to_rfc3339(),
                token: record.token.clone(),
                physical_runner_count: record.physical_runner_count,
            })
            .collect::<Vec<_>>();
        let runners = self
            .runners
            .read()
            .await
            .values()
            .map(|runner| PersistedPhysicalRunner {
                id: runner.id.clone(),
                endpoint: runner.endpoint.clone(),
                physical_reservation_id: runner.physical_reservation_id.clone(),
                logical_reservation_id: runner.logical_reservation_id.clone(),
                state: runner.state.clone(),
                idle_since: runner.idle_since.map(|value| value.to_rfc3339()),
            })
            .collect::<Vec<_>>();
        persistence
            .save_state(PersistedReservationState {
                reservations,
                runners,
            })
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::ReservationStore;
    use crate::models::{ReservationCreateRequest, ReservationStatusKind, RunnerLifecycleState};
    use crate::services::config::PluginConfig;
    use crate::services::kubernetes::{KubernetesError, KubernetesRunnerApi, RunnerPod};
    use crate::services::runner_health::{RunnerHealthApi, RunnerHealthError, RunnerInfo};
    use crate::services::runner_resources::RunnerReservationSpec;

    #[derive(Default)]
    struct FakeKubernetesRunnerApi {
        applied: Mutex<Vec<RunnerReservationSpec>>,
        ready: Mutex<Vec<RunnerPod>>,
        deleted: Mutex<Vec<String>>,
    }

    #[derive(Default)]
    struct FakeRunnerHealthApi {
        info: Mutex<RunnerInfo>,
        rearmed: Mutex<Vec<String>>,
        released: Mutex<Vec<String>>,
        fail_rearm: Mutex<bool>,
    }

    impl FakeRunnerHealthApi {
        fn set_info(&self, info: RunnerInfo) {
            *self.info.lock().unwrap() = info;
        }

        fn rearmed_count(&self) -> usize {
            self.rearmed.lock().unwrap().len()
        }

        fn released_count(&self) -> usize {
            self.released.lock().unwrap().len()
        }

        fn fail_rearm(&self) {
            *self.fail_rearm.lock().unwrap() = true;
        }
    }

    #[async_trait]
    impl RunnerHealthApi for FakeRunnerHealthApi {
        async fn fetch_runner_info(
            &self,
            _endpoint: &str,
        ) -> Result<RunnerInfo, RunnerHealthError> {
            Ok(self.info.lock().unwrap().clone())
        }

        async fn rearm_runner(
            &self,
            endpoint: &str,
            _reservation_id: &str,
            _reservation_token: &str,
            _expires_at: Option<&str>,
        ) -> Result<(), RunnerHealthError> {
            self.rearmed.lock().unwrap().push(endpoint.to_owned());
            if *self.fail_rearm.lock().unwrap() {
                return Err(RunnerHealthError::Unavailable("runner gone".to_owned()));
            }
            *self.info.lock().unwrap() = RunnerInfo::default();
            Ok(())
        }

        async fn release_runner(&self, endpoint: &str) -> Result<(), RunnerHealthError> {
            self.released.lock().unwrap().push(endpoint.to_owned());
            Ok(())
        }
    }

    impl FakeKubernetesRunnerApi {
        fn applied_count(&self) -> usize {
            self.applied.lock().unwrap().len()
        }

        fn set_ready(&self, pods: Vec<RunnerPod>) {
            *self.ready.lock().unwrap() = pods;
        }

        fn deleted(&self, reservation_id: &str) -> bool {
            self.deleted
                .lock()
                .unwrap()
                .iter()
                .any(|item| item == reservation_id)
        }
    }

    #[async_trait]
    impl KubernetesRunnerApi for FakeKubernetesRunnerApi {
        async fn apply_reservation_resources(
            &self,
            spec: &RunnerReservationSpec,
        ) -> Result<(), KubernetesError> {
            self.applied.lock().unwrap().push(spec.clone());
            Ok(())
        }

        async fn list_ready_runner_pods(
            &self,
            _reservation_id: &str,
        ) -> Result<Vec<RunnerPod>, KubernetesError> {
            Ok(self.ready.lock().unwrap().clone())
        }

        async fn update_runner_state_label(
            &self,
            _reservation_id: &str,
            _state: RunnerLifecycleState,
        ) -> Result<(), KubernetesError> {
            Ok(())
        }

        async fn delete_reservation_resources(
            &self,
            reservation_id: &str,
        ) -> Result<(), KubernetesError> {
            self.deleted.lock().unwrap().push(reservation_id.to_owned());
            Ok(())
        }
    }

    fn test_request(count: usize) -> ReservationCreateRequest {
        ReservationCreateRequest {
            execution_id: "exec-1".to_owned(),
            pipeline_id: "pipe-1".to_owned(),
            count,
        }
    }

    #[tokio::test]
    async fn persisted_ready_reservation_survives_store_restart() {
        let db_dir = tempfile::tempdir().expect("temp db dir");
        let database_url = format!(
            "sqlite://{}",
            db_dir.path().join("plugin.sqlite3").display()
        );
        let api = std::sync::Arc::new(FakeKubernetesRunnerApi::default());
        api.set_ready(vec![RunnerPod {
            name: "runner-0".to_owned(),
            ordinal: 0,
            pod_ip: "10.20.0.10".to_owned(),
            endpoint: "http://runner-0:7373".to_owned(),
        }]);
        let persistence = std::sync::Arc::new(
            crate::services::persistence::SqlReservationPersistence::connect(&database_url)
                .await
                .expect("connect persistence"),
        );
        let store = ReservationStore::for_test(api.clone(), PluginConfig::test_default())
            .with_persistence(persistence.clone())
            .await
            .expect("load persisted reservations");

        let created = store.create(test_request(1)).await.unwrap();
        let ready = store
            .reconcile_once(&created.reservation_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ready.status, ReservationStatusKind::Ready);
        let token = ready.reservation_token.clone().expect("ready token");

        let restarted = ReservationStore::for_test(api, PluginConfig::test_default())
            .with_persistence(persistence)
            .await
            .expect("reload persisted reservations");
        let loaded = restarted
            .get(&created.reservation_id)
            .await
            .expect("reservation after restart");

        assert_eq!(loaded.status, ReservationStatusKind::Ready);
        assert_eq!(loaded.reservation_token.as_deref(), Some(token.as_str()));
        assert_eq!(loaded.runners[0].endpoint, "http://runner-0:7373");
    }

    #[tokio::test]
    async fn persisted_idle_reusable_runner_can_be_reclaimed_after_restart() {
        let db_dir = tempfile::tempdir().expect("temp db dir");
        let database_url = format!(
            "sqlite://{}",
            db_dir.path().join("plugin.sqlite3").display()
        );
        let api = std::sync::Arc::new(FakeKubernetesRunnerApi::default());
        let health = std::sync::Arc::new(FakeRunnerHealthApi::default());
        let persistence = std::sync::Arc::new(
            crate::services::persistence::SqlReservationPersistence::connect(&database_url)
                .await
                .expect("connect persistence"),
        );
        let store = ReservationStore::for_test(api.clone(), PluginConfig::test_default())
            .with_runner_health(health.clone())
            .with_persistence(persistence.clone())
            .await
            .expect("load persisted reservations");
        let first = store.create(test_request(1)).await.unwrap();
        api.set_ready(vec![RunnerPod {
            name: "runner-0".to_owned(),
            ordinal: 0,
            pod_ip: "10.20.0.10".to_owned(),
            endpoint: "http://runner-0:7373".to_owned(),
        }]);
        let _ = store.reconcile_once(&first.reservation_id).await.unwrap();
        health.set_info(RunnerInfo {
            busy: false,
            started_execution_count: 1,
            ..Default::default()
        });
        store.reconcile_all_once().await;
        assert_eq!(
            store.get(&first.reservation_id).await.unwrap().status,
            ReservationStatusKind::IdleReusable
        );

        let restarted = ReservationStore::for_test(api, PluginConfig::test_default())
            .with_runner_health(health.clone())
            .with_persistence(persistence)
            .await
            .expect("reload persisted idle runner");
        let second = restarted.create(test_request(1)).await.unwrap();

        assert_eq!(second.status, ReservationStatusKind::Ready);
        assert_eq!(second.runners[0].endpoint, "http://runner-0:7373");
        assert_eq!(health.rearmed_count(), 1);
    }

    #[tokio::test]
    async fn rehydrated_provisioning_reservation_reapplies_kubernetes_resources() {
        let db_dir = tempfile::tempdir().expect("temp db dir");
        let database_url = format!(
            "sqlite://{}",
            db_dir.path().join("plugin.sqlite3").display()
        );
        let persistence = std::sync::Arc::new(
            crate::services::persistence::SqlReservationPersistence::connect(&database_url)
                .await
                .expect("connect persistence"),
        );
        let initial_api = std::sync::Arc::new(FakeKubernetesRunnerApi::default());
        let initial = ReservationStore::for_test(initial_api, PluginConfig::test_default())
            .with_persistence(persistence.clone())
            .await
            .expect("load persisted reservations");
        let created = initial.create(test_request(2)).await.unwrap();

        let restarted_api = std::sync::Arc::new(FakeKubernetesRunnerApi::default());
        let restarted =
            ReservationStore::for_test(restarted_api.clone(), PluginConfig::test_default())
                .with_persistence(persistence)
                .await
                .expect("reload provisioning reservation");
        let status = restarted
            .reconcile_once(&created.reservation_id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(status.status, ReservationStatusKind::Provisioning);
        assert_eq!(restarted_api.applied_count(), 1);
    }

    #[tokio::test]
    async fn create_reservation_starts_in_provisioning() {
        let store = ReservationStore::default();
        let status = store.create(test_request(3)).await.unwrap();

        assert_eq!(status.status, ReservationStatusKind::Provisioning);
        assert_eq!(status.requested_runners, 3);
        assert_eq!(status.ready_runners, 0);
        assert!(status.reservation_token.is_none());
    }

    #[tokio::test]
    async fn ready_reservation_gets_token_expiry_and_runners() {
        let store = ReservationStore::default();
        let status = store.create(test_request(1)).await.unwrap();

        let ready = store
            .mark_ready_for_test(
                &status.reservation_id,
                vec!["http://10.0.0.1:55880".to_owned()],
            )
            .await
            .expect("ready reservation");

        assert_eq!(ready.status, ReservationStatusKind::Ready);
        assert_eq!(ready.ready_runners, 1);
        assert!(ready.reservation_token.is_some());
        assert!(ready.expires_at.is_some());
        assert_eq!(ready.runners[0].endpoint, "http://10.0.0.1:55880");
    }

    #[tokio::test]
    async fn create_reservation_applies_resources_and_starts_provisioning() {
        let api = std::sync::Arc::new(FakeKubernetesRunnerApi::default());
        let store = ReservationStore::for_test(api.clone(), PluginConfig::test_default());

        let status = store.create(test_request(3)).await.unwrap();

        assert_eq!(status.status, ReservationStatusKind::Provisioning);
        assert_eq!(status.requested_runners, 3);
        assert_eq!(api.applied_count(), 1);
    }

    #[tokio::test]
    async fn kubernetes_reservation_becomes_ready_when_all_pods_are_ready() {
        let api = std::sync::Arc::new(FakeKubernetesRunnerApi::default());
        let store = ReservationStore::for_test(api.clone(), PluginConfig::test_default());
        let status = store.create(test_request(2)).await.unwrap();
        api.set_ready(vec![
            RunnerPod {
                name: "new-runner-0".to_owned(),
                ordinal: 0,
                pod_ip: "10.20.0.10".to_owned(),
                endpoint: "http://runner-0:7373".to_owned(),
            },
            RunnerPod {
                name: "new-runner-1".to_owned(),
                ordinal: 1,
                pod_ip: "10.20.0.11".to_owned(),
                endpoint: "http://runner-1:7373".to_owned(),
            },
        ]);

        let ready = store
            .reconcile_once(&status.reservation_id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(ready.status, ReservationStatusKind::Ready);
        assert_eq!(ready.ready_runners, 2);
        assert!(ready.reservation_token.is_some());
        assert_eq!(ready.runners.len(), 2);
    }

    #[tokio::test]
    async fn provisioning_reservation_fails_after_timeout() {
        let api = std::sync::Arc::new(FakeKubernetesRunnerApi::default());
        let config = PluginConfig::test_default().with_provision_timeout_seconds(0);
        let store = ReservationStore::for_test(api, config);
        let status = store.create(test_request(1)).await.expect("reservation");

        let failed = store
            .reconcile_once(&status.reservation_id)
            .await
            .expect("reconcile")
            .expect("status");

        assert_eq!(failed.status, ReservationStatusKind::Failed);
        assert_eq!(
            failed.reason,
            Some(crate::models::ReservationFailureReason::ProvisionTimeout)
        );
    }

    #[tokio::test]
    async fn cancel_deletes_kubernetes_resources() {
        let api = std::sync::Arc::new(FakeKubernetesRunnerApi::default());
        let store = ReservationStore::for_test(api.clone(), PluginConfig::test_default());
        let status = store.create(test_request(1)).await.unwrap();

        assert!(store.cancel(&status.reservation_id).await);
        assert!(api.deleted(&status.reservation_id));
    }

    #[tokio::test]
    async fn expired_unconsumed_reservation_is_deleted() {
        let api = std::sync::Arc::new(FakeKubernetesRunnerApi::default());
        let config = PluginConfig::test_default().with_reservation_ttl_seconds(1);
        let store = ReservationStore::for_test(api.clone(), config);
        let status = store.create(test_request(1)).await.unwrap();
        api.set_ready(vec![RunnerPod {
            name: "runner-0".to_owned(),
            ordinal: 0,
            pod_ip: "10.20.0.10".to_owned(),
            endpoint: "http://runner-0:7373".to_owned(),
        }]);
        let ready = store
            .reconcile_once(&status.reservation_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ready.status, ReservationStatusKind::Ready);

        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        store.reconcile_all_once().await;

        let expired = store.get(&status.reservation_id).await.unwrap();
        assert_eq!(expired.status, ReservationStatusKind::Expired);
        assert!(api.deleted(&status.reservation_id));
    }

    #[tokio::test]
    async fn busy_runner_is_not_deleted_after_idle_ttl() {
        let api = std::sync::Arc::new(FakeKubernetesRunnerApi::default());
        let health = std::sync::Arc::new(FakeRunnerHealthApi::default());
        let config = PluginConfig::test_default().with_idle_ttl_seconds(1);
        let store =
            ReservationStore::for_test(api.clone(), config).with_runner_health(health.clone());
        let status = store.create(test_request(1)).await.unwrap();
        api.set_ready(vec![RunnerPod {
            name: "runner-0".to_owned(),
            ordinal: 0,
            pod_ip: "10.20.0.10".to_owned(),
            endpoint: "http://runner-0:7373".to_owned(),
        }]);
        let _ = store.reconcile_once(&status.reservation_id).await.unwrap();
        health.set_info(RunnerInfo {
            busy: true,
            started_execution_count: 1,
            ..Default::default()
        });

        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        store.reconcile_all_once().await;

        assert!(!api.deleted(&status.reservation_id));
        assert_eq!(
            store.get(&status.reservation_id).await.unwrap().status,
            ReservationStatusKind::Running
        );
    }

    #[tokio::test]
    async fn idle_runner_after_first_execution_is_deleted_after_idle_ttl() {
        let api = std::sync::Arc::new(FakeKubernetesRunnerApi::default());
        let health = std::sync::Arc::new(FakeRunnerHealthApi::default());
        let config = PluginConfig::test_default().with_idle_ttl_seconds(1);
        let store =
            ReservationStore::for_test(api.clone(), config).with_runner_health(health.clone());
        let status = store.create(test_request(1)).await.unwrap();
        api.set_ready(vec![RunnerPod {
            name: "runner-0".to_owned(),
            ordinal: 0,
            pod_ip: "10.20.0.10".to_owned(),
            endpoint: "http://runner-0:7373".to_owned(),
        }]);
        let _ = store.reconcile_once(&status.reservation_id).await.unwrap();
        health.set_info(RunnerInfo {
            busy: false,
            started_execution_count: 1,
            ..Default::default()
        });
        store.reconcile_all_once().await;
        assert_eq!(
            store.get(&status.reservation_id).await.unwrap().status,
            ReservationStatusKind::IdleReusable
        );

        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        store.reconcile_all_once().await;

        assert!(api.deleted(&status.reservation_id));
        assert_eq!(
            store.get(&status.reservation_id).await.unwrap().status,
            ReservationStatusKind::Terminating
        );
    }

    #[tokio::test]
    async fn new_reservation_reuses_idle_runner_without_creating_more_kubernetes_resources() {
        let api = std::sync::Arc::new(FakeKubernetesRunnerApi::default());
        let health = std::sync::Arc::new(FakeRunnerHealthApi::default());
        let store = ReservationStore::for_test(api.clone(), PluginConfig::test_default())
            .with_runner_health(health.clone());
        let first = store.create(test_request(1)).await.unwrap();
        api.set_ready(vec![RunnerPod {
            name: "runner-0".to_owned(),
            ordinal: 0,
            pod_ip: "10.20.0.10".to_owned(),
            endpoint: "http://runner-0:7373".to_owned(),
        }]);
        let ready = store
            .reconcile_once(&first.reservation_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ready.status, ReservationStatusKind::Ready);
        health.set_info(RunnerInfo {
            busy: false,
            started_execution_count: 1,
            ..Default::default()
        });
        store.reconcile_all_once().await;

        let second = store.create(test_request(1)).await.unwrap();

        assert_eq!(second.status, ReservationStatusKind::Ready);
        assert_eq!(second.ready_runners, 1);
        assert_eq!(second.runners[0].endpoint, "http://runner-0:7373");
        assert_eq!(api.applied_count(), 1);
        assert_eq!(health.rearmed_count(), 1);
    }

    #[tokio::test]
    async fn new_reservation_reuses_idle_runners_and_creates_only_missing_delta() {
        let api = std::sync::Arc::new(FakeKubernetesRunnerApi::default());
        let health = std::sync::Arc::new(FakeRunnerHealthApi::default());
        let store = ReservationStore::for_test(api.clone(), PluginConfig::test_default())
            .with_runner_health(health.clone());
        let first = store.create(test_request(1)).await.unwrap();
        api.set_ready(vec![RunnerPod {
            name: "runner-0".to_owned(),
            ordinal: 0,
            pod_ip: "10.20.0.10".to_owned(),
            endpoint: "http://runner-0:7373".to_owned(),
        }]);
        let _ = store.reconcile_once(&first.reservation_id).await.unwrap();
        health.set_info(RunnerInfo {
            busy: false,
            started_execution_count: 1,
            ..Default::default()
        });
        store.reconcile_all_once().await;

        let second = store.create(test_request(3)).await.unwrap();

        assert_eq!(second.status, ReservationStatusKind::Provisioning);
        assert_eq!(second.ready_runners, 1);
        assert_eq!(api.applied.lock().unwrap().last().unwrap().count, 2);
        assert_eq!(health.rearmed_count(), 1);
        api.set_ready(vec![
            RunnerPod {
                name: "new-runner-0".to_owned(),
                ordinal: 0,
                pod_ip: "10.20.0.20".to_owned(),
                endpoint: "http://new-runner-0:7373".to_owned(),
            },
            RunnerPod {
                name: "new-runner-1".to_owned(),
                ordinal: 1,
                pod_ip: "10.20.0.21".to_owned(),
                endpoint: "http://new-runner-1:7373".to_owned(),
            },
        ]);

        let ready = store
            .reconcile_once(&second.reservation_id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(ready.status, ReservationStatusKind::Ready);
        assert_eq!(ready.ready_runners, 3);
        assert_eq!(ready.runners.len(), 3);
    }

    #[tokio::test]
    async fn failed_idle_rearm_is_skipped_and_missing_runner_is_created() {
        let api = std::sync::Arc::new(FakeKubernetesRunnerApi::default());
        let health = std::sync::Arc::new(FakeRunnerHealthApi::default());
        let store = ReservationStore::for_test(api.clone(), PluginConfig::test_default())
            .with_runner_health(health.clone());
        let first = store.create(test_request(1)).await.unwrap();
        api.set_ready(vec![RunnerPod {
            name: "runner-0".to_owned(),
            ordinal: 0,
            pod_ip: "10.20.0.10".to_owned(),
            endpoint: "http://runner-0:7373".to_owned(),
        }]);
        let _ = store.reconcile_once(&first.reservation_id).await.unwrap();
        health.set_info(RunnerInfo {
            busy: false,
            started_execution_count: 1,
            ..Default::default()
        });
        store.reconcile_all_once().await;
        health.fail_rearm();

        let second = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            store.create(test_request(1)),
        )
        .await
        .expect("reservation creation should not loop forever")
        .expect("reservation should be created");

        assert_eq!(second.status, ReservationStatusKind::Provisioning);
        assert_eq!(second.ready_runners, 0);
        assert_eq!(health.rearmed_count(), 1);
        assert_eq!(api.applied.lock().unwrap().last().unwrap().count, 1);
    }

    #[tokio::test]
    async fn cancelling_reused_reservation_releases_runner_for_next_reuse() {
        let api = std::sync::Arc::new(FakeKubernetesRunnerApi::default());
        let health = std::sync::Arc::new(FakeRunnerHealthApi::default());
        let store = ReservationStore::for_test(api.clone(), PluginConfig::test_default())
            .with_runner_health(health.clone());
        let first = store.create(test_request(1)).await.unwrap();
        api.set_ready(vec![RunnerPod {
            name: "runner-0".to_owned(),
            ordinal: 0,
            pod_ip: "10.20.0.10".to_owned(),
            endpoint: "http://runner-0:7373".to_owned(),
        }]);
        let _ = store.reconcile_once(&first.reservation_id).await.unwrap();
        health.set_info(RunnerInfo {
            busy: false,
            started_execution_count: 1,
            ..Default::default()
        });
        store.reconcile_all_once().await;
        let second = store.create(test_request(1)).await.unwrap();

        assert!(store.cancel(&second.reservation_id).await);
        let third = store.create(test_request(1)).await.unwrap();

        assert_eq!(health.released_count(), 1);
        assert_eq!(health.rearmed_count(), 2);
        assert_eq!(third.status, ReservationStatusKind::Ready);
        assert_eq!(third.runners[0].endpoint, "http://runner-0:7373");
    }

    #[tokio::test]
    async fn original_physical_resources_are_not_deleted_while_reused_runner_is_leased() {
        let api = std::sync::Arc::new(FakeKubernetesRunnerApi::default());
        let health = std::sync::Arc::new(FakeRunnerHealthApi::default());
        let config = PluginConfig::test_default().with_idle_ttl_seconds(1);
        let store =
            ReservationStore::for_test(api.clone(), config).with_runner_health(health.clone());
        let first = store.create(test_request(1)).await.unwrap();
        api.set_ready(vec![RunnerPod {
            name: "runner-0".to_owned(),
            ordinal: 0,
            pod_ip: "10.20.0.10".to_owned(),
            endpoint: "http://runner-0:7373".to_owned(),
        }]);
        let _ = store.reconcile_once(&first.reservation_id).await.unwrap();
        health.set_info(RunnerInfo {
            busy: false,
            started_execution_count: 1,
            ..Default::default()
        });
        store.reconcile_all_once().await;
        let second = store.create(test_request(1)).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        store.reconcile_all_once().await;

        assert!(!api.deleted(&first.reservation_id));
        assert_eq!(
            store.get(&second.reservation_id).await.unwrap().status,
            ReservationStatusKind::Ready
        );
    }
}
