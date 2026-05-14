mod auth;
mod browser;
mod cli;
mod compose;
mod config;
mod download;
mod envfile;
mod export;
mod health;
mod init;
mod local_push;
mod logs;
mod mcp_cli;
mod output;
mod paths;
mod pipeline_import;
mod process;
mod pull;
mod runner_cli;
mod runtime;
mod selectors;

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use clap::Parser;
use reqwest::Client;
use tokio::time::sleep;

use crate::auth::{run_login, run_logout, run_token, run_whoami};
use crate::browser::{build_open_url, open_browser};
use crate::cli::{
    Cli, Commands, DownArgs, ExportArgs, ExportTarget, InitArgs, LocalArgs, LocalCommands,
    LocalExportArgs, LocalImportArgs, LogsArgs, McpArgs, OpenArgs, PsArgs, PullArgs, RestartArgs,
    StatusArgs, UpArgs,
};
use crate::compose::{
    ComposeProject, MAIN_SERVICE_NAME, ServiceInspect, compose_project_from_state,
    desired_state_from_resolved, write_generated_compose,
};
use crate::config::{ResolvedUpConfig, resolve_up_config};
use crate::envfile::{read_env_file, write_env_file};
use crate::export::export_pipelines;
use crate::health::{
    DerivedState, probe_health, state_from_pid_and_health, state_from_running_and_health,
};
use crate::init::init_compose;
use crate::local_push::push_project;
use crate::logs::{follow_logs, print_logs};
use crate::mcp_cli::run_mcp;
use crate::output::{
    ListEntryJson, ProcessJson, StatusJson, StatusProcessJson, print_list_human,
    print_process_rows, print_status_human,
};
use crate::paths::{PreviaPaths, StackPaths};
use crate::pipeline_import::{import_pipelines, resolve_import_config};
use crate::process::{
    BindingConflict, BindingConflictKind, SpawnedStack, conflict_message, graceful_shutdown_pids,
    monitor_foreground_stack, pid_exists, spawn_detached_stack, spawn_foreground_stack,
    startup_binding_conflicts, validate_startup_bindings,
};
use crate::pull::pull_images;
use crate::runner_cli::run_runner_cli;
use crate::runtime::{
    DetachedRuntimeState, LocalRunnerRuntime, MainRuntime, RuntimeBackend, acquire_lock,
    read_runtime_state, remove_runtime_state, write_runtime_state,
};
use crate::selectors::{RunnerSelector, parse_stack_name};

pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    let home = effective_home(&cli);
    let paths = PreviaPaths::discover(home.as_deref())?;
    let http = Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .context("failed to build HTTP client")?;

    match cli.command {
        Commands::Login(args) => run_login(&paths, &http, args).await,
        Commands::Logout(args) => run_logout(&paths, args).await,
        Commands::Whoami(args) => run_whoami(&paths, &http, args).await,
        Commands::Token(args) => run_token(&paths, &http, args).await,
        Commands::Init(args) => cmd_init(args),
        Commands::Local(args) => cmd_local(&paths, &http, args).await,
        Commands::Up(args) => cmd_up(&paths, &http, args).await,
        Commands::Mcp(args) => cmd_mcp(&paths, &http, args).await,
        Commands::Runner(args) => run_runner_cli(&paths, &http, args).await,
        Commands::Pull(args) => cmd_pull(args).await,
        Commands::Down(args) => cmd_down(&paths, args).await,
        Commands::Restart(args) => cmd_restart(&paths, &http, args).await,
        Commands::Status(args) => cmd_status(&paths, &http, args).await,
        Commands::List(args) => cmd_list(&paths, &http, args.json).await,
        Commands::Ps(args) => cmd_ps(&paths, &http, args).await,
        Commands::Logs(args) => cmd_logs(&paths, args).await,
        Commands::Open(args) => cmd_open(&paths, args).await,
        Commands::Export(args) => cmd_export(&paths, &http, args).await,
        Commands::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

fn effective_home(cli: &Cli) -> Option<PathBuf> {
    if cli.home.is_none() && matches!(cli.command, Commands::Local(_)) {
        return Some(PathBuf::from("./.previa"));
    }

    cli.home.clone()
}

async fn cmd_local(paths: &PreviaPaths, http: &Client, args: LocalArgs) -> Result<()> {
    match args.command {
        LocalCommands::Up(args) => cmd_up(paths, http, args).await,
        LocalCommands::Push(args) => cmd_local_push(paths, http, args).await,
        LocalCommands::Import(args) => cmd_local_import(paths, args).await,
        LocalCommands::Export(args) => cmd_local_export(paths, args).await,
        LocalCommands::Runner(args) => run_runner_cli(paths, http, args).await,
        LocalCommands::Down(args) => cmd_down(paths, args).await,
        LocalCommands::Status(args) => cmd_status(paths, http, args).await,
        LocalCommands::Logs(args) => cmd_logs(paths, args).await,
        LocalCommands::Open(args) => cmd_open(paths, args).await,
    }
}

async fn cmd_local_push(
    paths: &PreviaPaths,
    _http: &Client,
    args: crate::cli::LocalPushArgs,
) -> Result<()> {
    let stack_name = parse_stack_name(&args.context)?;
    let stack_paths = paths.stack(&stack_name);
    let state = read_required_state(&stack_paths)?;
    let local_base_url = crate::browser::main_url(&state.main.address, state.main.port);
    let http = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to build HTTP client")?;
    let outcome = push_project(paths, &http, &local_base_url, &args).await?;

    if let Some(replaced) = outcome.remote_project_replaced.as_deref() {
        println!(
            "replaced remote project '{}' with '{}' ({})",
            replaced, outcome.project_name, outcome.project_id
        );
    } else {
        println!(
            "created remote project '{}' ({})",
            outcome.project_name, outcome.project_id
        );
    }
    println!(
        "pushed {} pipeline(s), {} spec(s), {} e2e history record(s), {} load history record(s)",
        outcome.pipelines_imported,
        outcome.specs_imported,
        outcome.e2e_history_imported,
        outcome.load_history_imported
    );
    Ok(())
}

async fn cmd_local_import(paths: &PreviaPaths, args: LocalImportArgs) -> Result<()> {
    let stack_name = parse_stack_name(&args.context)?;
    let stack_paths = paths.stack(&stack_name);
    let state = read_required_state(&stack_paths)?;
    let local_base_url = crate::browser::main_url(&state.main.address, state.main.port);
    let bytes = std::fs::read(&args.path)
        .with_context(|| format!("failed to read sqlite import '{}'", args.path.display()))?;
    let http = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .context("failed to build HTTP client")?;
    let include_history = !args.no_history;
    let url = format!("{local_base_url}/api/v1/projects/import?includeHistory={include_history}");
    let auth_path = crate::auth::auth_path_for_context(paths, &stack_name)?;
    let response = crate::auth::apply_optional_bearer(http.post(&url), &auth_path)?
        .header("content-type", "application/vnd.sqlite3")
        .body(bytes)
        .send()
        .await
        .with_context(|| format!("failed to import sqlite into local context at '{url}'"))?;

    if !response.status().is_success() {
        let status = response.status();
        let message = decode_api_error(response).await;
        bail!("failed to import sqlite projects: {} ({status})", message);
    }

    let payload = response
        .json::<serde_json::Value>()
        .await
        .context("failed to decode sqlite import response")?;
    let imported = payload
        .get("projectsImported")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    println!(
        "imported {imported} project(s) from '{}'",
        args.path.display()
    );
    if let Some(projects) = payload.get("projects").and_then(|value| value.as_array()) {
        for project in projects {
            let name = project
                .get("projectName")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let id = project
                .get("projectId")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            println!("{name} ({id})");
        }
    }
    Ok(())
}

async fn cmd_local_export(paths: &PreviaPaths, args: LocalExportArgs) -> Result<()> {
    if !args.all && args.projects.is_empty() {
        bail!("use --all or at least one --project <PROJECT_ID>");
    }
    if args.output.exists() && !args.overwrite {
        bail!(
            "output '{}' already exists; pass --overwrite to replace it",
            args.output.display()
        );
    }
    if let Some(parent) = args
        .output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory '{}'", parent.display()))?;
    }

    let stack_name = parse_stack_name(&args.context)?;
    let stack_paths = paths.stack(&stack_name);
    let state = read_required_state(&stack_paths)?;
    let local_base_url = crate::browser::main_url(&state.main.address, state.main.port);
    let include_history = !args.no_history;
    let body = serde_json::json!({
        "all": args.all,
        "projectIds": args.projects,
        "includeHistory": include_history,
    });
    let http = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .context("failed to build HTTP client")?;
    let url = format!("{local_base_url}/api/v1/projects/export");
    let auth_path = crate::auth::auth_path_for_context(paths, &stack_name)?;
    let response = crate::auth::apply_optional_bearer(http.post(&url), &auth_path)?
        .json(&body)
        .send()
        .await
        .with_context(|| format!("failed to export sqlite from local context at '{url}'"))?;

    if !response.status().is_success() {
        let status = response.status();
        let message = decode_api_error(response).await;
        bail!("failed to export sqlite projects: {} ({status})", message);
    }

    let bytes = response
        .bytes()
        .await
        .context("failed to read sqlite export response")?;
    std::fs::write(&args.output, &bytes)
        .with_context(|| format!("failed to write sqlite export '{}'", args.output.display()))?;
    println!(
        "exported {} byte(s) to '{}'",
        bytes.len(),
        args.output.display()
    );
    Ok(())
}

