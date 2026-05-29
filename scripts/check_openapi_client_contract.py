#!/usr/bin/env python3
"""Check that browser API clients use endpoints present in the OpenAPI source."""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
DOCS_RS = REPO_ROOT / "main/src/server/docs.rs"
HANDLERS_DIR = REPO_ROOT / "main/src/server/handlers"


@dataclass(frozen=True, order=True)
class Endpoint:
    method: str
    path: str
    source: str


CLIENT_ENDPOINTS = [
    Endpoint("POST", "/api/v1/auth/login", "app/src/lib/auth-client.ts"),
    Endpoint("GET", "/api/v1/auth/me", "app/src/lib/auth-client.ts"),
    Endpoint("PATCH", "/api/v1/auth/me", "app/src/lib/auth-client.ts"),
    Endpoint("GET", "/api/v1/users", "app/src/lib/auth-client.ts"),
    Endpoint("POST", "/api/v1/users", "app/src/lib/auth-client.ts"),
    Endpoint("PATCH", "/api/v1/users/{userId}", "app/src/lib/auth-client.ts"),
    Endpoint("DELETE", "/api/v1/users/{userId}", "app/src/lib/auth-client.ts"),
    Endpoint("GET", "/api/v1/api-tokens", "app/src/lib/auth-client.ts"),
    Endpoint("POST", "/api/v1/api-tokens", "app/src/lib/auth-client.ts"),
    Endpoint("PATCH", "/api/v1/api-tokens/{tokenId}", "app/src/lib/auth-client.ts"),
    Endpoint("DELETE", "/api/v1/api-tokens/{tokenId}", "app/src/lib/auth-client.ts"),
    Endpoint(
        "GET",
        "/api/v1/projects/{projectId}/pipelines/{pipelineId}/runner-reservation/latest",
        "app/src/lib/api-client.ts",
    ),
    Endpoint("GET", "/api/v1/projects", "app/src/lib/api-client.ts"),
    Endpoint("POST", "/api/v1/projects", "app/src/lib/api-client.ts"),
    Endpoint("GET", "/api/v1/projects/{projectId}", "app/src/lib/api-client.ts"),
    Endpoint("PUT", "/api/v1/projects/{projectId}", "app/src/lib/api-client.ts"),
    Endpoint("DELETE", "/api/v1/projects/{projectId}", "app/src/lib/api-client.ts"),
    Endpoint("GET", "/api/v1/projects/{projectId}/shares", "app/src/lib/api-client.ts"),
    Endpoint("POST", "/api/v1/projects/{projectId}/shares", "app/src/lib/api-client.ts"),
    Endpoint("DELETE", "/api/v1/projects/{projectId}/shares/{userId}", "app/src/lib/api-client.ts"),
    Endpoint("PUT", "/api/v1/projects/{projectId}/visibility", "app/src/lib/api-client.ts"),
    Endpoint("GET", "/api/v1/runners", "app/src/lib/api-client.ts"),
    Endpoint("POST", "/api/v1/runners", "app/src/lib/api-client.ts"),
    Endpoint("PATCH", "/api/v1/runners/{runnerId}", "app/src/lib/api-client.ts"),
    Endpoint("DELETE", "/api/v1/runners/{runnerId}", "app/src/lib/api-client.ts"),
    Endpoint("GET", "/api/v1/projects/{projectId}/pipelines", "app/src/lib/api-client.ts"),
    Endpoint("POST", "/api/v1/projects/{projectId}/pipelines", "app/src/lib/api-client.ts"),
    Endpoint("GET", "/api/v1/projects/{projectId}/pipelines/{pipelineId}", "app/src/lib/api-client.ts"),
    Endpoint("PUT", "/api/v1/projects/{projectId}/pipelines/{pipelineId}", "app/src/lib/api-client.ts"),
    Endpoint("DELETE", "/api/v1/projects/{projectId}/pipelines/{pipelineId}", "app/src/lib/api-client.ts"),
    Endpoint(
        "GET",
        "/api/v1/projects/{projectId}/pipelines/{pipelineId}/shares",
        "app/src/lib/api-client.ts",
    ),
    Endpoint(
        "POST",
        "/api/v1/projects/{projectId}/pipelines/{pipelineId}/shares",
        "app/src/lib/api-client.ts",
    ),
    Endpoint(
        "DELETE",
        "/api/v1/projects/{projectId}/pipelines/{pipelineId}/shares/{userId}",
        "app/src/lib/api-client.ts",
    ),
    Endpoint(
        "PUT",
        "/api/v1/projects/{projectId}/pipelines/{pipelineId}/visibility",
        "app/src/lib/api-client.ts",
    ),
    Endpoint("GET", "/api/v1/projects/{projectId}/specs", "app/src/lib/api-client.ts"),
    Endpoint("POST", "/api/v1/projects/{projectId}/specs", "app/src/lib/api-client.ts"),
    Endpoint("GET", "/api/v1/projects/{projectId}/specs/{specId}", "app/src/lib/api-client.ts"),
    Endpoint("PUT", "/api/v1/projects/{projectId}/specs/{specId}", "app/src/lib/api-client.ts"),
    Endpoint("DELETE", "/api/v1/projects/{projectId}/specs/{specId}", "app/src/lib/api-client.ts"),
    Endpoint("GET", "/api/v1/projects/{projectId}/env-groups", "app/src/lib/api-client.ts"),
    Endpoint("POST", "/api/v1/projects/{projectId}/env-groups", "app/src/lib/api-client.ts"),
    Endpoint("PUT", "/api/v1/projects/{projectId}/env-groups/{envGroupId}", "app/src/lib/api-client.ts"),
    Endpoint("DELETE", "/api/v1/projects/{projectId}/env-groups/{envGroupId}", "app/src/lib/api-client.ts"),
    Endpoint("GET", "/api/v1/projects/{projectId}/tests/e2e", "app/src/lib/api-client.ts"),
    Endpoint("DELETE", "/api/v1/projects/{projectId}/tests/e2e", "app/src/lib/api-client.ts"),
    Endpoint("GET", "/api/v1/projects/{projectId}/tests/e2e/{test_id}", "app/src/lib/api-client.ts"),
    Endpoint("GET", "/api/v1/projects/{projectId}/tests/load", "app/src/lib/api-client.ts"),
    Endpoint("DELETE", "/api/v1/projects/{projectId}/tests/load", "app/src/lib/api-client.ts"),
    Endpoint("GET", "/api/v1/projects/{projectId}/tests/load/{test_id}", "app/src/lib/api-client.ts"),
    Endpoint("POST", "/api/v1/tests/load/capacity-preview", "app/src/lib/api-client.ts"),
    Endpoint("POST", "/api/v1/specs/validate", "app/src/lib/api-client.ts"),
    Endpoint("POST", "/api/v1/executions/{executionId}/cancel", "app/src/lib/api-client.ts"),
    Endpoint("POST", "/api/v1/projects/{projectId}/tests/e2e/queue", "app/src/lib/api-client.ts"),
    Endpoint("GET", "/api/v1/projects/{projectId}/tests/e2e/queue", "app/src/lib/api-client.ts"),
    Endpoint("GET", "/api/v1/projects/{projectId}/tests/e2e/queue/{queueId}", "app/src/lib/api-client.ts"),
    Endpoint("DELETE", "/api/v1/projects/{projectId}/tests/e2e/queue/{queueId}", "app/src/lib/api-client.ts"),
    Endpoint("GET", "/api/v1/projects/{projectId}/export", "app/src/lib/api-client.ts"),
    Endpoint("POST", "/api/v1/projects/export", "app/src/lib/api-client.ts"),
    Endpoint("POST", "/api/v1/projects/import", "app/src/lib/api-client.ts"),
]


