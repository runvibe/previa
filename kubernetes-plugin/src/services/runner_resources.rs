use std::collections::BTreeMap;

use k8s_openapi::api::apps::v1::{StatefulSet, StatefulSetSpec};
use k8s_openapi::api::core::v1::{
    Capabilities, Container, ContainerPort, EmptyDirVolumeSource, EnvVar, HTTPGetAction,
    PodAffinityTerm, PodAntiAffinity, PodSecurityContext, PodSpec, PodTemplateSpec, Probe,
    ResourceRequirements, SecurityContext, Service, ServicePort, ServiceSpec, Volume, VolumeMount,
};
use k8s_openapi::api::policy::v1::{PodDisruptionBudget, PodDisruptionBudgetSpec};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;

use crate::models::RunnerLifecycleState;
use crate::services::config::PluginConfig;

const APP_NAME: &str = "previa";
const RUNNER_COMPONENT: &str = "runner";
const LABEL_APP_NAME: &str = "app.kubernetes.io/name";
const LABEL_COMPONENT: &str = "app.kubernetes.io/component";
const LABEL_RESERVATION_ID: &str = "previa.runvibe.com/reservation-id";
const LABEL_STATE: &str = "previa.runvibe.com/state";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerReservationSpec {
    pub reservation_id: String,
    pub reservation_token: String,
    pub count: usize,
    pub expires_at: Option<String>,
}

impl RunnerReservationSpec {
    pub fn new(
        reservation_id: impl Into<String>,
        reservation_token: impl Into<String>,
        count: usize,
    ) -> Self {
        Self {
            reservation_id: reservation_id.into(),
            reservation_token: reservation_token.into(),
            count,
            expires_at: None,
        }
    }

    pub fn with_expires_at(mut self, expires_at: Option<String>) -> Self {
        self.expires_at = expires_at;
        self
    }
}