async fn decode_api_error(response: reqwest::Response) -> String {
    let body = response.text().await.unwrap_or_default();
    serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|payload| {
            payload
                .get("message")
                .and_then(|message| message.as_str())
                .map(str::to_owned)
        })
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| body.trim().to_owned())
}

fn cmd_init(args: InitArgs) -> Result<()> {
    let path = init_compose(args.force)?;
    println!("created '{}'", path.display());
    Ok(())
}

async fn cmd_mcp(paths: &PreviaPaths, http: &Client, args: McpArgs) -> Result<()> {
    run_mcp(paths, http, args).await
}

async fn cmd_pull(args: PullArgs) -> Result<()> {
    pull_images(args.target, &args.version).await
}

async fn cmd_up(paths: &PreviaPaths, http: &Client, mut args: UpArgs) -> Result<()> {
    let import_config = resolve_import_config(&args)?;
    let stack_name = parse_stack_name(&args.context)?;
    let stack_paths = paths.stack(&stack_name);
    if args.protected && args.root_password_stdin {
        args.root_password = Some(read_stdin_secret("root password")?);
    }
    let mut resolved = resolve_up_config(paths, &stack_paths, args).await?;

    if resolved.dry_run {
        validate_startup_bindings(&resolved)?;
        print_dry_run(&resolved);
        return Ok(());
    }

    resolve_port_conflicts(&mut resolved)?;

    let _lock = acquire_lock(&stack_paths)?;
    ensure_context_not_running(&stack_paths).await?;
    persist_generated_runtime_secrets(&stack_paths, &resolved)?;

    match resolved.backend {
        RuntimeBackend::Compose => {
            write_generated_compose(&resolved)?;
            let state = desired_state_from_resolved(&resolved, Utc::now().to_rfc3339());
            let compose = compose_project_from_state(&state);

            if resolved.detach {
                if stack_paths.runtime_file.exists() {
                    remove_runtime_state(&stack_paths)?;
                }

                if let Err(err) = compose.up(true, false).await {
                    let _ = compose.down().await;
                    return Err(err);
                }

                if let Err(err) = wait_for_detached_startup(&compose, &state, http).await {
                    let _ = compose.down().await;
                    return Err(err);
                }

                write_runtime_state(&stack_paths, &state)?;
                println!(
                    "context '{}' started in detached mode (main: {}:{})",
                    stack_name, state.main.address, state.main.port
                );
                if let Some(import_config) = import_config.as_ref() {
                    let auth_path = crate::auth::auth_path_for_context(paths, &stack_name)?;
                    let outcome = import_pipelines(
                        http,
                        &state.main.address,
                        state.main.port,
                        Some(&auth_path),
                        import_config,
                    )
                    .await?;
                    println!(
                        "imported {} pipeline(s) into stack '{}' ({})",
                        outcome.pipelines_imported, outcome.stack_name, outcome.project_id
                    );
                }
                Ok(())
            } else {
                let result = compose.up(false, false).await;
                if result.is_err() {
                    let _ = compose.down().await;
                }
                result
            }
        }
        RuntimeBackend::Bin => {
            if resolved.detach {
                if stack_paths.runtime_file.exists() {
                    remove_runtime_state(&stack_paths)?;
                }

                let spawned = spawn_detached_stack(&resolved, http).await?;
                let state = detached_state_from_spawn(&resolved, &spawned)?;
                write_runtime_state(&stack_paths, &state)?;
                println!(
                    "context '{}' started in detached mode (main: {}:{})",
                    stack_name, state.main.address, state.main.port
                );
                if let Some(import_config) = import_config.as_ref() {
                    let auth_path = crate::auth::auth_path_for_context(paths, &stack_name)?;
                    let outcome = import_pipelines(
                        http,
                        &state.main.address,
                        state.main.port,
                        Some(&auth_path),
                        import_config,
                    )
                    .await?;
                    println!(
                        "imported {} pipeline(s) into stack '{}' ({})",
                        outcome.pipelines_imported, outcome.stack_name, outcome.project_id
                    );
                }
                Ok(())
            } else {
                let foreground = spawn_foreground_stack(&resolved, http).await?;
                monitor_foreground_stack(foreground).await
            }
        }
    }
}

