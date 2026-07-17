use anyhow::{Context, Result, anyhow};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};

use crate::paths::StackPaths;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PortRange {
    pub start: u16,
    pub end: u16,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeBackend {
    #[default]
    Compose,
    Bin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MainRuntime {
    #[serde(default)]
    pub service_name: String,
    #[serde(default)]
    pub pid: u32,
    pub address: String,
    pub port: u16,
    #[serde(default)]
    pub log_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalRunnerRuntime {
    #[serde(default)]
    pub service_name: String,
    #[serde(default)]
    pub pid: u32,
    pub address: String,
    pub port: u16,
    #[serde(default)]
    pub log_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetachedRuntimeState {
    pub name: String,
    pub mode: String,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default)]
    pub backend: RuntimeBackend,
    #[serde(default)]
    pub image_tag: String,
    #[serde(default)]
    pub compose_file: String,
    #[serde(default)]
    pub compose_project: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_auth_key: Option<String>,
    pub main: MainRuntime,
    pub runner_port_range: PortRange,
    pub attached_runners: Vec<String>,
    pub runners: Vec<LocalRunnerRuntime>,
}

pub struct StackLock {
    _file: File,
}

pub fn acquire_lock(stack_paths: &StackPaths) -> Result<StackLock> {
    stack_paths.ensure_parent_dirs()?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&stack_paths.lock_file)
        .with_context(|| format!("failed to open '{}'", stack_paths.lock_file.display()))?;
    file.try_lock_exclusive().map_err(|_| {
        anyhow!(
            "context '{}' is locked by another mutating operation",
            stack_paths.name
        )
    })?;
    Ok(StackLock { _file: file })
}

pub fn read_runtime_state(stack_paths: &StackPaths) -> Result<Option<DetachedRuntimeState>> {
    if !stack_paths.runtime_file.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(&stack_paths.runtime_file)
        .with_context(|| format!("failed to read '{}'", stack_paths.runtime_file.display()))?;
    let state = serde_json::from_str::<DetachedRuntimeState>(&contents)
        .with_context(|| format!("failed to parse '{}'", stack_paths.runtime_file.display()))?;
    Ok(Some(normalize_runtime_state(stack_paths, state)))
}

pub fn write_runtime_state(stack_paths: &StackPaths, state: &DetachedRuntimeState) -> Result<()> {
    stack_paths.ensure_parent_dirs()?;
    let tmp = stack_paths.run_dir.join("state.json.tmp");
    let contents = serde_json::to_vec_pretty(state).context("failed to encode runtime state")?;
    std::fs::write(&tmp, contents)
        .with_context(|| format!("failed to write '{}'", tmp.display()))?;
    std::fs::rename(&tmp, &stack_paths.runtime_file).with_context(|| {
        format!(
            "failed to move '{}' to '{}'",
            tmp.display(),
            stack_paths.runtime_file.display()
        )
    })?;
    Ok(())
}

pub fn remove_runtime_state(stack_paths: &StackPaths) -> Result<()> {
    if stack_paths.runtime_file.exists() {
        std::fs::remove_file(&stack_paths.runtime_file).with_context(|| {
            format!("failed to remove '{}'", stack_paths.runtime_file.display())
        })?;
    }
    Ok(())
}

fn normalize_runtime_state(
    stack_paths: &StackPaths,
    mut state: DetachedRuntimeState,
) -> DetachedRuntimeState {
    let looks_like_legacy_bin = state.compose_file.is_empty()
        && state.compose_project.is_empty()
        && state.main.service_name.is_empty()
        && state.main.pid > 0
        && state.runners.iter().all(|runner| {
            runner.service_name.is_empty() && runner.pid > 0 && !runner.log_path.is_empty()
        });

    if looks_like_legacy_bin {
        state.backend = RuntimeBackend::Bin;
        return state;
    }

    if state.backend == RuntimeBackend::Compose {
        if state.compose_file.is_empty() {
            state.compose_file = stack_paths.compose_file.display().to_string();
        }
        if state.compose_project.is_empty() {
            state.compose_project = compose_project_name(&stack_paths.name);
        }
        if state.main.service_name.is_empty() {
            state.main.service_name = "main".to_owned();
        }
        for runner in &mut state.runners {
            if runner.service_name.is_empty() {
                runner.service_name = format!("runner-{}", runner.port);
            }
        }
    }

    state
}

fn compose_project_name(context: &str) -> String {
    let sanitized = context
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    format!("previa_{sanitized}")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{DetachedRuntimeState, PortRange, RuntimeBackend, normalize_runtime_state};
    use crate::paths::StackPaths;
    use crate::runtime::{LocalRunnerRuntime, MainRuntime};

    fn stack_paths() -> StackPaths {
        StackPaths {
            name: "default".to_owned(),
            config_dir: PathBuf::from("/tmp/previa/default/config"),
            main_env: PathBuf::from("/tmp/previa/default/config/main.env"),
            runner_env: PathBuf::from("/tmp/previa/default/config/runner.env"),
            main_data_dir: PathBuf::from("/tmp/previa/default/data/main"),
            runner_logs_dir: PathBuf::from("/tmp/previa/default/logs/runners"),
            main_log: PathBuf::from("/tmp/previa/default/logs/main.log"),
            run_dir: PathBuf::from("/tmp/previa/default/run"),
            lock_file: PathBuf::from("/tmp/previa/default/run/.lock"),
            compose_file: PathBuf::from("/tmp/previa/default/run/docker-compose.generated.yaml"),
            runtime_file: PathBuf::from("/tmp/previa/default/run/state.json"),
        }
    }

    #[test]
    fn normalizes_legacy_bin_runtime_state() {
        let state = DetachedRuntimeState {
            name: "default".to_owned(),
            mode: "detached".to_owned(),
            started_at: "2026-03-13T00:00:00Z".to_owned(),
            source: None,
            backend: RuntimeBackend::Compose,
            image_tag: String::new(),
            compose_file: String::new(),
            compose_project: String::new(),
            runner_auth_key: None,
            main: MainRuntime {
                service_name: String::new(),
                pid: 10,
                address: "0.0.0.0".to_owned(),
                port: 5588,
                log_path: "/tmp/main.log".to_owned(),
            },
            runner_port_range: PortRange {
                start: 55880,
                end: 55979,
            },
            attached_runners: Vec::new(),
            runners: vec![LocalRunnerRuntime {
                service_name: String::new(),
                pid: 11,
                address: "127.0.0.1".to_owned(),
                port: 55880,
                log_path: "/tmp/runner.log".to_owned(),
            }],
        };

        let normalized = normalize_runtime_state(&stack_paths(), state);
        assert_eq!(normalized.backend, RuntimeBackend::Bin);
        assert!(normalized.compose_file.is_empty());
        assert!(normalized.compose_project.is_empty());
    }

    #[test]
    fn fills_missing_compose_metadata_for_compose_runtime() {
        let state = DetachedRuntimeState {
            name: "default".to_owned(),
            mode: "detached".to_owned(),
            started_at: "2026-03-13T00:00:00Z".to_owned(),
            source: None,
            backend: RuntimeBackend::Compose,
            image_tag: "latest".to_owned(),
            compose_file: String::new(),
            compose_project: String::new(),
            runner_auth_key: Some("secret".to_owned()),
            main: MainRuntime {
                service_name: String::new(),
                pid: 0,
                address: "0.0.0.0".to_owned(),
                port: 5588,
                log_path: String::new(),
            },
            runner_port_range: PortRange {
                start: 55880,
                end: 55979,
            },
            attached_runners: Vec::new(),
            runners: vec![LocalRunnerRuntime {
                service_name: String::new(),
                pid: 0,
                address: "127.0.0.1".to_owned(),
                port: 55880,
                log_path: String::new(),
            }],
        };

        let normalized = normalize_runtime_state(&stack_paths(), state);
        assert_eq!(normalized.backend, RuntimeBackend::Compose);
        assert_eq!(
            normalized.compose_file,
            "/tmp/previa/default/run/docker-compose.generated.yaml"
        );
        assert_eq!(normalized.compose_project, "previa_default");
        assert_eq!(normalized.main.service_name, "main");
        assert_eq!(normalized.runners[0].service_name, "runner-55880");
    }
}
