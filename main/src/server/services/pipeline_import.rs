use std::collections::HashSet;

use crate::server::db::DbPool;
use previa_runner::Pipeline;

use crate::server::db::{
    create_project_with_pipelines, load_existing_pipeline_ids, project_name_exists,
};
use crate::server::models::PipelineImportResponse;
use crate::server::validation::pipelines::validate_pipeline_templates;

#[derive(Debug)]
pub enum PipelineImportError {
    EmptyStackName,
    EmptyPipelines,
    EmptyPipelineName(usize),
    DuplicatePipelineId(String),
    ExistingPipelineId(String),
    ProjectExists(String),
    Validation(String),
    Database(sqlx::Error),
}

impl From<sqlx::Error> for PipelineImportError {
    fn from(value: sqlx::Error) -> Self {
        Self::Database(value)
    }
}

pub async fn import_pipelines_as_project(
    db: &DbPool,
    stack_name: String,
    pipelines: Vec<Pipeline>,
) -> Result<PipelineImportResponse, PipelineImportError> {
    let stack_name = stack_name.trim().to_owned();
    if stack_name.is_empty() {
        return Err(PipelineImportError::EmptyStackName);
    }
    if pipelines.is_empty() {
        return Err(PipelineImportError::EmptyPipelines);
    }

    if project_name_exists(db, &stack_name).await? {
        return Err(PipelineImportError::ProjectExists(stack_name));
    }

    let mut normalized = Vec::with_capacity(pipelines.len());
    let mut seen_ids = HashSet::new();
    for (index, mut pipeline) in pipelines.into_iter().enumerate() {
        pipeline.name = pipeline.name.trim().to_owned();
        if pipeline.name.is_empty() {
            return Err(PipelineImportError::EmptyPipelineName(index + 1));
        }

        if let Some(id) = pipeline.id.as_mut() {
            *id = id.trim().to_owned();
            if id.is_empty() {
                pipeline.id = None;
            }
        }

        if let Some(id) = pipeline.id.clone() {
            if !seen_ids.insert(id.clone()) {
                return Err(PipelineImportError::DuplicatePipelineId(id));
            }
        }

        let validation_errors = validate_pipeline_templates(&pipeline, None, None, None);
        if !validation_errors.is_empty() {
            return Err(PipelineImportError::Validation(format!(
                "pipeline '{}': {}",
                pipeline.name,
                validation_errors.join("; ")
            )));
        }

        normalized.push(pipeline);
    }

    let pipeline_ids = normalized
        .iter()
        .filter_map(|pipeline| pipeline.id.clone())
        .collect::<Vec<_>>();
    let existing_ids = load_existing_pipeline_ids(db, &pipeline_ids).await?;
    if let Some(existing_id) = existing_ids.into_iter().next() {
        return Err(PipelineImportError::ExistingPipelineId(existing_id));
    }

    let pipelines_imported = normalized.len();
    let project = create_project_with_pipelines(db, stack_name.clone(), normalized).await?;
    Ok(PipelineImportResponse {
        project_id: project.id,
        stack_name,
        pipelines_imported,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use previa_runner::Pipeline;

    use super::{PipelineImportError, import_pipelines_as_project};
    use crate::server::db::{create_project_with_pipelines, load_pipelines_for_project};

    fn sample_pipeline(name: &str, pipeline_id: Option<&str>) -> Pipeline {
        Pipeline {
            id: pipeline_id.map(str::to_owned),
            name: name.to_owned(),
            description: Some(format!("Pipeline {name}")),
            steps: vec![previa_runner::PipelineStep {
                id: "step-1".to_owned(),
                name: "Request".to_owned(),
                description: None,
                method: "GET".to_owned(),
                url: "https://example.com".to_owned(),
                headers: HashMap::new(),
                body: None,
                operation_id: None,
                delay: None,
                retry: None,
                asserts: Vec::new(),
            }],
        }
    }

    async fn db() -> crate::server::db::DbPool {
        let db = crate::server::db::DbPool::connect_test_sqlite("sqlite::memory:", 1)
            .await
            .expect("sqlite memory db");
        sqlx::migrate!("./migrations/sqlite")
            .run(db.pool())
            .await
            .expect("migrations");
        db
    }

    #[tokio::test]
    async fn imports_pipelines_into_new_project_and_preserves_ids() {
        let db = db().await;

        let response = import_pipelines_as_project(
            &db,
            "my-stack".to_owned(),
            vec![
                sample_pipeline("alpha", Some("pipe-alpha")),
                sample_pipeline("beta", None),
            ],
        )
        .await
        .expect("import pipelines");

        assert_eq!(response.stack_name, "my-stack");
        assert_eq!(response.pipelines_imported, 2);

        let stored = load_pipelines_for_project(&db, &response.project_id)
            .await
            .expect("load pipelines");
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].id.as_deref(), Some("pipe-alpha"));
        assert_eq!(stored[0].name, "alpha");
        assert_eq!(stored[1].name, "beta");
        assert!(stored[1].id.as_deref().is_some_and(|id| !id.is_empty()));
    }

    #[tokio::test]
    async fn rejects_existing_project_name() {
        let db = db().await;
        create_project_with_pipelines(
            &db,
            "shared-stack".to_owned(),
            vec![sample_pipeline("one", None)],
        )
        .await
        .expect("seed project");

        let error = import_pipelines_as_project(
            &db,
            "shared-stack".to_owned(),
            vec![sample_pipeline("two", None)],
        )
        .await
        .expect_err("project name conflict");

        assert!(matches!(
            error,
            PipelineImportError::ProjectExists(name) if name == "shared-stack"
        ));
    }

    #[tokio::test]
    async fn rejects_duplicate_and_existing_pipeline_ids() {
        let db = db().await;

        let duplicate_error = import_pipelines_as_project(
            &db,
            "dup-stack".to_owned(),
            vec![
                sample_pipeline("one", Some("pipe-1")),
                sample_pipeline("two", Some("pipe-1")),
            ],
        )
        .await
        .expect_err("duplicate pipeline id");
        assert!(matches!(
            duplicate_error,
            PipelineImportError::DuplicatePipelineId(id) if id == "pipe-1"
        ));

        create_project_with_pipelines(
            &db,
            "existing-project".to_owned(),
            vec![sample_pipeline("seed", Some("pipe-existing"))],
        )
        .await
        .expect("seed existing pipeline");

        let existing_error = import_pipelines_as_project(
            &db,
            "new-stack".to_owned(),
            vec![sample_pipeline("three", Some("pipe-existing"))],
        )
        .await
        .expect_err("existing pipeline id");
        assert!(matches!(
            existing_error,
            PipelineImportError::ExistingPipelineId(id) if id == "pipe-existing"
        ));
    }
}
