use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use reqwest::Client;
use serde_json::{Map, Value, json};
use toml_edit::{DocumentMut, Item, Table, value};

use crate::auth::{auth_path_for_context, auth_path_for_url, resolve_token};
use crate::browser::main_url;
use crate::cli::{
    McpAction, McpArgs, McpInstallArgs, McpPrintArgs, McpScope, McpStatusArgs, McpTarget,
    McpUninstallArgs,
};
use crate::envfile::read_env_file;
use crate::paths::PreviaPaths;
use crate::runtime::read_runtime_state;
use crate::selectors::parse_stack_name;

const DEFAULT_CONTEXT: &str = "default";
const DEFAULT_MCP_PATH: &str = "/mcp";

pub async fn run_mcp(paths: &PreviaPaths, http: &Client, args: McpArgs) -> Result<()> {
    ensure_linux()?;

    match args.action {
        McpAction::Install(args) => install(paths, http, args).await,
        McpAction::Uninstall(args) => uninstall(paths, args),
        McpAction::Status(args) => status(http, paths, args).await,
        McpAction::Print(args) => print(paths, args),
    }
}

async fn install(paths: &PreviaPaths, http: &Client, args: McpInstallArgs) -> Result<()> {
    if matches!(args.target, McpTarget::ClaudeDesktop) {
        bail!(
            "automatic install is not supported for {} in this version; use `previa mcp print claude-desktop` for manual guidance",
            target_label(args.target)
        );
    }
    if matches!(args.target, McpTarget::Warp) {
        ensure_scope(args.target, args.scope, McpScope::Global)?;
    }

    let url = resolve_target_url(paths, &args.url, args.context.as_deref())?;
    let auth = resolve_mcp_auth(
        paths,
        args.url.as_deref(),
        args.context.as_deref(),
        args.token_env.as_deref(),
    )?;
    if !args.no_verify {
        verify_mcp_endpoint(http, &url, auth.verify_header.as_deref()).await?;
    }

    match args.target {
        McpTarget::Codex => {
            let path = codex_config_path(args.scope)?;
            let outcome = install_codex(
                &path,
                &args.name,
                &url,
                auth.config_header.as_deref(),
                args.force,
            )?;
            print_install_result(args.target, &args.name, &url, &path, outcome);
        }
        McpTarget::Cursor => {
            let path = cursor_config_path(args.scope)?;
            let outcome = install_json_client(
                &path,
                &args.name,
                &url,
                auth.config_header.as_deref(),
                args.force,
            )?;
            print_install_result(args.target, &args.name, &url, &path, outcome);
        }
        McpTarget::CopilotVscode => {
            let path = copilot_vscode_config_path(args.scope)?;
            let outcome = install_json_client(
                &path,
                &args.name,
                &url,
                auth.config_header.as_deref(),
                args.force,
            )?;
            print_install_result(args.target, &args.name, &url, &path, outcome);
        }
        McpTarget::Warp => {
            let path = warp_config_path(paths, &args.name);
            let outcome = install_json_client(
                &path,
                &args.name,
                &url,
                auth.config_header.as_deref(),
                args.force,
            )?;
            print_install_result(args.target, &args.name, &url, &path, outcome);
        }
        McpTarget::ClaudeCode => {
            let scope = claude_scope(args.scope);
            let status = claude_code_status_internal(args.scope, &args.name)?;
            if let Some(configured_url) = status.url {
                if configured_url == url {
                    println!(
                        "MCP server '{}' already configured for {}",
                        args.name,
                        target_label(args.target)
                    );
                    return Ok(());
                }
                if !args.force {
                    bail!(
                        "MCP server '{}' already exists for {} with a different configuration; rerun with --force to replace it",
                        args.name,
                        target_label(args.target)
                    );
                }
            } else if status.installed && !args.force {
                bail!(
                    "MCP server '{}' already exists for {} with an unknown configuration; rerun with --force to replace it",
                    args.name,
                    target_label(args.target)
                );
            }

            let mut command = vec![
                "mcp".to_owned(),
                "add".to_owned(),
                "--scope".to_owned(),
                scope.to_owned(),
                "--transport".to_owned(),
                "http".to_owned(),
            ];
            if let Some(header) = auth.config_header.as_deref() {
                command.push("--header".to_owned());
                command.push(format!("Authorization: {header}"));
            }
            command.push(args.name.clone());
            command.push(url.clone());
            run_claude_command_vec(&command)?;
            println!(
                "installed MCP server '{}' for {}",
                args.name,
                target_label(args.target)
            );
            println!("url: {url}");
        }
        McpTarget::ClaudeDesktop => unreachable!("handled above"),
    }

    Ok(())
}

