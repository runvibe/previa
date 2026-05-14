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
use crate::services::runner_health::RunnerHealthApi;
use crate::services::runner_resources::RunnerReservationSpec;

#[derive(Debug, Error)]
pub enum ReservationStoreError {
    #[error("kubernetes error: {0}")]
    Kubernetes(#[from] KubernetesError),
}

#[derive(Clone)]
pub struct ReservationStore {
    inner: Arc<RwLock<HashMap<String, ReservationRecord>>>,
    config: PluginConfig,
    kubernetes: Option<Arc<dyn KubernetesRunnerApi>>,
    runner_health: Option<Arc<dyn RunnerHealthApi>>,
}

#[derive(Clone)]
struct ReservationRecord {
    request: ReservationCreateRequest,
    status: ReservationStatus,
    created_at: DateTime<Utc>,
    token: String,
}

impl Default for ReservationStore {
    fn default() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            config: PluginConfig::from_pairs(std::iter::empty::<(&str, &str)>()),
            kubernetes: None,
            runner_health: None,
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
            config,
            kubernetes,
            runner_health: None,
        }
    }

    pub fn with_runner_health(mut self, runner_health: Arc<dyn RunnerHealthApi>) -> Self {
        self.runner_health = Some(runner_health);
        self
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
                if let Some(api) = self.kubernetes.as_ref() {
                    api.apply_reservation_resources(
                        &RunnerReservationSpec::new(
                            reservation_id.clone(),
                            token.clone(),
                            request.count,
                        )
                        .with_expires_at(expires_at.clone()),
                    )
                    .await?;
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
            },
        );
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
            return Ok(Some(record.status.clone()));
        }

        if self.config.capacity_mode == CapacityMode::Kubernetes {
            if let Some(api) = self.kubernetes.as_ref() {
                let pods = api.list_ready_runner_pods(reservation_id).await?;
                if pods.len() >= record.request.count {
                    record.status.status = ReservationStatusKind::Ready;
                    record.status.ready_runners = record.request.count;
                    record.status.reservation_token = Some(record.token.clone());
                    record.status.expires_at = Some(
                        (record.created_at
                            + Duration::seconds(self.config.reservation_ttl_seconds))
                        .to_rfc3339(),
                    );
                    record.status.runners = pods
                        .into_iter()
                        .take(record.request.count)
                        .map(|pod| ReservationRunner {
                            id: pod.name,
                            endpoint: pod.endpoint,
                        })
                        .collect();
                    record.status.updated_at = Utc::now().to_rfc3339();
                }
            }
        }

        let status = record.status.clone();
        drop(lock);
        self.reconcile_runner_lifecycle(reservation_id).await?;
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
        Some(record.status.clone())
    }

    pub async fn cancel(&self, reservation_id: &str) -> bool {
        let mut lock = self.inner.write().await;
        let Some(mut record) = lock.remove(reservation_id) else {
            return false;
        };
        record.status.status = ReservationStatusKind::Cancelled;
        drop(lock);
        if let Some(api) = self.kubernetes.as_ref() {
            if let Err(error) = api.delete_reservation_resources(reservation_id).await {
                warn!(%reservation_id, %error, "failed to delete cancelled reservation resources");
            }
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
                record.status.status = ReservationStatusKind::Idle;
                desired_state_label = Some(RunnerLifecycleState::Idle);
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

        if delete {
            self.delete_and_mark(reservation_id, ReservationStatusKind::Terminating)
                .await;
        } else if let (Some(api), Some(state)) = (self.kubernetes.as_ref(), desired_state_label) {
            let _ = api.update_runner_state_label(reservation_id, state).await;
        }
        Ok(())
    }

    async fn expire_and_delete(&self, reservation_id: &str) {
        self.delete_and_mark(reservation_id, ReservationStatusKind::Expired)
            .await;
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
    }

    impl FakeRunnerHealthApi {
        fn set_info(&self, info: RunnerInfo) {
            *self.info.lock().unwrap() = info;
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
                name: "runner-0".to_owned(),
                ordinal: 0,
                pod_ip: "10.20.0.10".to_owned(),
                endpoint: "http://runner-0:7373".to_owned(),
            },
            RunnerPod {
                name: "runner-1".to_owned(),
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

        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        store.reconcile_all_once().await;

        assert!(api.deleted(&status.reservation_id));
        assert_eq!(
            store.get(&status.reservation_id).await.unwrap().status,
            ReservationStatusKind::Terminating
        );
    }
}
