# Env Groups Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add project-level environment groups that can be selected at execution time and resolved in pipelines with `{{envs.current.<name>}}` or explicitly with `{{envs.<group>.<name>}}`, while keeping OpenAPI specs as optional contract metadata.

**Architecture:** Env groups become runtime configuration owned by the project, separate from OpenAPI specs. The orchestrator stores env groups, exposes CRUD APIs, resolves a selected env group for project executions, forwards env groups and selection to runners, and the engine renders a new `envs` template namespace. Existing `{{specs.<slug>.url.<env>}}` and legacy `{{url.<slug>.<env>}}` remain supported.

**Tech Stack:** Rust/Axum/SQLx/utoipa for `previa-main` and `previa-runner`, `previa-engine` for template resolution, React/TypeScript/Zustand/Vitest for the IDE.

---

## Key Decisions

1. **Env group means runtime preset.**
   A group is a named set of URLs such as `local`, `hml`, `prd`, or `custom1`. Entries inside the group are named targets such as `api`, `auth`, `payments`, or `my-env`.

2. **Selection must affect execution.**
   If the user selects env group `hml`, the engine exposes that group under `{{envs.current.*}}`.

3. **Explicit references still work.**
   A pipeline may use `{{envs.hml.api}}` when it should always target one group regardless of the run selector.

4. **Specs stay, but stop owning runtime environments.**
   Spec server URLs remain compatible, but new authoring flows prefer env groups.

5. **Do not implement global override of explicit templates.**
   Selecting `prd` does not rewrite `{{envs.hml.api}}`. Pipelines that should follow run selection must use `{{envs.current.api}}`.

## Runtime Examples

Project env groups:

```json
[
  {
    "slug": "local",
    "name": "Local",
    "entries": [
      { "name": "api", "url": "http://localhost:3000", "description": "Main API" },
      { "name": "auth", "url": "http://localhost:3001", "description": "Auth API" }
    ]
  },
  {
    "slug": "hml",
    "name": "Homolog",
    "entries": [
      { "name": "api", "url": "https://api-hml.example.com", "description": "Main API" },
      { "name": "auth", "url": "https://auth-hml.example.com", "description": "Auth API" }
    ]
  }
]
```

Pipeline URLs:

```text
{{envs.current.api}}/health
{{envs.current.auth}}/oauth/token
{{envs.hml.api}}/health
{{specs.users-api.url.hml}}/users
```

Execution request:

```json
{
  "pipelineId": "pipeline-id",
  "selectedEnvGroupSlug": "hml",
  "envGroups": [
    {
      "slug": "hml",
      "urls": {
        "api": "https://api-hml.example.com",
        "auth": "https://auth-hml.example.com"
      }
    }
  ],
  "specs": []
}
```

## File Map

- `engine/src/core/types.rs`: add `RuntimeEnvGroup`.
- `engine/src/template/resolve.rs`: build `envs` and `envs.current` template context.
- `engine/src/execution/engine.rs`: thread env groups and selected group through execution.
- `engine/src/lib.rs`: export new type and new runtime execution functions.
- `runner/src/server/models.rs`: accept `envGroups` and `selectedEnvGroupSlug`.
- `runner/src/server/handlers/e2e.rs`: pass env groups to engine.
- `runner/src/server/handlers/load.rs`: pass env groups to engine for each load iteration.
- `runner/src/lib.rs`: re-export `RuntimeEnvGroup` and `execute_pipeline_with_runtime_hooks`.
- `main/migrations/sqlite/202605010001_add_env_groups.sql`: SQLite table.
- `main/migrations/postgres/202605010001_add_env_groups.sql`: Postgres table.
- `main/src/server/models.rs`: env group models and execution request fields.
- `main/src/server/validation/env_groups.rs`: validate slugs and entries.
- `main/src/server/db/env_groups.rs`: CRUD and project export/import helpers.
- `main/src/server/db/mod.rs`: re-export env group DB functions.
- `main/src/server/handlers/env_groups.rs`: HTTP CRUD handlers.
- `main/src/server/handlers/mod.rs`: expose handler module.
- `main/src/server/mod.rs`: register routes.
- `main/src/server/docs.rs`: OpenAPI docs for env group routes and models.
- `main/src/server/execution/runtime_specs.rs`: keep the filename and add env group runtime loading beside the existing spec helpers.
- `main/src/server/execution/e2e.rs`: resolve env groups, validate templates, store request context.
- `main/src/server/execution/e2e_queue.rs`: persist env groups in queue request and pass them to each execution.
- `main/src/server/execution/load.rs`: resolve env groups, validate templates, forward to runners.
- `main/src/server/validation/pipelines.rs`: validate `envs.current.*` and `envs.<group>.*`.
- `main/src/server/db/transfers.rs`: export/import env groups.
- `main/src/server/services/sqlite_transfer.rs`: rewrite env group IDs/project IDs on SQLite import.
- `app/src/types/project.ts`: add env group types and `Project.envGroups`.
- `app/src/lib/api-client.ts`: CRUD client, project load, execution payload fields.
- `app/src/stores/useProjectStore.ts`: load and mutate env groups.
- `app/src/lib/remote-executor.ts`: send env groups and selected group for E2E/load.
- `app/src/stores/useExecutionHistoryStore.ts`: pass env runtime context to E2E.
- `app/src/stores/useLoadTestHistoryStore.ts`: pass env runtime context to load tests.
- `app/src/pages/TestExecutionPage.tsx`: E2E and batch selector.
- `app/src/components/LoadTestConfigPanel.tsx`: load-test selector.
- `app/src/components/ProjectEnvGroupsPanel.tsx`: project-level env group manager.
- `app/src/components/StepCreatorPanel.tsx`: prefer `envs.current` when creating URLs.
- `app/src/lib/template-validator.ts`: frontend validation for env templates.
- `app/src/lib/monaco-template-setup.ts`: autocomplete for `envs`.
- `app/src/components/PipelineDocsPanel.tsx`: docs/examples.
- `app/src/components/AIPipelineChat.tsx`: prompt instructions.
- `PROJECT.md`: note env groups as runtime config distinct from specs.

