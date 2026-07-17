use utoipa::OpenApi;

use crate::server::models::{ErrorResponse, RunnerInfoResponse};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Previa Runner Operations API",
        version = "1.0.0",
        description = "Superficie operacional do runner. Execucoes e eventos trafegam exclusivamente pela fila Postgres.",
        contact(
            name = "Previa Labs",
            email = "previa@previa.dev"
        )
    ),
    paths(crate::server::handlers::system::info_runtime),
    components(schemas(RunnerInfoResponse, ErrorResponse)),
    tags(
        (name = "operations", description = "Saude e informacoes operacionais")
    ),
    servers(
        (url = "http://localhost:55880", description = "Runner")
    )
)]
pub struct ApiDoc;
