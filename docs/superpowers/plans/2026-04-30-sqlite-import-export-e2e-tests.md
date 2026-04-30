# SQLite Import/Export E2E Tests Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add end-to-end coverage for SQLite project export/import through the HTTP API and `previa local` CLI.

**Architecture:** Keep transfer business rules covered in `main/src/server/services/sqlite_transfer.rs`, then add route-level E2E coverage in `main/src/server/handlers/transfers.rs` and CLI-level E2E coverage in `previa/tests/cli.rs`. The API test validates real Axum routing and SQLite bytes; the CLI test validates the user workflow against a spawned `previa-main` using a temporary local runtime state.

**Tech Stack:** Rust, Tokio, Axum `oneshot`, SQLx SQLite migrations, `assert_cmd`, `tempfile`, `reqwest`, `cargo test`.

---

## Test Matrix

| Area | Scenario | Expected |
| --- | --- | --- |
| API | `POST /api/v1/projects/export` with `{"all":true}` | returns `200`, `content-type: application/vnd.sqlite3`, valid SQLite bytes |
| API | `POST /api/v1/projects/import` with SQLite bytes | returns `201`, imports all projects |
| API | import SQLite into a target with same project name | imported project name gets `-imported` suffix |
| API | `POST /api/v1/projects/export` without `all` or `projectIds` | returns `400` |
| CLI | `previa local export --all --output file.sqlite3` | writes SQLite file |
| CLI | `previa local import file.sqlite3` | imports all projects into the target context |
| CLI | existing output without `--overwrite` | command fails with overwrite guidance |
| CLI | export selected project with `--project` | SQLite contains only selected project |

## Files

- Modify: `main/src/server/handlers/transfers.rs`
  - Add route-level tests for SQLite export/import.
- Modify: `previa/tests/cli.rs`
  - Add CLI E2E tests that spawn `previa-main`, create projects, export SQLite, import SQLite, and assert final state.
- No production code changes expected unless tests expose a real bug.

---

### Task 1: API E2E Coverage for SQLite Export/Import

**Files:**
- Modify: `main/src/server/handlers/transfers.rs`

- [ ] **Step 1: Add test imports**

Inside `#[cfg(test)] mod tests` at the bottom of `main/src/server/handlers/transfers.rs`, add imports matching the existing handler test style:

```rust
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::server::build_app;
use crate::server::db::{DbPool, list_project_records};
use crate::server::mcp::models::McpConfig;
use crate::server::models::{ProjectListQuery, ProjectMetadataUpsertRequest};
```

- [ ] **Step 2: Add test helpers**

Add helpers in the same test module:

```rust
async fn migrated_db() -> DbPool {
    let db = DbPool::connect("sqlite::memory:", 1)
        .await
        .expect("connect db");
    sqlx::migrate!("./migrations/sqlite")
        .run(db.pool())
        .await
        .expect("migrate db");
    db
}

async fn add_project(db: &DbPool, id: &str, name: &str) {
    crate::server::db::upsert_project_metadata(
        db,
        id.to_owned(),
        ProjectMetadataUpsertRequest {
            name: name.to_owned(),
            description: None,
        },
    )
    .await
    .expect("add project");
}

async fn project_names(db: &DbPool) -> Vec<String> {
    list_project_records(
        db,
        ProjectListQuery {
            limit: None,
            offset: None,
            order: None,
        },
    )
    .await
    .expect("list projects")
    .into_iter()
    .map(|project| project.name)
    .collect()
}
```

- [ ] **Step 3: Write the failing route E2E test**

Add:

```rust
#[tokio::test]
async fn sqlite_export_bytes_can_be_imported_through_project_import_route() {
    let db = migrated_db().await;
    add_project(&db, "source-project", "SQLite Transfer App").await;

    let state = crate::server::tests::test_state(db.clone(), vec![]);
    let app = build_app(state, &McpConfig::disabled());

    let export_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/projects/export")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "all": true,
                        "projectIds": [],
                        "includeHistory": true
                    })
                    .to_string(),
                ))
                .expect("build export request"),
        )
        .await
        .expect("export response");

    assert_eq!(export_response.status(), StatusCode::OK);
    assert_eq!(
        export_response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/vnd.sqlite3"
    );
    let sqlite_bytes = to_bytes(export_response.into_body(), usize::MAX)
        .await
        .expect("sqlite bytes");
    assert!(sqlite_bytes.starts_with(b"SQLite format 3"));

    let import_response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/projects/import?includeHistory=true")
                .header(header::CONTENT_TYPE, "application/vnd.sqlite3")
                .body(Body::from(sqlite_bytes))
                .expect("build import request"),
        )
        .await
        .expect("import response");

    assert_eq!(import_response.status(), StatusCode::CREATED);
    let body = to_bytes(import_response.into_body(), usize::MAX)
        .await
        .expect("import body");
    let payload: Value = serde_json::from_slice(&body).expect("import json");
    assert_eq!(payload["projectsImported"], 1);
    assert_eq!(payload["projects"][0]["projectName"], "SQLite Transfer App-imported");

    let names = project_names(&db).await;
    assert!(names.contains(&"SQLite Transfer App".to_owned()));
    assert!(names.contains(&"SQLite Transfer App-imported".to_owned()));
}
```