fn uninstall(paths: &PreviaPaths, args: McpUninstallArgs) -> Result<()> {
    match args.target {
        McpTarget::Codex => {
            let path = codex_config_path(args.scope)?;
            let removed = uninstall_codex(&path, &args.name)?;
            print_uninstall_result(args.target, &args.name, &path, removed);
        }
        McpTarget::Cursor => {
            let path = cursor_config_path(args.scope)?;
            let removed = uninstall_json_client(&path, &args.name)?;
            print_uninstall_result(args.target, &args.name, &path, removed);
        }
        McpTarget::CopilotVscode => {
            let path = copilot_vscode_config_path(args.scope)?;
            let removed = uninstall_json_client(&path, &args.name)?;
            print_uninstall_result(args.target, &args.name, &path, removed);
        }
        McpTarget::Warp => {
            ensure_scope(args.target, args.scope, McpScope::Global)?;
            let path = warp_config_path(paths, &args.name);
            let removed = uninstall_warp(&path)?;
            print_uninstall_result(args.target, &args.name, &path, removed);
        }
        McpTarget::ClaudeCode => {
            let scope = claude_scope(args.scope);
            let status = claude_code_status_internal(args.scope, &args.name)?;
            if !status.installed {
                println!(
                    "MCP server '{}' is not configured for {}",
                    args.name,
                    target_label(args.target)
                );
                return Ok(());
            }
            run_claude_command(["mcp", "remove", "--scope", scope, args.name.as_str()])?;
            println!(
                "removed MCP server '{}' from {}",
                args.name,
                target_label(args.target)
            );
        }
        McpTarget::ClaudeDesktop => {
            bail!(
                "automatic uninstall is not supported for {} in this version; use `previa mcp print claude-desktop` for manual guidance",
                target_label(args.target)
            );
        }
    }

    Ok(())
}

async fn status(http: &Client, paths: &PreviaPaths, args: McpStatusArgs) -> Result<()> {
    match args.target {
        McpTarget::Codex => {
            let path = codex_config_path(args.scope)?;
            let status = status_codex(&path, &args.name)?;
            print_status_report(
                http,
                args.target,
                args.scope,
                apply_status_auth_override(status, args.token_env.as_deref())?,
            )
            .await?;
        }
        McpTarget::Cursor => {
            let path = cursor_config_path(args.scope)?;
            let status = status_json_client(&path, &args.name)?;
            print_status_report(
                http,
                args.target,
                args.scope,
                apply_status_auth_override(status, args.token_env.as_deref())?,
            )
            .await?;
        }
        McpTarget::CopilotVscode => {
            let path = copilot_vscode_config_path(args.scope)?;
            let status = status_json_client(&path, &args.name)?;
            print_status_report(
                http,
                args.target,
                args.scope,
                apply_status_auth_override(status, args.token_env.as_deref())?,
            )
            .await?;
        }
        McpTarget::Warp => {
            ensure_scope(args.target, args.scope, McpScope::Global)?;
            let path = warp_config_path(paths, &args.name);
            let status = status_json_client(&path, &args.name)?;
            print_status_report(
                http,
                args.target,
                args.scope,
                apply_status_auth_override(status, args.token_env.as_deref())?,
            )
            .await?;
        }
        McpTarget::ClaudeCode => {
            let status = claude_code_status_internal(args.scope, &args.name)?;
            print_status_report(
                http,
                args.target,
                args.scope,
                apply_status_auth_override(status, args.token_env.as_deref())?,
            )
            .await?;
        }
        McpTarget::ClaudeDesktop => {
            bail!(
                "status is not supported for {} in this version; use `previa mcp print claude-desktop` for manual guidance",
                target_label(args.target)
            );
        }
    }

    Ok(())
}

