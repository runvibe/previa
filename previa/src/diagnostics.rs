use std::net::{TcpListener, ToSocketAddrs};
use std::path::Path;
use std::process::{Command as StdCommand, Stdio};

use serde::Serialize;

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
    if command_available("docker", &["compose", "version"]) {
        return DiagnosticCheck {
            id: "docker-compose".to_owned(),
            status: DiagnosticStatus::Ok,
            summary: "Docker Compose is available".to_owned(),
            detail: "`docker compose version` succeeded.".to_owned(),
            action: "Run `previa up -d` to start the default Docker-backed runtime.".to_owned(),
        };
    }

    if command_available("docker-compose", &["version"]) {
        return DiagnosticCheck {
            id: "docker-compose".to_owned(),
            status: DiagnosticStatus::Ok,
            summary: "docker-compose is available".to_owned(),
            detail: "`docker-compose version` succeeded.".to_owned(),
            action: "Run `previa up -d` to start the default Docker-backed runtime.".to_owned(),
        };
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
