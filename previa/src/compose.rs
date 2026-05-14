use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command as StdCommand, Stdio};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::config::ResolvedUpConfig;
use crate::pull::{main_image_ref, runner_image_ref};
use crate::runtime::{DetachedRuntimeState, LocalRunnerRuntime, MainRuntime, RuntimeBackend};

const MAIN_DATA_DIR_IN_CONTAINER: &str = "/previa/data/main";
pub const MAIN_SERVICE_NAME: &str = "main";

#[derive(Debug, Clone)]
pub struct ComposeProject {
    pub project_name: String,
    pub compose_file: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComposeCli {
    DockerPlugin,
    DockerComposeBinary,
}

#[derive(Debug, Clone)]
pub struct ServiceInspect {
    pub running: bool,
    pub pid: u32,
    pub log_path: String,
}

#[derive(Debug, Serialize)]
struct ComposeDocument {
    services: BTreeMap<String, ComposeService>,
}

#[derive(Debug, Serialize)]
struct ComposeService {
    image: String,
    environment: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ports: Vec<ComposePort>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    volumes: Vec<ComposeVolume>,
}

#[derive(Debug)]
struct ComposePort {
    target: u16,
    published: u16,
    host_ip: String,
}

impl Serialize for ComposePort {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!(
            "{}:{}:{}",
            self.host_ip, self.published, self.target
        ))
    }
}

#[derive(Debug, Serialize)]
struct ComposeVolume {
    #[serde(rename = "type")]
    kind: &'static str,
    source: String,
    target: &'static str,
}

#[derive(Debug, Deserialize)]
struct InspectState {
    #[serde(rename = "Running")]
    running: bool,
    #[serde(rename = "Pid")]
    pid: u32,
}

#[derive(Debug, Deserialize)]
struct InspectRecord {
    #[serde(rename = "LogPath")]
    log_path: String,
    #[serde(rename = "State")]
    state: InspectState,
}

pub fn compose_project_name(context: &str) -> String {
    let sanitized = context
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    format!("previa_{sanitized}")
}

pub fn runner_service_name(port: u16) -> String {
    format!("runner-{port}")
}

pub fn compose_project_from_state(state: &DetachedRuntimeState) -> ComposeProject {
    ComposeProject {
        project_name: state.compose_project.clone(),
        compose_file: PathBuf::from(&state.compose_file),
    }
}

pub fn write_generated_compose(resolved: &ResolvedUpConfig) -> Result<()> {
    let doc = compose_document(resolved)?;
    let contents =
        serde_json::to_vec_pretty(&doc).context("failed to encode generated compose file")?;
    std::fs::write(&resolved.stack_paths.compose_file, contents).with_context(|| {
        format!(
            "failed to write '{}'",
            resolved.stack_paths.compose_file.display()
        )
    })?;
    Ok(())
}

pub fn desired_state_from_resolved(
    resolved: &ResolvedUpConfig,
    started_at: String,
) -> DetachedRuntimeState {
    DetachedRuntimeState {
        name: resolved.stack_paths.name.clone(),
        mode: "detached".to_owned(),
        started_at,
        source: resolved
            .source
            .as_ref()
            .map(|path| path.display().to_string()),
        backend: RuntimeBackend::Compose,
        image_tag: resolved.image_tag.clone(),
        compose_file: resolved.stack_paths.compose_file.display().to_string(),
        compose_project: compose_project_name(&resolved.stack_paths.name),
        runner_auth_key: resolved.runner_auth_key.clone(),
        main: MainRuntime {
            service_name: MAIN_SERVICE_NAME.to_owned(),
            pid: 0,
            address: resolved.main.address.clone(),
            port: resolved.main.port,
            log_path: String::new(),
        },
        runner_port_range: resolved.runner_port_range,
        attached_runners: resolved.attached_runners.clone(),
        runners: resolved
            .local_runner_ports
            .iter()
            .map(|(address, port)| LocalRunnerRuntime {
                service_name: runner_service_name(*port),
                pid: 0,
                address: address.clone(),
                port: *port,
                log_path: String::new(),
            })
            .collect(),
    }
}

