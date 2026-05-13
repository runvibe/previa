use std::hash::{Hash, Hasher};
use std::io::{self, Read};
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use reqwest::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::browser::main_url;
use crate::cli::{
    AuthContextArgs, LoginArgs, TokenArgs, TokenCommands, TokenCreateArgs, TokenListArgs,
    TokenRevokeArgs, TokenUseArgs,
};
use crate::paths::{PreviaPaths, StackPaths};
use crate::runtime::read_runtime_state;
use crate::selectors::parse_stack_name;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredAuth {
    pub base_url: String,
    pub username: Option<String>,
    pub token_kind: String,
    pub token: String,
    pub token_id: Option<String>,
    pub token_prefix: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginResponse {
    token_kind: String,
    token: String,
    record: Option<ApiTokenRecord>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiTokenRecord {
    id: String,
    name: Option<String>,
    token_prefix: String,
    role: Option<String>,
    active: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiTokenCreateResponse {
    token: String,
    record: ApiTokenRecord,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginRequest<'a> {
    username: &'a str,
    password: &'a str,
    client_kind: &'a str,
    token_name: &'a str,
}

pub async fn run_login(paths: &PreviaPaths, http: &Client, args: LoginArgs) -> Result<()> {
    let target = resolve_auth_target(paths, args.url.as_deref(), &args.context)?;
    let password = read_password(args.password_stdin)?;
    let response = http
        .post(format!("{}/api/v1/auth/login", target.base_url))
        .json(&LoginRequest {
            username: &args.username,
            password: &password,
            client_kind: "api_token",
            token_name: "previa-cli",
        })
        .send()
        .await
        .context("failed to call login API")?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        bail!("login failed: HTTP {status}: {text}");
    }
    let payload = response
        .json::<LoginResponse>()
        .await
        .context("failed to decode login response")?;
    if payload.token_kind != "api_token" {
        bail!("login did not return an API token");
    }
    let stored = StoredAuth {
        base_url: target.base_url,
        username: Some(args.username),
        token_kind: "api_token".to_owned(),
        token: payload.token,
        token_id: payload.record.as_ref().map(|record| record.id.clone()),
        token_prefix: payload.record.map(|record| record.token_prefix),
    };
    write_auth(&target.path, &stored)?;
    println!(
        "stored API token for {}",
        stored.username.as_deref().unwrap_or("user")
    );
    Ok(())
}

pub async fn run_logout(paths: &PreviaPaths, args: AuthContextArgs) -> Result<()> {
    let target = resolve_auth_target(paths, args.url.as_deref(), &args.context)?;
    if target.path.exists() {
        std::fs::remove_file(&target.path)
            .with_context(|| format!("failed to remove '{}'", target.path.display()))?;
    }
    println!("removed stored API token");
    Ok(())
}

pub async fn run_whoami(paths: &PreviaPaths, http: &Client, args: AuthContextArgs) -> Result<()> {
    let target = resolve_auth_target(paths, args.url.as_deref(), &args.context)?;
    let token = resolve_token(&target.path)?;
    let response = http
        .get(format!("{}/api/v1/auth/me", target.base_url))
        .bearer_auth(token)
        .send()
        .await
        .context("failed to call auth API")?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        bail!("whoami failed: HTTP {status}: {text}");
    }
    let payload = response.json::<Value>().await.context("decode whoami")?;
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(())
}

pub async fn run_token(paths: &PreviaPaths, http: &Client, args: TokenArgs) -> Result<()> {
    match args.command {
        TokenCommands::List(args) => token_list(paths, http, args).await,
        TokenCommands::Create(args) => token_create(paths, http, args).await,
        TokenCommands::Revoke(args) => token_revoke(paths, http, args).await,
        TokenCommands::Use(args) => token_use(paths, args).await,
    }
}

async fn token_list(paths: &PreviaPaths, http: &Client, args: TokenListArgs) -> Result<()> {
    let target = resolve_auth_target(paths, args.url.as_deref(), &args.context)?;
    let token = resolve_token(&target.path)?;
    let response = http
        .get(format!("{}/api/v1/api-tokens", target.base_url))
        .bearer_auth(token)
        .send()
        .await
        .context("failed to call token API")?;
    let response = ensure_success(response, "list tokens").await?;
    let records = response
        .json::<Vec<ApiTokenRecord>>()
        .await
        .context("failed to decode token list")?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&records)?);
    } else if records.is_empty() {
        println!("no API tokens");
    } else {
        for record in records {
            println!(
                "{}\t{}\t{}\t{}",
                record.id,
                record.name.unwrap_or_default(),
                record.role.unwrap_or_default(),
                if record.active.unwrap_or(false) {
                    "active"
                } else {
                    "inactive"
                }
            );
        }
    }
    Ok(())
}

async fn token_create(paths: &PreviaPaths, http: &Client, args: TokenCreateArgs) -> Result<()> {
    let target = resolve_auth_target(paths, args.url.as_deref(), &args.context)?;
    let token = resolve_token(&target.path)?;
    let response = http
        .post(format!("{}/api/v1/api-tokens", target.base_url))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "name": args.name,
            "role": args.role,
        }))
        .send()
        .await
        .context("failed to call token API")?;
    let response = ensure_success(response, "create token").await?;
    let payload = response
        .json::<ApiTokenCreateResponse>()
        .await
        .context("failed to decode token create response")?;
    println!("{}", payload.token);
    println!(
        "created {} ({})",
        payload.record.token_prefix, payload.record.id
    );
    Ok(())
}

