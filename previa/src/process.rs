use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::net::TcpListener;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;
use tokio::time::sleep;

use crate::config::ResolvedUpConfig;
use crate::health::probe_health;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingConflictKind {
    Main,
    Runner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingConflict {
    pub kind: BindingConflictKind,
    pub address: String,
    pub port: u16,
}

pub struct SpawnedStack {
    pub main: Child,
    pub runners: Vec<Child>,
}

pub struct ForegroundStack {
    pub main: Child,
    pub runners: Vec<Child>,
    pub _log_tasks: Vec<JoinHandle<()>>,
}

pub fn validate_startup_bindings(config: &ResolvedUpConfig) -> Result<()> {
    if let Some(conflict) = startup_binding_conflicts(config).into_iter().next() {
        bail!("{}", conflict_message(&conflict));
    }
    Ok(())
}

pub fn startup_binding_conflicts(config: &ResolvedUpConfig) -> Vec<BindingConflict> {
    let mut held_listeners = Vec::new();
    let mut conflicts = Vec::new();

    if bind_target(&config.main.address, config.main.port)
        .map(|listener| held_listeners.push(listener))
        .is_err()
    {
        conflicts.push(BindingConflict {
            kind: BindingConflictKind::Main,
            address: config.main.address.clone(),
            port: config.main.port,
        });
    }
    for runner in &config.local_runners {
        if bind_target(&runner.address, runner.port)
            .map(|listener| held_listeners.push(listener))
            .is_err()
        {
            conflicts.push(BindingConflict {
                kind: BindingConflictKind::Runner,
                address: runner.address.clone(),
                port: runner.port,
            });
        }
    }

    conflicts
}

pub async fn spawn_detached_stack(
    config: &ResolvedUpConfig,
    http: &reqwest::Client,
) -> Result<SpawnedStack> {
    validate_startup_bindings(config)?;
    let main_binary = config.previa_paths.main_binary()?;
    let runner_binary = if config.local_runners.is_empty() {
        None
    } else {
        Some(config.previa_paths.runner_binary()?)
    };
    let mut runners = Vec::new();
    for launch in &config.local_runners {
        let log_path = config.stack_paths.runner_log(launch.port);
        let child = match spawn_detached_process(
            runner_binary.as_ref().expect("runner binary"),
            &launch.env,
            &log_path,
        ) {
            Ok(child) => child,
            Err(err) => {
                cleanup_started_children(&mut runners).await?;
                return Err(err);
            }
        };
        match wait_for_startup(
            child,
            &launch.health_url(),
            launch.env.get("RUNNER_AUTH_KEY").map(String::as_str),
            http,
        )
        .await
        {
            Ok(child) => runners.push(child),
            Err(err) => {
                cleanup_started_children(&mut runners).await?;
                return Err(err);
            }
        }
    }

    let main = match spawn_detached_process(
        &main_binary,
        &config.main_env,
        &config.stack_paths.main_log,
    ) {
        Ok(child) => child,
        Err(err) => {
            cleanup_started_children(&mut runners).await?;
            return Err(err);
        }
    };
    let main = match wait_for_startup(main, &config.main_health_url(), None, http).await {
        Ok(child) => child,
        Err(err) => {
            cleanup_started_children(&mut runners).await?;
            return Err(err);
        }
    };
    Ok(SpawnedStack { main, runners })
}

pub async fn spawn_foreground_stack(
    config: &ResolvedUpConfig,
    http: &reqwest::Client,
) -> Result<ForegroundStack> {
    validate_startup_bindings(config)?;
    let main_binary = config.previa_paths.main_binary()?;
    let runner_binary = if config.local_runners.is_empty() {
        None
    } else {
        Some(config.previa_paths.runner_binary()?)
    };
    let mut runners = Vec::new();
    let mut tasks = Vec::new();
    for launch in &config.local_runners {
        let (child, mut child_tasks) = match spawn_foreground_process(
            runner_binary.as_ref().expect("runner binary"),
            &launch.env,
            "runner",
        ) {
            Ok(value) => value,
            Err(err) => {
                cleanup_started_children(&mut runners).await?;
                return Err(err);
            }
        };
        tasks.append(&mut child_tasks);
        match wait_for_startup(
            child,
            &launch.health_url(),
            launch.env.get("RUNNER_AUTH_KEY").map(String::as_str),
            http,
        )
        .await
        {
            Ok(child) => runners.push(child),
            Err(err) => {
                cleanup_started_children(&mut runners).await?;
                return Err(err);
            }
        }
    }

    let (main, mut child_tasks) =
        match spawn_foreground_process(&main_binary, &config.main_env, "main") {
            Ok(value) => value,
            Err(err) => {
                cleanup_started_children(&mut runners).await?;
                return Err(err);
            }
        };
    tasks.append(&mut child_tasks);
    let main = match wait_for_startup(main, &config.main_health_url(), None, http).await {
        Ok(child) => child,
        Err(err) => {
            cleanup_started_children(&mut runners).await?;
            return Err(err);
        }
    };
    Ok(ForegroundStack {
        main,
        runners,
        _log_tasks: tasks,
    })
}

pub async fn monitor_foreground_stack(mut stack: ForegroundStack) -> Result<()> {
    loop {
        if let Some(status) = stack.main.try_wait()? {
            let pids = child_ids(&stack.runners);
            graceful_shutdown_pids(&pids, Duration::from_secs(3)).await?;
            bail!("previa-main exited unexpectedly with status {status}");
        }

        for runner in &mut stack.runners {
            if let Some(status) = runner.try_wait()? {
                let mut pids = child_ids(&stack.runners);
                if let Some(main_pid) = stack.main.id() {
                    pids.push(main_pid);
                }
                graceful_shutdown_pids(&pids, Duration::from_secs(3)).await?;
                bail!("previa-runner exited unexpectedly with status {status}");
            }
        }

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                let mut pids = child_ids(&stack.runners);
                if let Some(main_pid) = stack.main.id() {
                    pids.push(main_pid);
                }
                graceful_shutdown_pids(&pids, Duration::from_secs(3)).await?;
                return Ok(());
            }
            _ = sigterm() => {
                let mut pids = child_ids(&stack.runners);
                if let Some(main_pid) = stack.main.id() {
                    pids.push(main_pid);
                }
                graceful_shutdown_pids(&pids, Duration::from_secs(3)).await?;
                return Ok(());
            }
            _ = sleep(Duration::from_millis(200)) => {}
        }
    }
}