impl ComposeProject {
    pub async fn up(&self, detached: bool, force_recreate: bool) -> Result<()> {
        let (mut command, compose_name) = self.compose_command()?;
        command.arg("up");
        if detached {
            command.arg("-d");
        }
        if force_recreate {
            command.arg("--force-recreate");
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        run_status(command, &format!("{compose_name} up")).await
    }

    pub async fn down(&self) -> Result<()> {
        let (mut command, compose_name) = self.compose_command()?;
        command
            .args(["down", "--remove-orphans"])
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        run_status(command, &format!("{compose_name} down")).await
    }

    pub async fn stop_services(&self, services: &[String]) -> Result<()> {
        if services.is_empty() {
            return Ok(());
        }

        let (mut command, compose_name) = self.compose_command()?;
        command.arg("stop").args(services);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        run_status(command, &format!("{compose_name} stop")).await
    }

    pub async fn remove_services(&self, services: &[String]) -> Result<()> {
        if services.is_empty() {
            return Ok(());
        }

        let (mut command, compose_name) = self.compose_command()?;
        command.arg("rm").arg("-f").args(services);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        run_status(command, &format!("{compose_name} rm")).await
    }

    pub async fn logs_output(&self, services: &[String], tail: Option<usize>) -> Result<String> {
        let (mut command, compose_name) = self.compose_command()?;
        command.arg("logs").arg("--no-color");
        if let Some(tail) = tail {
            command.arg("--tail").arg(tail.to_string());
        }
        command.args(services);
        command.stdin(Stdio::null());
        let output = run_output(command, &format!("{compose_name} logs")).await?;
        Ok(String::from_utf8(output.stdout).context("compose logs output was not UTF-8")?)
    }

    pub async fn logs_follow(&self, services: &[String], tail: Option<usize>) -> Result<()> {
        let (mut command, compose_name) = self.compose_command()?;
        command.arg("logs").arg("--no-color").arg("--follow");
        if let Some(tail) = tail {
            command.arg("--tail").arg(tail.to_string());
        }
        command
            .args(services)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        run_status(command, &format!("{compose_name} logs")).await
    }

    pub async fn inspect_service(&self, service_name: &str) -> Result<Option<ServiceInspect>> {
        let (mut ps_command, compose_name) = self.compose_command()?;
        ps_command
            .args(["ps", "-aq", service_name])
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .stdout(Stdio::piped());
        let output = run_output(ps_command, &format!("{compose_name} ps")).await?;
        let container_id = String::from_utf8(output.stdout)
            .context("compose ps output was not UTF-8")?
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_owned);

        let Some(container_id) = container_id else {
            return Ok(None);
        };

        let mut inspect_command = Command::new("docker");
        inspect_command
            .arg("inspect")
            .arg(&container_id)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = run_output(inspect_command, "docker inspect").await?;
        let mut records = serde_json::from_slice::<Vec<InspectRecord>>(&output.stdout)
            .context("failed to parse docker inspect output")?;
        let Some(record) = records.pop() else {
            return Ok(None);
        };

        Ok(Some(ServiceInspect {
            running: record.state.running,
            pid: record.state.pid,
            log_path: record.log_path,
        }))
    }

    fn compose_command(&self) -> Result<(Command, &'static str)> {
        let cli = resolve_compose_cli()?;
        let mut command = Command::new(cli.program());
        if matches!(cli, ComposeCli::DockerPlugin) {
            command.arg("compose");
        }
        command
            .arg("-p")
            .arg(&self.project_name)
            .arg("-f")
            .arg(&self.compose_file);
        Ok((command, cli.display_name()))
    }
}

fn compose_document(resolved: &ResolvedUpConfig) -> Result<ComposeDocument> {
    let mut services = BTreeMap::new();

    let mut main_environment = resolved.main_env.clone();
    main_environment.insert("ADDRESS".to_owned(), "0.0.0.0".to_owned());
    main_environment.insert("PORT".to_owned(), resolved.main.port.to_string());
    main_environment
        .entry("PREVIA_APP_ENABLED".to_owned())
        .or_insert_with(|| "true".to_owned());
    main_environment.insert(
        "RUNNER_ENDPOINTS".to_owned(),
        resolved
            .local_runner_ports
            .iter()
            .map(|(_, port)| format!("http://{}:{port}", runner_service_name(*port)))
            .chain(resolved.attached_runners.iter().cloned())
            .collect::<Vec<_>>()
            .join(","),
    );
    main_environment.insert(
        "ORCHESTRATOR_DATABASE_URL".to_owned(),
        format!("sqlite://{MAIN_DATA_DIR_IN_CONTAINER}/orchestrator.db"),
    );

    services.insert(
        MAIN_SERVICE_NAME.to_owned(),
        ComposeService {
            image: main_image_ref(&resolved.image_tag)?,
            environment: main_environment,
            ports: vec![ComposePort {
                target: resolved.main.port,
                published: resolved.main.port,
                host_ip: resolved.main.address.clone(),
            }],
            volumes: vec![ComposeVolume {
                kind: "bind",
                source: resolved.stack_paths.main_data_dir.display().to_string(),
                target: MAIN_DATA_DIR_IN_CONTAINER,
            }],
        },
    );

    for runner in &resolved.local_runners {
        let mut environment = runner.env.clone();
        environment.insert("ADDRESS".to_owned(), "0.0.0.0".to_owned());
        environment.insert("PORT".to_owned(), runner.port.to_string());

        services.insert(
            runner_service_name(runner.port),
            ComposeService {
                image: runner_image_ref(&resolved.image_tag)?,
                environment,
                ports: vec![ComposePort {
                    target: runner.port,
                    published: runner.port,
                    host_ip: runner.address.clone(),
                }],
                volumes: Vec::new(),
            },
        );
    }

    Ok(ComposeDocument { services })
}

async fn run_status(mut command: Command, description: &str) -> Result<()> {
    let status = command
        .status()
        .await
        .with_context(|| docker_spawn_error(description))?;

    if status.success() || status.code() == Some(130) {
        return Ok(());
    }

    bail!("{description} failed with status {status}");
}

