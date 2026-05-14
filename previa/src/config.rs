use std::collections::BTreeMap;
use std::env;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use uuid::Uuid;

use crate::cli::UpArgs;
use crate::download::ensure_runtime_binaries;
use crate::envfile::{
    default_main_env_map, default_runner_env_map, ensure_default_env_files, read_env_file,
};
use crate::paths::{PreviaPaths, StackPaths, sqlite_database_url};
use crate::pull::normalize_image_tag;
use crate::runtime::{DetachedRuntimeState, PortRange, RuntimeBackend};
use crate::selectors::normalize_attach_runner;

#[derive(Debug, Clone)]
pub struct ResolvedUpConfig {
    pub previa_paths: PreviaPaths,
    pub stack_paths: StackPaths,
    pub backend: RuntimeBackend,
    pub source: Option<PathBuf>,
    pub image_tag: String,
    pub main: MainResolvedConfig,
    pub main_env: BTreeMap<String, String>,
    pub local_runner_count: usize,
    pub runner_port_range: PortRange,
    pub local_runners: Vec<RunnerLaunch>,
    pub local_runner_ports: Vec<(String, u16)>,
    pub attached_runners: Vec<String>,
    pub runner_auth_key: Option<String>,
    pub generated_runner_auth_key: Option<String>,
    pub auth_config_changed: bool,
    pub dry_run: bool,
    pub detach: bool,
}

#[derive(Debug, Clone)]
pub struct MainResolvedConfig {
    pub address: String,
    pub port: u16,
}

#[derive(Debug, Clone)]
pub struct RunnerLaunch {
    pub address: String,
    pub port: u16,
    pub env: BTreeMap<String, String>,
}

impl RunnerLaunch {
    pub fn health_url(&self) -> String {
        format!("http://{}:{}/health", self.address, self.port)
    }
}

impl ResolvedUpConfig {
    pub async fn from_runtime(
        paths: &PreviaPaths,
        stack_paths: &StackPaths,
        state: &DetachedRuntimeState,
        version_override: Option<&str>,
    ) -> Result<Self> {
        if state.backend == RuntimeBackend::Bin && version_override.is_some() {
            bail!("--version is only supported for compose-backed runtimes");
        }

        let main_env = read_env_file(&stack_paths.main_env)?;
        let runner_env = read_env_file(&stack_paths.runner_env)?;
        let local_runner_count = state.runners.len();
        let local_runners = state
            .runners
            .iter()
            .map(|runner| {
                let mut env = runner_env.clone();
                env.insert("ADDRESS".to_owned(), runner.address.clone());
                env.insert("PORT".to_owned(), runner.port.to_string());
                if let Some(runner_auth_key) = state.runner_auth_key.as_ref() {
                    env.insert("RUNNER_AUTH_KEY".to_owned(), runner_auth_key.clone());
                }
                RunnerLaunch {
                    address: runner.address.clone(),
                    port: runner.port,
                    env,
                }
            })
            .collect::<Vec<_>>();

        if state.backend == RuntimeBackend::Bin {
            ensure_runtime_binaries(paths, local_runner_count).await?;
        }

        let mut main_env = merge_env(default_main_env_map(stack_paths), main_env);
        main_env.insert("ADDRESS".to_owned(), state.main.address.clone());
        main_env.insert("PORT".to_owned(), state.main.port.to_string());
        main_env.insert("PREVIA_CONTEXT".to_owned(), stack_paths.name.clone());
        main_env
            .entry("PREVIA_APP_ENABLED".to_owned())
            .or_insert_with(|| "true".to_owned());
        if let Some(runner_auth_key) = state.runner_auth_key.as_ref() {
            main_env.insert("RUNNER_AUTH_KEY".to_owned(), runner_auth_key.clone());
        }
        main_env.insert(
            "RUNNER_ENDPOINTS".to_owned(),
            state
                .runners
                .iter()
                .map(|runner| format!("http://{}:{}", runner.address, runner.port))
                .chain(state.attached_runners.clone())
                .collect::<Vec<_>>()
                .join(","),
        );

        Ok(Self {
            previa_paths: paths.clone(),
            stack_paths: stack_paths.clone(),
            backend: state.backend,
            source: state.source.as_ref().map(PathBuf::from),
            image_tag: if state.backend == RuntimeBackend::Compose {
                normalize_image_tag(version_override.unwrap_or(state.image_tag.as_str()))?
            } else {
                String::new()
            },
            main: MainResolvedConfig {
                address: state.main.address.clone(),
                port: state.main.port,
            },
            main_env,
            local_runner_count,
            runner_port_range: state.runner_port_range,
            local_runners: local_runners.clone(),
            local_runner_ports: local_runners
                .iter()
                .map(|runner| (runner.address.clone(), runner.port))
                .collect(),
            attached_runners: state.attached_runners.clone(),
            runner_auth_key: state.runner_auth_key.clone(),
            generated_runner_auth_key: None,
            auth_config_changed: false,
            dry_run: false,
            detach: true,
        })
    }

