use std::sync::Arc;

use async_trait::async_trait;
use k8s_openapi::api::apps::v1::StatefulSet;
use k8s_openapi::api::core::v1::{Pod, Service};
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use kube::api::{DeleteParams, ListParams, Patch, PatchParams};
use kube::{Api, Client, Resource, ResourceExt};
use thiserror::Error;

use crate::models::RunnerLifecycleState;
use crate::services::config::PluginConfig;
use crate::services::runner_resources::{
    RunnerReservationSpec, build_runner_pdb, build_runner_service, build_runner_statefulset,
    reservation_resource_name, runner_endpoint,
};

#[derive(Debug, Error)]
pub enum KubernetesError {
    #[error("failed to create kubernetes client: {0}")]
    Client(#[from] kube::Error),
    #[error("runner pod {pod_name} is missing an ordinal")]
    MissingOrdinal { pod_name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerPod {
    pub name: String,
    pub ordinal: usize,
    pub pod_ip: String,
    pub endpoint: String,
}

#[async_trait]
pub trait KubernetesRunnerApi: Send + Sync {
    async fn apply_reservation_resources(
        &self,
        spec: &RunnerReservationSpec,
    ) -> Result<(), KubernetesError>;

    async fn list_ready_runner_pods(
        &self,
        reservation_id: &str,
    ) -> Result<Vec<RunnerPod>, KubernetesError>;

    async fn update_runner_state_label(
        &self,
        reservation_id: &str,
        state: RunnerLifecycleState,
    ) -> Result<(), KubernetesError>;

    async fn delete_reservation_resources(
        &self,
        reservation_id: &str,
    ) -> Result<(), KubernetesError>;
}

#[derive(Clone)]
pub struct KubeRunnerApi {
    client: Client,
    config: Arc<PluginConfig>,
}

impl KubeRunnerApi {
    pub async fn new(config: PluginConfig) -> Result<Self, KubernetesError> {
        Ok(Self {
            client: Client::try_default().await?,
            config: Arc::new(config),
        })
    }

    fn statefulsets(&self) -> Api<StatefulSet> {
        Api::namespaced(self.client.clone(), &self.config.namespace)
    }

    fn services(&self) -> Api<Service> {
        Api::namespaced(self.client.clone(), &self.config.namespace)
    }

    fn pdbs(&self) -> Api<PodDisruptionBudget> {
        Api::namespaced(self.client.clone(), &self.config.namespace)
    }

    fn pods(&self) -> Api<Pod> {
        Api::namespaced(self.client.clone(), &self.config.namespace)
    }
}

#[async_trait]
impl KubernetesRunnerApi for KubeRunnerApi {
    async fn apply_reservation_resources(
        &self,
        spec: &RunnerReservationSpec,
    ) -> Result<(), KubernetesError> {
        let name = reservation_resource_name(&spec.reservation_id);
        let params = PatchParams::apply("previa-kubernetes-plugin").force();
        self.services()
            .patch(
                &name,
                &params,
                &Patch::Apply(build_runner_service(&self.config, spec)),
            )
            .await?;
        self.statefulsets()
            .patch(
                &name,
                &params,
                &Patch::Apply(build_runner_statefulset(&self.config, spec)),
            )
            .await?;
        self.pdbs()
            .patch(
                &name,
                &params,
                &Patch::Apply(build_runner_pdb(&self.config, spec)),
            )
            .await?;
        Ok(())
    }

    async fn list_ready_runner_pods(
        &self,
        reservation_id: &str,
    ) -> Result<Vec<RunnerPod>, KubernetesError> {
        let selector = format!("previa.runvibe.com/reservation-id={reservation_id}");
        let pods = self
            .pods()
            .list(&ListParams::default().labels(&selector))
            .await?;
        let spec = RunnerReservationSpec::new(reservation_id, "", 0);
        let mut ready = Vec::new();
        for pod in pods {
            if pod.meta().deletion_timestamp.is_some() || !pod_is_ready(&pod) {
                continue;
            }
            let Some(pod_ip) = pod.status.as_ref().and_then(|status| status.pod_ip.clone()) else {
                continue;
            };
            let name = pod.name_any();
            let ordinal = pod_ordinal(&name).ok_or_else(|| KubernetesError::MissingOrdinal {
                pod_name: name.clone(),
            })?;
            ready.push(RunnerPod {
                name,
                ordinal,
                pod_ip,
                endpoint: runner_endpoint(&self.config, &spec, ordinal),
            });
        }
        ready.sort_by_key(|pod| pod.ordinal);
        Ok(ready)
    }

    async fn update_runner_state_label(
        &self,
        reservation_id: &str,
        state: RunnerLifecycleState,
    ) -> Result<(), KubernetesError> {
        let selector = format!("previa.runvibe.com/reservation-id={reservation_id}");
        let pods = self
            .pods()
            .list(&ListParams::default().labels(&selector))
            .await?;
        let params = PatchParams::default();
        let patch = serde_json::json!({
            "metadata": {
                "labels": {
                    "previa.runvibe.com/state": state.as_label_value()
                }
            }
        });
        for pod in pods {
            self.pods()
                .patch(&pod.name_any(), &params, &Patch::Merge(&patch))
                .await?;
        }
        Ok(())
    }

    async fn delete_reservation_resources(
        &self,
        reservation_id: &str,
    ) -> Result<(), KubernetesError> {
        let name = reservation_resource_name(reservation_id);
        let params = DeleteParams::background();
        let selector = format!("previa.runvibe.com/reservation-id={reservation_id}");
        let mut first_error = None;
        remember_first_error(
            &mut first_error,
            ignore_not_found(self.statefulsets().delete(&name, &params).await),
        );
        remember_first_error(
            &mut first_error,
            ignore_not_found(
                self.pods()
                    .delete_collection(&params, &ListParams::default().labels(&selector))
                    .await,
            ),
        );
        remember_first_error(
            &mut first_error,
            ignore_not_found(self.services().delete(&name, &params).await),
        );
        remember_first_error(
            &mut first_error,
            ignore_not_found(self.pdbs().delete(&name, &params).await),
        );
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    }
}

fn ignore_not_found<T>(result: Result<T, kube::Error>) -> Result<(), KubernetesError> {
    match result {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(error)) if error.code == 404 => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn remember_first_error(
    first_error: &mut Option<KubernetesError>,
    result: Result<(), KubernetesError>,
) {
    if first_error.is_none() {
        if let Err(error) = result {
            *first_error = Some(error);
        }
    }
}

fn pod_is_ready(pod: &Pod) -> bool {
    pod.status
        .as_ref()
        .and_then(|status| status.conditions.as_ref())
        .map(|conditions| {
            conditions
                .iter()
                .any(|condition| condition.type_ == "Ready" && condition.status == "True")
        })
        .unwrap_or(false)
}

fn pod_ordinal(name: &str) -> Option<usize> {
    name.rsplit_once('-')?.1.parse::<usize>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_statefulset_pod_ordinals() {
        assert_eq!(pod_ordinal("previa-runner-rr-test-0"), Some(0));
        assert_eq!(pod_ordinal("previa-runner-rr-test-12"), Some(12));
        assert_eq!(pod_ordinal("previa-runner-rr-test"), None);
    }
}
