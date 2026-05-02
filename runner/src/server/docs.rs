use utoipa::OpenApi;

use previa_runner::{
    AssertionResult, Pipeline, PipelineStep, RuntimeSpec, StepAssertion, StepExecutionResult,
    StepRequest, StepResponse,
};

use crate::server::models::{
    E2eSummary, E2eTestRequest, ErrorResponse, ExecutionInitEvent, LoadInterpolation, LoadPoint,
    LoadProfile, LoadTestConfig, LoadTestMetrics, LoadTestRequest, RunnerInfoResponse,
    StepStartEvent,
};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Test Execution API",
        version = "1.0.0",
        description = "API para execucao remota de testes end-to-end e carga via HTTP streaming (SSE).",
        contact(
            name = "Previa Labs",
            email = "previa@previa.dev"
        )
    ),
    paths(
        crate::server::handlers::e2e::run_e2e_test,
        crate::server::handlers::load::run_load_test,
        crate::server::handlers::system::info_runtime
    ),
    components(schemas(
        E2eTestRequest,
        LoadTestRequest,
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
