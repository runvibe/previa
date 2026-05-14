#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapacityMode {
    Kubernetes,
    StaticDev,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginConfig {
    pub namespace: String,
    pub runner_image: String,
    pub runner_command: String,
    pub runner_install_enabled: bool,
    pub runner_install_url_template: String,
    pub runner_port: u16,
    pub service_name: String,
    pub reservation_ttl_seconds: i64,
    pub idle_ttl_seconds: i64,
    pub provision_timeout_seconds: i64,
    pub runner_cpu_request: String,
    pub runner_memory_request: String,
    pub runner_cpu_limit: String,
    pub runner_memory_limit: String,
    pub node_pool: Option<String>,
    pub tolerations: Vec<RunnerTolerationConfig>,
    pub capacity_mode: CapacityMode,
    pub static_runner_endpoints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerTolerationConfig {
    pub key: String,
    pub value: Option<String>,
    pub effect: Option<String>,
}

impl PluginConfig {
    pub fn from_env() -> Self {
        Self::from_pairs(std::env::vars())
    }

    pub fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let values = pairs
            .into_iter()
            .map(|(key, value)| (key.as_ref().to_owned(), value.as_ref().to_owned()))
            .collect::<std::collections::HashMap<_, _>>();
        let static_runner_endpoints = split_csv(values.get("PREVIA_STATIC_RUNNER_ENDPOINTS"));
        let capacity_mode = if static_runner_endpoints.is_empty() {
            CapacityMode::Kubernetes
        } else {
            CapacityMode::StaticDev
        };

        Self {
            namespace: string_value(&values, "PREVIA_RUNNER_NAMESPACE", "previa"),
            runner_image: string_value(
                &values,
                "PREVIA_RUNNER_IMAGE",
                "gcr.io/distroless/cc-debian12:nonroot",
            ),
            runner_command: string_value(
                &values,
                "PREVIA_RUNNER_COMMAND",
                "/opt/previa/previa-runner",
            ),
            runner_install_enabled: bool_value(&values, "PREVIA_RUNNER_INSTALL_ENABLED", true),
            runner_install_url_template: string_value(
                &values,
                "PREVIA_RUNNER_INSTALL_URL_TEMPLATE",
                "https://github.com/runvibe/previa/releases/download/v1.0.0-alpha.22/previa-runner-linux-${previa_arch}",
            ),
            runner_port: u16_value(&values, "PREVIA_RUNNER_PORT", 7373),
            service_name: string_value(&values, "PREVIA_RUNNER_SERVICE_NAME", "previa-runner"),
            reservation_ttl_seconds: i64_value(&values, "PREVIA_RESERVATION_TTL_SECONDS", 300),
            idle_ttl_seconds: i64_value(&values, "PREVIA_IDLE_TTL_SECONDS", 300),
            provision_timeout_seconds: i64_value(&values, "PREVIA_PROVISION_TIMEOUT_SECONDS", 300),
            runner_cpu_request: string_value(&values, "PREVIA_RUNNER_CPU_REQUEST", "250m"),
            runner_memory_request: string_value(&values, "PREVIA_RUNNER_MEMORY_REQUEST", "256Mi"),
            runner_cpu_limit: string_value(&values, "PREVIA_RUNNER_CPU_LIMIT", "1"),
            runner_memory_limit: string_value(&values, "PREVIA_RUNNER_MEMORY_LIMIT", "1Gi"),
            node_pool: optional_string(&values, "PREVIA_KARPENTER_NODE_POOL"),
            tolerations: parse_tolerations(values.get("PREVIA_RUNNER_TOLERATIONS")),
            capacity_mode,
            static_runner_endpoints,
        }
    }

    #[cfg(test)]
    pub fn test_default() -> Self {
        Self::from_pairs([
            ("PREVIA_RUNNER_NAMESPACE", "previa"),
            ("PREVIA_RUNNER_IMAGE", "runner:test"),
        ])
    }

    #[cfg(test)]
    pub fn with_provision_timeout_seconds(mut self, value: i64) -> Self {
        self.provision_timeout_seconds = value;
        self
    }

    #[cfg(test)]
    pub fn with_reservation_ttl_seconds(mut self, value: i64) -> Self {
        self.reservation_ttl_seconds = value;
        self
    }

    #[cfg(test)]
    pub fn with_idle_ttl_seconds(mut self, value: i64) -> Self {
        self.idle_ttl_seconds = value;
        self
    }
}