pub async fn graceful_shutdown_pids(pids: &[u32], timeout: Duration) -> Result<()> {
    for pid in pids {
        if pid_exists(*pid) {
            let _ = terminate_pid(*pid);
        }
    }

    let start = Instant::now();
    while start.elapsed() < timeout {
        if pids.iter().all(|pid| !pid_exists(*pid)) {
            return Ok(());
        }
        sleep(Duration::from_millis(100)).await;
    }

    for pid in pids {
        if pid_exists(*pid) {
            let _ = force_kill_pid(*pid);
        }
    }
    Ok(())
}

pub fn pid_exists(pid: u32) -> bool {
    pid_exists_impl(pid)
}

fn spawn_detached_process(
    binary: &std::path::Path,
    env: &BTreeMap<String, String>,
    log_path: &std::path::Path,
) -> Result<Child> {
    if !binary.exists() {
        bail!("missing binary '{}'", binary.display());
    }
    let stdout = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(log_path)
        .with_context(|| format!("failed to open '{}'", log_path.display()))?;
    let stderr = stdout
        .try_clone()
        .with_context(|| format!("failed to clone '{}'", log_path.display()))?;

    let mut command = Command::new(binary);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    command.envs(env);
    command
        .spawn()
        .with_context(|| format!("failed to spawn '{}'", binary.display()))
}