## Task 1: Engine Template Runtime

**Files:**
- Modify `engine/src/core/types.rs`
- Modify `engine/src/template/resolve.rs`
- Modify `engine/src/execution/engine.rs`
- Modify `engine/src/lib.rs`

- [ ] **Step 1: Add failing template tests**

Add these tests to `engine/src/template/resolve.rs` inside the existing test module:

```rust
#[test]
fn resolves_explicit_env_group_url_variable() {
    let env_groups = [RuntimeEnvGroup {
        slug: "hml".to_owned(),
        urls: HashMap::from([("api".to_owned(), "https://api-hml.example.com".to_owned())]),
    }];
    let context = build_template_context(&HashMap::new(), None, Some(&env_groups), Some("hml"));
    let rendered = resolve_template_variables_with_context(
        &Value::String("{{envs.hml.api}}/health".to_owned()),
        &context,
    );
    assert_eq!(
        rendered,
        Value::String("https://api-hml.example.com/health".to_owned())
    );
}

#[test]
fn resolves_current_env_group_url_variable() {
    let env_groups = [
        RuntimeEnvGroup {
            slug: "local".to_owned(),
            urls: HashMap::from([("api".to_owned(), "http://localhost:3000".to_owned())]),
        },
        RuntimeEnvGroup {
            slug: "hml".to_owned(),
            urls: HashMap::from([("api".to_owned(), "https://api-hml.example.com".to_owned())]),
        },
    ];
    let context = build_template_context(&HashMap::new(), None, Some(&env_groups), Some("hml"));
    let rendered = resolve_template_variables_with_context(
        &Value::String("{{envs.current.api}}/health".to_owned()),
        &context,
    );
    assert_eq!(
        rendered,
        Value::String("https://api-hml.example.com/health".to_owned())
    );
}

#[test]
fn leaves_current_env_variable_unresolved_without_selection() {
    let env_groups = [RuntimeEnvGroup {
        slug: "hml".to_owned(),
        urls: HashMap::from([("api".to_owned(), "https://api-hml.example.com".to_owned())]),
    }];
    let context = build_template_context(&HashMap::new(), None, Some(&env_groups), None);
    let rendered = resolve_template_variables_with_context(
        &Value::String("{{envs.current.api}}/health".to_owned()),
        &context,
    );
    assert_eq!(rendered, Value::String("{{envs.current.api}}/health".to_owned()));
}
```

Expected first run:

```bash
cargo test -p previa-engine resolves_current_env_group_url_variable
```

Expected result: compile failure because `RuntimeEnvGroup` and the new context signature do not exist.

- [ ] **Step 2: Add `RuntimeEnvGroup`**