pub fn reservation_resource_name(reservation_id: &str) -> String {
    let normalized = reservation_id
        .chars()
        .map(|ch| match ch {
            'a'..='z' | '0'..='9' => ch,
            'A'..='Z' => ch.to_ascii_lowercase(),
            _ => '-',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned();
    format!("previa-runner-{}", normalized)
}

pub fn runner_dns_name(
    config: &PluginConfig,
    spec: &RunnerReservationSpec,
    ordinal: usize,
) -> String {
    let name = reservation_resource_name(&spec.reservation_id);
    format!(
        "{name}-{ordinal}.{name}.{}.svc.cluster.local",
        config.namespace
    )
}

pub fn runner_endpoint(
    config: &PluginConfig,
    spec: &RunnerReservationSpec,
    ordinal: usize,
) -> String {
    format!(
        "http://{}:{}",
        runner_dns_name(config, spec, ordinal),
        config.runner_port
    )
}

pub fn reservation_labels(
    reservation_id: &str,
    state: RunnerLifecycleState,
) -> BTreeMap<String, String> {
    BTreeMap::from([
        (LABEL_APP_NAME.to_owned(), APP_NAME.to_owned()),
        (LABEL_COMPONENT.to_owned(), RUNNER_COMPONENT.to_owned()),
        (LABEL_RESERVATION_ID.to_owned(), reservation_id.to_owned()),
        (LABEL_STATE.to_owned(), state.as_label_value().to_owned()),
    ])
}

pub fn build_runner_statefulset(
    config: &PluginConfig,
    spec: &RunnerReservationSpec,
) -> StatefulSet {
    let name = reservation_resource_name(&spec.reservation_id);
    let labels = reservation_labels(&spec.reservation_id, RunnerLifecycleState::Reserved);
    let selector = LabelSelector {
        match_labels: Some(labels.clone()),
        ..Default::default()
    };
    let mut node_selector = BTreeMap::new();
    if let Some(node_pool) = config.node_pool.as_deref() {
        node_selector.insert("karpenter.sh/nodepool".to_owned(), node_pool.to_owned());
    }

    StatefulSet {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: Some(config.namespace.clone()),
            labels: Some(labels.clone()),
            ..Default::default()
        },
        spec: Some(StatefulSetSpec {
            replicas: Some(spec.count as i32),
            selector: selector.clone(),
            service_name: name.clone(),
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(labels.clone()),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    affinity: Some(k8s_openapi::api::core::v1::Affinity {
                        pod_anti_affinity: Some(PodAntiAffinity {
                            required_during_scheduling_ignored_during_execution: Some(vec![
                                PodAffinityTerm {
                                    label_selector: Some(selector),
                                    topology_key: "kubernetes.io/hostname".to_owned(),
                                    ..Default::default()
                                },
                            ]),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    automount_service_account_token: Some(false),
                    containers: vec![runner_container(config, spec)],
                    init_containers: Some(vec![install_runner_container()]),
                    node_selector: (!node_selector.is_empty()).then_some(node_selector),
                    security_context: Some(PodSecurityContext {
                        fs_group: Some(65532),
                        fs_group_change_policy: Some("OnRootMismatch".to_owned()),
                        ..Default::default()
                    }),
                    volumes: Some(vec![
                        Volume {
                            name: "previa-bin".to_owned(),
                            empty_dir: Some(EmptyDirVolumeSource::default()),
                            ..Default::default()
                        },
                        Volume {
                            name: "tmp".to_owned(),
                            empty_dir: Some(EmptyDirVolumeSource::default()),
                            ..Default::default()
                        },
                    ]),
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub fn build_runner_service(config: &PluginConfig, spec: &RunnerReservationSpec) -> Service {
    let name = reservation_resource_name(&spec.reservation_id);
    let labels = reservation_labels(&spec.reservation_id, RunnerLifecycleState::Reserved);
    Service {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: Some(config.namespace.clone()),
            labels: Some(labels.clone()),
            ..Default::default()
        },
        spec: Some(ServiceSpec {
            cluster_ip: Some("None".to_owned()),
            ports: Some(vec![ServicePort {
                name: Some("http".to_owned()),
                port: config.runner_port as i32,
                target_port: Some(IntOrString::String("http".to_owned())),
                ..Default::default()
            }]),
            selector: Some(labels),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub fn build_runner_pdb(
    config: &PluginConfig,
    spec: &RunnerReservationSpec,
) -> PodDisruptionBudget {
    let name = reservation_resource_name(&spec.reservation_id);
    let labels = reservation_labels(&spec.reservation_id, RunnerLifecycleState::Reserved);
    PodDisruptionBudget {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: Some(config.namespace.clone()),
            labels: Some(labels.clone()),
            ..Default::default()
        },
        spec: Some(PodDisruptionBudgetSpec {
            min_available: Some(IntOrString::Int(spec.count as i32)),
            selector: Some(LabelSelector {
                match_labels: Some(labels),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn runner_container(config: &PluginConfig, spec: &RunnerReservationSpec) -> Container {
    let mut env = vec![
        EnvVar {
            name: "ADDRESS".to_owned(),
            value: Some("0.0.0.0".to_owned()),
            ..Default::default()
        },
        EnvVar {
            name: "PORT".to_owned(),
            value: Some(config.runner_port.to_string()),
            ..Default::default()
        },
        EnvVar {
            name: "RUST_LOG".to_owned(),
            value: Some("info".to_owned()),
            ..Default::default()
        },
        EnvVar {
            name: "PREVIA_RESERVATION_ID".to_owned(),
            value: Some(spec.reservation_id.clone()),
            ..Default::default()
        },
        EnvVar {
            name: "PREVIA_RESERVATION_TOKEN".to_owned(),
            value: Some(spec.reservation_token.clone()),
            ..Default::default()
        },
    ];
    if let Some(expires_at) = spec.expires_at.as_deref() {
        env.push(EnvVar {
            name: "PREVIA_RESERVATION_EXPIRES_AT".to_owned(),
            value: Some(expires_at.to_owned()),
            ..Default::default()
        });
    }

    Container {
        name: "previa-runner".to_owned(),
        image: Some(config.runner_image.clone()),
        image_pull_policy: Some("IfNotPresent".to_owned()),
        command: Some(vec!["/opt/previa/previa-runner".to_owned()]),
        env: Some(env),
        ports: Some(vec![ContainerPort {
            container_port: config.runner_port as i32,
            name: Some("http".to_owned()),
            ..Default::default()
        }]),
        liveness_probe: Some(http_probe("/health", 15, 20, 3, 5)),
        readiness_probe: Some(http_probe("/ready", 5, 10, 2, 3)),
        resources: Some(ResourceRequirements {
            limits: Some(resource_map([
                ("cpu", &config.runner_cpu_limit),
                ("memory", &config.runner_memory_limit),
            ])),
            requests: Some(resource_map([
                ("cpu", &config.runner_cpu_request),
                ("memory", &config.runner_memory_request),
            ])),
            ..Default::default()
        }),
        security_context: Some(container_security_context(true)),
        volume_mounts: Some(vec![
            VolumeMount {
                mount_path: "/opt/previa".to_owned(),
                name: "previa-bin".to_owned(),
                read_only: Some(true),
                ..Default::default()
            },
            VolumeMount {
                mount_path: "/tmp".to_owned(),
                name: "tmp".to_owned(),
                ..Default::default()
            },
        ]),
        ..Default::default()
    }
}

fn install_runner_container() -> Container {
    Container {
        name: "install-previa-runner".to_owned(),
        image: Some("curlimages/curl:8.10.1".to_owned()),
        image_pull_policy: Some("IfNotPresent".to_owned()),
        command: Some(vec![
            "sh".to_owned(),
            "-c".to_owned(),
            r#"set -eu
case "$(uname -m)" in
  x86_64) previa_arch="amd64" ;;
  aarch64|arm64) previa_arch="arm64" ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac
curl -fsSL \
  "https://github.com/runvibe/previa/releases/download/v1.0.0-alpha.22/previa-runner-linux-${previa_arch}" \
  -o /opt/previa/previa-runner
chmod 0755 /opt/previa/previa-runner
"#
            .to_owned(),
        ]),
        security_context: Some(container_security_context(false)),
        volume_mounts: Some(vec![VolumeMount {
            mount_path: "/opt/previa".to_owned(),
            name: "previa-bin".to_owned(),
            ..Default::default()
        }]),
        ..Default::default()
    }
}

fn http_probe(
    path: &str,
    initial_delay_seconds: i32,
    period_seconds: i32,
    timeout_seconds: i32,
    failure_threshold: i32,
) -> Probe {
    Probe {
        http_get: Some(HTTPGetAction {
            path: Some(path.to_owned()),
            port: IntOrString::String("http".to_owned()),
            scheme: Some("HTTP".to_owned()),
            ..Default::default()
        }),
        initial_delay_seconds: Some(initial_delay_seconds),
        period_seconds: Some(period_seconds),
        timeout_seconds: Some(timeout_seconds),
        failure_threshold: Some(failure_threshold),
        success_threshold: Some(1),
        ..Default::default()
    }
}

fn resource_map<const N: usize>(items: [(&str, &String); N]) -> BTreeMap<String, Quantity> {
    items
        .into_iter()
        .map(|(key, value)| (key.to_owned(), Quantity(value.clone())))
        .collect()
}

fn container_security_context(read_only_root_filesystem: bool) -> SecurityContext {
    SecurityContext {
        allow_privilege_escalation: Some(false),
        capabilities: Some(Capabilities {
            drop: Some(vec!["ALL".to_owned()]),
            ..Default::default()
        }),
        read_only_root_filesystem: Some(read_only_root_filesystem),
        run_as_group: Some(65532),
        run_as_non_root: Some(true),
        run_as_user: Some(65532),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::config::PluginConfig;

    #[test]
    fn builds_statefulset_with_one_runner_per_node_anti_affinity() {
        let config = PluginConfig::test_default();
        let spec = RunnerReservationSpec::new("rr_test", "rt_secret", 3);
        let statefulset = build_runner_statefulset(&config, &spec);

        assert_eq!(
            statefulset.metadata.name.as_deref(),
            Some("previa-runner-rr-test")
        );
        assert_eq!(statefulset.spec.as_ref().unwrap().replicas, Some(3));
        let template = &statefulset.spec.as_ref().unwrap().template;
        assert!(template.spec.as_ref().unwrap().affinity.is_some());
    }

    #[test]
    fn builds_headless_service_for_stable_runner_dns() {
        let config = PluginConfig::test_default();
        let spec = RunnerReservationSpec::new("rr_test", "rt_secret", 2);
        let service = build_runner_service(&config, &spec);

        assert_eq!(
            service.spec.as_ref().unwrap().cluster_ip.as_deref(),
            Some("None")
        );
        assert_eq!(
            runner_dns_name(&config, &spec, 0),
            "previa-runner-rr-test-0.previa-runner-rr-test.previa.svc.cluster.local"
        );
    }

    #[test]
    fn builds_pdb_for_reserved_runners() {
        let config = PluginConfig::test_default();
        let spec = RunnerReservationSpec::new("rr_test", "rt_secret", 2);
        let pdb = build_runner_pdb(&config, &spec);

        assert!(pdb.spec.as_ref().unwrap().min_available.is_some());
    }

    #[test]
    fn runner_probes_use_tolerant_timeouts() {
        let config = PluginConfig::test_default();
        let spec = RunnerReservationSpec::new("rr_test", "rt_secret", 1);
        let statefulset = build_runner_statefulset(&config, &spec);
        let container = &statefulset.spec.unwrap().template.spec.unwrap().containers[0];

        assert_eq!(
            container.readiness_probe.as_ref().unwrap().timeout_seconds,
            Some(2)
        );
        assert_eq!(
            container.liveness_probe.as_ref().unwrap().timeout_seconds,
            Some(3)
        );
        assert_eq!(
            container.liveness_probe.as_ref().unwrap().failure_threshold,
            Some(5)
        );
    }
}