fn spawn_foreground_process(
    binary: &std::path::Path,
    env: &BTreeMap<String, String>,
    label: &str,
) -> Result<(Child, Vec<JoinHandle<()>>)> {
    if !binary.exists() {
        bail!("missing binary '{}'", binary.display());
    }
    let mut command = Command::new(binary);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.envs(env);
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn '{}'", binary.display()))?;

    let mut tasks = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let prefix = format!("[{label}]");
        tasks.push(tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                println!("{prefix} {line}");
            }
        }));
    }
    if let Some(stderr) = child.stderr.take() {
        let prefix = format!("[{label}]");
        tasks.push(tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                eprintln!("{prefix} {line}");
            }
        }));
    }
    Ok((child, tasks))
}

async fn wait_for_startup(
    mut child: Child,
    health_url: &str,
    authorization: Option<&str>,
    http: &reqwest::Client,
) -> Result<Child> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child
            .try_wait()
            .context("failed to inspect child process startup state")?
        {
            bail!("process exited during startup with status {status}");
        }
        if probe_health(http, health_url, authorization).await {
            return Ok(child);
        }
        if Instant::now() >= deadline {
            if let Some(pid) = child.id() {
                graceful_shutdown_pids(&[pid], Duration::from_secs(1)).await?;
            }
            bail!("process did not become healthy at {health_url}");
        }
        sleep(Duration::from_millis(100)).await;
    }
}

fn child_ids(children: &[Child]) -> Vec<u32> {
    children.iter().filter_map(Child::id).collect()
}

async fn cleanup_started_children(children: &mut [Child]) -> Result<()> {
    let pids = child_ids(children);
    if pids.is_empty() {
        return Ok(());
    }
    graceful_shutdown_pids(&pids, Duration::from_secs(3)).await?;
    for child in children {
        let _ = child.wait().await;
    }
    Ok(())
}

async fn sigterm() {
    #[cfg(unix)]
    {
        let mut stream = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to listen for SIGTERM");
        let _ = stream.recv().await;
    }
    #[cfg(not(unix))]
    std::future::pending::<()>().await;
}

fn bind_target(address: &str, port: u16) -> Result<TcpListener> {
    TcpListener::bind(format_bind_address(address, port)).context("bind target unavailable")
}

pub fn conflict_message(conflict: &BindingConflict) -> String {
    let bind_address = format_bind_address(&conflict.address, conflict.port);
    let role = match conflict.kind {
        BindingConflictKind::Main => "main",
        BindingConflictKind::Runner => "runner",
    };
    format!("Requested {role} bind target '{bind_address}' is already in use or unavailable")
}

fn format_bind_address(address: &str, port: u16) -> String {
    if address.contains(':') && !address.starts_with('[') {
        format!("[{address}]:{port}")
    } else {
        format!("{address}:{port}")
    }
}

#[cfg(unix)]
fn pid_exists_impl(pid: u32) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    kill(Pid::from_raw(pid as i32), None).is_ok()
}

#[cfg(windows)]
fn pid_exists_impl(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    unsafe {
        let handle = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        );
        if handle.is_null() {
            return false;
        }
        let wait = WaitForSingleObject(handle, 0);
        let _ = CloseHandle(handle);
        wait != WAIT_OBJECT_0
    }
}

#[cfg(unix)]
fn terminate_pid(pid: u32) -> Result<()> {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;

    let _ = kill(Pid::from_raw(pid as i32), Some(Signal::SIGTERM));
    Ok(())
}

#[cfg(windows)]
fn terminate_pid(pid: u32) -> Result<()> {
    let status = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string()])
        .status()
        .context("failed to run taskkill")?;
    if status.success() {
        Ok(())
    } else {
        bail!("taskkill failed for pid {pid} with status {status}")
    }
}

#[cfg(unix)]
fn force_kill_pid(pid: u32) -> Result<()> {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;

    let _ = kill(Pid::from_raw(pid as i32), Some(Signal::SIGKILL));
    Ok(())
}

#[cfg(windows)]
fn force_kill_pid(pid: u32) -> Result<()> {
    let status = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F", "/T"])
        .status()
        .context("failed to run taskkill")?;
    if status.success() {
        Ok(())
    } else {
        bail!("taskkill /F failed for pid {pid} with status {status}")
    }
}