In `engine/src/core/types.rs`, add next to `RuntimeSpec`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeEnvGroup {
    pub slug: String,
    #[serde(default)]
    pub urls: HashMap<String, String>,
}
```

- [ ] **Step 3: Extend template context**

Change `resolve_template_variables` signature in `engine/src/template/resolve.rs`:

```rust
pub(crate) fn resolve_template_variables(
    value: &Value,
    context: &HashMap<String, StepExecutionResult>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
) -> Value {
    let template_context =
        build_template_context(context, specs, env_groups, selected_env_group_slug);
    resolve_template_variables_with_context(value, &template_context)
}
```

Change `build_template_context` to:

```rust
pub(crate) fn build_template_context(
    steps: &HashMap<String, StepExecutionResult>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
) -> Value {
    let mut root = Map::new();

    let mut steps_map = Map::new();
    for (step_id, result) in steps {
        let step_body = result
            .response
            .as_ref()
            .map(|response| response.body.clone())
            .unwrap_or(Value::Null);
        steps_map.insert(step_id.clone(), step_body);
    }
    root.insert("steps".to_owned(), Value::Object(steps_map));

    let mut specs_map = Map::new();
    if let Some(specs) = specs {
        for spec in specs {
            let slug = spec.slug.trim();
            if slug.is_empty() {
                continue;
            }
            let mut urls_map = Map::new();
            for (name, url) in &spec.servers {
                let name = name.trim();
                let url = url.trim();
                if !name.is_empty() && !url.is_empty() {
                    urls_map.insert(name.to_owned(), Value::String(url.to_owned()));
                }
            }
            let mut spec_entry = Map::new();
            spec_entry.insert("url".to_owned(), Value::Object(urls_map));
            specs_map.insert(slug.to_owned(), Value::Object(spec_entry));
        }
    }
    root.insert("specs".to_owned(), Value::Object(specs_map));

    let mut envs_map = Map::new();
    let selected_slug = selected_env_group_slug.map(str::trim).filter(|value| !value.is_empty());
    if let Some(env_groups) = env_groups {
        for group in env_groups {
            let slug = group.slug.trim();
            if slug.is_empty() {
                continue;
            }
            let mut urls_map = Map::new();
            for (name, url) in &group.urls {
                let name = name.trim();
                let url = url.trim();
                if !name.is_empty() && !url.is_empty() {
                    urls_map.insert(name.to_owned(), Value::String(url.to_owned()));
                }
            }
            if selected_slug == Some(slug) {
                envs_map.insert("current".to_owned(), Value::Object(urls_map.clone()));
            }
            envs_map.insert(slug.to_owned(), Value::Object(urls_map));
        }
    }
    root.insert("envs".to_owned(), Value::Object(envs_map));

    Value::Object(root)
}
```

- [ ] **Step 4: Thread envs through engine execution**

In `engine/src/execution/engine.rs`, add a new public function:

```rust
pub async fn execute_pipeline_with_runtime_hooks<FStart, FResult, FCancel>(
    pipeline: &Pipeline,
    selected_base_url_key: Option<&str>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
    on_step_start: FStart,
    on_step_result: FResult,
    should_cancel: FCancel,
) -> Vec<StepExecutionResult>
where
    FStart: FnMut(&str),
    FResult: FnMut(&StepExecutionResult),
    FCancel: FnMut() -> bool,
{
    let client = Client::new();
    execute_pipeline_with_client_runtime_hooks(
        &client,
        pipeline,
        selected_base_url_key,
        specs,
        env_groups,
        selected_env_group_slug,
        on_step_start,
        on_step_result,
        should_cancel,
    )
    .await
}
```

Rename the internal `execute_pipeline_with_client_specs_hooks` to `execute_pipeline_with_client_runtime_hooks` and add `env_groups` plus `selected_env_group_slug` parameters. Every call to `resolve_template_variables` in that function must pass both new parameters.

Keep compatibility wrappers:

```rust
pub async fn execute_pipeline_with_specs_hooks<FStart, FResult, FCancel>(
    pipeline: &Pipeline,
    selected_base_url_key: Option<&str>,
    specs: Option<&[RuntimeSpec]>,
    on_step_start: FStart,
    on_step_result: FResult,
    should_cancel: FCancel,
) -> Vec<StepExecutionResult>
where
    FStart: FnMut(&str),
    FResult: FnMut(&StepExecutionResult),
    FCancel: FnMut() -> bool,
{
    execute_pipeline_with_runtime_hooks(
        pipeline,
        selected_base_url_key,
        specs,
        None,
        None,
        on_step_start,
        on_step_result,
        should_cancel,
    )
    .await
}
```

- [ ] **Step 5: Export new API**

In `engine/src/lib.rs`, export `RuntimeEnvGroup` and `execute_pipeline_with_runtime_hooks`.

Run:

```bash
cargo test -p previa-engine resolves_explicit_env_group_url_variable
cargo test -p previa-engine resolves_current_env_group_url_variable
cargo test -p previa-engine leaves_current_env_variable_unresolved_without_selection
```

Expected: all three tests pass.

Commit:

```bash
git add engine
git commit -m "feat(engine): support runtime env groups"
```

## Task 2: Runner Request Support

**Files:**
- Modify `runner/src/server/models.rs`
- Modify `runner/src/server/handlers/e2e.rs`
- Modify `runner/src/server/handlers/load.rs`
- Modify `runner/src/lib.rs`

- [ ] **Step 1: Add request fields**

In `runner/src/server/models.rs`, import `RuntimeEnvGroup` and add to both request structs:

```rust
pub selected_env_group_slug: Option<String>,
#[serde(default)]
pub env_groups: Vec<RuntimeEnvGroup>,
```

Keep `selected_base_url_key` and `specs` unchanged.

- [ ] **Step 2: Pass envs to E2E**

In `runner/src/server/handlers/e2e.rs`, change the import:

```rust
use previa_runner::execute_pipeline_with_runtime_hooks;
```

Clone request fields:

```rust
let selected_env_group_slug = payload.selected_env_group_slug.clone();
let env_groups = payload.env_groups.clone();
```

Call:

```rust
let results = execute_pipeline_with_runtime_hooks(
    &pipeline,
    selected_key.as_deref(),
    Some(specs.as_slice()),
    Some(env_groups.as_slice()),
    selected_env_group_slug.as_deref(),
    on_step_start,
    on_step_result,
    || token.is_cancelled(),
)
.await;
```

- [ ] **Step 3: Pass envs to load test iterations**

In `runner/src/server/handlers/load.rs`, clone `selected_env_group_slug` and `env_groups` before spawning workers. Inside each worker, call `execute_pipeline_with_runtime_hooks` with `Some(env_groups.as_slice())` and `selected_env_group_slug.as_deref()`.

- [ ] **Step 4: Verify runner**

Run:

```bash
cargo test -p previa-runner
```

Expected: tests pass.

Commit:

```bash
git add runner
git commit -m "feat(runner): accept runtime env groups"
```

## Task 3: Orchestrator Persistence and Models

**Files:**
- Create `main/migrations/sqlite/202605010001_add_env_groups.sql`
- Create `main/migrations/postgres/202605010001_add_env_groups.sql`
- Create `main/src/server/validation/env_groups.rs`
- Create `main/src/server/db/env_groups.rs`
- Modify `main/src/server/models.rs`
- Modify `main/src/server/db/mod.rs`
- Modify `main/src/server/validation/mod.rs`

- [ ] **Step 1: Add migrations**

SQLite:

```sql
CREATE TABLE IF NOT EXISTS project_env_groups (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    slug TEXT NOT NULL,
    name TEXT NOT NULL,
    entries_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL DEFAULT 0,
    updated_at_ms INTEGER NOT NULL DEFAULT 0,
    UNIQUE(project_id, slug),
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_project_env_groups_project_id_updated
    ON project_env_groups(project_id, updated_at_ms DESC);
```

Postgres:

```sql
CREATE TABLE IF NOT EXISTS project_env_groups (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    slug TEXT NOT NULL,
    name TEXT NOT NULL,
    entries_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL DEFAULT 0,
    updated_at_ms BIGINT NOT NULL DEFAULT 0,
    UNIQUE(project_id, slug),
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_project_env_groups_project_id_updated
    ON project_env_groups(project_id, updated_at_ms DESC);
```

- [ ] **Step 2: Add server models**

In `main/src/server/models.rs`, import `RuntimeEnvGroup` and add:

```rust
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvGroupEntry {
    pub name: String,
    pub url: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectEnvGroupUpsertRequest {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub entries: Vec<EnvGroupEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEnvGroupRecord {
    pub id: String,
    pub project_id: String,
    pub slug: String,
    pub name: String,
    pub entries: Vec<EnvGroupEntry>,
    pub created_at: String,
    pub updated_at: String,
}
```

Add execution fields to `LoadTestRequest`, `E2eTestRequest`, `ProjectE2eTestRequest`, `ProjectE2eQueueRequest`, and `ProjectLoadTestRequest`:

```rust
pub selected_env_group_slug: Option<String>,
#[serde(default)]
pub env_groups: Vec<RuntimeEnvGroup>,
```

Add to `ProjectExportProject`:

```rust
pub env_groups: Vec<ProjectEnvGroupRecord>,
```

Add to `ProjectImportResponse` and SQLite import item:

```rust
pub env_groups_imported: usize,
```

- [ ] **Step 3: Add validation**

Create `main/src/server/validation/env_groups.rs`:

```rust
use std::collections::HashSet;

use crate::server::models::{EnvGroupEntry, ProjectEnvGroupUpsertRequest};

pub fn normalize_env_group_payload(
    mut payload: ProjectEnvGroupUpsertRequest,
) -> Result<ProjectEnvGroupUpsertRequest, &'static str> {
    payload.slug = normalize_env_slug(&payload.slug)?;
    payload.name = payload.name.trim().to_owned();
    if payload.name.is_empty() {
        return Err("env group name is required");
    }
    payload.entries = normalize_env_entries(payload.entries)?;
    Ok(payload)
}

pub fn normalize_env_slug(raw: &str) -> Result<String, &'static str> {
    let value = raw.trim();
    if value.is_empty() {
        return Err("env group slug is required");
    }
    if value == "current" {
        return Err("env group slug 'current' is reserved");
    }
    if value.starts_with('-') || value.ends_with('-') || value.contains("--") {
        return Err("env group slug cannot start/end with '-' or contain repeated separators");
    }
    if !value.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-') {
        return Err("env group slug must use lowercase letters, numbers, or '-'");
    }
    Ok(value.to_owned())
}

pub fn normalize_env_entries(entries: Vec<EnvGroupEntry>) -> Result<Vec<EnvGroupEntry>, &'static str> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::with_capacity(entries.len());
    for entry in entries {
        let name = entry.name.trim().to_ascii_lowercase();
        if name.is_empty() {
            return Err("env entries[].name is required");
        }
        if !name.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_') {
            return Err("env entries[].name must use lowercase letters, numbers, '-' or '_'");
        }
        if !seen.insert(name.clone()) {
            return Err("env entries[].name must be unique");
        }
        let url = entry.url.trim().to_owned();
        if url.is_empty() {
            return Err("env entries[].url is required");
        }
        normalized.push(EnvGroupEntry {
            name,
            url,
            description: entry.description.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned),
        });
    }
    Ok(normalized)
}
```

- [ ] **Step 4: Add DB CRUD**

Create `main/src/server/db/env_groups.rs` with these public functions:

```rust
pub fn project_env_group_from_row(row: &sqlx::any::AnyRow) -> ProjectEnvGroupRecord
pub async fn list_project_env_group_records(db: &DbPool, project_id: &str) -> Result<Vec<ProjectEnvGroupRecord>, sqlx::Error>
pub async fn load_project_env_group_record_by_id(db: &DbPool, project_id: &str, env_group_id: &str) -> Result<Option<ProjectEnvGroupRecord>, sqlx::Error>
pub async fn insert_project_env_group_record(db: &DbPool, project_id: &str, payload: ProjectEnvGroupUpsertRequest) -> Result<ProjectEnvGroupRecord, sqlx::Error>
pub async fn update_project_env_group_record(db: &DbPool, project_id: &str, env_group_id: &str, payload: ProjectEnvGroupUpsertRequest) -> Result<Option<ProjectEnvGroupRecord>, sqlx::Error>
pub async fn delete_project_env_group_record(db: &DbPool, project_id: &str, env_group_id: &str) -> Result<bool, sqlx::Error>
pub fn runtime_env_group_from_record(record: &ProjectEnvGroupRecord) -> Option<RuntimeEnvGroup>
```

Use the same transaction pattern as `main/src/server/db/specs.rs`: insert/update/delete inside a transaction and call `touch_project_updated_at`.

- [ ] **Step 5: Add tests**

Add tests in `main/src/server/db/env_groups.rs`:

```rust
#[tokio::test]
async fn env_group_crud_roundtrip() { /* create migrated sqlite::memory:, insert, list, update, delete */ }

