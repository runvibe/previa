use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

use crate::paths::{StackPaths, sqlite_database_url};

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
    if !stack_paths.main_env.exists() {
        write_file(&stack_paths.main_env, &default_main_env(stack_paths))?;
    }
    if !stack_paths.runner_env.exists() {
        write_file(&stack_paths.runner_env, &default_runner_env())?;
    }
    Ok(())
}

pub fn default_main_env_map(stack_paths: &StackPaths) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    values.insert("ADDRESS".to_owned(), "0.0.0.0".to_owned());
    values.insert("PORT".to_owned(), "5588".to_owned());
    values.insert(
        "ORCHESTRATOR_DATABASE_URL".to_owned(),
        sqlite_database_url(&stack_paths.orchestrator_db),
    );
    values.insert("PREVIA_APP_ENABLED".to_owned(), "true".to_owned());
    values.insert(
        "RUNNER_ENDPOINTS".to_owned(),
        "http://127.0.0.1:55880".to_owned(),
    );
    values.insert("RUST_LOG".to_owned(), "info".to_owned());
    values
}

pub fn default_runner_env_map() -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    values.insert("ADDRESS".to_owned(), "127.0.0.1".to_owned());
    values.insert("PORT".to_owned(), "55880".to_owned());
    values.insert("RUST_LOG".to_owned(), "info".to_owned());
    values
}

fn default_main_env(stack_paths: &StackPaths) -> String {
    let values = default_main_env_map(stack_paths);
    render_env(values)
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
    Ok(())
}