async fn cmd_down(paths: &PreviaPaths, args: DownArgs) -> Result<()> {
    if args.all_context {
        if !args.runners.is_empty() {
            bail!("--all-contexts and --runner are mutually exclusive");
        }
        return cmd_down_all_contexts(paths).await;
    }

    let stack_name = parse_stack_name(&args.context)?;
    let stack_paths = paths.stack(&stack_name);
    let selectors = parse_runner_selectors(&args.runners)?;
    let _lock = acquire_lock(&stack_paths)?;
    let state = read_required_state(&stack_paths)?;

    if selectors.is_empty() {
        stop_detached_context(&stack_paths, &state).await?;
        return Ok(());
    }

    let selected = select_runner_indexes(&state.runners, &selectors)?;
    let remaining_local = state.runners.len().saturating_sub(selected.len());
    if remaining_local == 0 && state.attached_runners.is_empty() {
        bail!(
            "cannot remove the selected runners because the context would have zero runner sources"
        );
    }

    match state.backend {
        RuntimeBackend::Compose => {
            let compose = compose_project_from_state(&state);
            let selected_services = selected
                .iter()
                .map(|idx| state.runners[*idx].service_name.clone())
                .collect::<Vec<_>>();
            compose.stop_services(&selected_services).await?;
            compose.remove_services(&selected_services).await?;

            let next_state = DetachedRuntimeState {
                runners: state
                    .runners
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, runner)| {
                        (!selected.contains(&idx)).then_some(runner.clone())
                    })
                    .collect(),
                ..state.clone()
            };
            let resolved =
                ResolvedUpConfig::from_runtime(paths, &stack_paths, &next_state, None).await?;
            write_generated_compose(&resolved)?;
            write_runtime_state(&stack_paths, &next_state)?;
        }
        RuntimeBackend::Bin => {
            let selected_pids = selected
                .iter()
                .map(|idx| state.runners[*idx].pid)
                .collect::<Vec<_>>();
            graceful_shutdown_pids(&selected_pids, Duration::from_secs(3)).await?;

            let next_state = DetachedRuntimeState {
                runners: state
                    .runners
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, runner)| {
                        (!selected.contains(&idx)).then_some(runner.clone())
                    })
                    .collect(),
                ..state.clone()
            };
            write_runtime_state(&stack_paths, &next_state)?;
        }
    }

    println!("context '{}' updated", stack_name);
    Ok(())
}

async fn cmd_restart(paths: &PreviaPaths, http: &Client, args: RestartArgs) -> Result<()> {
    let stack_name = parse_stack_name(&args.context)?;
    let stack_paths = paths.stack(&stack_name);
    let _lock = acquire_lock(&stack_paths)?;
    let state = read_required_state(&stack_paths)?;

    match state.backend {
        RuntimeBackend::Compose => {
            let resolved = ResolvedUpConfig::from_runtime(
                paths,
                &stack_paths,
                &state,
                args.version.as_deref(),
            )
            .await?;
            write_generated_compose(&resolved)?;

            let next_state = desired_state_from_resolved(&resolved, Utc::now().to_rfc3339());
            let compose = compose_project_from_state(&next_state);
            compose.down().await?;
            compose.up(true, true).await?;

            if let Err(err) = wait_for_detached_startup(&compose, &next_state, http).await {
                let _ = compose.down().await;
                return Err(err);
            }

            write_runtime_state(&stack_paths, &next_state)?;
        }
        RuntimeBackend::Bin => {
            if args.version.is_some() {
                bail!("--version is only supported for compose-backed runtimes");
            }

            let pids = all_runtime_pids(&state);
            graceful_shutdown_pids(&pids, Duration::from_secs(3)).await?;
            sleep(Duration::from_millis(200)).await;
            remove_runtime_state(&stack_paths)?;

            let resolved =
                ResolvedUpConfig::from_runtime(paths, &stack_paths, &state, None).await?;
            let spawned = spawn_detached_stack(&resolved, http).await?;
            let next_state = detached_state_from_spawn(&resolved, &spawned)?;
            write_runtime_state(&stack_paths, &next_state)?;
        }
    }

    println!("context '{}' restarted", stack_name);
    Ok(())
}

async fn cmd_status(paths: &PreviaPaths, http: &Client, args: StatusArgs) -> Result<()> {
    let stack_name = parse_stack_name(&args.context)?;
    if args.main && args.runner.is_some() {
        bail!("--main and --runner are mutually exclusive");
    }

    let stack_paths = paths.stack(&stack_name);
    let state = read_runtime_state(&stack_paths)?;
    let selector = args
        .runner
        .as_deref()
        .map(RunnerSelector::parse)
        .transpose()?;

    let status = build_status_json(
        &stack_paths,
        state.as_ref(),
        http,
        state
            .as_ref()
            .and_then(|state| state.runner_auth_key.as_deref()),
        selector.as_ref(),
        args.main,
    )
    .await?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&status).context("failed to serialize status JSON")?
        );
    } else {
        print_status_human(&status, args.main, selector.is_some());
    }
    Ok(())
}