#[test]
fn rejects_reserved_current_slug() {
    let err = normalize_env_slug("current").expect_err("current is reserved");
    assert!(err.contains("reserved"));
}

#[test]
fn rejects_duplicate_entry_names() { /* two entries named api produce unique error */ }
```

Run:

```bash
cargo test -p previa-main env_group
```

Expected: all env group model, validation, and DB tests pass.

Commit:

```bash
git add main/migrations/sqlite/202605010001_add_env_groups.sql main/migrations/postgres/202605010001_add_env_groups.sql main/src/server/models.rs main/src/server/validation main/src/server/db
git commit -m "feat(main): persist project env groups"
```

## Task 4: Orchestrator API, Export, and Import

**Files:**
- Create `main/src/server/handlers/env_groups.rs`
- Modify `main/src/server/handlers/mod.rs`
- Modify `main/src/server/mod.rs`
- Modify `main/src/server/docs.rs`
- Modify `main/src/server/db/transfers.rs`
- Modify `main/src/server/services/sqlite_transfer.rs`

- [ ] **Step 1: Add CRUD handlers**

Create handlers with the same response style as `main/src/server/handlers/specs.rs`:

```rust
pub async fn list_project_env_groups(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
) -> Response

pub async fn create_project_env_group(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Json(payload): Json<ProjectEnvGroupUpsertRequest>,
) -> Response