fn print(paths: &PreviaPaths, args: McpPrintArgs) -> Result<()> {
    let url = resolve_target_url(paths, &args.url, args.context.as_deref())?;
    let auth = resolve_mcp_auth(
        paths,
        args.url.as_deref(),
        args.context.as_deref(),
        args.token_env.as_deref(),
    )?;
    match args.target {
        McpTarget::Codex => {
            let path = codex_config_path(args.scope)?;
            println!("config: {}", path.display());
            println!();
            println!("[mcp_servers.{}]", args.name);
            println!("enabled = true");
            println!("url = \"{url}\"");
            if let Some(header) = auth.config_header.as_deref() {
                println!("[mcp_servers.{}.headers]", args.name);
                println!("Authorization = \"{header}\"");
            }
        }
        McpTarget::Cursor => {
            let path = cursor_config_path(args.scope)?;
            println!("config: {}", path.display());
            println!();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "mcpServers": {
                        &args.name: {
                            "url": url,
                            "headers": auth.json_headers()
                        }
                    }
                }))
                .expect("cursor print json")
            );
        }
        McpTarget::CopilotVscode => {
            let path = copilot_vscode_config_path(args.scope)?;
            println!("config: {}", path.display());
            println!();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "mcpServers": {
                        &args.name: {
                            "url": url,
                            "headers": auth.json_headers()
                        }
                    }
                }))
                .expect("copilot print json")
            );
        }
        McpTarget::Warp => {
            ensure_scope(args.target, args.scope, McpScope::Global)?;
            let path = warp_config_path(paths, &args.name);
            println!("config: {}", path.display());
            println!();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "mcpServers": {
                        &args.name: {
                            "url": url,
                            "headers": auth.json_headers()
                        }
                    }
                }))
                .expect("warp print json")
            );
            println!();
            println!("run:");
            println!("oz agent run --mcp {}", path.display());
        }
        McpTarget::ClaudeCode => {
            if let Some(header) = auth.config_header.as_deref() {
                println!(
                    "claude mcp add --scope {} --transport http --header \"Authorization: {}\" {} {}",
                    claude_scope(args.scope),
                    header,
                    args.name,
                    url
                );
            } else {
                println!(
                    "claude mcp add --scope {} --transport http {} {}",
                    claude_scope(args.scope),
                    args.name,
                    url
                );
            }
        }
        McpTarget::ClaudeDesktop => {
            println!("Previa MCP URL:");
            println!("{url}");
            println!();
            println!("Claude Desktop remote HTTP MCP install is manual-only in this version.");
            println!("Use the product's connector/manual flow and point it at the URL above.");
            if let Some(header) = auth.config_header.as_deref() {
                println!("Authorization header: {header}");
            }
        }
    }

    Ok(())
}

fn resolve_target_url(
    paths: &PreviaPaths,
    explicit_url: &Option<String>,
    context: Option<&str>,
) -> Result<String> {
    if let Some(url) = explicit_url {
        return Ok(url.clone());
    }

    let context = context.unwrap_or(DEFAULT_CONTEXT);
    let stack_name = parse_stack_name(context)?;
    let stack_paths = paths.stack(&stack_name);
    let state = read_runtime_state(&stack_paths)?.ok_or_else(|| {
        anyhow!(
            "no detached runtime exists for context '{}'",
            stack_paths.name
        )
    })?;

    let mcp_path = read_env_file(&stack_paths.main_env)?
        .get("MCP_PATH")
        .cloned()
        .unwrap_or_else(|| DEFAULT_MCP_PATH.to_owned());
    let normalized_path = normalize_mcp_path(&mcp_path);
    Ok(format!(
        "{}{}",
        main_url(&state.main.address, state.main.port),
        normalized_path
    ))
}

async fn verify_mcp_endpoint(http: &Client, url: &str, authorization: Option<&str>) -> Result<()> {
    let mut request = http.request(reqwest::Method::OPTIONS, url);
    if let Some(authorization) = authorization {
        request = request.header(reqwest::header::AUTHORIZATION, authorization);
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("failed to reach MCP endpoint '{url}'"))?;

    if !response.status().is_success() {
        bail!(
            "MCP endpoint '{}' returned unexpected status {}",
            url,
            response.status()
        );
    }

    Ok(())
}

#[derive(Debug, Clone, Default)]
struct McpAuth {
    config_header: Option<String>,
    verify_header: Option<String>,
}