    pub fn main_health_url(&self) -> String {
        format!("http://{}:{}/health", self.main.address, self.main.port)
    }

    pub fn set_main_port(&mut self, port: u16) {
        self.main.port = port;
        self.main_env.insert("PORT".to_owned(), port.to_string());
    }

    pub fn shift_runner_ports(&mut self, offset: u16) {
        self.runner_port_range.start += offset;
        self.runner_port_range.end += offset;

        for (index, runner) in self.local_runners.iter_mut().enumerate() {
            let port = runner.port + offset;
            runner.port = port;
            runner.env.insert("PORT".to_owned(), port.to_string());
            self.local_runner_ports[index] = (runner.address.clone(), port);
        }

        self.main_env.insert(
            "RUNNER_ENDPOINTS".to_owned(),
            self.local_runners
                .iter()
                .map(|runner| format!("http://{}:{}", runner.address, runner.port))
                .chain(self.attached_runners.clone())
                .collect::<Vec<_>>()
                .join(","),
        );
    }
}

fn validate_port(port: u16, label: &str) -> Result<u16> {
    if port == 0 {
        bail!("invalid {label} '0'");
    }
    Ok(port)
}

fn validate_port_range(range: PortRange) -> Result<PortRange> {
    validate_port(range.start, "runner port range")?;
    validate_port(range.end, "runner port range")?;
    if range.start > range.end {
        bail!("invalid runner port range");
    }
    Ok(range)
}

