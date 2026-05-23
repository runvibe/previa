use utoipa::OpenApi;

use crate::server::auth::permissions::Role;
use crate::server::models::{
    ApiTokenCreateRequest, ApiTokenCreateResponse, ApiTokenRecord, ApiTokenUpdateRequest,
    AuthClientKind, AuthLoginRequest, AuthLoginResponse, AuthPrincipalSource, AuthTokenKind,
    AuthUserResponse, CancelExecutionResponse, ConsolidatedLoadMetrics, E2eHistoryRecord,
    E2eQueuePipelineRecord, E2eQueueRecord, E2eQueueStatus, EnvGroupEntry, ErrorResponse,
    HistoryOrder, HistoryQuery, KubernetesReservationCreateRequest, KubernetesReservationRunner,
    KubernetesReservationStatus, LoadCapacityPreviewRequest, LoadCapacityPreviewResponse,
    LoadExecutionStartResponse, LoadHistoryRecord, LoadInterpolation, LoadPoint, LoadProfile,
    LoadTestConfig, OpenApiValidationPoint, OpenApiValidationRequest, OpenApiValidationResponse,
    OpenApiValidationSeverity, OpenApiValidationStatus, OrchestratorInfoResponse,
    OrchestratorSseEventData, PipelineExecutionKind, PipelineExecutionRef, PipelineImportRequest,
    PipelineImportResponse, PipelineInput, PipelineQueueRef, PipelineRuntimeState,
    PipelineRuntimeStatus, ProjectE2eQueueRequest, ProjectE2eRerunFromStepRequest,
    ProjectE2eTestRequest, ProjectEnvGroupRecord, ProjectEnvGroupUpsertRequest,
    ProjectExportEnvelope, ProjectExportProject, ProjectHistoryExport, ProjectImportResponse,
    ProjectListQuery, ProjectLoadTestRequest, ProjectMetadataUpsertRequest, ProjectPipelineRecord,
    ProjectRecord, ProjectSpecRecord, ProjectSpecUpsertRequest, ProjectSqliteExportRequest,
    ProjectTransferQuery, ProjectUpsertRequest, ProxyRequest, RunnerInfo, RunnerLoadLine,
    RunnerRecord, RunnerReservationRecord, RunnerRuntimeInfo, RunnerUpdateRequest,
    RunnerUpsertRequest, SpecUrlEntry, UserCreateRequest, UserRecord, UserUpdateRequest,
};

