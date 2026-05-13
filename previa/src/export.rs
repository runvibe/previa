use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use previa_runner::Pipeline;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use urlencoding::encode;

use crate::auth::apply_optional_bearer;
use crate::cli::{PipelineExportArgs, PipelineExportFormat};

#[derive(Debug, Clone)]
pub struct PipelineExportOutcome {
    pub project_id: String,
    pub project_name: String,
    pub output_dir: PathBuf,
    pub format: PipelineExportFormat,
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
struct ProjectRecord {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
    message: Option<String>,
}

#[derive(Debug, Clone)]
struct PlannedExport {
    path: PathBuf,
    pipeline: Pipeline,
}

impl fmt::Display for PipelineExportFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Yaml => write!(f, "yaml"),
            Self::Json => write!(f, "json"),
        }
    }
}

pub async fn export_pipelines(
    http: &Client,
    main_address: &str,
    main_port: u16,
    auth_path: Option<&std::path::PathBuf>,
    args: &PipelineExportArgs,
) -> Result<PipelineExportOutcome> {
    let api_base = format!("http://{}:{}", main_address, main_port);
    let project = resolve_project(http, &api_base, auth_path, &args.project).await?;
    let pipelines = load_project_pipelines(http, &api_base, auth_path, &project.id).await?;
    let selected = select_pipelines(&pipelines, &args.pipelines)?;
    let planned = plan_exports(&args.output_dir, &selected, args.format, args.overwrite)?;
    write_exports(&planned, args.format)?;

    Ok(PipelineExportOutcome {
        project_id: project.id,
        project_name: project.name,
        output_dir: args.output_dir.clone(),
        format: args.format,
        files: planned.into_iter().map(|item| item.path).collect(),
    })
}

async fn resolve_project(
    http: &Client,
    api_base: &str,
    auth_path: Option<&std::path::PathBuf>,
    selector: &str,
) -> Result<ProjectRecord> {
    if let Some(project) = load_project_by_id(http, api_base, auth_path, selector).await? {
        return Ok(project);
    }

    let projects = list_all_projects(http, api_base, auth_path).await?;
    let matches = projects
        .into_iter()
        .filter(|project| project.name == selector)
        .collect::<Vec<_>>();

    match matches.len() {
        0 => bail!("project '{selector}' not found"),
        1 => Ok(matches.into_iter().next().expect("single project match")),
        _ => {
            let ids = matches
                .iter()
                .map(|project| project.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "project name '{}' is ambiguous; matched project ids: {}. Retry with --project <id>",
                selector,
                ids
            );
        }
    }
}

async fn load_project_by_id(
    http: &Client,
    api_base: &str,
    auth_path: Option<&std::path::PathBuf>,
    selector: &str,
) -> Result<Option<ProjectRecord>> {
    let url = format!("{}/api/v1/projects/{}", api_base, encode(selector));
    let request = http.get(&url);
    let request = match auth_path {
        Some(path) => apply_optional_bearer(request, path)?,
        None => request,
    };
    let response = request
        .send()
        .await
        .with_context(|| format!("failed to query local project API at '{url}'"))?;

    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        let status = response.status();
        let message = decode_error_message(response).await;
        bail!("failed to query local project API: {} ({status})", message);
    }

    response
        .json::<ProjectRecord>()
        .await
        .with_context(|| format!("failed to decode local project response from '{url}'"))
        .map(Some)
}

