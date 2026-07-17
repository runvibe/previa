use std::collections::BTreeMap;
use std::net::{TcpListener, ToSocketAddrs};
use std::path::Path;
use std::process::{Command as StdCommand, Stdio};

use anyhow::{Context, Result};
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;

use crate::envfile::{default_main_env_map, default_runner_env_map, read_env_file};
use crate::paths::StackPaths;
use crate::runtime::DetachedRuntimeState;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticStatus {
    Ok,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticCheck {
    pub id: String,
    pub status: DiagnosticStatus,
    pub summary: String,
    pub detail: String,
    pub action: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    pub context: String,
    pub overall: DiagnosticStatus,
    pub checks: Vec<DiagnosticCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorBindTarget {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorBindTargets {
    pub main: DoctorBindTarget,
    pub runner: DoctorBindTarget,
}

pub fn command_available(program: &str, args: &[&str]) -> bool {
    StdCommand::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn check_docker_compose() -> DiagnosticCheck {
    check_docker_compose_with_probe(command_available)
}

pub fn check_docker_compose_with_probe<F>(probe: F) -> DiagnosticCheck
where
    F: Fn(&str, &[&str]) -> bool,
{
    if probe("docker", &["compose", "version"]) {
        return docker_daemon_check("Docker Compose", "`docker compose version`", &probe);
    }

    if probe("docker-compose", &["version"]) {
        return docker_daemon_check("docker-compose", "`docker-compose version`", &probe);
    }

    DiagnosticCheck {
        id: "docker-compose".to_owned(),
        status: DiagnosticStatus::Error,
        summary: "Docker Compose was not found".to_owned(),
        detail: "Neither `docker compose version` nor `docker-compose version` succeeded."
            .to_owned(),
        action: "Install Docker Desktop or Docker Engine with the Compose plugin, then rerun `previa doctor`.".to_owned(),
    }
}

fn docker_daemon_check<F>(compose_name: &str, compose_command: &str, probe: &F) -> DiagnosticCheck
where
    F: Fn(&str, &[&str]) -> bool,
{
    if probe("docker", &["info"]) {
        return DiagnosticCheck {
            id: "docker-compose".to_owned(),
            status: DiagnosticStatus::Ok,
            summary: format!("{compose_name} is available"),
            detail: format!("{compose_command} and `docker info` succeeded."),
            action: "Run `previa up -d` to start the default Docker-backed runtime.".to_owned(),
        };
    }

    DiagnosticCheck {
        id: "docker-compose".to_owned(),
        status: DiagnosticStatus::Error,
        summary: "Docker daemon is not available".to_owned(),
        detail: format!("{compose_command} succeeded, but `docker info` failed."),
        action: "Start Docker Desktop or the Docker daemon, then rerun `previa doctor`.".to_owned(),
    }
}

pub fn doctor_bind_targets(stack_paths: &StackPaths) -> Result<DoctorBindTargets> {
    let main_env = read_env_with_defaults(
        &stack_paths.main_env,
        default_main_env_map(stack_paths),
        "main",
    )?;
    let runner_env =
        read_env_with_defaults(&stack_paths.runner_env, default_runner_env_map(), "runner")?;

    Ok(DoctorBindTargets {
        main: DoctorBindTarget {
            host: env_string(&main_env, "ADDRESS", "0.0.0.0"),
            port: env_port(&main_env, "PORT", "main")?,
        },
        runner: DoctorBindTarget {
            host: env_string(&runner_env, "ADDRESS", "127.0.0.1"),
            port: env_port(&runner_env, "PORT", "runner")?,
        },
    })
}

fn read_env_with_defaults(
    path: &Path,
    mut defaults: BTreeMap<String, String>,
    label: &str,
) -> Result<BTreeMap<String, String>> {
    for (key, value) in read_env_file(path)
        .with_context(|| format!("failed to read {label} env file '{}'", path.display()))?
    {
        defaults.insert(key, value);
    }
    Ok(defaults)
}

fn env_string(values: &BTreeMap<String, String>, key: &str, fallback: &str) -> String {
    values
        .get(key)
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| fallback.to_owned())
}

fn env_port(values: &BTreeMap<String, String>, key: &str, label: &str) -> Result<u16> {
    let raw = env_string(values, key, "");
    raw.parse::<u16>()
        .with_context(|| format!("invalid {label} port '{raw}'"))
}

pub fn check_port_available(host: &str, port: u16, label: &str) -> DiagnosticCheck {
    let bind_host = if host == "0.0.0.0" { "127.0.0.1" } else { host };
    let address = format!("{bind_host}:{port}");
    let available = address
        .to_socket_addrs()
        .ok()
        .and_then(|mut addrs| addrs.next())
        .map(|addr| TcpListener::bind(addr).is_ok())
        .unwrap_or(false);

    if available {
        DiagnosticCheck {
            id: format!("port-{port}"),
            status: DiagnosticStatus::Ok,
            summary: format!("{label} port {port} is available"),
            detail: format!("Previa can bind {address}."),
            action: "No action required.".to_owned(),
        }
    } else {
        DiagnosticCheck {
            id: format!("port-{port}"),
            status: DiagnosticStatus::Error,
            summary: format!("{label} port {port} is already in use"),
            detail: format!("Previa could not bind {address}."),
            action: format!(
                "Stop the process using port {port}, or pass `--main-port` / `--runner-port-range` with free ports."
            ),
        }
    }
}

pub fn report_status(checks: &[DiagnosticCheck]) -> DiagnosticStatus {
    if checks
        .iter()
        .any(|check| check.status == DiagnosticStatus::Error)
    {
        DiagnosticStatus::Error
    } else if checks
        .iter()
        .any(|check| check.status == DiagnosticStatus::Warning)
    {
        DiagnosticStatus::Warning
    } else {
        DiagnosticStatus::Ok
    }
}

pub fn runtime_state_check(path: &Path, state: Option<&DetachedRuntimeState>) -> DiagnosticCheck {
    match state {
        Some(state) => DiagnosticCheck {
            id: "runtime-state".to_owned(),
            status: DiagnosticStatus::Ok,
            summary: format!("Context '{}' has runtime state", state.name),
            detail: format!("Runtime state file: {}", path.display()),
            action: "Run `previa status` for live health details.".to_owned(),
        },
        None => DiagnosticCheck {
            id: "runtime-state".to_owned(),
            status: DiagnosticStatus::Warning,
            summary: "Context is not running".to_owned(),
            detail: format!("No runtime state file found at {}.", path.display()),
            action: "Run `previa up -d` to start this context.".to_owned(),
        },
    }
}

pub async fn check_postgres_queue(http: &Client, state: &DetachedRuntimeState) -> DiagnosticCheck {
    let host = match state.main.address.as_str() {
        "0.0.0.0" | "::" => "127.0.0.1",
        value => value,
    };
    let url = format!("http://{host}:{}/api/v1/queue/diagnostics", state.main.port);
    let response = match http.get(&url).send().await {
        Ok(response) => response,
        Err(error) => {
            return DiagnosticCheck {
                id: "postgres-queue".to_owned(),
                status: DiagnosticStatus::Error,
                summary: "Postgres queue is unreachable".to_owned(),
                detail: format!("Main queue diagnostics request failed: {error}"),
                action: "Check the main and Postgres containers, then rerun `previa doctor`."
                    .to_owned(),
            };
        }
    };
    if !response.status().is_success() {
        return DiagnosticCheck {
            id: "postgres-queue".to_owned(),
            status: DiagnosticStatus::Warning,
            summary: "Postgres queue diagnostics require API access".to_owned(),
            detail: format!(
                "Main returned HTTP {} without exposing database credentials.",
                response.status()
            ),
            action: "Use an API token to inspect `/api/v1/queue/diagnostics`, or check main logs."
                .to_owned(),
        };
    }

    let payload = response.json::<Value>().await.unwrap_or(Value::Null);
    let actual = payload
        .get("protocolVersion")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let expected = i64::from(previa_runner::queue::QUEUE_PROTOCOL_VERSION.0);
    let valid = actual == expected;
    DiagnosticCheck {
        id: "postgres-queue".to_owned(),
        status: if valid {
            DiagnosticStatus::Ok
        } else {
            DiagnosticStatus::Error
        },
        summary: if valid {
            "Postgres queue is reachable and migrated".to_owned()
        } else {
            "Postgres queue protocol is incompatible".to_owned()
        },
        detail: format!(
            "Queue diagnostics reported protocol {actual}; this CLI expects protocol {expected}."
        ),
        action: if valid {
            "No action required.".to_owned()
        } else {
            "Upgrade or roll back main and runners together, then rerun migrations.".to_owned()
        },
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{DiagnosticStatus, check_docker_compose_with_probe, doctor_bind_targets};
    use crate::paths::PreviaPaths;

    fn temp_stack() -> (TempDir, crate::paths::StackPaths) {
        let temp = TempDir::new().expect("tempdir");
        let paths = PreviaPaths {
            home: temp.path().to_path_buf(),
            workspace_root: None,
        };
        let stack_paths = paths.stack("default");
        (temp, stack_paths)
    }

    #[test]
    fn check_docker_compose_errors_when_daemon_is_unavailable() {
        let check = check_docker_compose_with_probe(|program, args| {
            program == "docker" && args == ["compose", "version"]
        });

        assert_eq!(check.status, DiagnosticStatus::Error);
        assert!(check.summary.contains("Docker daemon is not available"));
        assert!(check.detail.contains("`docker info` failed"));
        assert!(check.action.contains("Start Docker Desktop"));
        assert!(check.action.contains("previa doctor"));
    }

    #[test]
    fn doctor_bind_targets_use_existing_env_files() {
        let (_temp, stack_paths) = temp_stack();
        stack_paths
            .ensure_parent_dirs()
            .expect("ensure stack parent dirs");
        std::fs::write(&stack_paths.main_env, "ADDRESS=127.0.0.1\nPORT=5688\n")
            .expect("write main env");
        std::fs::write(&stack_paths.runner_env, "ADDRESS=0.0.0.0\nPORT=56880\n")
            .expect("write runner env");

        let targets = doctor_bind_targets(&stack_paths).expect("doctor bind targets");

        assert_eq!(targets.main.host, "127.0.0.1");
        assert_eq!(targets.main.port, 5688);
        assert_eq!(targets.runner.host, "0.0.0.0");
        assert_eq!(targets.runner.port, 56880);
    }
}