pub async fn get_project_env_group(
    State(state): State<AppState>,
    Path((project_id, env_group_id)): Path<(String, String)>,
) -> Response

pub async fn upsert_project_env_group(
    State(state): State<AppState>,
    Path((project_id, env_group_id)): Path<(String, String)>,
    Json(payload): Json<ProjectEnvGroupUpsertRequest>,
) -> Response

pub async fn delete_project_env_group(
    State(state): State<AppState>,
    Path((project_id, env_group_id)): Path<(String, String)>,
) -> Response
```

Return `400` for validation errors, `404` for missing project/group, and `500` only for unexpected DB errors.

- [ ] **Step 2: Register routes**

In `main/src/server/mod.rs`, register:

```rust
.route(
    "/api/v1/projects/{projectId}/env-groups",
    get(list_project_env_groups).post(create_project_env_group),
)
.route(
    "/api/v1/projects/{projectId}/env-groups/{envGroupId}",
    get(get_project_env_group)
        .put(upsert_project_env_group)
        .delete(delete_project_env_group),
)
```

- [ ] **Step 3: Include env groups in export/import**

In `main/src/server/db/transfers.rs`, load env groups in `load_project_export` and insert them in `import_project_bundle`.

In `main/src/server/services/sqlite_transfer.rs`, rewrite imported env group `id` and `project_id` the same way specs are rewritten.

- [ ] **Step 4: Add API and transfer tests**

Add tests to cover:

```text
POST /api/v1/projects/{projectId}/env-groups creates a group.
GET /api/v1/projects/{projectId}/env-groups lists created groups.
PUT updates entries.
DELETE removes the group.
SQLite export/import preserves env groups with rewritten IDs.
```

Run:

```bash
cargo test -p previa-main env_group
cargo test -p previa-main exports_selected_projects_to_sqlite
```

Expected: tests pass.

Commit:

```bash
git add main/src/server/handlers main/src/server/mod.rs main/src/server/docs.rs main/src/server/db/transfers.rs main/src/server/services/sqlite_transfer.rs
git commit -m "feat(main): expose env group APIs"
```

## Task 5: Orchestrator Runtime Resolution and Validation

**Files:**
- Modify `main/src/server/execution/runtime_specs.rs`
- Modify `main/src/server/execution/e2e.rs`
- Modify `main/src/server/execution/e2e_queue.rs`
- Modify `main/src/server/execution/load.rs`
- Modify `main/src/server/validation/pipelines.rs`

- [ ] **Step 1: Resolve runtime env groups**

Add to `main/src/server/execution/runtime_specs.rs`:

```rust
pub async fn load_runtime_env_groups_for_project(
    db: &DbPool,
    project_id: &str,
) -> Result<Vec<RuntimeEnvGroup>, sqlx::Error>

pub async fn resolve_runtime_env_groups_for_execution(
    db: &DbPool,
    project_id: Option<&str>,
    payload_env_groups: &[RuntimeEnvGroup],
) -> Result<Option<Vec<RuntimeEnvGroup>>, sqlx::Error>

