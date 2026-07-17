use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::paths::StackPaths;

pub fn read_env_file(path: &Path) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    if !path.exists() {
        return Ok(values);
    }
    for entry in dotenvy::from_path_iter(path)
        .with_context(|| format!("failed to read env file '{}'", path.display()))?
    {
        let (key, value) =
            entry.with_context(|| format!("failed to parse env file '{}'", path.display()))?;
        values.insert(key, value);
    }
    Ok(values)
}

pub fn write_env_file(path: &Path, values: &BTreeMap<String, String>) -> Result<()> {
    write_file(path, &render_env(values.clone()))
}

pub fn ensure_default_env_files(stack_paths: &StackPaths) -> Result<()> {
    stack_paths.ensure_parent_dirs()?;
    let mut main = if stack_paths.main_env.exists() {
        read_env_file(&stack_paths.main_env)?
    } else {
        default_main_env_map(stack_paths)
    };
    main.entry("PREVIA_POSTGRES_PASSWORD".to_owned())
        .or_insert_with(|| Uuid::new_v4().simple().to_string());
    main.entry("PREVIA_RUNNER_POSTGRES_PASSWORD".to_owned())
        .or_insert_with(|| Uuid::new_v4().simple().to_string());
    write_env_file(&stack_paths.main_env, &main)?;
    if !stack_paths.runner_env.exists() {
        write_file(&stack_paths.runner_env, &default_runner_env())?;
    }
    Ok(())
}

pub fn default_main_env_map(_stack_paths: &StackPaths) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    values.insert("ADDRESS".to_owned(), "0.0.0.0".to_owned());
    values.insert("PORT".to_owned(), "5588".to_owned());
    values.insert(
        "DATABASE_URL".to_owned(),
        "postgres://previa_main@127.0.0.1:5432/previa".to_owned(),
    );
    values.insert("PREVIA_APP_ENABLED".to_owned(), "true".to_owned());
    values.insert("PREVIA_QUEUE_JOB_LEASE_MS".to_owned(), "30000".to_owned());
    values.insert("PREVIA_QUEUE_JOB_MAX_ATTEMPTS".to_owned(), "3".to_owned());
    values.insert(
        "PREVIA_QUEUE_MAINTENANCE_INTERVAL_MS".to_owned(),
        "1000".to_owned(),
    );
    values.insert(
        "PREVIA_QUEUE_PROJECTION_POLL_INTERVAL_MS".to_owned(),
        "1000".to_owned(),
    );
    values.insert(
        "PREVIA_QUEUE_RETRY_BACKOFF_BASE_MS".to_owned(),
        "1000".to_owned(),
    );
    values.insert(
        "PREVIA_QUEUE_RETRY_BACKOFF_MAX_MS".to_owned(),
        "30000".to_owned(),
    );
    values.insert(
        "PREVIA_QUEUE_RUNNER_STALE_AFTER_MS".to_owned(),
        "15000".to_owned(),
    );
    values.insert("RUST_LOG".to_owned(), "info".to_owned());
    values
}

pub fn default_runner_env_map() -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    values.insert("ADDRESS".to_owned(), "127.0.0.1".to_owned());
    values.insert("PORT".to_owned(), "55880".to_owned());
    values.insert(
        "PREVIA_QUEUE_HEARTBEAT_INTERVAL_MS".to_owned(),
        "5000".to_owned(),
    );
    values.insert(
        "PREVIA_QUEUE_LEASE_RENEW_INTERVAL_MS".to_owned(),
        "10000".to_owned(),
    );
    values.insert(
        "PREVIA_QUEUE_POLL_INTERVAL_MS".to_owned(),
        "1000".to_owned(),
    );
    values.insert(
        "PREVIA_QUEUE_EVENT_FLUSH_INTERVAL_MS".to_owned(),
        "250".to_owned(),
    );
    values.insert("PREVIA_QUEUE_EVENT_BATCH_SIZE".to_owned(), "200".to_owned());
    values.insert(
        "PREVIA_QUEUE_EVENT_BUFFER_MAX".to_owned(),
        "5000".to_owned(),
    );
    values.insert("RUST_LOG".to_owned(), "info".to_owned());
    values
}

fn default_runner_env() -> String {
    render_env(default_runner_env_map())
}

fn render_env(values: BTreeMap<String, String>) -> String {
    values
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn write_file(path: &Path, contents: &str) -> Result<()> {
    let mut file = std::fs::File::create(path)
        .with_context(|| format!("failed to create '{}'", path.display()))?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("failed to write '{}'", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to secure '{}'", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ensure_default_env_files, read_env_file};
    use crate::paths::PreviaPaths;

    #[test]
    fn context_env_uses_postgres_and_persists_distinct_credentials() {
        let temp = tempfile::tempdir().expect("tempdir");
        let stack = PreviaPaths {
            home: temp.path().to_path_buf(),
            workspace_root: None,
        }
        .stack("default");

        ensure_default_env_files(&stack).expect("first defaults");
        let first = read_env_file(&stack.main_env).expect("first env");
        ensure_default_env_files(&stack).expect("second defaults");
        let second = read_env_file(&stack.main_env).expect("second env");

        assert!(first["DATABASE_URL"].starts_with("postgres://"));
        assert!(!first.contains_key("ORCHESTRATOR_DATABASE_URL"));
        assert!(!first.contains_key("RUNNER_ENDPOINTS"));
        assert_ne!(
            first["PREVIA_POSTGRES_PASSWORD"],
            first["PREVIA_RUNNER_POSTGRES_PASSWORD"]
        );
        assert_eq!(
            first["PREVIA_POSTGRES_PASSWORD"],
            second["PREVIA_POSTGRES_PASSWORD"]
        );
        assert_eq!(
            first["PREVIA_RUNNER_POSTGRES_PASSWORD"],
            second["PREVIA_RUNNER_POSTGRES_PASSWORD"]
        );
    }
}