async fn cmd_list(paths: &PreviaPaths, http: &Client, json: bool) -> Result<()> {
    let mut entries = Vec::new();
    for stack_paths in paths.stack_roots()? {
        let state = read_runtime_state(&stack_paths)?;
        let overall = overall_stack_state(state.as_ref(), http).await?;
        entries.push(ListEntryJson {
            name: stack_paths.name.clone(),
            state: overall.as_str().to_owned(),
            runtime_file: stack_paths.runtime_file.display().to_string(),
        });
    }

    entries.sort_by(|left, right| left.name.cmp(&right.name));
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&entries).context("failed to serialize list JSON")?
        );
    } else {
        print_list_human(&entries);
    }
    Ok(())
}

async fn cmd_ps(paths: &PreviaPaths, http: &Client, args: PsArgs) -> Result<()> {
    let stack_name = parse_stack_name(&args.context)?;
    let stack_paths = paths.stack(&stack_name);
    let state = read_runtime_state(&stack_paths)?;
    let rows = process_rows(state.as_ref(), http).await?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&rows).context("failed to serialize ps JSON")?
        );
    } else {
        print_process_rows(&rows);
    }
    Ok(())
}

async fn cmd_logs(paths: &PreviaPaths, args: LogsArgs) -> Result<()> {
    let stack_name = parse_stack_name(&args.context)?;
    if args.main && args.runner.is_some() {
        bail!("--main and --runner are mutually exclusive");
    }

    let stack_paths = paths.stack(&stack_name);
    let state = read_required_state(&stack_paths)?;

    match state.backend {
        RuntimeBackend::Compose => {
            let compose = compose_project_from_state(&state);
            let services = if args.main {
                vec![state.main.service_name.clone()]
            } else if let Some(selector) = args.runner.as_deref() {
                let selector = RunnerSelector::parse(selector)?;
                let indexes = select_runner_indexes(&state.runners, &[selector])?;
                indexes
                    .into_iter()
                    .map(|idx| state.runners[idx].service_name.clone())
                    .collect::<Vec<_>>()
            } else {
                let mut services = vec![state.main.service_name.clone()];
                let mut runners = state.runners.clone();
                runners.sort_by_key(|runner| runner.port);
                services.extend(runners.into_iter().map(|runner| runner.service_name));
                services
            };

            if args.follow {
                compose.logs_follow(&services, args.tail).await
            } else {
                print!("{}", compose.logs_output(&services, args.tail).await?);
                Ok(())
            }
        }
        RuntimeBackend::Bin => {
            let logs = if args.main {
                vec![("main".to_owned(), PathBuf::from(&state.main.log_path))]
            } else if let Some(selector) = args.runner.as_deref() {
                let selector = RunnerSelector::parse(selector)?;
                let indexes = select_runner_indexes(&state.runners, &[selector])?;
                indexes
                    .into_iter()
                    .map(|idx| {
                        let runner = &state.runners[idx];
                        (
                            format!("runner:{}:{}", runner.address, runner.port),
                            PathBuf::from(&runner.log_path),
                        )
                    })
                    .collect()
            } else {
                let mut files = vec![("main".to_owned(), PathBuf::from(&state.main.log_path))];
                let mut runners = state.runners.clone();
                runners.sort_by_key(|runner| runner.port);
                files.extend(runners.into_iter().map(|runner| {
                    (
                        format!("runner:{}:{}", runner.address, runner.port),
                        PathBuf::from(runner.log_path),
                    )
                }));
                files
            };

            if args.follow {
                follow_logs(logs, args.tail).await
            } else {
                print_logs(logs, args.tail).await
            }
        }
    }
}

async fn cmd_open(paths: &PreviaPaths, args: OpenArgs) -> Result<()> {
    let stack_name = parse_stack_name(&args.context)?;
    let stack_paths = paths.stack(&stack_name);
    let state = read_required_state(&stack_paths)?;
    let url = build_open_url(&state.main.address, state.main.port)?;
    match open_browser(&url) {
        Ok(()) => {
            println!("{url}");
            Ok(())
        }
        Err(error) => {
            println!("{url}");
            Err(anyhow!(
                "\x1b[31mfailed to open the browser automatically: {error:#}\x1b[0m\nopen the URL above manually"
            ))
        }
    }
}

async fn cmd_export(paths: &PreviaPaths, http: &Client, args: ExportArgs) -> Result<()> {
    match args.target {
        ExportTarget::Pipelines(args) => {
            let stack_name = parse_stack_name(&args.context)?;
            let stack_paths = paths.stack(&stack_name);
            let state = read_required_state(&stack_paths)?;
            let auth_path = crate::auth::auth_path_for_context(paths, &args.context)?;
            let outcome = export_pipelines(
                http,
                &state.main.address,
                state.main.port,
                Some(&auth_path),
                &args,
            )
            .await?;

            println!(
                "exported {} pipeline(s) from project '{}' ({}) to '{}' as {}",
                outcome.files.len(),
                outcome.project_name,
                outcome.project_id,
                outcome.output_dir.display(),
                outcome.format
            );
            for path in outcome.files {
                println!("{}", path.display());
            }
            Ok(())
        }
    }
}

fn print_dry_run(resolved: &ResolvedUpConfig) {
    println!("context: {}", resolved.stack_paths.name);
    println!(
        "backend: {}",
        if resolved.backend == RuntimeBackend::Compose {
            "compose"
        } else {
            "bin"
        }
    );
    if resolved.backend == RuntimeBackend::Compose {
        println!("image tag: {}", resolved.image_tag);
    }
    println!("main: {}:{}", resolved.main.address, resolved.main.port);
    println!(
        "local runners: {} ({:?}-{:?})",
        resolved.local_runner_count,
        resolved.runner_port_range.start,
        resolved.runner_port_range.end
    );
    println!("attached runners: {}", resolved.attached_runners.join(", "));
    if let Some(source) = &resolved.source {
        println!("source: {}", source.display());
    }
}