const API_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Previa Orchestrator API",
        version = "1.0.0",
        description = "API orquestradora para distribuicao de carga entre runners. A desconexão do cliente SSE não interrompe a execução no orchestrator; use a rota de cancelamento manual para interromper."
    ),
    paths(
        crate::server::handlers::health::health,
        crate::server::handlers::auth::login,
        crate::server::handlers::auth::me,
        crate::server::handlers::users::list_users,
        crate::server::handlers::users::create_user,
        crate::server::handlers::users::update_user,
        crate::server::handlers::users::delete_user,
        crate::server::handlers::api_tokens::list_api_tokens,
        crate::server::handlers::api_tokens::create_api_token,
        crate::server::handlers::api_tokens::update_api_token,
        crate::server::handlers::api_tokens::delete_api_token,
        crate::server::handlers::health::get_info,
        crate::server::handlers::proxy::proxy_request,
        crate::server::handlers::projects::list_projects,
        crate::server::handlers::projects::get_project,
        crate::server::handlers::transfers::export_project,
        crate::server::handlers::transfers::export_projects_sqlite,
        crate::server::handlers::transfers::import_project,
        crate::server::handlers::transfers::import_pipelines,
        crate::server::handlers::runners::list_runners,
        crate::server::handlers::runners::create_runner,
        crate::server::handlers::runners::get_runner,
        crate::server::handlers::runners::update_runner,
        crate::server::handlers::runners::delete_runner,
        crate::server::handlers::specs::validate_openapi_spec,
        crate::server::handlers::specs::list_project_specs,
        crate::server::handlers::specs::create_project_spec,
        crate::server::handlers::specs::get_project_spec,
        crate::server::handlers::env_groups::list_project_env_groups,
        crate::server::handlers::env_groups::create_project_env_group,
        crate::server::handlers::env_groups::get_project_env_group,
        crate::server::handlers::env_groups::upsert_project_env_group,
        crate::server::handlers::env_groups::delete_project_env_group,
        crate::server::handlers::pipelines::list_project_pipelines,
        crate::server::handlers::pipelines::get_project_pipeline,
        crate::server::handlers::projects::create_project,
        crate::server::handlers::specs::upsert_project_spec,
        crate::server::handlers::pipelines::create_project_pipeline,
        crate::server::handlers::projects::upsert_project,
        crate::server::handlers::pipelines::upsert_project_pipeline,
        crate::server::handlers::runner_reservations::get_latest_runner_reservation_for_pipeline,
        crate::server::handlers::specs::delete_project_spec,
        crate::server::handlers::pipelines::delete_project_pipeline,
        crate::server::handlers::projects::delete_project,
        crate::server::handlers::history_e2e::get_e2e_test_by_id,
        crate::server::handlers::history_e2e::delete_e2e_test_by_id,
        crate::server::handlers::history_load::get_load_test_by_id,
        crate::server::handlers::history_load::delete_load_test_by_id,
        crate::server::handlers::tests_e2e::run_e2e_test_for_project,
        crate::server::handlers::tests_e2e::run_e2e_rerun_from_step_for_project,
        crate::server::handlers::tests_e2e_queue::get_current_e2e_queue_for_project,
        crate::server::handlers::tests_e2e_queue::create_e2e_queue_for_project,
        crate::server::handlers::tests_e2e_queue::get_e2e_queue_for_project,
        crate::server::handlers::tests_e2e_queue::delete_e2e_queue_for_project,
        crate::server::handlers::tests_load::preview_load_capacity,
        crate::server::handlers::tests_load::run_load_test_for_project,
        crate::server::handlers::executions::stream_execution_events,
        crate::server::handlers::executions::stream_execution,
        crate::server::handlers::executions::cancel_execution,
        crate::server::handlers::history_e2e::list_e2e_history,
        crate::server::handlers::history_e2e::delete_e2e_history,
        crate::server::handlers::history_load::list_load_history,
        crate::server::handlers::history_load::delete_load_history
    ),
    components(schemas(
        ProjectE2eTestRequest,
        ProjectE2eRerunFromStepRequest,
        ProjectE2eQueueRequest,
        ProjectLoadTestRequest,
        previa_runner::RuntimeSpec,
        previa_runner::RuntimeEnvGroup,
        LoadTestConfig,
        LoadCapacityPreviewRequest,
        LoadCapacityPreviewResponse,
        LoadExecutionStartResponse,
        KubernetesReservationCreateRequest,
        KubernetesReservationStatus,
        KubernetesReservationRunner,
        LoadProfile,
        LoadPoint,
        LoadInterpolation,
        HistoryQuery,
        ProjectListQuery,
        ProjectTransferQuery,
        ProjectUpsertRequest,
        ProjectMetadataUpsertRequest,
        ProjectHistoryExport,
        ProjectExportProject,
        ProjectExportEnvelope,
        ProjectSqliteExportRequest,
        ProjectImportResponse,
        PipelineImportRequest,
        PipelineImportResponse,
        ProxyRequest,
        OpenApiValidationRequest,
        OpenApiValidationSeverity,
        OpenApiValidationStatus,
        OpenApiValidationPoint,
        OpenApiValidationResponse,
        ProjectRecord,
        ProjectPipelineRecord,
        PipelineInput,
        PipelineRuntimeState,
        PipelineRuntimeStatus,
        PipelineExecutionKind,
        PipelineExecutionRef,
        PipelineQueueRef,
        SpecUrlEntry,
        EnvGroupEntry,
        ProjectEnvGroupUpsertRequest,
        ProjectEnvGroupRecord,
        ProjectSpecUpsertRequest,
        ProjectSpecRecord,
        HistoryOrder,
        ErrorResponse,
        CancelExecutionResponse,
        E2eHistoryRecord,
        E2eQueueStatus,
        E2eQueuePipelineRecord,
        E2eQueueRecord,
        LoadHistoryRecord,
        RunnerReservationRecord,
        RunnerRuntimeInfo,
        RunnerInfo,
        RunnerRecord,
        RunnerUpsertRequest,
        RunnerUpdateRequest,
        OrchestratorInfoResponse,
        OrchestratorSseEventData,
        RunnerLoadLine,
        ConsolidatedLoadMetrics,
        Role,
        AuthClientKind,
        AuthLoginRequest,
        AuthLoginResponse,
        AuthTokenKind,
        AuthPrincipalSource,
        AuthUserResponse,
        UserRecord,
        UserCreateRequest,
        UserUpdateRequest,
        ApiTokenRecord,
        ApiTokenCreateRequest,
        ApiTokenUpdateRequest,
        ApiTokenCreateResponse
    )),
    servers(
        (url = "http://localhost:5588", description = "Orchestrator local")
    )
)]
pub struct ApiDoc;

pub fn build_openapi_document() -> utoipa::openapi::OpenApi {
    let mut openapi = ApiDoc::openapi();
    openapi.info.title = env!("CARGO_PKG_NAME").to_owned();
    openapi.info.version = API_VERSION.to_owned();
    let package_description = env!("CARGO_PKG_DESCRIPTION").trim();
    let package_authors = env!("CARGO_PKG_AUTHORS")
        .split(':')
        .map(str::trim)
        .filter(|author| !author.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    let mut description_parts = Vec::new();
    if !package_description.is_empty() {
        description_parts.push(package_description.to_owned());
    }
    if !package_authors.is_empty() {
        description_parts.push(format!("Authors: {}", package_authors));
    }
    openapi.info.description = if description_parts.is_empty() {
        None
    } else {
        Some(description_parts.join("\n\n"))
    };
    openapi
}

#[cfg(test)]
mod tests {
    use super::build_openapi_document;

    #[test]
    fn openapi_info_version_matches_cargo_package_version() {
        let document = build_openapi_document();

        assert_eq!(document.info.version, env!("CARGO_PKG_VERSION"));
    }
}