pub async fn resolve_up_config(
    paths: &PreviaPaths,
    stack_paths: &StackPaths,
    args: UpArgs,
) -> Result<ResolvedUpConfig> {
    if args.dry_run && args.detach {
        bail!("--dry-run cannot be combined with --detach");
    }
    if args.bin_requested() && args.version != env!("CARGO_PKG_VERSION") {
        bail!("--version cannot be used with --bin");
    }
    let backend = if args.bin_requested() {
        RuntimeBackend::Bin
    } else {
        RuntimeBackend::Compose
    };
    let image_tag = if backend == RuntimeBackend::Compose {
        normalize_image_tag(&args.version)?
    } else {
        String::new()
    };

    stack_paths.ensure_parent_dirs()?;
    if !args.dry_run {
        ensure_default_env_files(stack_paths)?;
    }

    let source = resolve_compose_source(args.source.as_deref())?;
    let compose = if let Some(source) = &source {
        Some(read_compose_file(source)?)
    } else {
        None
    };

    let main_env_file = if stack_paths.main_env.exists() {
        read_env_file(&stack_paths.main_env)?
    } else {
        default_main_env_map(stack_paths)
    };
    let runner_env_file = if stack_paths.runner_env.exists() {
        read_env_file(&stack_paths.runner_env)?
    } else {
        default_runner_env_map()
    };

    let main_address = args
        .main_address
        .clone()
        .or_else(|| {
            compose
                .as_ref()
                .and_then(|compose| compose.main.as_ref()?.address.clone())
        })
        .or_else(|| main_env_file.get("ADDRESS").cloned())
        .unwrap_or_else(|| "0.0.0.0".to_owned());
    validate_address(&main_address)?;

    let main_port = args
        .main_port
        .or_else(|| {
            compose
                .as_ref()
                .and_then(|compose| compose.main.as_ref()?.port)
        })
        .or_else(|| {
            main_env_file
                .get("PORT")
                .and_then(|value| value.parse::<u16>().ok())
        })
        .unwrap_or(5588);
    let main_port = validate_port(main_port, "main port")?;

    let runner_address = args
        .runner_address
        .clone()
        .or_else(|| {
            compose
                .as_ref()
                .and_then(|compose| compose.runners.as_ref()?.local.as_ref()?.address.clone())
        })
        .or_else(|| runner_env_file.get("ADDRESS").cloned())
        .unwrap_or_else(|| "127.0.0.1".to_owned());
    validate_address(&runner_address)?;

    let runner_port_range = if let Some(raw) = args.runner_port_range.as_deref() {
        parse_port_range(raw)?
    } else if let Some(compose) = compose.as_ref() {
        if let Some(local) = compose
            .runners
            .as_ref()
            .and_then(|runners| runners.local.as_ref())
        {
            PortRange {
                start: local
                    .port_range
                    .as_ref()
                    .and_then(|value| value.start)
                    .unwrap_or(55880),
                end: local
                    .port_range
                    .as_ref()
                    .and_then(|value| value.end)
                    .unwrap_or(55979),
            }
        } else {
            PortRange {
                start: 55880,
                end: 55979,
            }
        }
    } else {
        PortRange {
            start: 55880,
            end: 55979,
        }
    };
    let runner_port_range = validate_port_range(runner_port_range)?;

    let local_runner_count = args
        .runners
        .or_else(|| {
            compose
                .as_ref()
                .and_then(|compose| compose.runners.as_ref()?.local.as_ref()?.count)
        })
        .unwrap_or(1);

    let attached_raw = if !args.attach_runners.is_empty() {
        args.attach_runners.clone()
    } else {
        compose
            .as_ref()
            .and_then(|compose| compose.runners.as_ref()?.attach.clone())
            .unwrap_or_default()
    };
    let attached_runners = attached_raw
        .iter()
        .map(|value| normalize_attach_runner(value))
        .collect::<Result<Vec<_>>>()?;
    let compose_main_env = compose
        .as_ref()
        .and_then(|compose| compose.main.as_ref()?.env.clone());
    let compose_runner_env = compose
        .as_ref()
        .and_then(|compose| compose.runners.as_ref()?.local.as_ref()?.env.clone())
        .unwrap_or_default();

    if local_runner_count == 0 && attached_runners.is_empty() {
        bail!("up requires at least one local or attached runner");
    }
    let capacity = (runner_port_range.end - runner_port_range.start + 1) as usize;
    if local_runner_count > capacity {
        bail!("requested local runner count exceeds the configured port range");
    }
    if backend == RuntimeBackend::Bin {
        ensure_runtime_binaries(paths, local_runner_count).await?;
    }

    let mut main_env = merge_env(default_main_env_map(stack_paths), main_env_file);
    if let Some(extra_env) = &compose_main_env {
        main_env = merge_env(main_env, extra_env.clone());
    }
    main_env.insert("ADDRESS".to_owned(), main_address.clone());
    main_env.insert("PORT".to_owned(), main_port.to_string());
    main_env.insert("PREVIA_CONTEXT".to_owned(), stack_paths.name.clone());
    main_env
        .entry("PREVIA_APP_ENABLED".to_owned())
        .or_insert_with(|| "true".to_owned());
    main_env
        .entry("ORCHESTRATOR_DATABASE_URL".to_owned())
        .or_insert_with(|| sqlite_database_url(&stack_paths.orchestrator_db));

    let auth_config_changed = apply_access_management_config(&mut main_env, &args)?;

    let mut effective_runner_auth_key = resolve_runner_auth_key(
        process_runner_auth_key(),
        compose_main_env.as_ref(),
        &compose_runner_env,
        &main_env,
        &runner_env_file,
    );
    let generated_runner_auth_key = if effective_runner_auth_key.is_none()
        && attached_runners.is_empty()
        && local_runner_count > 0
    {
        let key = Uuid::new_v4().to_string();
        effective_runner_auth_key = Some(key.clone());
        Some(key)
    } else {
        None
    };
    if let Some(runner_auth_key) = effective_runner_auth_key.as_ref() {
        main_env.insert("RUNNER_AUTH_KEY".to_owned(), runner_auth_key.clone());
    }
    if !attached_runners.is_empty() && effective_runner_auth_key.is_none() {
        bail!("RUNNER_AUTH_KEY is required when using --attach-runner");
    }

    let mut local_runners = Vec::with_capacity(local_runner_count);
    let mut local_runner_ports = Vec::with_capacity(local_runner_count);

    for offset in 0..local_runner_count {
        let port = runner_port_range.start + offset as u16;
        let mut env = merge_env(default_runner_env_map(), runner_env_file.clone());
        env = merge_env(env, compose_runner_env.clone());
        env.insert("ADDRESS".to_owned(), runner_address.clone());
        env.insert("PORT".to_owned(), port.to_string());
        if let Some(runner_auth_key) = effective_runner_auth_key.as_ref() {
            env.insert("RUNNER_AUTH_KEY".to_owned(), runner_auth_key.clone());
        }
        local_runners.push(RunnerLaunch {
            address: runner_address.clone(),
            port,
            env,
        });
        local_runner_ports.push((runner_address.clone(), port));
    }

    let runner_endpoints = local_runners
        .iter()
        .map(|runner| format!("http://{}:{}", runner.address, runner.port))
        .chain(attached_runners.iter().cloned())
        .collect::<Vec<_>>();
    main_env.insert("RUNNER_ENDPOINTS".to_owned(), runner_endpoints.join(","));

    Ok(ResolvedUpConfig {
        previa_paths: paths.clone(),
        stack_paths: stack_paths.clone(),
        backend,
        source,
        image_tag,
        main: MainResolvedConfig {
            address: main_address,
            port: main_port,
        },
        main_env,
        local_runner_count,
        runner_port_range,
        local_runners,
        local_runner_ports,
        attached_runners,
        runner_auth_key: effective_runner_auth_key,
        generated_runner_auth_key,
        auth_config_changed,
        dry_run: args.dry_run,
        detach: args.detach,
    })
}