pub fn sanitize_runtime_env_groups(env_groups: &[RuntimeEnvGroup]) -> Vec<RuntimeEnvGroup>
```

Rules:

```text
Payload envGroups win over stored project env groups.
Groups with empty slug are dropped.
Group slug "current" is dropped.
Entries with empty names or URLs are dropped.
Groups with no valid entries are dropped.
```

- [ ] **Step 2: Validate env templates**

Change `validate_pipeline_templates` signature:

```rust
pub fn validate_pipeline_templates(
    pipeline: &Pipeline,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
) -> Vec<String>
```

Validation rules:

```text
{{envs.current.api}} is valid only when selectedEnvGroupSlug matches a known group containing api.
{{envs.hml.api}} is valid when group hml exists and contains api.
{{envs.current}} is invalid.
{{envs.unknown.api}} is invalid.
{{envs.hml.unknown}} is invalid.
```

Keep existing `steps`, `helpers`, `specs`, and legacy `url` validation behavior.

- [ ] **Step 3: Use env groups in project E2E execution**

In `main/src/server/execution/e2e.rs`, resolve specs and env groups before validation, call the updated validator, include both in history request JSON, and forward both to the runner or direct execution path.

The request JSON saved in history must include:

```json
{
  "selectedEnvGroupSlug": "hml",
  "envGroups": [...]
}
```

- [ ] **Step 4: Use env groups in E2E queues**

In `main/src/server/execution/e2e_queue.rs`, include `selectedEnvGroupSlug` and `envGroups` in `queue_request_json`. When each queued pipeline runs, pass the same env context used when the queue was created.

- [ ] **Step 5: Use env groups in load execution**

In `main/src/server/execution/load.rs`, resolve env groups, validate templates with selected group, include env groups in `request_payload`, and forward to runners:

```rust
let request_payload = json!({
    "pipeline": runner_pipeline,
    "config": runner_config,
    "selectedBaseUrlKey": runner_selected_base_url_key,
    "selectedEnvGroupSlug": runner_selected_env_group_slug,
    "specs": runtime_specs_for_runner,
    "envGroups": runtime_env_groups_for_runner,
});
```

- [ ] **Step 6: Add runtime tests**

Add tests:

```text
validate_pipeline_templates accepts envs.current.api with selected group hml.
validate_pipeline_templates rejects envs.current.api without selected group.
E2E request with envGroups resolves envs.current.api.
Queue request persists envGroups in request_json.
Load request forwards envGroups to runner payload.
```

Run:

```bash
cargo test -p previa-main validate_pipeline_templates
cargo test -p previa-main e2e_queue
cargo test -p previa-main load
```

Expected: tests pass.

Commit:

```bash
git add main/src/server/execution main/src/server/validation
git commit -m "feat(main): resolve env groups during execution"
```

## Task 6: Frontend Project State and API Client

**Files:**
- Modify `app/src/types/project.ts`
- Modify `app/src/lib/api-client.ts`
- Modify `app/src/stores/useProjectStore.ts`
- Modify `app/src/lib/project-io.ts`
- Modify `app/src/lib/project-db.ts` if local/offline project persistence still uses this path.

- [ ] **Step 1: Add frontend types**

In `app/src/types/project.ts`:

```ts
export interface ProjectEnvEntry {
  name: string;
  url: string;
  description?: string | null;
}

export interface ProjectEnvGroup {
  id: string;
  projectId: string;
  slug: string;
  name: string;
  entries: ProjectEnvEntry[];
  createdAt: string;
  updatedAt: string;
}
```

Add to `Project`:

```ts
envGroups: ProjectEnvGroup[];
```

- [ ] **Step 2: Add API client methods**

In `app/src/lib/api-client.ts`, add:

```ts
export interface ProjectEnvGroupUpsertRequest {
  slug: string;
  name: string;
  entries: ProjectEnvEntry[];
}

export async function listProjectEnvGroups(baseUrl: string, projectId: string): Promise<ProjectEnvGroup[]> {
  return request<ProjectEnvGroup[]>(`${ensureApiPrefix(baseUrl)}/projects/${projectId}/env-groups`);
}

export async function createProjectEnvGroup(baseUrl: string, projectId: string, data: ProjectEnvGroupUpsertRequest): Promise<ProjectEnvGroup> {
  return request<ProjectEnvGroup>(`${ensureApiPrefix(baseUrl)}/projects/${projectId}/env-groups`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(data),
  });
}