impl McpAuth {
    fn json_headers(&self) -> Value {
        match self.config_header.as_deref() {
            Some(header) => json!({ "Authorization": header }),
            None => json!({}),
        }
    }
}

fn resolve_mcp_auth(
    paths: &PreviaPaths,
    explicit_url: Option<&str>,
    context: Option<&str>,
    token_env: Option<&str>,
) -> Result<McpAuth> {
    if let Some(token_env) = token_env {
        let token_env = token_env.trim();
        if token_env.is_empty() {
            bail!("--token-env cannot be empty");
        }
        return Ok(McpAuth {
            config_header: Some(format!("Bearer ${{{token_env}}}")),
            verify_header: std::env::var(token_env)
                .ok()
                .map(|token| token.trim().to_owned())
                .filter(|token| !token.is_empty())
                .map(|token| format!("Bearer {token}")),
        });
    }

    let auth_path = if let Some(url) = explicit_url {
        auth_path_for_url(paths, url)
    } else {
        auth_path_for_context(paths, context.unwrap_or(DEFAULT_CONTEXT))?
    };
    let token = resolve_token(&auth_path).ok();
    Ok(McpAuth {
        config_header: token.as_ref().map(|token| format!("Bearer {token}")),
        verify_header: token.map(|token| format!("Bearer {token}")),
    })
}

fn apply_status_auth_override(
    mut report: StatusReport,
    token_env: Option<&str>,
) -> Result<StatusReport> {
    let Some(token_env) = token_env else {
        return Ok(report);
    };
    let token_env = token_env.trim();
    if token_env.is_empty() {
        bail!("--token-env cannot be empty");
    }
    report.authorization = std::env::var(token_env)
        .ok()
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
        .map(|token| format!("Bearer {token}"))
        .or_else(|| Some(format!("Bearer ${{{token_env}}}")));
    Ok(report)
}

fn ensure_linux() -> Result<()> {
    if cfg!(target_os = "linux") {
        Ok(())
    } else {
        bail!("`previa mcp` is currently supported on Linux only")
    }
}

fn ensure_scope(target: McpTarget, actual: McpScope, expected: McpScope) -> Result<()> {
    if actual == expected {
        return Ok(());
    }
    bail!(
        "target '{}' supports only --scope {}",
        target_label(target),
        scope_label(expected)
    )
}

fn target_label(target: McpTarget) -> &'static str {
    match target {
        McpTarget::Codex => "codex",
        McpTarget::Cursor => "cursor",
        McpTarget::ClaudeDesktop => "claude-desktop",
        McpTarget::ClaudeCode => "claude-code",
        McpTarget::Warp => "warp",
        McpTarget::CopilotVscode => "copilot-vscode",
    }
}

fn scope_label(scope: McpScope) -> &'static str {
    match scope {
        McpScope::Global => "global",
        McpScope::Project => "project",
    }
}

fn claude_scope(scope: McpScope) -> &'static str {
    match scope {
        McpScope::Global => "user",
        McpScope::Project => "project",
    }
}

fn codex_config_path(scope: McpScope) -> Result<PathBuf> {
    match scope {
        McpScope::Global => Ok(home_dir()?.join(".codex").join("config.toml")),
        McpScope::Project => Ok(env::current_dir()
            .context("failed to read current directory")?
            .join(".codex")
            .join("config.toml")),
    }
}

fn cursor_config_path(scope: McpScope) -> Result<PathBuf> {
    match scope {
        McpScope::Global => Ok(home_dir()?.join(".cursor").join("mcp.json")),
        McpScope::Project => Ok(env::current_dir()
            .context("failed to read current directory")?
            .join(".cursor")
            .join("mcp.json")),
    }
}

fn copilot_vscode_config_path(scope: McpScope) -> Result<PathBuf> {
    match scope {
        McpScope::Global => Ok(home_dir()?
            .join(".config")
            .join("Code")
            .join("User")
            .join("mcp.json")),
        McpScope::Project => Ok(env::current_dir()
            .context("failed to read current directory")?
            .join(".vscode")
            .join("mcp.json")),
    }
}

fn warp_config_path(paths: &PreviaPaths, name: &str) -> PathBuf {
    paths
        .home
        .join("clients")
        .join("warp")
        .join(format!("{name}.json"))
}