async fn ensure_context_not_running(stack_paths: &StackPaths) -> Result<()> {
    let Some(state) = read_runtime_state(stack_paths)? else {
        return Ok(());
    };

    match state.backend {
        RuntimeBackend::Compose => {
            let compose = compose_project_from_state(&state);
            let mut service_names = vec![state.main.service_name.clone()];
            service_names.extend(
                state
                    .runners
                    .iter()
                    .map(|runner| runner.service_name.clone()),
            );

            for service_name in &service_names {
                if compose
                    .inspect_service(service_name)
                    .await?
                    .is_some_and(|service| service.running)
                {
                    bail!("{}", running_context_message(&state));
                }
            }
        }
        RuntimeBackend::Bin => {
            if all_runtime_pids(&state).into_iter().any(pid_exists) {
                bail!("{}", running_context_message(&state));
            }
        }
    }

    Ok(())
}

fn persist_generated_runtime_secrets(
    stack_paths: &StackPaths,
    resolved: &ResolvedUpConfig,
) -> Result<()> {
    if let Some(generated_key) = resolved.generated_runner_auth_key.as_ref() {
        let mut main_env = read_env_file(&stack_paths.main_env)?;
        main_env.insert("RUNNER_AUTH_KEY".to_owned(), generated_key.clone());
        write_env_file(&stack_paths.main_env, &main_env)?;

        let mut runner_env = read_env_file(&stack_paths.runner_env)?;
        runner_env.insert("RUNNER_AUTH_KEY".to_owned(), generated_key.clone());
        write_env_file(&stack_paths.runner_env, &runner_env)?;
    }

    if resolved.auth_config_changed {
        let mut main_env = read_env_file(&stack_paths.main_env)?;
        for key in [
            "PREVIA_AUTH_ANONYMOUS",
            "PREVIA_ROOT_USERNAME",
            "PREVIA_ROOT_PASSWORD",
            "PREVIA_JWT_SECRET",
            "PREVIA_JWT_TTL_SECONDS",
        ] {
            if let Some(value) = resolved.main_env.get(key) {
                main_env.insert(key.to_owned(), value.clone());
            }
        }
        write_env_file(&stack_paths.main_env, &main_env)?;
    }

    Ok(())
}

fn read_stdin_secret(label: &str) -> Result<String> {
    let mut value = String::new();
    io::stdin()
        .read_to_string(&mut value)
        .with_context(|| format!("failed to read {label} from stdin"))?;
    let value = value.trim_end_matches(['\n', '\r']).to_owned();
    if value.trim().is_empty() {
        bail!("{label} cannot be empty");
    }
    Ok(value)
}

fn running_context_message(state: &DetachedRuntimeState) -> String {
    let mut lines = vec![
        format!("context '{}' is already running", state.name),
        format!("main: {}:{}", state.main.address, state.main.port),
    ];
    for runner in &state.runners {
        lines.push(format!("runner: {}:{}", runner.address, runner.port));
    }
    for attached in &state.attached_runners {
        lines.push(format!("attached-runner: {attached}"));
    }
    lines.join("\n")
}

async fn cmd_down_all_contexts(paths: &PreviaPaths) -> Result<()> {
    let mut stack_paths = paths.stack_roots()?;
    stack_paths.sort_by(|left, right| left.name.cmp(&right.name));

    let mut stopped = 0usize;
    for stack_path in stack_paths {
        let Some(state) = read_runtime_state(&stack_path)? else {
            continue;
        };
        let _lock = acquire_lock(&stack_path)?;
        stop_detached_context(&stack_path, &state).await?;
        stopped += 1;
    }

    println!("stopped {stopped} context(s)");
    Ok(())
}

async fn stop_detached_context(
    stack_paths: &StackPaths,
    state: &DetachedRuntimeState,
) -> Result<()> {
    match state.backend {
        RuntimeBackend::Compose => {
            let compose = compose_project_from_state(state);
            compose.down().await?;
        }
        RuntimeBackend::Bin => {
            let pids = all_runtime_pids(state);
            graceful_shutdown_pids(&pids, Duration::from_secs(3)).await?;
        }
    }
    remove_runtime_state(stack_paths)?;
    println!("context '{}' stopped", stack_paths.name);
    Ok(())
}

fn resolve_port_conflicts(resolved: &mut ResolvedUpConfig) -> Result<()> {
    loop {
        let conflicts = startup_binding_conflicts(resolved);
        if conflicts.is_empty() {
            return Ok(());
        }

        if let Some(conflict) = conflicts
            .iter()
            .find(|conflict| conflict.kind == BindingConflictKind::Main)
        {
            let next_port = conflict.port.checked_add(100).ok_or_else(|| {
                anyhow!(
                    "{}; rerun with -p <port> to choose a free main port",
                    conflict_message(conflict)
                )
            })?;
            if !prompt_for_suggested_port(
                conflict,
                next_port,
                &format!("-p {next_port}"),
                "use -p <port> to define the main port explicitly",
            )? {
                bail!(
                    "{}; rerun with -p <port> to choose a free main port",
                    conflict_message(conflict)
                );
            }
            resolved.set_main_port(next_port);
            continue;
        }

        if let Some(conflict) = conflicts
            .iter()
            .find(|conflict| conflict.kind == BindingConflictKind::Runner)
        {
            let next_start = resolved.runner_port_range.start.checked_add(100);
            let next_end = resolved.runner_port_range.end.checked_add(100);
            let (Some(next_start), Some(next_end)) = (next_start, next_end) else {
                bail!(
                    "{}; rerun with -P <start:end> to choose a free runner port range",
                    conflict_message(conflict)
                );
            };
            if !prompt_for_suggested_port(
                conflict,
                next_start,
                &format!("-P {next_start}:{next_end}"),
                "use -P <start:end> to define the runner port range explicitly",
            )? {
                bail!(
                    "{}; rerun with -P <start:end> to choose a free runner port range",
                    conflict_message(conflict)
                );
            }
            resolved.shift_runner_ports(100);
            continue;
        }
    }
}

