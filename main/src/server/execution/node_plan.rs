use std::time::Duration;

use reqwest::Client;

use crate::server::execution::runner_auth::apply_runner_auth;
use crate::server::models::{NodePlan, RunnerInfo, RunnerRuntimeInfo};

pub async fn collect_runner_statuses(
    client: &Client,
    runner_endpoints: &[String],
    runner_auth_key: Option<&str>,
) -> Vec<RunnerInfo> {
    let mut runners = Vec::with_capacity(runner_endpoints.len());

    for endpoint in runner_endpoints {
        let (runtime, runtime_error) =
            fetch_runner_runtime_info(client, endpoint, runner_auth_key).await;
        runners.push(RunnerInfo {
            endpoint: endpoint.clone(),
            active: is_runner_healthy(client, endpoint, runner_auth_key).await,
            runtime,
            runtime_error,
        });
    }

    runners
}

pub async fn fetch_runner_runtime_info(
    client: &Client,
    endpoint: &str,
    runner_auth_key: Option<&str>,
) -> (Option<RunnerRuntimeInfo>, Option<String>) {
    let url = format!("{}/info", endpoint.trim_end_matches('/'));
    let request = apply_runner_auth(client.get(url), runner_auth_key);
    match tokio::time::timeout(Duration::from_secs(2), request.send()).await {
        Ok(Ok(response)) => {
            if !response.status().is_success() {
                let status = response.status().as_u16();
                let body = response.text().await.unwrap_or_default();
                return (
                    None,
                    Some(format!("runner /info returned HTTP {}: {}", status, body)),
                );
            }

            match response.json::<RunnerRuntimeInfo>().await {
                Ok(runtime) => (Some(runtime), None),
                Err(err) => (None, Some(format!("invalid /info payload: {}", err))),
            }
        }
        Ok(Err(err)) => (None, Some(format!("runner /info request failed: {}", err))),
        Err(_) => (None, Some("runner /info request timeout".to_owned())),
    }
}

pub async fn is_runner_healthy(
    client: &Client,
    endpoint: &str,
    runner_auth_key: Option<&str>,
) -> bool {
    let url = format!("{}/health", endpoint.trim_end_matches('/'));
    let request = apply_runner_auth(client.get(url), runner_auth_key);

    match tokio::time::timeout(Duration::from_secs(2), request.send()).await {
        Ok(Ok(response)) => response.status().is_success(),
        _ => false,
    }
}

pub fn calculate_node_plan(
    requested_concurrency: u64,
    rps_per_node: u64,
    nodes_found: usize,
    total_requests: usize,
    concurrency: usize,
) -> NodePlan {
    let capacity_required_nodes = requested_concurrency.div_ceil(rps_per_node).max(1) as usize;

    let mut nodes_used = nodes_found;
    nodes_used = nodes_used.min(total_requests.max(1));
    nodes_used = nodes_used.min(concurrency.max(1));

    if nodes_used == 0 && nodes_found > 0 {
        nodes_used = 1;
    }

    let requested_nodes = capacity_required_nodes.max(nodes_used);

    let warning = if capacity_required_nodes > nodes_found {
        Some(format!(
            "Requested concurrency {} needs {} nodes at {} req/s capacity per node, but only {} active nodes were found. Distributing across available nodes.",
            requested_concurrency, capacity_required_nodes, rps_per_node, nodes_found
        ))
    } else {
        None
    };

    NodePlan {
        requested_nodes,
        nodes_found,
        nodes_used,
        warning,
    }
}

pub fn split_even(total: usize, parts: usize) -> Vec<usize> {
    if parts == 0 {
        return Vec::new();
    }
    let base = total / parts;
    let rem = total % parts;

    (0..parts)
        .map(|i| if i < rem { base + 1 } else { base })
        .collect()
}

pub fn parse_runner_endpoints() -> Vec<String> {
    std::env::var("RUNNER_ENDPOINTS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.trim_end_matches('/').to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use axum::Json;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::get;
    use axum::{Router, response::IntoResponse};
    use reqwest::Client;
    use tokio::net::TcpListener;

    use crate::server::execution::node_plan::{
        calculate_node_plan, fetch_runner_runtime_info, is_runner_healthy, split_even,
    };
    use crate::server::models::RunnerRuntimeInfo;

    #[test]
    fn warns_when_not_enough_nodes_for_requested_rps() {
        let plan = calculate_node_plan(10_000, 1_000, 2, 100_000, 100);
        assert_eq!(plan.requested_nodes, 10);
        assert_eq!(plan.nodes_found, 2);
        assert_eq!(plan.nodes_used, 2);
        assert!(plan.warning.is_some());
    }

    #[test]
    fn does_not_warn_when_capacity_is_enough() {
        let plan = calculate_node_plan(2_000, 1_000, 3, 100_000, 100);
        assert_eq!(plan.requested_nodes, 3);
        assert_eq!(plan.nodes_used, 3);
        assert!(plan.warning.is_none());
    }

    #[test]
    fn uses_available_nodes_for_load_distribution() {
        let plan = calculate_node_plan(10, 1_000, 3, 100, 10);
        assert_eq!(plan.requested_nodes, 3);
        assert_eq!(plan.nodes_used, 3);
        assert!(plan.warning.is_none());
    }

    #[test]
    fn splits_evenly() {
        assert_eq!(split_even(10, 3), vec![4, 3, 3]);
    }

    #[tokio::test]
    async fn runner_health_and_info_include_authorization_when_configured() {
        let endpoint = spawn_runner_probe_server(Some("secret")).await;
        let client = Client::new();

        assert!(is_runner_healthy(&client, &endpoint, Some("secret")).await);
        let (runtime, error) = fetch_runner_runtime_info(&client, &endpoint, Some("secret")).await;
        assert!(error.is_none());
        assert_eq!(runtime.expect("runtime").pid, 42);
    }

    #[tokio::test]
    async fn protected_runner_appears_unhealthy_without_matching_authorization() {
        let endpoint = spawn_runner_probe_server(Some("secret")).await;
        let client = Client::new();

        assert!(!is_runner_healthy(&client, &endpoint, None).await);
        let (runtime, error) = fetch_runner_runtime_info(&client, &endpoint, None).await;
        assert!(runtime.is_none());
        assert!(
            error
                .as_deref()
                .is_some_and(|message| message.contains("HTTP 401"))
        );
    }

    async fn spawn_runner_probe_server(expected_auth: Option<&str>) -> String {
        let app = Router::new()
            .route("/health", get(health))
            .route("/info", get(info))
            .with_state(expected_auth.map(str::to_owned));

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let address = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve runner probes");
        });

        format!("http://{}", address)
    }

    async fn health(
        State(expected_auth): State<Option<String>>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        if authorization_ok(&headers, expected_auth.as_deref()) {
            StatusCode::OK
        } else {
            StatusCode::UNAUTHORIZED
        }
    }

    async fn info(
        State(expected_auth): State<Option<String>>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        if !authorization_ok(&headers, expected_auth.as_deref()) {
            return StatusCode::UNAUTHORIZED.into_response();
        }

        Json(RunnerRuntimeInfo {
            pid: 42,
            memory_bytes: 1024,
            virtual_memory_bytes: 2048,
            cpu_usage_percent: 1.5,
            network_tx_bytes: 0,
            network_rx_bytes: 0,
            network_total_bytes: 0,
        })
        .into_response()
    }

    fn authorization_ok(headers: &HeaderMap, expected_auth: Option<&str>) -> bool {
        match expected_auth {
            Some(expected) => headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value == expected),
            None => true,
        }
    }
}