fn home_dir() -> Result<PathBuf> {
    env::var("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InstallOutcome {
    Installed,
    AlreadyInstalled,
}

#[derive(Debug, Clone)]
struct StatusReport {
    installed: bool,
    path: Option<PathBuf>,
    url: Option<String>,
    authorization: Option<String>,
    enabled: Option<bool>,
    mode: &'static str,
}

fn install_codex(
    path: &Path,
    name: &str,
    url: &str,
    authorization: Option<&str>,
    force: bool,
) -> Result<InstallOutcome> {
    let mut document = read_toml_document(path)?;

    let existing_url = codex_entry_url(&document, name);
    let existing_enabled = codex_entry_enabled(&document, name);
    if let Some(current_url) = existing_url {
        if current_url == url
            && existing_enabled.unwrap_or(false)
            && codex_entry_authorization(&document, name).as_deref() == authorization
        {
            return Ok(InstallOutcome::AlreadyInstalled);
        }
        if !force {
            bail!(
                "MCP server '{}' already exists in '{}' with a different configuration; rerun with --force to replace it",
                name,
                path.display()
            );
        }
    }

    if document.get("mcp_servers").is_none() {
        document["mcp_servers"] = Item::Table(Table::new());
    }

    let mut entry = Table::new();
    entry["enabled"] = value(true);
    entry["url"] = value(url);
    if let Some(authorization) = authorization {
        let mut headers = Table::new();
        headers["Authorization"] = value(authorization);
        entry["headers"] = Item::Table(headers);
    }
    document["mcp_servers"][name] = Item::Table(entry);
    write_toml_document(path, &document)?;
    Ok(InstallOutcome::Installed)
}

fn uninstall_codex(path: &Path, name: &str) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }

    let mut document = read_toml_document(path)?;
    let Some(mcp_servers) = document
        .get_mut("mcp_servers")
        .and_then(Item::as_table_like_mut)
    else {
        return Ok(false);
    };
    let removed = mcp_servers.remove(name).is_some();
    if removed {
        write_toml_document(path, &document)?;
    }
    Ok(removed)
}

fn status_codex(path: &Path, name: &str) -> Result<StatusReport> {
    if !path.exists() {
        return Ok(StatusReport {
            installed: false,
            path: Some(path.to_path_buf()),
            url: None,
            authorization: None,
            enabled: None,
            mode: "file",
        });
    }

    let document = read_toml_document(path)?;
    Ok(StatusReport {
        installed: codex_entry_exists(&document, name),
        path: Some(path.to_path_buf()),
        url: codex_entry_url(&document, name),
        authorization: codex_entry_authorization(&document, name),
        enabled: codex_entry_enabled(&document, name),
        mode: "file",
    })
}

fn install_json_client(
    path: &Path,
    name: &str,
    url: &str,
    authorization: Option<&str>,
    force: bool,
) -> Result<InstallOutcome> {
    let mut root = read_json_document(path)?;
    let servers = ensure_json_servers(&mut root)?;

    if let Some(existing) = servers.get(name) {
        let current_url = existing
            .get("url")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let current_authorization = existing
            .get("headers")
            .and_then(Value::as_object)
            .and_then(|headers| headers.get("Authorization"))
            .and_then(Value::as_str);
        if current_url.as_deref() == Some(url) && current_authorization == authorization {
            return Ok(InstallOutcome::AlreadyInstalled);
        }
        if !force {
            bail!(
                "MCP server '{}' already exists in '{}' with a different configuration; rerun with --force to replace it",
                name,
                path.display()
            );
        }
    }

    let entry = match authorization {
        Some(authorization) => json!({
            "url": url,
            "headers": {
                "Authorization": authorization
            }
        }),
        None => json!({ "url": url }),
    };
    servers.insert(name.to_owned(), entry);
    write_json_document(path, &Value::Object(root))?;
    Ok(InstallOutcome::Installed)
}

fn uninstall_json_client(path: &Path, name: &str) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }

    let mut root = read_json_document(path)?;
    let Some(servers) = root.get_mut("mcpServers").and_then(Value::as_object_mut) else {
        return Ok(false);
    };
    let removed = servers.remove(name).is_some();
    if removed {
        write_json_document(path, &Value::Object(root))?;
    }
    Ok(removed)
}

fn uninstall_warp(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    fs::remove_file(path).with_context(|| format!("failed to remove '{}'", path.display()))?;
    Ok(true)
}