fn optional_string(
    values: &std::collections::HashMap<String, String>,
    key: &str,
) -> Option<String> {
    values
        .get(key)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn string_value(
    values: &std::collections::HashMap<String, String>,
    key: &str,
    default: &str,
) -> String {
    optional_string(values, key).unwrap_or_else(|| default.to_owned())
}

fn i64_value(values: &std::collections::HashMap<String, String>, key: &str, default: i64) -> i64 {
    optional_string(values, key)
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn u16_value(values: &std::collections::HashMap<String, String>, key: &str, default: u16) -> u16 {
    optional_string(values, key)
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn bool_value(
    values: &std::collections::HashMap<String, String>,
    key: &str,
    default: bool,
) -> bool {
    optional_string(values, key)
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn split_csv(value: Option<&String>) -> Vec<String> {
    value
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_tolerations(value: Option<&String>) -> Vec<RunnerTolerationConfig> {
    split_csv(value)
        .into_iter()
        .filter_map(|entry| {
            let (key_value, effect) = entry
                .split_once(':')
                .map(|(left, right)| (left, Some(right.to_owned())))
                .unwrap_or((entry.as_str(), None));
            let (key, value) = key_value
                .split_once('=')
                .map(|(key, value)| (key, Some(value.to_owned())))
                .unwrap_or((key_value, None));
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            Some(RunnerTolerationConfig {
                key: key.to_owned(),
                value: value.map(|value| value.trim().to_owned()),
                effect: effect.map(|effect| effect.trim().to_owned()),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe_for_aws_karpenter_v0() {
        let config = PluginConfig::from_pairs([
            ("PREVIA_RUNNER_NAMESPACE", "previa"),
            (
                "PREVIA_RUNNER_IMAGE",
                "gcr.io/distroless/cc-debian12:nonroot",
            ),
        ]);

        assert_eq!(config.namespace, "previa");
        assert_eq!(config.reservation_ttl_seconds, 300);
        assert_eq!(config.idle_ttl_seconds, 300);
        assert_eq!(config.runner_port, 7373);
        assert_eq!(config.runner_command, "/opt/previa/previa-runner");
        assert!(config.runner_install_enabled);
        assert!(
            config
                .runner_install_url_template
                .contains("previa-runner-linux-${previa_arch}")
        );
        assert_eq!(config.provision_timeout_seconds, 300);
        assert_eq!(config.capacity_mode, CapacityMode::Kubernetes);
    }

    #[test]
    fn static_endpoints_are_dev_only() {
        let config = PluginConfig::from_pairs([
            ("PREVIA_RUNNER_NAMESPACE", "previa"),
            ("PREVIA_RUNNER_IMAGE", "runner:dev"),
            ("PREVIA_STATIC_RUNNER_ENDPOINTS", "http://127.0.0.1:17373"),
        ]);

        assert_eq!(config.capacity_mode, CapacityMode::StaticDev);
        assert_eq!(
            config.static_runner_endpoints,
            vec!["http://127.0.0.1:17373"]
        );
    }

    #[test]
    fn packaged_runner_image_and_tolerations_are_configurable() {
        let config = PluginConfig::from_pairs([
            ("PREVIA_RUNNER_IMAGE", "ghcr.io/runvibe/previa-runner:test"),
            ("PREVIA_RUNNER_COMMAND", "/app/previa-runner"),
            ("PREVIA_RUNNER_INSTALL_ENABLED", "false"),
            (
                "PREVIA_RUNNER_INSTALL_URL_TEMPLATE",
                "https://example.test/runner-${previa_arch}",
            ),
            (
                "PREVIA_RUNNER_TOLERATIONS",
                "workload.cloudvibe.dev/previa=arm:NoSchedule",
            ),
        ]);

        assert_eq!(config.runner_command, "/app/previa-runner");
        assert!(!config.runner_install_enabled);
        assert_eq!(
            config.runner_install_url_template,
            "https://example.test/runner-${previa_arch}"
        );
        assert_eq!(config.tolerations.len(), 1);
        assert_eq!(config.tolerations[0].key, "workload.cloudvibe.dev/previa");
        assert_eq!(config.tolerations[0].value.as_deref(), Some("arm"));
        assert_eq!(config.tolerations[0].effect.as_deref(), Some("NoSchedule"));
    }
}