fn prompt_for_suggested_port(
    conflict: &BindingConflict,
    suggested_port: u16,
    suggested_flag: &str,
    override_hint: &str,
) -> Result<bool> {
    let prompt = match conflict.kind {
        BindingConflictKind::Main => format!(
            "{}. You can rerun with {suggested_flag} to define the main port manually ({override_hint}), or press [Y] to continue with main port {suggested_port} [Y/n]: ",
            conflict_message(conflict)
        ),
        BindingConflictKind::Runner => format!(
            "{}. You can rerun with {suggested_flag} to define the runner ports manually ({override_hint}), or press [Y] to continue with runner ports starting at {suggested_port} [Y/n]: ",
            conflict_message(conflict)
        ),
    };

    eprint!("{prompt}");
    io::stderr().flush().context("failed to flush prompt")?;

    let mut answer = String::new();
    let bytes_read = io::stdin()
        .read_line(&mut answer)
        .context("failed to read prompt response")?;
    eprintln!();

    if bytes_read == 0 {
        return Ok(false);
    }

    let normalized = answer.trim().to_ascii_lowercase();
    Ok(normalized.is_empty() || normalized == "y" || normalized == "yes")
}

fn detached_state_from_spawn(
    resolved: &ResolvedUpConfig,
    spawned: &SpawnedStack,
) -> Result<DetachedRuntimeState> {
    Ok(DetachedRuntimeState {
        name: resolved.stack_paths.name.clone(),
        mode: "detached".to_owned(),
        started_at: Utc::now().to_rfc3339(),
        source: resolved
            .source
            .as_ref()
            .map(|path| path.display().to_string()),
        backend: RuntimeBackend::Bin,
        image_tag: String::new(),
        compose_file: String::new(),
        compose_project: String::new(),
        runner_auth_key: resolved.runner_auth_key.clone(),
        main: MainRuntime {
            service_name: String::new(),
            pid: child_id(&spawned.main)?,
            address: resolved.main.address.clone(),
            port: resolved.main.port,
            log_path: resolved.stack_paths.main_log.display().to_string(),
        },
        runner_port_range: resolved.runner_port_range,
        attached_runners: resolved.attached_runners.clone(),
        runners: resolved
            .local_runner_ports
            .iter()
            .zip(spawned.runners.iter())
            .map(|((address, port), child)| {
                Ok(LocalRunnerRuntime {
                    service_name: String::new(),
                    pid: child_id(child)?,
                    address: address.clone(),
                    port: *port,
                    log_path: resolved.stack_paths.runner_log(*port).display().to_string(),
                })
            })
            .collect::<Result<Vec<_>>>()?,
    })
}

fn child_id(child: &tokio::process::Child) -> Result<u32> {
    child
        .id()
        .ok_or_else(|| anyhow!("spawned process has no pid"))
}

async fn build_status_json(
    stack_paths: &StackPaths,
    state: Option<&DetachedRuntimeState>,
    http: &Client,
    runner_auth_key: Option<&str>,
    runner_selector: Option<&RunnerSelector>,
    main_only: bool,
) -> Result<StatusJson> {
    let runtime_file = stack_paths.runtime_file.display().to_string();
    let Some(state) = state else {
        if runner_selector.is_some() {
            bail!(
                "no detached runtime exists for context '{}'",
                stack_paths.name
            );
        }
        return Ok(StatusJson {
            name: stack_paths.name.clone(),
            state: "stopped".to_owned(),
            runtime_file,
            main: None,
            runners: Vec::new(),
            attached_runners: Vec::new(),
        });
    };

    let main = if runner_selector.is_none() {
        Some(match state.backend {
            RuntimeBackend::Compose => {
                let compose = compose_project_from_state(state);
                status_process_json_from_main_compose(&compose, &state.main, http).await?
            }
            RuntimeBackend::Bin => status_process_json_from_main_bin(&state.main, http).await?,
        })
    } else {
        None
    };

    let selected_runners = if main_only {
        Vec::new()
    } else if let Some(selector) = runner_selector {
        let indexes = select_runner_indexes(&state.runners, std::slice::from_ref(selector))?;
        indexes
            .into_iter()
            .map(|idx| state.runners[idx].clone())
            .collect::<Vec<_>>()
    } else {
        state.runners.clone()
    };

    let runners = match state.backend {
        RuntimeBackend::Compose => {
            let compose = compose_project_from_state(state);
            collect_status_runner_json_compose(&compose, &selected_runners, runner_auth_key, http)
                .await?
        }
        RuntimeBackend::Bin => {
            collect_status_runner_json_bin(&selected_runners, runner_auth_key, http).await?
        }
    };
    let runner_states = runners
        .iter()
        .map(|runner| runner.state.clone())
        .collect::<Vec<_>>();
    let state_name = derive_overall_state(
        main.as_ref().map(|main| main.state.as_str()),
        &runner_states,
        main_only,
        runner_selector.is_some(),
    );

    Ok(StatusJson {
        name: state.name.clone(),
        state: state_name.as_str().to_owned(),
        runtime_file,
        main,
        runners,
        attached_runners: state.attached_runners.clone(),
    })
}

async fn process_rows(
    state: Option<&DetachedRuntimeState>,
    http: &Client,
) -> Result<Vec<ProcessJson>> {
    let Some(state) = state else {
        return Ok(Vec::new());
    };

    match state.backend {
        RuntimeBackend::Compose => {
            let compose = compose_project_from_state(state);
            let mut rows = Vec::new();
            rows.push(process_json_from_main_compose(&compose, &state.main, http).await?);
            rows.extend(
                collect_runner_json_compose(
                    &compose,
                    &state.runners,
                    state.runner_auth_key.as_deref(),
                    http,
                )
                .await?,
            );
            Ok(rows)
        }
        RuntimeBackend::Bin => {
            let mut rows = Vec::new();
            rows.push(process_json_from_main_bin(&state.main, http).await?);
            rows.extend(
                collect_runner_json_bin(&state.runners, state.runner_auth_key.as_deref(), http)
                    .await?,
            );
            Ok(rows)
        }
    }
}

async fn process_json_from_main_compose(
    compose: &ComposeProject,
    main: &MainRuntime,
    http: &Client,
) -> Result<ProcessJson> {
    let health_url = format!("http://{}:{}/health", main.address, main.port);
    let inspect = compose.inspect_service(&main.service_name).await?;
    let (state, pid, log_path) = runtime_state_from_inspect(inspect, &health_url, None, http).await;
    Ok(ProcessJson {
        role: "main".to_owned(),
        state: state.as_str().to_owned(),
        pid,
        address: main.address.clone(),
        port: main.port,
        health_url,
        log_path,
    })
}