fn status_json_client(path: &Path, name: &str) -> Result<StatusReport> {
    if !path.exists() {
        return Ok(StatusReport {
            installed: false,
            path: Some(path.to_path_buf()),
            url: None,
            authorization: None,
            enabled: None,
            mode: "file",
        });
    }

    let root = read_json_document(path)?;
    let url = root
        .get("mcpServers")
        .and_then(Value::as_object)
        .and_then(|servers| servers.get(name))
        .and_then(|entry| entry.get("url"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let authorization = root
        .get("mcpServers")
        .and_then(Value::as_object)
        .and_then(|servers| servers.get(name))
        .and_then(|entry| entry.get("headers"))
        .and_then(Value::as_object)
        .and_then(|headers| headers.get("Authorization"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    Ok(StatusReport {
        installed: url.is_some(),
        path: Some(path.to_path_buf()),
        url,
        authorization,
        enabled: None,
        mode: "file",
    })
}

fn claude_code_status_internal(scope: McpScope, name: &str) -> Result<StatusReport> {
    let scope = claude_scope(scope);
    let output = run_claude_command_allow_failure(["mcp", "get", "--scope", scope, name])?;
    if !output.status.success() {
        return Ok(StatusReport {
            installed: false,
            path: None,
            url: None,
            authorization: None,
            enabled: None,
            mode: "claude-cli",
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(StatusReport {
        installed: true,
        path: None,
        url: extract_first_url(&stdout),
        authorization: None,
        enabled: None,
        mode: "claude-cli",
    })
}

async fn print_status_report(
    http: &Client,
    target: McpTarget,
    scope: McpScope,
    report: StatusReport,
) -> Result<()> {
    println!("target: {}", target_label(target));
    println!("scope: {}", scope_label(scope));
    println!("mode: {}", report.mode);
    if let Some(path) = report.path {
        println!("config: {}", path.display());
    }
    println!("installed: {}", yes_no(report.installed));
    if let Some(enabled) = report.enabled {
        println!("enabled: {}", yes_no(enabled));
    }
    if report.authorization.is_some() {
        println!("authorization: configured");
    }
    if let Some(url) = report.url.as_deref() {
        println!("url: {url}");
        let live = verify_mcp_endpoint(http, url, report.authorization.as_deref())
            .await
            .is_ok();
        println!("live: {}", if live { "reachable" } else { "unreachable" });
    }
    Ok(())
}

fn print_install_result(
    target: McpTarget,
    name: &str,
    url: &str,
    path: &Path,
    outcome: InstallOutcome,
) {
    match outcome {
        InstallOutcome::Installed => {
            println!(
                "installed MCP server '{}' for {} ({})",
                name,
                target_label(target),
                path.display()
            );
            println!("url: {url}");
        }
        InstallOutcome::AlreadyInstalled => {
            println!(
                "MCP server '{}' already configured for {} ({})",
                name,
                target_label(target),
                path.display()
            );
            println!("url: {url}");
        }
    }
}

fn print_uninstall_result(target: McpTarget, name: &str, path: &Path, removed: bool) {
    if removed {
        println!(
            "removed MCP server '{}' from {} ({})",
            name,
            target_label(target),
            path.display()
        );
    } else {
        println!(
            "MCP server '{}' is not configured for {} ({})",
            name,
            target_label(target),
            path.display()
        );
    }
}

fn read_toml_document(path: &Path) -> Result<DocumentMut> {
    if !path.exists() {
        return Ok(DocumentMut::new());
    }
    let contents =
        fs::read_to_string(path).with_context(|| format!("failed to read '{}'", path.display()))?;
    contents
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse '{}'", path.display()))
}

fn write_toml_document(path: &Path, document: &DocumentMut) -> Result<()> {
    ensure_parent_dir(path)?;
    fs::write(path, document.to_string())
        .with_context(|| format!("failed to write '{}'", path.display()))
}

fn codex_entry_exists(document: &DocumentMut, name: &str) -> bool {
    document
        .get("mcp_servers")
        .is_some_and(|_| !document["mcp_servers"][name].is_none())
}

fn codex_entry_url(document: &DocumentMut, name: &str) -> Option<String> {
    document.get("mcp_servers").and_then(|_| {
        document["mcp_servers"][name]["url"]
            .as_str()
            .map(ToOwned::to_owned)
    })
}

fn codex_entry_enabled(document: &DocumentMut, name: &str) -> Option<bool> {
    document
        .get("mcp_servers")
        .and_then(|_| document["mcp_servers"][name]["enabled"].as_bool())
}

fn codex_entry_authorization(document: &DocumentMut, name: &str) -> Option<String> {
    document.get("mcp_servers").and_then(|_| {
        document["mcp_servers"][name]["headers"]["Authorization"]
            .as_str()
            .map(ToOwned::to_owned)
    })
}

fn read_json_document(path: &Path) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }

    let contents =
        fs::read_to_string(path).with_context(|| format!("failed to read '{}'", path.display()))?;
    let value = serde_json::from_str::<Value>(&contents)
        .with_context(|| format!("failed to parse '{}'", path.display()))?;
    let Value::Object(map) = value else {
        bail!("expected JSON object in '{}'", path.display());
    };
    Ok(map)
}

fn write_json_document(path: &Path, value: &Value) -> Result<()> {
    ensure_parent_dir(path)?;
    let output = serde_json::to_string_pretty(value).context("failed to encode JSON config")?;
    fs::write(path, format!("{output}\n"))
        .with_context(|| format!("failed to write '{}'", path.display()))
}

fn ensure_json_servers(root: &mut Map<String, Value>) -> Result<&mut Map<String, Value>> {
    if !root.contains_key("mcpServers") {
        root.insert("mcpServers".to_owned(), Value::Object(Map::new()));
    }
    root.get_mut("mcpServers")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow!("expected 'mcpServers' to be a JSON object"))
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create '{}'", parent.display()))?;
    }
    Ok(())
}

fn normalize_mcp_path(value: &str) -> String {
    if value.is_empty() {
        return DEFAULT_MCP_PATH.to_owned();
    }
    if value.starts_with('/') {
        value.to_owned()
    } else {
        format!("/{value}")
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn run_claude_command<const N: usize>(args: [&str; N]) -> Result<std::process::Output> {
    let output = run_claude_command_allow_failure(args)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("`claude {}` failed: {}", args.join(" "), stderr.trim());
    }
    Ok(output)
}

fn run_claude_command_vec(args: &[String]) -> Result<std::process::Output> {
    let output = Command::new("claude").args(args).output().with_context(
        || "failed to run `claude`; make sure Claude Code is installed and on PATH",
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("`claude {}` failed: {}", args.join(" "), stderr.trim());
    }
    Ok(output)
}

fn run_claude_command_allow_failure<const N: usize>(
    args: [&str; N],
) -> Result<std::process::Output> {
    Command::new("claude")
        .args(args)
        .output()
        .with_context(|| "failed to run `claude`; make sure Claude Code is installed and on PATH")
}

fn extract_first_url(input: &str) -> Option<String> {
    input
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|ch: char| matches!(ch, '"' | '\'' | ',' | ';' | ')' | '('))
        })
        .map(|token| token.strip_prefix("url=").unwrap_or(token))
        .find(|token| token.starts_with("http://") || token.starts_with("https://"))
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{install_codex, install_json_client};

    #[test]
    fn codex_install_writes_authorization_header() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.toml");

        install_codex(
            &path,
            "previa",
            "http://127.0.0.1:5588/mcp",
            Some("Bearer ${PREVIA_API_TOKEN}"),
            false,
        )
        .expect("install codex");

        let contents = std::fs::read_to_string(path).expect("read config");
        assert!(contents.contains("url = \"http://127.0.0.1:5588/mcp\""));
        assert!(contents.contains("Authorization = \"Bearer ${PREVIA_API_TOKEN}\""));
    }

    #[test]
    fn json_install_writes_authorization_header() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("mcp.json");

        install_json_client(
            &path,
            "previa",
            "http://127.0.0.1:5588/mcp",
            Some("Bearer ${PREVIA_API_TOKEN}"),
            false,
        )
        .expect("install json client");

        let contents = std::fs::read_to_string(path).expect("read config");
        let payload: serde_json::Value = serde_json::from_str(&contents).expect("json");
        assert_eq!(
            payload["mcpServers"]["previa"]["headers"]["Authorization"],
            "Bearer ${PREVIA_API_TOKEN}"
        );
    }
}