async fn run_output(mut command: Command, description: &str) -> Result<std::process::Output> {
    let output = command
        .output()
        .await
        .with_context(|| docker_spawn_error(description))?;
    if output.status.success() {
        return Ok(output);
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        bail!("{description} failed with status {}", output.status);
    }
    bail!("{description} failed: {stderr}");
}

fn docker_spawn_error(description: &str) -> String {
    format!(
        "{description}: failed to spawn Docker Compose; ensure `docker compose` or `docker-compose` is installed and available in PATH"
    )
}

fn resolve_compose_cli() -> Result<ComposeCli> {
    resolve_compose_cli_with(
        || command_available("docker", &["compose", "version"]),
        || command_available("docker-compose", &["version"]),
    )
}

fn resolve_compose_cli_with<D, L>(
    docker_plugin_available: D,
    docker_compose_available: L,
) -> Result<ComposeCli>
where
    D: FnOnce() -> bool,
    L: FnOnce() -> bool,
{
    if docker_plugin_available() {
        return Ok(ComposeCli::DockerPlugin);
    }

    if docker_compose_available() {
        return Ok(ComposeCli::DockerComposeBinary);
    }

    bail!(
        "failed to find Docker Compose; ensure `docker compose` or `docker-compose` is installed and available in PATH"
    )
}

fn command_available(program: &str, args: &[&str]) -> bool {
    StdCommand::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

impl ComposeCli {
    fn program(self) -> &'static str {
        match self {
            ComposeCli::DockerPlugin => "docker",
            ComposeCli::DockerComposeBinary => "docker-compose",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            ComposeCli::DockerPlugin => "docker compose",
            ComposeCli::DockerComposeBinary => "docker-compose",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use crate::config::{MainResolvedConfig, ResolvedUpConfig, RunnerLaunch};
    use crate::runtime::{PortRange, RuntimeBackend};

    use super::{
        ComposeCli, MAIN_SERVICE_NAME, compose_project_name, resolve_compose_cli_with,
        runner_service_name, write_generated_compose,
    };

    #[test]
    fn project_name_sanitizes_context() {
        assert_eq!(compose_project_name("default"), "previa_default");
        assert_eq!(compose_project_name("api.local"), "previa_api_local");
    }

    #[test]
    fn runner_service_uses_published_port() {
        assert_eq!(runner_service_name(55880), "runner-55880");
    }

    #[test]
    fn generated_compose_contains_expected_services() {
        let temp = tempfile::tempdir().expect("tempdir");
        let stack_paths = crate::paths::PreviaPaths {
            home: temp.path().to_path_buf(),
            workspace_root: None,
        }
        .stack("default");
        stack_paths.ensure_parent_dirs().expect("dirs");

        let resolved = ResolvedUpConfig {
            previa_paths: crate::paths::PreviaPaths {
                home: temp.path().to_path_buf(),
                workspace_root: None,
            },
            stack_paths: stack_paths.clone(),
            backend: RuntimeBackend::Compose,
            source: None,
            image_tag: "latest".to_owned(),
            main: MainResolvedConfig {
                address: "127.0.0.1".to_owned(),
                port: 5588,
            },
            main_env: BTreeMap::new(),
            local_runner_count: 1,
            runner_port_range: PortRange {
                start: 55880,
                end: 55880,
            },
            local_runners: vec![RunnerLaunch {
                address: "127.0.0.1".to_owned(),
                port: 55880,
                env: BTreeMap::new(),
            }],
            local_runner_ports: vec![("127.0.0.1".to_owned(), 55880)],
            attached_runners: vec!["http://10.0.0.10:55880".to_owned()],
            runner_auth_key: None,
            generated_runner_auth_key: None,
            auth_config_changed: false,
            dry_run: false,
            detach: true,
        };

        write_generated_compose(&resolved).expect("compose file");
        let contents = std::fs::read_to_string(PathBuf::from(&stack_paths.compose_file))
            .expect("compose contents");
        assert!(contents.contains(MAIN_SERVICE_NAME));
        assert!(contents.contains("runner-55880"));
        assert!(contents.contains("ghcr.io/runvibe/main:latest"));
        assert!(contents.contains("ghcr.io/runvibe/runner:latest"));
        assert!(contents.contains("\"127.0.0.1:5588:5588\""));
        assert!(contents.contains("\"127.0.0.1:55880:55880\""));
    }

    #[test]
    fn prefers_docker_compose_plugin_when_available() {
        let resolved = resolve_compose_cli_with(|| true, || true).expect("compose cli");
        assert_eq!(resolved, ComposeCli::DockerPlugin);
    }

    #[test]
    fn falls_back_to_docker_compose_binary() {
        let resolved = resolve_compose_cli_with(|| false, || true).expect("compose cli");
        assert_eq!(resolved, ComposeCli::DockerComposeBinary);
    }

    #[test]
    fn errors_when_no_compose_runtime_is_available() {
        let err = resolve_compose_cli_with(|| false, || false).expect_err("missing compose");
        assert!(
            err.to_string().contains("failed to find Docker Compose"),
            "unexpected error: {err}"
        );
    }
}
