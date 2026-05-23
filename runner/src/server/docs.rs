use utoipa::OpenApi;

use previa_runner::{
    AssertionResult, Pipeline, PipelineStep, RuntimeSpec, StepAssertion, StepExecutionResult,
    StepRequest, StepResponse,
};

use crate::server::models::{
    E2eSummary, E2eTestRequest, ErrorResponse, ExecutionInitEvent, LoadInterpolation, LoadPoint,
    LoadProfile, LoadStartResponse, LoadTelemetryAckRequest, LoadTelemetryAckResponse,
    LoadTelemetryBucket, LoadTelemetryQuery, LoadTelemetryResponse, LoadTestConfig,
    LoadTestMetrics, LoadTestRequest, RunnerInfoResponse, StepStartEvent,
};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Test Execution API",
        version = "1.0.0",
        description = "API para execucao remota de testes end-to-end e carga. O runner ainda suporta SSE legado, mas o fluxo escalavel de carga usa start + polling de telemetria.",
        contact(
            name = "Previa Labs",
            email = "previa@previa.dev"
        )
    ),
    paths(
        crate::server::handlers::e2e::run_e2e_test,
        crate::server::handlers::load::run_load_test,
        crate::server::handlers::load::start_load_test,
        crate::server::handlers::load::get_load_telemetry,
        crate::server::handlers::load::ack_load_telemetry,
        crate::server::handlers::load::get_load_status,
        crate::server::handlers::load::cancel_load_test,
        crate::server::handlers::system::info_runtime
    ),
    components(schemas(
        E2eTestRequest,
        LoadTestRequest,
        LoadStartResponse,
        LoadTelemetryQuery,
        LoadTelemetryBucket,
        LoadTelemetryResponse,
        LoadTelemetryAckRequest,
        LoadTelemetryAckResponse,
        Pipeline,
        PipelineStep,
        StepAssertion,
        AssertionResult,
        RuntimeSpec,
        StepRequest,
        StepResponse,
        StepExecutionResult,
        LoadTestConfig,
        LoadProfile,
        LoadPoint,
        LoadInterpolation,
        ExecutionInitEvent,
        StepStartEvent,
        E2eSummary,
        LoadTestMetrics,
        RunnerInfoResponse,
        ErrorResponse
    )),
    tags(
        (name = "tests", description = "Execucao remota de testes end-to-end e carga")
    ),
    servers(
        (url = "http://localhost:3000/api/v1", description = "URL base do servidor de execucao")
    )
)]
pub struct ApiDoc;