fn apply_access_management_config(
    main_env: &mut BTreeMap<String, String>,
    args: &UpArgs,
) -> Result<bool> {
    if args.anonymous {
        main_env.insert("PREVIA_AUTH_ANONYMOUS".to_owned(), "true".to_owned());
        return Ok(true);
    }
    if !args.protected {
        return Ok(false);
    }

    main_env.insert("PREVIA_AUTH_ANONYMOUS".to_owned(), "false".to_owned());
    let root_username = args
        .root_username
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| configured_env_value(main_env, "PREVIA_ROOT_USERNAME"))
        .unwrap_or_else(|| "root".to_owned());
    main_env.insert("PREVIA_ROOT_USERNAME".to_owned(), root_username);

    let root_password = args
        .root_password
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| configured_env_value(main_env, "PREVIA_ROOT_PASSWORD"))
        .ok_or_else(|| {
            anyhow!("--protected requires --root-password-stdin unless PREVIA_ROOT_PASSWORD already exists in main.env")
        })?;
    main_env.insert("PREVIA_ROOT_PASSWORD".to_owned(), root_password);

    if configured_env_value(main_env, "PREVIA_JWT_SECRET").is_none() {
        main_env.insert("PREVIA_JWT_SECRET".to_owned(), Uuid::new_v4().to_string());
    }

    Ok(true)
}