async fn process_json_from_main_bin(main: &MainRuntime, http: &Client) -> Result<ProcessJson> {
    let health_url = format!("http://{}:{}/health", main.address, main.port);
    let state = state_from_pid_and_health(
        if pid_exists(main.pid) { main.pid } else { 0 },
        probe_health(http, &health_url, None).await,
    );
    Ok(ProcessJson {
        role: "main".to_owned(),
        state: state.as_str().to_owned(),
        pid: if pid_exists(main.pid) { main.pid } else { 0 },
        address: main.address.clone(),
        port: main.port,
        health_url,
        log_path: main.log_path.clone(),
    })
}

async fn status_process_json_from_main_compose(
    compose: &ComposeProject,
    main: &MainRuntime,
    http: &Client,
) -> Result<StatusProcessJson> {
    let health_url = format!("http://{}:{}/health", main.address, main.port);
    let inspect = compose.inspect_service(&main.service_name).await?;
    let (state, pid, log_path) = runtime_state_from_inspect(inspect, &health_url, None, http).await;
    Ok(StatusProcessJson {
        state: state.as_str().to_owned(),
        pid,
        address: main.address.clone(),
        port: main.port,
        health_url,
        log_path,
    })
}

async fn status_process_json_from_main_bin(
    main: &MainRuntime,
    http: &Client,
) -> Result<StatusProcessJson> {
    let health_url = format!("http://{}:{}/health", main.address, main.port);
    let pid = if pid_exists(main.pid) { main.pid } else { 0 };
    let state = state_from_pid_and_health(pid, probe_health(http, &health_url, None).await);
    Ok(StatusProcessJson {
        state: state.as_str().to_owned(),
        pid,
        address: main.address.clone(),
        port: main.port,
        health_url,
        log_path: main.log_path.clone(),
    })
}

async fn collect_runner_json_compose(
    compose: &ComposeProject,
    runners: &[LocalRunnerRuntime],
    runner_auth_key: Option<&str>,
    http: &Client,
) -> Result<Vec<ProcessJson>> {
    let mut out = Vec::with_capacity(runners.len());
    for runner in runners {
        let health_url = format!("http://{}:{}/health", runner.address, runner.port);
        let inspect = compose.inspect_service(&runner.service_name).await?;
        let (state, pid, log_path) =
            runtime_state_from_inspect(inspect, &health_url, runner_auth_key, http).await;
        out.push(ProcessJson {
            role: "runner".to_owned(),
            state: state.as_str().to_owned(),
            pid,
            address: runner.address.clone(),
            port: runner.port,
            health_url,
            log_path,
        });
    }
    Ok(out)
}

async fn collect_runner_json_bin(
    runners: &[LocalRunnerRuntime],
    runner_auth_key: Option<&str>,
    http: &Client,
) -> Result<Vec<ProcessJson>> {
    let mut out = Vec::with_capacity(runners.len());
    for runner in runners {
        let health_url = format!("http://{}:{}/health", runner.address, runner.port);
        let pid = if pid_exists(runner.pid) {
            runner.pid
        } else {
            0
        };
        let state =
            state_from_pid_and_health(pid, probe_health(http, &health_url, runner_auth_key).await);
        out.push(ProcessJson {
            role: "runner".to_owned(),
            state: state.as_str().to_owned(),
            pid,
            address: runner.address.clone(),
            port: runner.port,
            health_url,
            log_path: runner.log_path.clone(),
        });
    }
    Ok(out)
}

async fn collect_status_runner_json_compose(
    compose: &ComposeProject,
    runners: &[LocalRunnerRuntime],
    runner_auth_key: Option<&str>,
    http: &Client,
) -> Result<Vec<StatusProcessJson>> {
    let mut out = Vec::with_capacity(runners.len());
    for runner in runners {
        let health_url = format!("http://{}:{}/health", runner.address, runner.port);
        let inspect = compose.inspect_service(&runner.service_name).await?;
        let (state, pid, log_path) =
            runtime_state_from_inspect(inspect, &health_url, runner_auth_key, http).await;
        out.push(StatusProcessJson {
            state: state.as_str().to_owned(),
            pid,
            address: runner.address.clone(),
            port: runner.port,
            health_url,
            log_path,
        });
    }
    Ok(out)
}

async fn collect_status_runner_json_bin(
    runners: &[LocalRunnerRuntime],
    runner_auth_key: Option<&str>,
    http: &Client,
) -> Result<Vec<StatusProcessJson>> {
    let mut out = Vec::with_capacity(runners.len());
    for runner in runners {
        let health_url = format!("http://{}:{}/health", runner.address, runner.port);
        let pid = if pid_exists(runner.pid) {
            runner.pid
        } else {
            0
        };
        let state =
            state_from_pid_and_health(pid, probe_health(http, &health_url, runner_auth_key).await);
        out.push(StatusProcessJson {
            state: state.as_str().to_owned(),
            pid,
            address: runner.address.clone(),
            port: runner.port,
            health_url,
            log_path: runner.log_path.clone(),
        });
    }
    Ok(out)
}

async fn overall_stack_state(
    state: Option<&DetachedRuntimeState>,
    http: &Client,
) -> Result<DerivedState> {
    let Some(state) = state else {
        return Ok(DerivedState::Stopped);
    };

    match state.backend {
        RuntimeBackend::Compose => {
            let compose = compose_project_from_state(state);
            let main = process_json_from_main_compose(&compose, &state.main, http).await?;
            let runners = collect_runner_json_compose(
                &compose,
                &state.runners,
                state.runner_auth_key.as_deref(),
                http,
            )
            .await?;
            let runner_states = runners
                .iter()
                .map(|runner| runner.state.clone())
                .collect::<Vec<_>>();
            Ok(derive_overall_state(
                Some(main.state.as_str()),
                &runner_states,
                false,
                false,
            ))
        }
        RuntimeBackend::Bin => {
            let main = process_json_from_main_bin(&state.main, http).await?;
            let runners =
                collect_runner_json_bin(&state.runners, state.runner_auth_key.as_deref(), http)
                    .await?;
            let runner_states = runners
                .iter()
                .map(|runner| runner.state.clone())
                .collect::<Vec<_>>();
            Ok(derive_overall_state(
                Some(main.state.as_str()),
                &runner_states,
                false,
                false,
            ))
        }
    }
}