async fn list_all_projects(
    http: &Client,
    api_base: &str,
    auth_path: Option<&std::path::PathBuf>,
) -> Result<Vec<ProjectRecord>> {
    let mut offset = 0u32;
    let limit = 500u32;
    let mut projects = Vec::new();

    loop {
        let url = format!("{}/api/v1/projects?limit={limit}&offset={offset}", api_base);
        let request = http.get(&url);
        let request = match auth_path {
            Some(path) => apply_optional_bearer(request, path)?,
            None => request,
        };
        let response = request
            .send()
            .await
            .with_context(|| format!("failed to query local projects API at '{url}'"))?;

        if !response.status().is_success() {
            let status = response.status();
            let message = decode_error_message(response).await;
            bail!("failed to query local projects API: {} ({status})", message);
        }

        let batch = response
            .json::<Vec<ProjectRecord>>()
            .await
            .with_context(|| format!("failed to decode local projects response from '{url}'"))?;
        let batch_len = batch.len();
        projects.extend(batch);
        if batch_len < limit as usize {
            break;
        }
        offset += limit;
    }

    Ok(projects)
}

async fn load_project_pipelines(
    http: &Client,
    api_base: &str,
    auth_path: Option<&std::path::PathBuf>,
    project_id: &str,
) -> Result<Vec<Pipeline>> {
    let url = format!(
        "{}/api/v1/projects/{}/pipelines",
        api_base,
        encode(project_id)
    );
    let request = http.get(&url);
    let request = match auth_path {
        Some(path) => apply_optional_bearer(request, path)?,
        None => request,
    };
    let response = request
        .send()
        .await
        .with_context(|| format!("failed to query local pipelines API at '{url}'"))?;

    if !response.status().is_success() {
        let status = response.status();
        let message = decode_error_message(response).await;
        bail!(
            "failed to query local pipelines API: {} ({status})",
            message
        );
    }

    response
        .json::<Vec<Pipeline>>()
        .await
        .with_context(|| format!("failed to decode local pipelines response from '{url}'"))
}

fn select_pipelines(pipelines: &[Pipeline], selectors: &[String]) -> Result<Vec<Pipeline>> {
    if selectors.is_empty() {
        return Ok(pipelines.to_vec());
    }

    let mut selected_indexes = BTreeSet::new();
    for selector in selectors {
        if let Some(index) = pipelines
            .iter()
            .position(|pipeline| pipeline.id.as_deref() == Some(selector.as_str()))
        {
            selected_indexes.insert(index);
            continue;
        }

        let matches = pipelines
            .iter()
            .enumerate()
            .filter(|(_, pipeline)| pipeline.name == *selector)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();

        match matches.len() {
            0 => bail!("pipeline '{}' not found", selector),
            1 => {
                selected_indexes.insert(matches[0]);
            }
            _ => {
                let ids = matches
                    .iter()
                    .map(|index| {
                        pipelines[*index]
                            .id
                            .as_deref()
                            .unwrap_or("<no-id>")
                            .to_owned()
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                bail!(
                    "pipeline name '{}' is ambiguous; matched pipeline ids: {}. Retry with --pipeline <id>",
                    selector,
                    ids
                );
            }
        }
    }

    Ok(selected_indexes
        .into_iter()
        .map(|index| pipelines[index].clone())
        .collect())
}

fn plan_exports(
    output_dir: &Path,
    pipelines: &[Pipeline],
    format: PipelineExportFormat,
    overwrite: bool,
) -> Result<Vec<PlannedExport>> {
    let mut planned = Vec::with_capacity(pipelines.len());
    let mut seen_paths = HashSet::new();

    for (index, pipeline) in pipelines.iter().enumerate() {
        let base_name = pipeline_base_name(pipeline, index + 1);
        let file_name = format!("{base_name}.{}", export_suffix(format));
        let path = output_dir.join(file_name);

        if !seen_paths.insert(path.clone()) {
            bail!(
                "multiple selected pipelines map to the same output file '{}'",
                path.display()
            );
        }

        planned.push(PlannedExport {
            path,
            pipeline: pipeline.clone(),
        });
    }

    fs::create_dir_all(output_dir).with_context(|| {
        format!(
            "failed to create output directory '{}'",
            output_dir.display()
        )
    })?;

    if !overwrite {
        if let Some(existing) = planned.iter().find(|item| item.path.exists()) {
            bail!(
                "output file '{}' already exists; retry with --overwrite to replace it",
                existing.path.display()
            );
        }
    }

    Ok(planned)
}

fn write_exports(planned: &[PlannedExport], format: PipelineExportFormat) -> Result<()> {
    let mut encoded = HashMap::new();
    for item in planned {
        let contents = serialize_pipeline(&item.pipeline, format).with_context(|| {
            format!("failed to serialize pipeline for '{}'", item.path.display())
        })?;
        encoded.insert(item.path.clone(), contents);
    }

    for item in planned {
        let contents = encoded
            .get(&item.path)
            .expect("serialized contents for planned export");
        fs::write(&item.path, contents)
            .with_context(|| format!("failed to write '{}'", item.path.display()))?;
    }

    Ok(())
}

fn serialize_pipeline(pipeline: &Pipeline, format: PipelineExportFormat) -> Result<String> {
    match format {
        PipelineExportFormat::Yaml => {
            let mut output =
                serde_yaml::to_string(pipeline).context("failed to serialize pipeline YAML")?;
            if !output.ends_with('\n') {
                output.push('\n');
            }
            Ok(output)
        }
        PipelineExportFormat::Json => {
            let mut output = serde_json::to_string_pretty(pipeline)
                .context("failed to serialize pipeline JSON")?;
            output.push('\n');
            Ok(output)
        }
    }
}

fn pipeline_base_name(pipeline: &Pipeline, position: usize) -> String {
    if let Some(id) = pipeline
        .id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        return id.to_owned();
    }

    let slug = slugify_filename(&pipeline.name);
    if slug.is_empty() {
        format!("pipeline-{position}")
    } else {
        slug
    }
}

fn slugify_filename(input: &str) -> String {
    let mut output = String::new();
    let mut last_was_separator = false;

    for ch in input.trim().chars().flat_map(|ch| ch.to_lowercase()) {
        let normalized = if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            Some(ch)
        } else {
            Some('-')
        };

        if let Some(ch) = normalized {
            if ch == '-' {
                if output.is_empty() || last_was_separator {
                    continue;
                }
                last_was_separator = true;
                output.push(ch);
            } else {
                last_was_separator = false;
                output.push(ch);
            }
        }
    }

    output.trim_matches(['-', '_']).to_owned()
}