def registered_handler_names() -> set[str]:
    source = DOCS_RS.read_text()
    return {
        f"{module}::{function}"
        for module, function in re.findall(
            r"crate::server::handlers::([a-zA-Z0-9_]+)::([a-zA-Z0-9_]+)",
            source,
        )
    }


def registered_openapi_endpoints() -> set[tuple[str, str]]:
    handlers = registered_handler_names()
    endpoints: set[tuple[str, str]] = set()
    block_re = re.compile(
        r"#\[utoipa::path\((?P<body>.*?)\)\]\s*(?:pub\s+)?async\s+fn\s+(?P<fn>[a-zA-Z0-9_]+)",
        re.DOTALL,
    )

    for path in HANDLERS_DIR.glob("*.rs"):
        module = path.stem
        source = path.read_text()
        for match in block_re.finditer(source):
            function = match.group("fn")
            if f"{module}::{function}" not in handlers:
                continue

            body = match.group("body")
            method_match = re.search(r"\b(get|post|put|patch|delete)\b\s*,", body)
            path_match = re.search(r'path\s*=\s*"([^"]+)"', body)
            if not method_match or not path_match:
                continue

            endpoints.add((method_match.group(1).upper(), path_match.group(1)))

    return endpoints


def main() -> int:
    openapi = registered_openapi_endpoints()
    missing = sorted(
        endpoint
        for endpoint in CLIENT_ENDPOINTS
        if (endpoint.method, endpoint.path) not in openapi
    )

    if missing:
        print("Client endpoints missing from generated OpenAPI source:", file=sys.stderr)
        for endpoint in missing:
            print(
                f"- {endpoint.method} {endpoint.path} ({endpoint.source})",
                file=sys.stderr,
            )
        return 1

    print(
        f"OpenAPI client contract check passed ({len(CLIENT_ENDPOINTS)} endpoints)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
