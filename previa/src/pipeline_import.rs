use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use previa_runner::Pipeline;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::auth::apply_optional_bearer;
use crate::browser::main_url;
use crate::cli::UpArgs;

#[derive(Debug, Clone)]
pub struct PipelineImportConfig {
    pub path: PathBuf,
    pub recursive: bool,
    pub stack_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineImportOutcome {
    pub project_id: String,
    pub stack_name: String,
    pub pipelines_imported: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PipelineImportRequest {
    stack_name: String,
    pipelines: Vec<Pipeline>,
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    message: String,
}

pub fn resolve_import_config(args: &UpArgs) -> Result<Option<PipelineImportConfig>> {
    let Some(import_path) = args.import_path.as_deref() else {
        if args.recursive {
            bail!("--recursive requires --import");
        }
        if args.stack.is_some() {
            bail!("--stack requires --import");
        }
        return Ok(None);
    };

    if !args.detach {
        bail!("--import requires --detach");
    }
    if args.dry_run {
        bail!("--import cannot be combined with --dry-run");
    }

    let stack_name = args
        .stack
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("--stack is required when using --import"))?
        .to_owned();

    let path = PathBuf::from(import_path);
    let canonical = path
        .canonicalize()
        .with_context(|| format!("failed to access import path '{}'", path.display()))?;

    if args.recursive {
        if !canonical.is_dir() {
            bail!("--recursive requires --import to point to a directory");
        }
    } else if !canonical.is_file() {
        bail!("--import requires a file path unless --recursive is used");
    } else if import_file_kind(&canonical).is_none() {
        bail!(
            "unsupported pipeline file '{}'; expected .previa, .previa.json, .previa.yaml, or .previa.yml",
            canonical.display()
        );
    }

    Ok(Some(PipelineImportConfig {
        path: canonical,
        recursive: args.recursive,
        stack_name,
    }))
}

pub async fn import_pipelines(
    http: &Client,
    address: &str,
    port: u16,
    auth_path: Option<&std::path::PathBuf>,
    config: &PipelineImportConfig,
) -> Result<PipelineImportOutcome> {
    let files = if config.recursive {
        collect_pipeline_files(&config.path)?
    } else {
        vec![config.path.clone()]
    };

    if files.is_empty() {
        if config.recursive {
            bail!("no pipeline files found in '{}'", config.path.display());
        }
        bail!("pipeline file '{}' does not exist", config.path.display());
    }

    let pipelines = files
        .iter()
        .map(|path| read_pipeline_file(path))
        .collect::<Result<Vec<_>>>()?;

    let request = http.post(format!(
        "{}/api/v1/projects/import/pipelines",
        main_url(address, port)
    ));
    let request = match auth_path {
        Some(path) => apply_optional_bearer(request, path)?,
        None => request,
    };
    let response = request
        .json(&PipelineImportRequest {
            stack_name: config.stack_name.clone(),
            pipelines,
        })
        .send()
        .await
        .context("failed to call local pipeline import API")?;

    if response.status().is_success() {
        return response
            .json::<PipelineImportOutcome>()
            .await
            .context("failed to decode local pipeline import response");
    }

    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|_| String::from("<unreadable response>"));
    let message = serde_json::from_str::<ErrorResponse>(&body)
        .map(|payload| payload.message)
        .unwrap_or_else(|_| body.trim().to_owned());
    bail!(
        "failed to import pipelines into stack '{}': {} ({status})",
        config.stack_name,
        message
    );
}

fn collect_pipeline_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_pipeline_files_recursive(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_pipeline_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read '{}'", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect '{}'", path.display()))?;
        if file_type.is_dir() {
            collect_pipeline_files_recursive(&path, files)?;
            continue;
        }
        if file_type.is_file() && import_file_kind(&path).is_some() {
            files.push(path);
        }
    }
    Ok(())
}

fn read_pipeline_file(path: &Path) -> Result<Pipeline> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read pipeline file '{}'", path.display()))?;
    match import_file_kind(path) {
        Some(ImportFileKind::Json) => serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse pipeline JSON '{}'", path.display())),
        Some(ImportFileKind::Yaml) => serde_yaml::from_str(&contents)
            .with_context(|| format!("failed to parse pipeline YAML '{}'", path.display())),
        None => bail!(
            "unsupported pipeline file '{}'; expected .previa, .previa.json, .previa.yaml, or .previa.yml",
            path.display()
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportFileKind {
    Json,
    Yaml,
}

fn import_file_kind(path: &Path) -> Option<ImportFileKind> {
    let file_name = path.file_name()?.to_str()?;
    if file_name.ends_with(".previa") || file_name.ends_with(".previa.json") {
        return Some(ImportFileKind::Json);
    }
    if file_name.ends_with(".previa.yaml") || file_name.ends_with(".previa.yml") {
        return Some(ImportFileKind::Yaml);
    }
    None
}