fn export_suffix(format: PipelineExportFormat) -> &'static str {
    match format {
        PipelineExportFormat::Yaml => "previa.yaml",
        PipelineExportFormat::Json => "previa.json",
    }
}

async fn decode_error_message(response: reqwest::Response) -> String {
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if let Ok(error) = serde_json::from_str::<ApiErrorResponse>(&text) {
        if let Some(message) = error.message.filter(|message| !message.trim().is_empty()) {
            return message;
        }
    }
    if text.trim().is_empty() {
        status.to_string()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use previa_runner::Pipeline;

    use super::{pipeline_base_name, slugify_filename};

    fn pipeline(id: Option<&str>, name: &str) -> Pipeline {
        Pipeline {
            id: id.map(str::to_owned),
            name: name.to_owned(),
            description: None,
            steps: Vec::new(),
        }
    }

    #[test]
    fn pipeline_base_name_prefers_id() {
        assert_eq!(
            pipeline_base_name(&pipeline(Some("pipe-id"), "Example Name"), 1),
            "pipe-id"
        );
    }

    #[test]
    fn pipeline_base_name_falls_back_to_slugified_name() {
        assert_eq!(
            pipeline_base_name(&pipeline(None, "Example Name / Smoke"), 2),
            "example-name-smoke"
        );
    }

    #[test]
    fn pipeline_base_name_uses_position_when_slug_is_empty() {
        assert_eq!(pipeline_base_name(&pipeline(None, "!!!"), 7), "pipeline-7");
    }

    #[test]
    fn slugify_filename_collapses_separators() {
        assert_eq!(
            slugify_filename("  My   API__Smoke///Test  "),
            "my-api__smoke-test"
        );
    }
}