- [ ] **Step 4: Write the invalid selection route test**

Add:

```rust
#[tokio::test]
async fn sqlite_export_requires_all_or_project_ids() {
    let db = migrated_db().await;
    let state = crate::server::tests::test_state(db, vec![]);
    let app = build_app(state, &McpConfig::disabled());

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/projects/export")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "all": false,
                        "projectIds": [],
                        "includeHistory": true
                    })
                    .to_string(),
                ))
                .expect("build request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let payload: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(payload["message"], "set all=true or provide projectIds");
}
```

- [ ] **Step 5: Run the API tests**

Run:

```bash
cargo test -p previa-main sqlite_export -- --nocapture
```

Expected: the two new tests pass. If `crate::server::tests::test_state` is private, move the tests to `main/src/server/mod.rs` or make a small local `AppState` helper inside `transfers.rs` tests using the pattern from `server/mod.rs`.

---

### Task 2: CLI E2E Coverage for `previa local export/import`

**Files:**
- Modify: `previa/tests/cli.rs`

- [ ] **Step 1: Add helpers near existing test helpers**

Add:

```rust
fn write_local_runtime_state(home: &Path, port: u16) {
    let run_dir = home.join("stacks/default/run");
    fs::create_dir_all(&run_dir).expect("create run dir");
    fs::write(
        run_dir.join("state.json"),
        format!(
            r#"{{
  "name": "default",
  "mode": "detached",
  "started_at": "2026-04-30T00:00:00Z",
  "backend": "bin",
  "image_tag": "test",
  "compose_file": "",
  "compose_project": "",
  "main": {{ "service_name": "previa-main", "pid": 0, "address": "127.0.0.1", "port": {port}, "log_path": "" }},
  "runner_port_range": {{ "start": 55880, "end": 55889 }},
  "attached_runners": [],
  "runners": []
}}"#
        ),
    )
    .expect("write runtime state");
}

fn create_project(base_url: &str, name: &str) -> String {
    let output = Command::new("curl")
        .args([
            "-fsS",
            &format!("{base_url}/api/v1/projects"),
            "-H",
            "content-type: application/json",
            "-d",
            &format!(
                r#"{{"name":"{name}","description":null,"spec":null,"pipelines":[]}}"#
            ),
        ])
        .output()
        .expect("curl create project");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let payload: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("create project json");
    payload["id"].as_str().expect("project id").to_owned()
}

fn list_project_names(base_url: &str) -> Vec<String> {
    let output = Command::new("curl")
        .args(["-fsS", &format!("{base_url}/api/v1/projects")])
        .output()
        .expect("curl list projects");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice::<Vec<serde_json::Value>>(&output.stdout)
        .expect("projects json")
        .into_iter()
        .map(|project| project["name"].as_str().expect("name").to_owned())
        .collect()
}
```

- [ ] **Step 2: Add spawned `previa-main` guard**

Add:

```rust
struct MainGuard {
    child: std::process::Child,
}

impl Drop for MainGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_real_main(port: u16, db_path: &Path) -> MainGuard {
    let mut child = Command::new(env!("CARGO_BIN_EXE_previa-main"))
        .env("ADDRESS", "127.0.0.1")
        .env("PORT", port.to_string())
        .env("ORCHESTRATOR_DATABASE_URL", format!("sqlite://{}", db_path.display()))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn previa-main");

    let base_url = format!("http://127.0.0.1:{port}");
    for _ in 0..80 {
        if Command::new("curl")
            .args(["-fsS", &format!("{base_url}/health")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            return MainGuard { child };
        }
        thread::sleep(Duration::from_millis(100));
    }

    let _ = child.kill();
    let _ = child.wait();
    panic!("previa-main did not become healthy");
}
```

- [ ] **Step 3: Write the CLI E2E test**

Add:

```rust
#[test]
fn local_sqlite_export_import_roundtrip_uses_real_main() {
    let temp = TempDir::new().expect("temp dir");
    let home = temp.path().join("home");
    let db_path = temp.path().join("orchestrator.db");
    let sqlite_export = temp.path().join("previa-projects.sqlite3");
    let port = free_port();
    let base_url = format!("http://127.0.0.1:{port}");
    let _main = spawn_real_main(port, &db_path);
    write_local_runtime_state(&home, port);

    create_project(&base_url, "CLI SQLite App");

    let mut export = cargo_bin();
    export
        .arg("--home")
        .arg(&home)
        .args(["local", "export", "--all", "--output"])
        .arg(&sqlite_export)
        .assert()
        .success()
        .stdout(predicates::str::contains("exported"));
    assert!(sqlite_export.exists());

    let mut import = cargo_bin();
    import
        .arg("--home")
        .arg(&home)
        .args(["local", "import"])
        .arg(&sqlite_export)
        .assert()
        .success()
        .stdout(predicates::str::contains("imported 1 project(s)"));

    let names = list_project_names(&base_url);
    assert!(names.contains(&"CLI SQLite App".to_owned()));
    assert!(names.contains(&"CLI SQLite App-imported".to_owned()));

    let mut without_overwrite = cargo_bin();
    without_overwrite
        .arg("--home")
        .arg(&home)
        .args(["local", "export", "--all", "--output"])
        .arg(&sqlite_export)
        .assert()
        .failure()
        .stderr(predicates::str::contains("pass --overwrite"));

    let mut with_overwrite = cargo_bin();
    with_overwrite
        .arg("--home")
        .arg(&home)
        .args(["local", "export", "--all", "--output"])
        .arg(&sqlite_export)
        .arg("--overwrite")
        .assert()
        .success();
}
```

- [ ] **Step 4: Run the CLI E2E test**

Run:

```bash
cargo test -p previa --test cli local_sqlite_export_import_roundtrip_uses_real_main -- --nocapture
```

Expected: test passes and leaves no running `previa-main` process after completion.

---

### Task 3: Selected Project Export Coverage

**Files:**
- Modify: `previa/tests/cli.rs`

- [ ] **Step 1: Write selected export/import test**

Add:

```rust
#[test]
fn local_sqlite_export_project_exports_only_selected_project() {
    let temp = TempDir::new().expect("temp dir");
    let home = temp.path().join("home");
    let db_path = temp.path().join("orchestrator.db");
    let sqlite_export = temp.path().join("selected.sqlite3");
    let port = free_port();
    let base_url = format!("http://127.0.0.1:{port}");
    let _main = spawn_real_main(port, &db_path);
    write_local_runtime_state(&home, port);

    let selected_id = create_project(&base_url, "Selected App");
    create_project(&base_url, "Not Selected App");

    let mut export = cargo_bin();
    export
        .arg("--home")
        .arg(&home)
        .args(["local", "export", "--project"])
        .arg(&selected_id)
        .args(["--output"])
        .arg(&sqlite_export)
        .assert()
        .success();

    let mut import = cargo_bin();
    import
        .arg("--home")
        .arg(&home)
        .args(["local", "import"])
        .arg(&sqlite_export)
        .assert()
        .success()
        .stdout(predicates::str::contains("imported 1 project(s)"));

    let names = list_project_names(&base_url);
    assert!(names.contains(&"Selected App".to_owned()));
    assert!(names.contains(&"Selected App-imported".to_owned()));
    assert!(names.contains(&"Not Selected App".to_owned()));
    assert!(!names.contains(&"Not Selected App-imported".to_owned()));
}
```

- [ ] **Step 2: Run selected export test**

Run:

```bash
cargo test -p previa --test cli local_sqlite_export_project_exports_only_selected_project -- --nocapture
```

Expected: test passes.

---

### Task 4: Full Verification

- [ ] **Step 1: Run focused tests**

```bash
cargo test -p previa-main sqlite_export -- --nocapture
cargo test -p previa --test cli local_sqlite_export -- --nocapture
```

Expected: all focused SQLite import/export tests pass.

- [ ] **Step 2: Run affected crate tests**

```bash
cargo test -p previa-main
cargo test -p previa
```

Expected: all tests pass.

- [ ] **Step 3: Run release build**

```bash
cargo build --release
```

Expected: release build succeeds.

- [ ] **Step 4: Commit and push**

```bash
git add main/src/server/handlers/transfers.rs previa/tests/cli.rs docs/superpowers/plans/2026-04-30-sqlite-import-export-e2e-tests.md
git commit -m "Add SQLite import export e2e tests"
git push origin main
```

Expected: commit is pushed to `main`.

---

## Self-Review

- Spec coverage: covers SQLite API export, SQLite API import, CLI export all, CLI import all, overwrite failure, overwrite success, and selected project export.
- Placeholder scan: no TBD/TODO/fill-in-later steps.
- Risk: `env!("CARGO_BIN_EXE_previa-main")` availability in `previa/tests/cli.rs` depends on Cargo building the `previa-main` binary for that integration test. If Cargo does not expose it, replace `spawn_real_main` with `Command::new("cargo").args(["run", "-p", "previa-main", "--"])`, keeping the same env vars and guard cleanup.