async fn token_revoke(paths: &PreviaPaths, http: &Client, args: TokenRevokeArgs) -> Result<()> {
    let target = resolve_auth_target(paths, args.url.as_deref(), &args.context)?;
    let token = resolve_token(&target.path)?;
    let response = http
        .delete(format!(
            "{}/api/v1/api-tokens/{}",
            target.base_url, args.token_id
        ))
        .bearer_auth(token)
        .send()
        .await
        .context("failed to call token API")?;
    ensure_success(response, "revoke token").await?;
    println!("revoked token '{}'", args.token_id);
    Ok(())
}

async fn token_use(paths: &PreviaPaths, args: TokenUseArgs) -> Result<()> {
    let token =
        std::env::var(&args.token_env).with_context(|| format!("{} is not set", args.token_env))?;
    let target = resolve_auth_target(paths, args.url.as_deref(), &args.context)?;
    let prefix = token
        .split_once('.')
        .map(|(prefix, _)| prefix.to_owned())
        .unwrap_or_default();
    write_auth(
        &target.path,
        &StoredAuth {
            base_url: target.base_url,
            username: None,
            token_kind: "api_token".to_owned(),
            token,
            token_id: None,
            token_prefix: Some(prefix),
        },
    )?;
    println!("stored API token from {}", args.token_env);
    Ok(())
}

pub fn resolve_token(auth_path: &PathBuf) -> Result<String> {
    if let Ok(token) = std::env::var("PREVIA_API_TOKEN") {
        if !token.trim().is_empty() {
            return Ok(token);
        }
    }
    let stored = read_auth(auth_path)?;
    Ok(stored.token)
}

pub fn apply_optional_bearer(
    request: RequestBuilder,
    auth_path: &PathBuf,
) -> Result<RequestBuilder> {
    Ok(match try_resolve_token(auth_path)? {
        Some(token) => request.bearer_auth(token),
        None => request,
    })
}

pub fn auth_path_for_context(paths: &PreviaPaths, context: &str) -> Result<PathBuf> {
    let stack_name = parse_stack_name(context)?;
    Ok(paths.stack(&stack_name).config_dir.join("auth.json"))
}

pub fn auth_path_for_url(paths: &PreviaPaths, url: &str) -> PathBuf {
    paths
        .home
        .join("auth")
        .join(format!("{}.json", stable_hash(url.trim_end_matches('/'))))
}

fn try_resolve_token(auth_path: &PathBuf) -> Result<Option<String>> {
    if let Ok(token) = std::env::var("PREVIA_API_TOKEN") {
        let token = token.trim().to_owned();
        if !token.is_empty() {
            return Ok(Some(token));
        }
    }
    if !auth_path.exists() {
        return Ok(None);
    }
    Ok(Some(read_auth(auth_path)?.token))
}

fn read_password(password_stdin: bool) -> Result<String> {
    if !password_stdin {
        bail!("use --password-stdin to provide the password");
    }
    let mut password = String::new();
    io::stdin()
        .read_to_string(&mut password)
        .context("failed to read password from stdin")?;
    Ok(password.trim_end_matches(['\n', '\r']).to_owned())
}

async fn ensure_success(response: reqwest::Response, action: &str) -> Result<reqwest::Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    bail!("{action} failed: HTTP {status}: {text}");
}

fn read_auth(path: &PathBuf) -> Result<StoredAuth> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read '{}'", path.display()))?;
    serde_json::from_slice(&bytes).context("failed to parse stored auth")
}

fn write_auth(path: &PathBuf, auth: &StoredAuth) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create '{}'", parent.display()))?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(auth)?)
        .with_context(|| format!("failed to write '{}'", path.display()))?;
    Ok(())
}

struct AuthTarget {
    base_url: String,
    path: PathBuf,
}

fn resolve_auth_target(
    paths: &PreviaPaths,
    url: Option<&str>,
    context: &str,
) -> Result<AuthTarget> {
    if let Some(url) = url {
        let normalized = url.trim_end_matches('/').to_owned();
        let hash = stable_hash(&normalized);
        return Ok(AuthTarget {
            base_url: normalized,
            path: paths.home.join("auth").join(format!("{hash}.json")),
        });
    }
    let stack_name = parse_stack_name(context)?;
    let stack_paths = paths.stack(&stack_name);
    Ok(AuthTarget {
        base_url: context_base_url(&stack_paths)?,
        path: stack_paths.config_dir.join("auth.json"),
    })
}

fn stable_hash(value: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn context_base_url(stack_paths: &StackPaths) -> Result<String> {
    let state = read_runtime_state(stack_paths)?.ok_or_else(|| {
        anyhow!(
            "no detached runtime exists for context '{}'",
            stack_paths.name
        )
    })?;
    Ok(main_url(&state.main.address, state.main.port))
}