fn derive_overall_state(
    main_state: Option<&str>,
    runner_states: &[String],
    main_only: bool,
    runner_only: bool,
) -> DerivedState {
    let mut states = Vec::new();
    if !runner_only {
        if let Some(main_state) = main_state {
            states.push(DerivedState::from_value(main_state));
        }
    }
    if !main_only {
        states.extend(
            runner_states
                .iter()
                .map(|runner_state| DerivedState::from_value(runner_state)),
        );
    }
    DerivedState::collapse(&states)
}

fn all_runtime_pids(state: &DetachedRuntimeState) -> Vec<u32> {
    let mut pids = vec![state.main.pid];
    pids.extend(state.runners.iter().map(|runner| runner.pid));
    pids.into_iter().filter(|pid| *pid > 0).collect()
}

fn select_runner_indexes(
    runners: &[LocalRunnerRuntime],
    selectors: &[RunnerSelector],
) -> Result<Vec<usize>> {
    let mut matches = Vec::new();
    for selector in selectors {
        let mut found = false;
        for (idx, runner) in runners.iter().enumerate() {
            if selector.matches(&runner.address, runner.port) && !matches.contains(&idx) {
                matches.push(idx);
                found = true;
            }
        }
        if !found {
            bail!(
                "runner selector '{}' did not match any local runner",
                selector.raw()
            );
        }
    }
    Ok(matches)
}

fn parse_runner_selectors(values: &[String]) -> Result<Vec<RunnerSelector>> {
    values
        .iter()
        .map(|value| RunnerSelector::parse(value))
        .collect()
}

fn read_required_state(stack_paths: &StackPaths) -> Result<DetachedRuntimeState> {
    read_runtime_state(stack_paths)?.ok_or_else(|| {
        anyhow!(
            "no detached runtime exists for context '{}'",
            stack_paths.name
        )
    })
}

async fn runtime_state_from_inspect(
    inspect: Option<ServiceInspect>,
    health_url: &str,
    authorization: Option<&str>,
    http: &Client,
) -> (DerivedState, u32, String) {
    let Some(inspect) = inspect else {
        return (DerivedState::Stopped, 0, String::new());
    };
    let healthy = if inspect.running {
        probe_health(http, health_url, authorization).await
    } else {
        false
    };
    (
        state_from_running_and_health(inspect.running, healthy),
        inspect.pid,
        inspect.log_path,
    )
}

async fn wait_for_detached_startup(
    compose: &ComposeProject,
    state: &DetachedRuntimeState,
    http: &Client,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let main = compose.inspect_service(MAIN_SERVICE_NAME).await?;
        if main.as_ref().is_some_and(|service| !service.running) {
            bail!("service '{MAIN_SERVICE_NAME}' exited during startup");
        }

        let main_healthy = main.is_some()
            && probe_health(
                http,
                &format!("http://{}:{}/health", state.main.address, state.main.port),
                None,
            )
            .await;

        let mut runners_healthy = true;
        for runner in &state.runners {
            let inspect = compose.inspect_service(&runner.service_name).await?;
            if inspect.as_ref().is_some_and(|service| !service.running) {
                bail!("service '{}' exited during startup", runner.service_name);
            }
            if inspect.is_none()
                || !probe_health(
                    http,
                    &format!("http://{}:{}/health", runner.address, runner.port),
                    state.runner_auth_key.as_deref(),
                )
                .await
            {
                runners_healthy = false;
            }
        }

        if main_healthy && runners_healthy {
            return Ok(());
        }

        if Instant::now() >= deadline {
            bail!("services did not become healthy before the startup timeout");
        }

        sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::effective_home;
    use crate::cli::{Cli, Commands, LocalArgs, LocalCommands, StatusArgs};
    use crate::selectors::{RunnerSelector, normalize_attach_runner};

    #[test]
    fn selector_matching_by_port_address_and_host() {
        let port = RunnerSelector::parse("55880").expect("port selector");
        assert!(port.matches("127.0.0.1", 55880));
        assert!(!port.matches("127.0.0.1", 55881));

        let addr_port = RunnerSelector::parse("10.0.0.8:55880").expect("addr:port");
        assert!(addr_port.matches("10.0.0.8", 55880));
        assert!(!addr_port.matches("10.0.0.8", 55881));

        let addr = RunnerSelector::parse("10.0.0.8").expect("addr");
        assert!(addr.matches("10.0.0.8", 55880));
        assert!(addr.matches("10.0.0.8", 55881));
    }

    #[test]
    fn attach_runner_normalization() {
        assert_eq!(
            normalize_attach_runner("55880").expect("normalize port"),
            "http://127.0.0.1:55880"
        );
        assert_eq!(
            normalize_attach_runner("10.0.0.8").expect("normalize host"),
            "http://10.0.0.8:55880"
        );
        assert_eq!(
            normalize_attach_runner("10.0.0.8:56000").expect("normalize host:port"),
            "http://10.0.0.8:56000"
        );
    }

    #[test]
    fn local_command_uses_project_local_home_when_home_is_omitted() {
        let cli = Cli {
            home: None,
            command: Commands::Local(LocalArgs {
                command: LocalCommands::Status(StatusArgs {
                    context: "default".to_owned(),
                    main: false,
                    runner: None,
                    json: false,
                }),
            }),
        };

        assert_eq!(
            effective_home(&cli).as_deref(),
            Some(Path::new("./.previa"))
        );
    }

    #[test]
    fn local_command_preserves_explicit_home() {
        let cli = Cli {
            home: Some("./custom".into()),
            command: Commands::Local(LocalArgs {
                command: LocalCommands::Status(StatusArgs {
                    context: "default".to_owned(),
                    main: false,
                    runner: None,
                    json: false,
                }),
            }),
        };

        assert_eq!(effective_home(&cli).as_deref(), Some(Path::new("./custom")));
    }
}