fn configured_env_value(env: &BTreeMap<String, String>, key: &str) -> Option<String> {
    env.get(key)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn process_runner_auth_key() -> Option<String> {
    env::var("RUNNER_AUTH_KEY")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn configured_runner_auth_key(env: &BTreeMap<String, String>) -> Option<&str> {
    env.get("RUNNER_AUTH_KEY")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

fn resolve_runner_auth_key(
    process_value: Option<String>,
    compose_main_env: Option<&BTreeMap<String, String>>,
    compose_runner_env: &BTreeMap<String, String>,
    main_env: &BTreeMap<String, String>,
    runner_env: &BTreeMap<String, String>,
) -> Option<String> {
    process_value.or_else(|| {
        compose_main_env
            .and_then(configured_runner_auth_key)
            .or_else(|| configured_runner_auth_key(compose_runner_env))
            .or_else(|| configured_runner_auth_key(main_env))
            .or_else(|| configured_runner_auth_key(runner_env))
            .map(ToOwned::to_owned)
    })
}

#[derive(Debug, Deserialize)]
struct ComposeFile {
    version: i64,
    main: Option<ComposeMain>,
    runners: Option<ComposeRunners>,
}

#[derive(Debug, Deserialize)]
struct ComposeMain {
    address: Option<String>,
    port: Option<u16>,
    #[serde(default)]
    env: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct ComposeRunners {
    local: Option<ComposeLocalRunners>,
    attach: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct ComposeLocalRunners {
    address: Option<String>,
    count: Option<usize>,
    port_range: Option<ComposePortRange>,
    #[serde(default)]
    env: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct ComposePortRange {
    start: Option<u16>,
    end: Option<u16>,
}

fn resolve_compose_source(source: Option<&str>) -> Result<Option<PathBuf>> {
    let Some(source) = source else {
        return Ok(None);
    };
    let path = PathBuf::from(source);
    if source == "." || path.is_dir() {
        let dir = if source == "." {
            std::env::current_dir().context("failed to read current directory")?
        } else {
            path.canonicalize()
                .with_context(|| format!("failed to access '{}'", path.display()))?
        };
        for candidate in [
            dir.join("previa-compose.yaml"),
            dir.join("previa-compose.yml"),
            dir.join("previa-compose.json"),
        ] {
            if candidate.exists() {
                return Ok(Some(candidate));
            }
        }
        bail!("missing compose file in '{}'", dir.display());
    }

    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("unsupported compose file extension"))?;
    if !matches!(extension, "json" | "yaml" | "yml") {
        bail!("unsupported compose file extension '{}'", extension);
    }
    Ok(Some(path.canonicalize().with_context(|| {
        format!("failed to access '{}'", path.display())
    })?))
}

fn read_compose_file(path: &Path) -> Result<ComposeFile> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read '{}'", path.display()))?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("unsupported compose file extension"))?;
    let compose = match extension {
        "json" => serde_json::from_str::<ComposeFile>(&contents)
            .with_context(|| format!("invalid JSON compose file '{}'", path.display()))?,
        "yaml" | "yml" => serde_yaml::from_str::<ComposeFile>(&contents)
            .with_context(|| format!("invalid YAML compose file '{}'", path.display()))?,
        _ => bail!("unsupported compose file extension '{}'", extension),
    };
    if compose.version != 1 {
        bail!("unsupported compose version '{}'", compose.version);
    }
    Ok(compose)
}

fn parse_port_range(raw: &str) -> Result<PortRange> {
    let (start, end) = raw
        .split_once(':')
        .ok_or_else(|| anyhow!("invalid runner port range '{}'", raw))?;
    let start = start
        .parse::<u16>()
        .with_context(|| format!("invalid runner port range '{}'", raw))?;
    let end = end
        .parse::<u16>()
        .with_context(|| format!("invalid runner port range '{}'", raw))?;
    Ok(PortRange { start, end })
}

fn validate_address(value: &str) -> Result<()> {
    if value.is_empty() {
        bail!("address cannot be empty");
    }
    if value.parse::<IpAddr>().is_ok() {
        return Ok(());
    }
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-'))
    {
        return Ok(());
    }
    bail!("invalid address '{}'", value)
}

fn merge_env(
    base: BTreeMap<String, String>,
    override_values: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut merged = base;
    for (key, value) in override_values {
        merged.insert(key, value);
    }
    merged
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    #[cfg(target_os = "linux")]
    use std::sync::{Arc, Mutex};

    #[cfg(target_os = "linux")]
    use axum::Router;
    #[cfg(target_os = "linux")]
    use axum::extract::State;
    #[cfg(target_os = "linux")]
    use axum::http::StatusCode;
    #[cfg(target_os = "linux")]
    use axum::routing::get;
    use tempfile::TempDir;
    #[cfg(target_os = "linux")]
    use tokio::net::TcpListener;

    use super::{configured_runner_auth_key, resolve_runner_auth_key, resolve_up_config};
    use crate::cli::UpArgs;
    use crate::paths::PreviaPaths;
    use crate::runtime::{
        DetachedRuntimeState, LocalRunnerRuntime, MainRuntime, PortRange, RuntimeBackend,
    };
    use uuid::Uuid;

    #[cfg(target_os = "linux")]
    #[derive(Clone)]
    struct TestServerState {
        binaries: BTreeMap<String, Vec<u8>>,
        requests: Arc<Mutex<Vec<String>>>,
    }

    #[cfg(target_os = "linux")]
    async fn binary_asset(
        State(state): State<TestServerState>,
        axum::extract::Path((version, name)): axum::extract::Path<(String, String)>,
    ) -> (StatusCode, Vec<u8>) {
        state
            .requests
            .lock()
            .expect("requests lock")
            .push(format!("/{version}/files/{name}"));
        match state.binaries.get(&name) {
            Some(bytes) => (StatusCode::OK, bytes.clone()),
            None => (StatusCode::NOT_FOUND, Vec::new()),
        }
    }

    #[cfg(target_os = "linux")]
    async fn spawn_test_server(
        binaries: BTreeMap<String, Vec<u8>>,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let address = listener.local_addr().expect("local addr");
        let base_url = format!("http://{address}");
        let state = TestServerState {
            binaries,
            requests: requests.clone(),
        };
        let app = Router::new()
            .route("/{version}/files/{name}", get(binary_asset))
            .with_state(state);
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve app");
        });

        (base_url, requests)
    }

    fn temp_paths() -> (TempDir, PreviaPaths) {
        let temp = TempDir::new().expect("tempdir");
        let paths = PreviaPaths {
            home: temp.path().to_path_buf(),
            workspace_root: Some(temp.path().join("__no_workspace__")),
        };
        (temp, paths)
    }

    fn base_args() -> UpArgs {
        UpArgs {
            context: "default".to_owned(),
            source: None,
            main_address: None,
            main_port: None,
            runner_address: None,
            runner_port_range: None,
            runners: None,
            import_path: None,
            recursive: false,
            stack: None,
            attach_runners: Vec::new(),
            dry_run: false,
            detach: false,
            protected: false,
            anonymous: false,
            root_username: None,
            root_password_stdin: false,
            root_password: None,
            #[cfg(target_os = "linux")]
            bin: true,
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }

    fn set_bin(_args: &mut UpArgs, value: bool) {
        #[cfg(target_os = "linux")]
        {
            _args.bin = value;
        }

        #[cfg(not(target_os = "linux"))]
        {
            assert!(
                !value,
                "--bin should not be set on non-Linux targets in these tests"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn resolve_up_config_downloads_missing_main_and_runner_for_bin_runtime() {
        let _guard = crate::download::DOWNLOAD_ENV_LOCK
            .lock()
            .expect("download env lock");
        let (_temp, paths) = temp_paths();
        let stack_paths = paths.stack("default");
        let asset_main = b"#!/bin/sh\necho main\n".to_vec();
        let asset_runner = b"#!/bin/sh\necho runner\n".to_vec();
        let binaries = BTreeMap::from([
            ("previa-main-linux-amd64".to_owned(), asset_main.clone()),
            ("previa-runner-linux-amd64".to_owned(), asset_runner.clone()),
        ]);
        let (base_url, requests) = spawn_test_server(binaries).await;
        unsafe {
            std::env::set_var("PREVIA_DOWNLOAD_BASE_URL", base_url.clone());
        }

        let resolved = resolve_up_config(&paths, &stack_paths, base_args())
            .await
            .expect("resolved config");

        unsafe {
            std::env::remove_var("PREVIA_DOWNLOAD_BASE_URL");
        }

        assert_eq!(resolved.local_runner_count, 1);
        assert!(paths.main_binary().is_ok());
        assert!(paths.runner_binary().is_ok());
        let requests = requests.lock().expect("requests lock");
        assert!(requests.iter().any(|value| value
            == &format!(
                "/{}/files/previa-main-linux-amd64",
                env!("CARGO_PKG_VERSION")
            )));
        assert!(requests.iter().any(|value| value
            == &format!(
                "/{}/files/previa-runner-linux-amd64",
                env!("CARGO_PKG_VERSION")
            )));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn resolve_up_config_attached_runner_only_does_not_download_runner_binary() {
        let _guard = crate::download::DOWNLOAD_ENV_LOCK
            .lock()
            .expect("download env lock");
        let (_temp, paths) = temp_paths();
        let stack_paths = paths.stack("default");
        stack_paths
            .ensure_parent_dirs()
            .expect("ensure stack parent dirs");
        std::fs::write(
            &stack_paths.main_env,
            "RUNNER_AUTH_KEY=attached-secret\nRUST_LOG=info\n",
        )
        .expect("write main.env");
        let binaries = BTreeMap::from([(
            "previa-main-linux-amd64".to_owned(),
            b"#!/bin/sh\necho main\n".to_vec(),
        )]);
        let (base_url, requests) = spawn_test_server(binaries).await;
        unsafe {
            std::env::set_var("PREVIA_DOWNLOAD_BASE_URL", base_url.clone());
        }

        let mut args = base_args();
        args.runners = Some(0);
        args.attach_runners = vec!["55880".to_owned()];
        let resolved = resolve_up_config(&paths, &stack_paths, args)
            .await
            .expect("resolved config");

        unsafe {
            std::env::remove_var("PREVIA_DOWNLOAD_BASE_URL");
        }

        assert_eq!(resolved.local_runner_count, 0);
        assert!(paths.main_binary().is_ok());
        assert!(paths.runner_binary().is_err());
        let requests = requests.lock().expect("requests lock");
        assert!(requests.iter().all(|value| value
            != &format!(
                "/{}/files/previa-runner-linux-amd64",
                env!("CARGO_PKG_VERSION")
            )));
    }

    #[tokio::test]
    async fn resolve_up_config_requires_runner_auth_key_for_attached_runners() {
        let (_temp, paths) = temp_paths();
        let stack_paths = paths.stack("default");
        let mut args = base_args();
        set_bin(&mut args, false);
        args.attach_runners = vec!["55880".to_owned()];

        let error = resolve_up_config(&paths, &stack_paths, args)
            .await
            .expect_err("missing attached runner auth must fail");

        assert!(
            error
                .to_string()
                .contains("RUNNER_AUTH_KEY is required when using --attach-runner")
        );
    }

    #[tokio::test]
    async fn resolve_up_config_allows_attached_runners_when_main_env_has_runner_auth_key() {
        let (_temp, paths) = temp_paths();
        let stack_paths = paths.stack("default");
        stack_paths
            .ensure_parent_dirs()
            .expect("ensure stack parent dirs");
        std::fs::write(
            &stack_paths.main_env,
            "RUNNER_AUTH_KEY=attached-secret\nRUST_LOG=info\n",
        )
        .expect("write main.env");

        let mut args = base_args();
        set_bin(&mut args, false);
        args.attach_runners = vec!["55880".to_owned()];

        let resolved = resolve_up_config(&paths, &stack_paths, args)
            .await
            .expect("attached runner auth from main.env");

        assert_eq!(
            configured_runner_auth_key(&resolved.main_env),
            Some("attached-secret")
        );
        assert_eq!(resolved.attached_runners, vec!["http://127.0.0.1:55880"]);
    }

    #[tokio::test]
    async fn resolve_up_config_generates_runner_auth_key_for_local_runners() {
        let (_temp, paths) = temp_paths();
        let stack_paths = paths.stack("default");
        let mut args = base_args();
        set_bin(&mut args, false);

        let resolved = resolve_up_config(&paths, &stack_paths, args)
            .await
            .expect("local-only config resolves");

        let generated = resolved
            .generated_runner_auth_key
            .as_ref()
            .expect("generated auth key");
        assert!(Uuid::parse_str(generated).is_ok());
        assert_eq!(
            configured_runner_auth_key(&resolved.main_env),
            Some(generated.as_str())
        );
        assert_eq!(
            configured_runner_auth_key(&resolved.local_runners[0].env),
            Some(generated.as_str())
        );
        assert_eq!(
            resolved.runner_auth_key.as_deref(),
            Some(generated.as_str())
        );
    }

    #[tokio::test]
    async fn resolve_up_config_persists_protected_auth_env() {
        let (_temp, paths) = temp_paths();
        let stack_paths = paths.stack("default");
        let mut args = base_args();
        set_bin(&mut args, false);
        args.protected = true;
        args.root_username = Some("admin".to_owned());
        args.root_password = Some("secret".to_owned());

        let resolved = resolve_up_config(&paths, &stack_paths, args)
            .await
            .expect("protected config resolves");

        assert_eq!(
            resolved
                .main_env
                .get("PREVIA_AUTH_ANONYMOUS")
                .map(String::as_str),
            Some("false")
        );
        assert_eq!(
            resolved
                .main_env
                .get("PREVIA_ROOT_USERNAME")
                .map(String::as_str),
            Some("admin")
        );
        assert_eq!(
            resolved
                .main_env
                .get("PREVIA_ROOT_PASSWORD")
                .map(String::as_str),
            Some("secret")
        );
        assert!(
            resolved
                .main_env
                .get("PREVIA_JWT_SECRET")
                .is_some_and(|value| !value.is_empty())
        );
    }

    #[tokio::test]
    async fn resolve_up_config_requires_root_password_for_new_protected_context() {
        let (_temp, paths) = temp_paths();
        let stack_paths = paths.stack("default");
        let mut args = base_args();
        set_bin(&mut args, false);
        args.protected = true;

        let error = resolve_up_config(&paths, &stack_paths, args)
            .await
            .expect_err("missing protected root password");

        assert!(error.to_string().contains("--root-password-stdin"));
    }

    #[tokio::test]
    async fn from_runtime_restores_runner_auth_key_into_main_and_runner_envs() {
        let (_temp, paths) = temp_paths();
        let stack_paths = paths.stack("default");
        stack_paths
            .ensure_parent_dirs()
            .expect("ensure stack parent dirs");

        let state = DetachedRuntimeState {
            name: "default".to_owned(),
            mode: "detached".to_owned(),
            started_at: "2026-03-19T00:00:00Z".to_owned(),
            source: None,
            backend: RuntimeBackend::Compose,
            image_tag: "latest".to_owned(),
            compose_file: stack_paths.compose_file.display().to_string(),
            compose_project: "previa_default".to_owned(),
            runner_auth_key: Some("state-secret".to_owned()),
            main: MainRuntime {
                service_name: "main".to_owned(),
                pid: 0,
                address: "127.0.0.1".to_owned(),
                port: 5588,
                log_path: String::new(),
            },
            runner_port_range: PortRange {
                start: 55880,
                end: 55880,
            },
            attached_runners: Vec::new(),
            runners: vec![LocalRunnerRuntime {
                service_name: "runner-55880".to_owned(),
                pid: 0,
                address: "127.0.0.1".to_owned(),
                port: 55880,
                log_path: String::new(),
            }],
        };

        let resolved = super::ResolvedUpConfig::from_runtime(&paths, &stack_paths, &state, None)
            .await
            .expect("resolved from runtime");

        assert_eq!(resolved.runner_auth_key.as_deref(), Some("state-secret"));
        assert_eq!(
            configured_runner_auth_key(&resolved.main_env),
            Some("state-secret")
        );
        assert_eq!(
            configured_runner_auth_key(&resolved.local_runners[0].env),
            Some("state-secret")
        );
    }

    #[test]
    fn resolve_runner_auth_key_prefers_process_then_compose_then_env_files() {
        let compose_main =
            BTreeMap::from([("RUNNER_AUTH_KEY".to_owned(), "compose-main".to_owned())]);
        let compose_runner =
            BTreeMap::from([("RUNNER_AUTH_KEY".to_owned(), "compose-runner".to_owned())]);
        let main_env = BTreeMap::from([("RUNNER_AUTH_KEY".to_owned(), "main-env".to_owned())]);
        let runner_env = BTreeMap::from([("RUNNER_AUTH_KEY".to_owned(), "runner-env".to_owned())]);

        assert_eq!(
            resolve_runner_auth_key(
                Some("process".to_owned()),
                Some(&compose_main),
                &compose_runner,
                &main_env,
                &runner_env
            ),
            Some("process".to_owned())
        );
        assert_eq!(
            resolve_runner_auth_key(
                None,
                Some(&compose_main),
                &compose_runner,
                &main_env,
                &runner_env
            ),
            Some("compose-main".to_owned())
        );
        assert_eq!(
            resolve_runner_auth_key(None, None, &compose_runner, &main_env, &runner_env),
            Some("compose-runner".to_owned())
        );
        assert_eq!(
            resolve_runner_auth_key(None, None, &BTreeMap::new(), &main_env, &runner_env),
            Some("main-env".to_owned())
        );
        assert_eq!(
            resolve_runner_auth_key(None, None, &BTreeMap::new(), &BTreeMap::new(), &runner_env),
            Some("runner-env".to_owned())
        );
    }
}