export async function updateProjectEnvGroup(baseUrl: string, projectId: string, envGroupId: string, data: ProjectEnvGroupUpsertRequest): Promise<ProjectEnvGroup> {
  return request<ProjectEnvGroup>(`${ensureApiPrefix(baseUrl)}/projects/${projectId}/env-groups/${envGroupId}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(data),
  });
}

export async function deleteProjectEnvGroup(baseUrl: string, projectId: string, envGroupId: string): Promise<void> {
  await request<void>(`${ensureApiPrefix(baseUrl)}/projects/${projectId}/env-groups/${envGroupId}`, { method: "DELETE" });
}
```

Update project loading to fetch `envGroups` in parallel with project, pipelines, and specs.

- [ ] **Step 3: Add store actions**

In `app/src/stores/useProjectStore.ts`, add:

```ts
createEnvGroup(projectId: string, payload: ProjectEnvGroupUpsertRequest): Promise<ProjectEnvGroup>
updateEnvGroup(projectId: string, envGroupId: string, payload: ProjectEnvGroupUpsertRequest): Promise<ProjectEnvGroup>
deleteEnvGroup(projectId: string, envGroupId: string): Promise<void>
```

Each action updates `currentProject.envGroups` and the matching project in `projects`.

- [ ] **Step 4: Verify frontend state**

Run:

```bash
cd app
npm test -- --run project
```

Expected: project-related tests pass.

Commit:

```bash
git add app/src/types/project.ts app/src/lib/api-client.ts app/src/stores/useProjectStore.ts app/src/lib/project-io.ts app/src/lib/project-db.ts
git commit -m "feat(app): load project env groups"
```

## Task 7: Env Group Management UI

**Files:**
- Create `app/src/components/ProjectEnvGroupsPanel.tsx`
- Modify `app/src/pages/ProjectFlowPage.tsx`
- Modify navigation/sidebar component that currently exposes Specs/Pipelines actions.
- Add i18n keys in `app/src/i18n/locales/en.json` and `app/src/i18n/locales/pt-BR.json`. For other locale files, add English fallback text in the same keys to keep builds deterministic.

- [ ] **Step 1: Create management panel**

Create `ProjectEnvGroupsPanel` with:

```ts
interface ProjectEnvGroupsPanelProps {
  projectId: string;
  envGroups: ProjectEnvGroup[];
  onCreate: (payload: ProjectEnvGroupUpsertRequest) => Promise<ProjectEnvGroup>;
  onUpdate: (envGroupId: string, payload: ProjectEnvGroupUpsertRequest) => Promise<ProjectEnvGroup>;
  onDelete: (envGroupId: string) => Promise<void>;
}
```

UI requirements:

```text
List env groups by name and slug.
Show entries as name + URL rows.
Allow add/remove entry rows before save.
Validate client-side required slug, name, entry name, and entry URL.
Disable saving while request is in flight.
Show destructive confirmation before delete.
```

- [ ] **Step 2: Wire panel into project flow**

Add an "Envs" or "Env Groups" view near existing specs/pipelines project views. It should be reachable without opening the OpenAPI spec editor.

- [ ] **Step 3: Verify UI build**

Run:

```bash
cd app
npm test -- --run Project
npm run build
```

Expected: tests and build pass.

Commit:

```bash
git add app/src/components/ProjectEnvGroupsPanel.tsx app/src/pages/ProjectFlowPage.tsx app/src
git commit -m "feat(app): manage project env groups"
```

## Task 8: Execution Selection UI

**Files:**
- Modify `app/src/pages/TestExecutionPage.tsx`
- Modify `app/src/components/LoadTestConfigPanel.tsx`
- Modify `app/src/components/LoadTestTab.tsx`
- Modify `app/src/lib/remote-executor.ts`
- Modify `app/src/stores/useExecutionHistoryStore.ts`
- Modify `app/src/stores/useLoadTestHistoryStore.ts`
- Modify `app/src/lib/api-client.ts`

- [ ] **Step 1: Add runtime conversion helper**

In `app/src/lib/api-client.ts` or a small local helper:

```ts
export function projectEnvGroupsToRuntime(envGroups: ProjectEnvGroup[]) {
  return envGroups.map((group) => ({
    slug: group.slug,
    urls: Object.fromEntries(group.entries.map((entry) => [entry.name, entry.url])),
  }));
}
```

- [ ] **Step 2: Add selected env group state**

In `TestExecutionPage`, add:

```ts
const [selectedEnvGroupSlug, setSelectedEnvGroupSlug] = useState<string | null>(null);
```

When `envGroups` changes, default to the first group slug if no selected group exists.

- [ ] **Step 3: Add selector near run controls**

Show a compact `Select` only when `envGroups.length > 0`. The selector label should be concise, for example `Env`. The selected value is sent to single E2E and batch queue creation.

- [ ] **Step 4: Send env context for E2E and batch**

Update `runRemoteIntegrationTest` body:

```ts
const body = {
  pipelineId: pipeline.id,
  selectedBaseUrlKey,
  selectedEnvGroupSlug,
  pipelineIndex,
  specs,
  envGroups,
};
```

Update `createE2eQueue` payload type and call site with:

```ts
selectedEnvGroupSlug,
envGroups: projectEnvGroupsToRuntime(envGroups),
```

- [ ] **Step 5: Send env context for load tests**

Add selected env group to `LoadTestConfigPanel` and `LoadTestTab`. Update `runRemoteLoadTest` body with `selectedEnvGroupSlug` and `envGroups`.

- [ ] **Step 6: Verify execution UI**

Run:

```bash
cd app
npm test -- --run load
npm run build
```

Expected: tests and build pass.

Commit:

```bash
git add app/src/pages/TestExecutionPage.tsx app/src/components/LoadTestConfigPanel.tsx app/src/components/LoadTestTab.tsx app/src/lib/remote-executor.ts app/src/stores/useExecutionHistoryStore.ts app/src/stores/useLoadTestHistoryStore.ts app/src/lib/api-client.ts
git commit -m "feat(app): select env group for executions"
```

## Task 9: Authoring, Validation, and Assistant Context

**Files:**
- Modify `app/src/lib/template-validator.ts`
- Modify `app/src/lib/monaco-template-setup.ts`
- Modify `app/src/components/MonacoInput.tsx` and `app/src/components/StepCreatorPanel.tsx`, because both build or pass `TemplateValidationContext`.
- Modify `app/src/components/StepCreatorPanel.tsx`
- Modify `app/src/components/PipelineDocsPanel.tsx`
- Modify `app/src/components/AIPipelineChat.tsx`
- Modify `app/src/lib/sample-pipeline.ts`
- Modify `engine/README.md`

- [ ] **Step 1: Extend frontend template context**

In `TemplateValidationContext`, add:

```ts
availableEnvGroups?: Array<{ slug: string; entries: string[] }>;
selectedEnvGroupSlug?: string | null;
```

Add `"envs"` to `VALID_NAMESPACES`.

- [ ] **Step 2: Validate env templates**

Rules:

```text
envs.current.<name> is valid when selectedEnvGroupSlug exists and selected group contains name.
envs.<group>.<name> is valid when group exists and contains name.
envs.current without name is error.
envs.<group> without name is error.
Unknown group or entry is warning when context is available.
```

- [ ] **Step 3: Add Monaco completions**

Completion flow:

```text
envs
envs.current
envs.current.<entry from selected group>
envs.<group>
envs.<group>.<entry>
```

- [ ] **Step 4: Prefer `envs.current` for new pipeline URLs**

In `StepCreatorPanel`, when env groups exist, new route-based URLs should use:

```text
{{envs.current.api}}/path
```

When no env groups exist but specs have servers, keep current `{{specs.<slug>.url.<env>}}/path` behavior.

- [ ] **Step 5: Update docs and assistant prompt**

Docs must state:

```text
Use {{envs.current.<name>}} when the test should follow the env selector.
Use {{envs.<group>.<name>}} for fixed references.
Existing {{specs.<slug>.url.<env>}} templates remain supported.
```

Assistant prompt must prefer env groups when available and avoid telling the AI to always use specs.

- [ ] **Step 6: Verify authoring tests**

Run:

```bash
cd app
npm test -- --run template-validator
npm run build
```

Expected: tests and build pass.

Commit:

```bash
git add app/src/lib/template-validator.ts app/src/lib/monaco-template-setup.ts app/src/components app/src/lib/sample-pipeline.ts engine/README.md
git commit -m "feat(app): author pipelines with env groups"
```

## Task 10: Final Verification and Release Discipline

**Files:**
- All modified files.

- [ ] **Step 1: Rust focused tests**

Run:

```bash
cargo test -p previa-engine envs
cargo test -p previa-runner
cargo test -p previa-main env_group
cargo test -p previa-main e2e_queue
cargo test -p previa-main load
```

Expected: all pass.

- [ ] **Step 2: Frontend tests and build**

Run:

```bash
cd app
npm test -- --run template-validator
npm run build
```

Expected: all pass.

- [ ] **Step 3: Full release build**

Required by `AGENTS.md`:

```bash
cargo build --release
```

Expected: release build succeeds.

- [ ] **Step 4: Manual smoke**

Use a local project and verify:

```text
Create env group local with api=http://localhost:3000.
Create env group hml with api=https://api-hml.example.com.
Create pipeline URL {{envs.current.api}}/health.
Run single E2E selecting local.
Run single E2E selecting hml.
Run batch selecting hml.
Run load test selecting hml.
Run an existing {{specs.<slug>.url.<env>}} pipeline.
Export project to SQLite and import it back.
Confirm imported project includes env groups.
```

- [ ] **Step 5: Project notes**

Update `PROJECT.md` with:

```markdown
## Env Groups

- Env groups are project-level runtime presets and should be used for new pipeline base URLs.
- OpenAPI specs remain project-level API contracts and should not be required only to configure runtime URLs.
- Prefer `{{envs.current.<name>}}` for pipelines that should follow the execution selector.
- Keep `{{specs.<slug>.url.<env>}}` compatible for existing spec-backed pipelines.
```

- [ ] **Step 6: Commit and push final branch**

```bash
git status --short
git push -u origin codex/env-groups
```

Expected: branch is pushed and ready for PR review.

## Explicit Non-Goals For This PR

- Do not remove OpenAPI specs.
- Do not migrate existing `{{specs.*}}` pipeline URLs automatically.
- Do not add secrets management; env group values are plain URLs.
- Do not add per-step env group overrides.
- Do not implement variable types beyond URL/string values.
- Do not rename existing `selectedBaseUrlKey`; keep it for compatibility until a separate cleanup.

## Self-Review

- The previous plan treated env groups as service-first (`envs.payments.hml`), which made an execution selector weak. This revision treats groups as selectable runtime presets and adds `envs.current`.
- The previous plan omitted project export/import. This revision includes DB transfer and SQLite transfer changes.
- The previous plan did not reserve `current`. This revision reserves it in validation and sanitizer logic.
- The previous plan was vague about selected env behavior. This revision states exactly when selection changes runtime output.
- The previous plan did not clearly include runner request models. This revision includes both orchestrator and runner request payloads.
- Deliberate follow-up: MCP env-group tools are out of scope for this PR and should be planned separately after the HTTP/UI/runtime path lands.
