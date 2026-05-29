# OSS V1 Docker Onboarding Errors Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Previa's first open-source release credible as a local-first QA runtime for AI agents by hardening the Docker-backed startup path, adding a minimal first-run onboarding path, and improving user/agent-readable error diagnostics.

**Architecture:** Keep the current Rust CLI, Axum API, React app, and generated OpenAPI contracts intact. Add focused diagnostic modules and UI components instead of broad rewrites. Treat Docker Compose as the default OSS v1 runtime, with `previa doctor` as the explicit preflight and troubleshooting entrypoint.

**Tech Stack:** Rust 2024, Clap, Tokio, Docker Compose CLI, Axum, React 18, Vite, Vitest, i18next, Zustand, SQLx, Utoipa.

---

## Scope Notes

- "Docker" in this plan means the Docker-backed OSS v1 runtime path: `previa up -d`, Compose generation, image availability, port conflicts, startup diagnostics, and release smoke checks.
- This plan also adds `previa doctor` because it is the practical command users and AI agents need when the Docker path fails.
- Kubernetes production, hosted SaaS, billing, and `previa link` are out of scope.
- The existing `scripts/start-local-load-target-stack.sh` remains a developer validation helper. OSS v1 onboarding should not require local source builds.

## File Structure

- Modify `previa/src/cli.rs`: add `DoctorArgs` and `Commands::Doctor`.
- Modify `previa/src/lib.rs`: route `doctor`, run Docker preflight before Compose startup, and print actionable hints on Compose startup failures.
- Create `previa/src/diagnostics.rs`: pure-ish diagnostics model plus Docker/Compose/port/image checks.
- Modify `previa/src/compose.rs`: expose Compose CLI detection result for diagnostics without shelling out twice where possible.
- Modify `previa/tests/cli.rs`: add end-to-end CLI tests for `doctor` and improved Docker startup failures.
- Modify `docs/previa/getting-started.md`, `docs/previa/troubleshooting.md`, and `docs/previa/release-install.md`: document the Docker-first flow and `doctor`.
- Create `app/src/components/AgentRuntimeOnboarding.tsx`: a compact first-run panel for empty projects.
- Modify `app/src/pages/ProjectsPage.tsx`: show onboarding panel in the empty state and wire create/import/open docs actions.
- Modify `app/src/components/OnboardingModal.tsx`: update copy from generic product guide to "QA runtime for AI agents" and fix install command if stale.
- Modify `app/src/i18n/locales/en.json` and `app/src/i18n/locales/pt-BR.json`: add onboarding/error copy for the primary supported launch languages.
- Modify `app/src/pages/ProjectsPage.test.tsx` and `app/src/components/AppShell.test.tsx`: cover onboarding behavior.
- Create `app/src/lib/api-errors.ts`: parse backend error payloads into stable, user-readable categories.
- Modify `app/src/lib/api-client.ts` and `app/src/lib/auth-client.ts`: reuse parsed API errors and preserve response metadata.
- Modify selected UI consumers in `app/src/stores/useProjectStore.ts`, `app/src/pages/RunnersPage.tsx`, and execution result components only where generic messages remain.
- Modify `main/src/server/errors.rs` only if a missing backend error code blocks client categorization.

---

### Task 1: Add CLI Docker Diagnostics Foundation

**Files:**
- Modify: `previa/src/cli.rs`
- Modify: `previa/src/lib.rs`
- Create: `previa/src/diagnostics.rs`
- Modify: `previa/src/compose.rs`
- Test: `previa/src/cli.rs`
- Test: `previa/tests/cli.rs`

- [ ] **Step 1: Add failing parser tests for `doctor`**

Add to the existing `#[cfg(test)]` module in `previa/src/cli.rs`:

```rust
#[test]
fn parses_doctor_with_default_context() {
    let cli = Cli::try_parse_from(["previa", "doctor"]).expect("parse doctor");
    match cli.command {
        Commands::Doctor(args) => {
            assert_eq!(args.context, "default");
            assert!(!args.json);
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn parses_doctor_with_context_and_json() {
    let cli = Cli::try_parse_from(["previa", "doctor", "--context", "demo", "--json"])
        .expect("parse doctor json");
    match cli.command {
        Commands::Doctor(args) => {
            assert_eq!(args.context, "demo");
            assert!(args.json);
        }
        other => panic!("unexpected command: {other:?}"),
    }
}
```

- [ ] **Step 2: Run parser tests and verify failure**

Run:

```bash
cargo test -p previa cli::tests::parses_doctor
```

Expected: compile failure because `Commands::Doctor` and `DoctorArgs` do not exist.

- [ ] **Step 3: Add `DoctorArgs` and command routing**

In `previa/src/cli.rs`, add the command variant near `Status`:

```rust
#[command(about = "Check the local Previa runtime prerequisites and current context")]
Doctor(DoctorArgs),
```

Add args near `StatusArgs`:

```rust
#[derive(Debug, Args)]
#[command(about = "Check the local Previa runtime prerequisites and current context")]
pub struct DoctorArgs {
    #[arg(
        long = "context",
        value_name = "CONTEXT",
        default_value = "default",
        help = "Context name"
    )]
    pub context: String,
    #[arg(long)]
    pub json: bool,
}
```

In `previa/src/lib.rs`, add `DoctorArgs` to the `use crate::cli::{ ... }` list and route it:

```rust
Commands::Doctor(args) => cmd_doctor(&paths, &http, args).await,
```

- [ ] **Step 4: Create diagnostics model with injectable probes**

Create `previa/src/diagnostics.rs`:

```rust
use std::net::{TcpListener, ToSocketAddrs};
use std::path::Path;
use std::process::{Command as StdCommand, Stdio};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::runtime::DetachedRuntimeState;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticStatus {
    Ok,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticCheck {
    pub id: String,
    pub status: DiagnosticStatus,
    pub summary: String,
    pub detail: String,
    pub action: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    pub context: String,
    pub overall: DiagnosticStatus,
    pub checks: Vec<DiagnosticCheck>,
}

pub fn command_available(program: &str, args: &[&str]) -> bool {
    StdCommand::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn check_docker_compose() -> DiagnosticCheck {
    if command_available("docker", &["compose", "version"]) {
        return DiagnosticCheck {
            id: "docker-compose".to_owned(),
            status: DiagnosticStatus::Ok,
            summary: "Docker Compose is available".to_owned(),
            detail: "`docker compose version` succeeded.".to_owned(),
            action: "Run `previa up -d` to start the default Docker-backed runtime.".to_owned(),
        };
    }

    if command_available("docker-compose", &["version"]) {
        return DiagnosticCheck {
            id: "docker-compose".to_owned(),
            status: DiagnosticStatus::Ok,
            summary: "docker-compose is available".to_owned(),
            detail: "`docker-compose version` succeeded.".to_owned(),
            action: "Run `previa up -d` to start the default Docker-backed runtime.".to_owned(),
        };
    }

    DiagnosticCheck {
        id: "docker-compose".to_owned(),
        status: DiagnosticStatus::Error,
        summary: "Docker Compose was not found".to_owned(),
        detail: "Neither `docker compose version` nor `docker-compose version` succeeded.".to_owned(),
        action: "Install Docker Desktop or Docker Engine with the Compose plugin, then rerun `previa doctor`.".to_owned(),
    }
}

pub fn check_port_available(host: &str, port: u16, label: &str) -> DiagnosticCheck {
    let bind_host = if host == "0.0.0.0" { "127.0.0.1" } else { host };
    let address = format!("{bind_host}:{port}");
    let available = address
        .to_socket_addrs()
        .ok()
        .and_then(|mut addrs| addrs.next())
        .map(|addr| TcpListener::bind(addr).is_ok())
        .unwrap_or(false);

    if available {
        DiagnosticCheck {
            id: format!("port-{port}"),
            status: DiagnosticStatus::Ok,
            summary: format!("{label} port {port} is available"),
            detail: format!("Previa can bind {address}."),
            action: "No action required.".to_owned(),
        }
    } else {
        DiagnosticCheck {
            id: format!("port-{port}"),
            status: DiagnosticStatus::Error,
            summary: format!("{label} port {port} is already in use"),
            detail: format!("Previa could not bind {address}."),
            action: format!("Stop the process using port {port}, or pass `--main-port` / `--runner-port-range` with free ports."),
        }
    }
}

pub fn report_status(checks: &[DiagnosticCheck]) -> DiagnosticStatus {
    if checks.iter().any(|check| check.status == DiagnosticStatus::Error) {
        DiagnosticStatus::Error
    } else if checks.iter().any(|check| check.status == DiagnosticStatus::Warning) {
        DiagnosticStatus::Warning
    } else {
        DiagnosticStatus::Ok
    }
}

pub fn runtime_state_check(path: &Path, state: Option<&DetachedRuntimeState>) -> Result<DiagnosticCheck> {
    Ok(match state {
        Some(state) => DiagnosticCheck {
            id: "runtime-state".to_owned(),
            status: DiagnosticStatus::Ok,
            summary: format!("Context '{}' has runtime state", state.name),
            detail: format!("Runtime state file: {}", path.display()),
            action: "Run `previa status` for live health details.".to_owned(),
        },
        None => DiagnosticCheck {
            id: "runtime-state".to_owned(),
            status: DiagnosticStatus::Warning,
            summary: "Context is not running".to_owned(),
            detail: format!("No runtime state file found at {}.", path.display()),
            action: "Run `previa up -d` to start this context.".to_owned(),
        },
    })
}
```

In `previa/src/lib.rs`, register the module:

```rust
mod diagnostics;
```

- [ ] **Step 5: Add `cmd_doctor` human and JSON output**

Add to `previa/src/lib.rs` near `cmd_status`:

```rust
async fn cmd_doctor(paths: &PreviaPaths, _http: &Client, args: DoctorArgs) -> Result<()> {
    let stack_name = parse_stack_name(&args.context)?;
    let stack_paths = paths.stack(&stack_name);
    let state = read_runtime_state(&stack_paths)?;
    let mut checks = vec![
        diagnostics::check_docker_compose(),
        diagnostics::runtime_state_check(&stack_paths.runtime_file, state.as_ref())?,
    ];

    if state.is_none() {
        checks.push(diagnostics::check_port_available("127.0.0.1", 5588, "main"));
        checks.push(diagnostics::check_port_available("127.0.0.1", 55880, "runner"));
    }

    let report = diagnostics::DoctorReport {
        context: stack_name,
        overall: diagnostics::report_status(&checks),
        checks,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("Previa doctor: {} ({:?})", report.context, report.overall);
    for check in &report.checks {
        println!("- [{:?}] {}", check.status, check.summary);
        println!("  {}", check.detail);
        println!("  Next: {}", check.action);
    }
    if report.overall == diagnostics::DiagnosticStatus::Error {
        anyhow::bail!("one or more checks failed");
    }
    Ok(())
}
```

- [ ] **Step 6: Run focused tests**

Run:

```bash
cargo test -p previa cli::tests::parses_doctor
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add previa/src/cli.rs previa/src/lib.rs previa/src/diagnostics.rs
git commit -m "feat: add previa doctor diagnostics"
```

---

### Task 2: Harden Docker Compose Startup Errors

**Files:**
- Modify: `previa/src/lib.rs`
- Modify: `previa/src/compose.rs`
- Modify: `previa/tests/cli.rs`
- Docs: `docs/previa/troubleshooting.md`

- [ ] **Step 1: Add failing CLI test for missing Compose guidance**

In `previa/tests/cli.rs`, add a test that prepends an empty temp `PATH` and asserts the error mentions `previa doctor`:

```rust
#[test]
fn up_reports_doctor_hint_when_compose_is_missing() {
    let temp = TempDir::new().expect("temp home");
    let empty_bin = temp.path().join("empty-bin");
    fs::create_dir_all(&empty_bin).expect("empty bin");

    let mut command = Command::cargo_bin("previa").expect("previa binary");
    command
        .env("PATH", empty_bin)
        .arg("--home")
        .arg(temp.path())
        .args(["up", "-d"]);

    command
        .assert()
        .failure()
        .stderr(predicates::str::contains("previa doctor"));
}
```

Add this import if missing:

```rust
use predicates::prelude::*;
```

- [ ] **Step 2: Run the focused test and verify failure**

Run:

```bash
cargo test -p previa --test cli up_reports_doctor_hint_when_compose_is_missing
```

Expected: FAIL because current Compose errors do not mention `previa doctor`.

- [ ] **Step 3: Improve Compose spawn and resolution errors**

In `previa/src/compose.rs`, update `docker_spawn_error`:

```rust
fn docker_spawn_error(description: &str) -> String {
    format!(
        "{description}: failed to spawn Docker Compose; install Docker Desktop or Docker Engine with the Compose plugin, then run `previa doctor` for a full local runtime check"
    )
}
```

Update `resolve_compose_cli_with` missing Compose error:

```rust
bail!(
    "failed to find Docker Compose; install Docker Desktop or Docker Engine with the Compose plugin, then run `previa doctor` for a full local runtime check"
)
```

Update the existing unit assertion in `errors_when_no_compose_runtime_is_available` to check the new phrase:

```rust
assert!(
    err.to_string().contains("previa doctor"),
    "unexpected error: {err}"
);
```

- [ ] **Step 4: Add Docker preflight before Compose startup**

In `previa/src/lib.rs`, inside `cmd_up`, before `write_generated_compose(&resolved)?` in the `RuntimeBackend::Compose` branch, add:

```rust
let docker_check = diagnostics::check_docker_compose();
if docker_check.status == diagnostics::DiagnosticStatus::Error {
    bail!("{}; {}", docker_check.summary, docker_check.action);
}
```

- [ ] **Step 5: Run focused tests**

Run:

```bash
cargo test -p previa compose::tests::errors_when_no_compose_runtime_is_available
cargo test -p previa --test cli up_reports_doctor_hint_when_compose_is_missing
```

Expected: PASS.

- [ ] **Step 6: Document Docker troubleshooting**

In `docs/previa/troubleshooting.md`, add a top-level section:

```markdown
## Docker-backed startup fails

Run:

```bash
previa doctor
```

The default OSS runtime uses Docker Compose. If `previa up -d` cannot start,
check:

- Docker Desktop or Docker Engine is running.
- `docker compose version` or `docker-compose version` succeeds.
- ports `5588` and `55880` are free, or start with alternate ports:

```bash
previa up -d --main-port 5688 --runner-port-range 56880:56979
```

- published images are pullable:

```bash
previa pull all
```
```

- [ ] **Step 7: Commit**

```bash
git add previa/src/lib.rs previa/src/compose.rs previa/tests/cli.rs docs/previa/troubleshooting.md
git commit -m "fix: improve docker startup diagnostics"
```

---

### Task 3: Add Minimal Agent-runtime Onboarding

**Files:**
- Create: `app/src/components/AgentRuntimeOnboarding.tsx`
- Modify: `app/src/pages/ProjectsPage.tsx`
- Modify: `app/src/i18n/locales/en.json`
- Modify: `app/src/i18n/locales/pt-BR.json`
- Test: `app/src/pages/ProjectsPage.test.tsx`

- [ ] **Step 1: Add failing ProjectsPage empty-state test**

In `app/src/pages/ProjectsPage.test.tsx`, add translations:

```ts
"onboarding.agent.title": "Give your agent a QA runtime",
"onboarding.agent.description": "Create a stack, import an OpenAPI spec, and let your agent run real workflows instead of guessing.",
"onboarding.agent.create": "Create stack",
"onboarding.agent.import": "Import stack",
"onboarding.agent.docs": "Open docs",
```

Add test:

```ts
it("shows agent-runtime onboarding when there are no stacks", () => {
  projectStoreMock.projects = [];

  renderPage();

  expect(screen.getByText("Give your agent a QA runtime")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Create stack" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Import stack" })).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Open docs" })).toHaveAttribute(
    "href",
    "https://github.com/runvibe/previa/tree/main/docs/previa",
  );
});
```

- [ ] **Step 2: Run test and verify failure**

Run:

```bash
npm --prefix app test -- src/pages/ProjectsPage.test.tsx
```

Expected: FAIL because `AgentRuntimeOnboarding` does not exist.

- [ ] **Step 3: Create `AgentRuntimeOnboarding`**

Create `app/src/components/AgentRuntimeOnboarding.tsx`:

```tsx
import { Bot, FileUp, FolderPlus, ExternalLink } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";

interface AgentRuntimeOnboardingProps {
  onCreateStack: () => void;
  onImportStack: () => void;
}

export function AgentRuntimeOnboarding({
  onCreateStack,
  onImportStack,
}: AgentRuntimeOnboardingProps) {
  const { t } = useTranslation();

  return (
    <section className="flex flex-col items-center justify-center rounded-lg border border-dashed border-border/50 px-4 py-12 text-center sm:py-16">
      <div className="mb-4 rounded-lg border border-border/70 bg-muted p-4">
        <Bot className="h-8 w-8 text-foreground" aria-hidden="true" />
      </div>
      <h3 className="mb-2 text-lg font-semibold">{t("onboarding.agent.title")}</h3>
      <p className="mb-6 max-w-xl text-sm leading-6 text-muted-foreground sm:text-base">
        {t("onboarding.agent.description")}
      </p>
      <div className="flex flex-col gap-2 sm:flex-row">
        <Button type="button" onClick={onCreateStack}>
          <FolderPlus className="h-4 w-4" aria-hidden="true" />
          {t("onboarding.agent.create")}
        </Button>
        <Button type="button" variant="outline" onClick={onImportStack}>
          <FileUp className="h-4 w-4" aria-hidden="true" />
          {t("onboarding.agent.import")}
        </Button>
        <Button type="button" variant="ghost" asChild>
          <a
            href="https://github.com/runvibe/previa/tree/main/docs/previa"
            target="_blank"
            rel="noreferrer"
          >
            <ExternalLink className="h-4 w-4" aria-hidden="true" />
            {t("onboarding.agent.docs")}
          </a>
        </Button>
      </div>
    </section>
  );
}
```

- [ ] **Step 4: Replace ProjectsPage empty state**

In `app/src/pages/ProjectsPage.tsx`, import the component:

```tsx
import { AgentRuntimeOnboarding } from "@/components/AgentRuntimeOnboarding";
```

Replace the final empty-state block with:

```tsx
<AgentRuntimeOnboarding
  onCreateStack={handleCreateProject}
  onImportStack={() => fileInputRef.current?.click()}
/>
```

Remove unused `FolderOpen` import if no longer used.

- [ ] **Step 5: Add English and Portuguese copy**

In `app/src/i18n/locales/en.json`, add near project strings:

```json
"onboarding.agent.title": "Give your agent a QA runtime",
"onboarding.agent.description": "Create a stack, import an OpenAPI spec, and let your agent run real workflows instead of guessing.",
"onboarding.agent.create": "Create stack",
"onboarding.agent.import": "Import stack",
"onboarding.agent.docs": "Open docs",
```

In `app/src/i18n/locales/pt-BR.json`, add:

```json
"onboarding.agent.title": "Dê um runtime de QA ao seu agente",
"onboarding.agent.description": "Crie uma stack, importe uma spec OpenAPI e deixe seu agente rodar fluxos reais em vez de apenas inferir.",
"onboarding.agent.create": "Criar stack",
"onboarding.agent.import": "Importar stack",
"onboarding.agent.docs": "Abrir docs",
```

- [ ] **Step 6: Run focused test**

Run:

```bash
npm --prefix app test -- src/pages/ProjectsPage.test.tsx
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add app/src/components/AgentRuntimeOnboarding.tsx app/src/pages/ProjectsPage.tsx app/src/pages/ProjectsPage.test.tsx app/src/i18n/locales/en.json app/src/i18n/locales/pt-BR.json
git commit -m "feat: add agent runtime onboarding"
```

---

### Task 4: Reposition Existing Onboarding Modal for OSS v1

**Files:**
- Modify: `app/src/components/OnboardingModal.tsx`
- Modify: `app/src/i18n/locales/en.json`
- Modify: `app/src/i18n/locales/pt-BR.json`
- Test: `app/src/components/AppShell.test.tsx`

- [ ] **Step 1: Add AppShell test for first-run guide copy**

In `app/src/components/AppShell.test.tsx`, stop fully mocking `OnboardingModal` for one test or add a focused `OnboardingModal` test file if the shell mock is too broad. The expected assertions are:

```tsx
expect(screen.getByText("QA runtime for AI agents")).toBeInTheDocument();
expect(screen.getByText("previa up -d")).toBeInTheDocument();
expect(screen.getByText("previa mcp install codex")).toBeInTheDocument();
```

- [ ] **Step 2: Run focused test and verify failure**

Run:

```bash
npm --prefix app test -- src/components/AppShell.test.tsx
```

Expected: FAIL because current guide copy still says generic onboarding and uses stale install/start commands.

- [ ] **Step 3: Update onboarding command snippets**

In `app/src/components/OnboardingModal.tsx`, change the install command:

```tsx
<CodeBlock>curl -fsSL https://raw.githubusercontent.com/runvibe/previa/main/install.sh | sh</CodeBlock>
```

Change first-run commands:

```tsx
<CodeBlock>previa up -d</CodeBlock>
<CodeBlock>previa open</CodeBlock>
<CodeBlock>previa mcp install codex --scope project</CodeBlock>
```

Keep the modal compact; do not add a multi-step wizard.

- [ ] **Step 4: Update English and Portuguese guide copy**

In `app/src/i18n/locales/en.json`, update the relevant `guide.*` values to say:

```json
"guide.title": "QA runtime for AI agents",
"guide.start.title": "Stop letting agents guess",
"guide.start.description": "Previa gives AI agents a local runtime where they can create, run, inspect, and debug real API workflows.",
"guide.start.flowTitle": "Agent loop",
"guide.start.flowDescription": "Import a spec, create a pipeline, run it, inspect failures, and rerun after the code changes.",
"guide.install.description": "Install the CLI, start the Docker-backed runtime, and connect your agent through MCP when needed.",
"guide.firstRun.step1": "Start the local Docker-backed runtime",
"guide.firstRun.step2": "Open the browser IDE",
"guide.firstRun.step3": "Install MCP for your agent"
```

In `app/src/i18n/locales/pt-BR.json`, update:

```json
"guide.title": "Runtime de QA para agentes de IA",
"guide.start.title": "Pare de deixar agentes inferirem",
"guide.start.description": "O Previa dá aos agentes de IA um runtime local para criar, executar, inspecionar e depurar fluxos reais de API.",
"guide.start.flowTitle": "Loop do agente",
"guide.start.flowDescription": "Importe uma spec, crie uma pipeline, execute, inspecione falhas e rode novamente após mudanças no código.",
"guide.install.description": "Instale o CLI, suba o runtime via Docker e conecte seu agente por MCP quando necessário.",
"guide.firstRun.step1": "Suba o runtime local via Docker",
"guide.firstRun.step2": "Abra a IDE no navegador",
"guide.firstRun.step3": "Instale o MCP para seu agente"
```

- [ ] **Step 5: Run focused tests**

Run:

```bash
npm --prefix app test -- src/components/AppShell.test.tsx src/pages/ProjectsPage.test.tsx
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add app/src/components/OnboardingModal.tsx app/src/i18n/locales/en.json app/src/i18n/locales/pt-BR.json app/src/components/AppShell.test.tsx
git commit -m "docs: reposition onboarding for ai agents"
```

---

### Task 5: Add Structured API Error Parsing in the UI

**Files:**
- Create: `app/src/lib/api-errors.ts`
- Modify: `app/src/lib/api-client.ts`
- Modify: `app/src/lib/auth-client.ts`
- Test: `app/src/lib/api-client.test.ts`

- [ ] **Step 1: Add failing API error parsing tests**

In `app/src/lib/api-client.test.ts`, import new symbols:

```ts
import { parseApiErrorText, userFacingApiErrorMessage } from "@/lib/api-errors";
```

Add tests:

```ts
describe("api error parsing", () => {
  it("parses structured backend errors", () => {
    expect(parseApiErrorText('{"error":"not_found","message":"project not found"}')).toEqual({
      code: "not_found",
      message: "project not found",
      raw: '{"error":"not_found","message":"project not found"}',
    });
  });

  it("maps network and auth categories to useful messages", () => {
    expect(userFacingApiErrorMessage({ code: "service_unavailable", message: "runner unavailable", raw: "" }))
      .toBe("Service unavailable: runner unavailable");
    expect(userFacingApiErrorMessage({ code: "forbidden", message: "forbidden", raw: "" }))
      .toBe("You do not have permission to perform this action.");
  });
});
```

- [ ] **Step 2: Run focused test and verify failure**

Run:

```bash
npm --prefix app test -- src/lib/api-client.test.ts
```

Expected: FAIL because `app/src/lib/api-errors.ts` does not exist.

- [ ] **Step 3: Create API error parser**

Create `app/src/lib/api-errors.ts`:

```ts
export interface ParsedApiError {
  code: string;
  message: string;
  raw: string;
}

export function parseApiErrorText(text: string): ParsedApiError {
  const raw = text ?? "";
  try {
    const payload = JSON.parse(raw) as { error?: unknown; message?: unknown };
    const code = typeof payload.error === "string" && payload.error.trim()
      ? payload.error
      : "http_error";
    const message = typeof payload.message === "string" && payload.message.trim()
      ? payload.message
      : raw || "Request failed";
    return { code, message, raw };
  } catch {
    return {
      code: "http_error",
      message: raw || "Request failed",
      raw,
    };
  }
}

export function userFacingApiErrorMessage(error: ParsedApiError): string {
  switch (error.code) {
    case "forbidden":
      return "You do not have permission to perform this action.";
    case "unauthorized":
      return "Authentication is required. Sign in again or configure an API token.";
    case "not_found":
      return `Not found: ${error.message}`;
    case "service_unavailable":
      return `Service unavailable: ${error.message}`;
    case "bad_request":
      return `Invalid request: ${error.message}`;
    case "conflict":
      return `Conflict: ${error.message}`;
    default:
      return error.message;
  }
}
```

- [ ] **Step 4: Extend `ApiError`**

In `app/src/lib/api-client.ts`, import:

```ts
import { parseApiErrorText, userFacingApiErrorMessage, type ParsedApiError } from "@/lib/api-errors";
```

Change `ApiError`:

```ts
export class ApiError extends Error {
  readonly parsed: ParsedApiError;
  readonly userMessage: string;

  constructor(
    message: string,
    readonly statusCode: number,
    readonly responseText: string,
  ) {
    super(message);
    this.name = "ApiError";
    this.parsed = parseApiErrorText(responseText);
    this.userMessage = userFacingApiErrorMessage(this.parsed);
  }
}
```

Change `apiErrorMessage`:

```ts
export function apiErrorMessage(
  error: unknown,
  fallbackMessage: string,
  permissionMessage = "Você não tem permissão para realizar esta ação.",
): string {
  if (isForbiddenApiError(error)) return permissionMessage;
  if (error instanceof ApiError) return error.userMessage;
  return fallbackMessage;
}
```

- [ ] **Step 5: Use parsed messages in event store entries**

In `request<T>`, after reading `text`, add:

```ts
const parsed = parseApiErrorText(text);
```

Change event message:

```ts
message: userFacingApiErrorMessage(parsed),
details: { method, url, statusCode, code: parsed.code, raw: parsed.raw },
```

- [ ] **Step 6: Reuse `ApiError` in auth client without duplicate parsing**

No API shape change is needed in `app/src/lib/auth-client.ts` because it already throws `ApiError`. Confirm imports still compile.

- [ ] **Step 7: Run focused tests**

Run:

```bash
npm --prefix app test -- src/lib/api-client.test.ts src/lib/auth-client.test.ts
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add app/src/lib/api-errors.ts app/src/lib/api-client.ts app/src/lib/auth-client.ts app/src/lib/api-client.test.ts
git commit -m "feat: parse structured api errors in app"
```

---

### Task 6: Replace Generic Toasts on Primary First-run Surfaces

**Files:**
- Modify: `app/src/stores/useProjectStore.ts`
- Modify: `app/src/pages/RunnersPage.tsx`
- Modify: execution result components if they still collapse backend details into generic errors
- Test: `app/src/stores/useProjectStore.test.ts`
- Test: `app/src/pages/RunnersPage.test.tsx`

- [ ] **Step 1: Add failing store test for structured load-project errors**

In `app/src/stores/useProjectStore.test.ts`, add a test where `api.listProjects` rejects with:

```ts
new api.ApiError(
  'HTTP 503: {"error":"service_unavailable","message":"runner registry unavailable"}',
  503,
  '{"error":"service_unavailable","message":"runner registry unavailable"}',
)
```

Expected toast:

```ts
expect(toastErrorMock).toHaveBeenCalledWith("Service unavailable: runner registry unavailable");
```

- [ ] **Step 2: Run focused test and verify failure**

Run:

```bash
npm --prefix app test -- src/stores/useProjectStore.test.ts
```

Expected: FAIL if the store still uses only translation fallback keys.

- [ ] **Step 3: Update project store toast calls**

In `app/src/stores/useProjectStore.ts`, replace calls shaped like:

```ts
toast.error(i18n.t(api.apiErrorTranslationKey(err, "store.loadProjectsError")));
```

with:

```ts
toast.error(api.apiErrorMessage(err, i18n.t("store.loadProjectsError"), i18n.t("store.permissionDeniedError")));
```

Apply the same pattern for:

- `store.loadProjectError`
- `store.createProjectError`
- `store.syncProjectError`
- spec/env group/pipeline CRUD fallback errors in this store

- [ ] **Step 4: Update RunnersPage generic errors**

In `app/src/pages/RunnersPage.tsx`, for `catch (err)` branches, use:

```ts
toast.error(apiErrorMessage(err, t("runners.loadError"), t("store.permissionDeniedError")));
```

Import the helper from `@/lib/api-client` if not already imported.

- [ ] **Step 5: Run focused tests**

Run:

```bash
npm --prefix app test -- src/stores/useProjectStore.test.ts src/pages/RunnersPage.test.tsx
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add app/src/stores/useProjectStore.ts app/src/stores/useProjectStore.test.ts app/src/pages/RunnersPage.tsx app/src/pages/RunnersPage.test.tsx
git commit -m "fix: surface actionable api errors"
```

---

### Task 7: Document the OSS v1 Agent-runtime Path

**Files:**
- Modify: `README.md`
- Modify: `docs/previa/getting-started.md`
- Modify: `docs/previa/mcp.md`
- Modify: `docs/previa/troubleshooting.md`
- Modify: `docs/previa/release-install.md`

- [ ] **Step 1: Update README top positioning**

Replace the current one-line product claim with:

```markdown
**Previa is a local-first QA runtime for AI agents. It gives assistants a real API testing environment to create, run, inspect, and debug end-to-end workflows instead of guessing.**
```

Add a short "Agent loop" section:

```markdown
## Agent Loop

```text
agent -> MCP/API -> previa-main -> previa-runner -> target API -> structured execution result -> agent
```

Use Previa when an agent needs to verify a real workflow:

1. Start the local runtime with `previa up -d`.
2. Open the IDE with `previa open`.
3. Import an OpenAPI spec or create a stack.
4. Let the agent create or update a pipeline through MCP/API.
5. Run the pipeline and inspect step-level request, response, assertion, and error details.
```

- [ ] **Step 2: Add Docker-first getting-started flow**

In `docs/previa/getting-started.md`, make the first command path:

```bash
previa doctor
previa up -d
previa open
```

Mention `previa doctor` before troubleshooting.

- [ ] **Step 3: Add MCP prompt example**

In `docs/previa/mcp.md`, add:

```markdown
## Agent Prompt Example

Use Previa as the QA runtime for this API change. Inspect the current project,
create or update a pipeline for the changed workflow, run the E2E test, and
summarize any failing step with request, response, assertion, and suggested fix.
Do not mark the change verified until the Previa execution passes.
```

- [ ] **Step 4: Update release-install docs**

In `docs/previa/release-install.md`, add `previa doctor` as the first post-install verification:

```bash
previa doctor
previa pull all
previa up -d
previa status
```

- [ ] **Step 5: Commit**

```bash
git add README.md docs/previa/getting-started.md docs/previa/mcp.md docs/previa/troubleshooting.md docs/previa/release-install.md
git commit -m "docs: document oss agent runtime path"
```

---

### Task 8: Full Validation and Release Smoke

**Files:**
- No code files unless prior tasks reveal a regression.
- Optional docs update if smoke commands expose stale docs.

- [ ] **Step 1: Run Rust tests**

Run:

```bash
cargo test --workspace
```

Expected: all Rust tests pass.

- [ ] **Step 2: Run app tests**

Run:

```bash
npm --prefix app test
```

Expected: all Vitest tests pass.

- [ ] **Step 3: Run OpenAPI/client drift checks**

Run:

```bash
cargo test -p previa-main server::docs
python3 scripts/check_openapi_client_contract.py
```

Expected: both pass.

- [ ] **Step 4: Run release build**

Run:

```bash
cargo build --release
```

Expected: release build succeeds.

- [ ] **Step 5: Run local Docker smoke**

Run:

```bash
target/release/previa --home /tmp/previa-oss-v1-smoke doctor
target/release/previa --home /tmp/previa-oss-v1-smoke up -d
target/release/previa --home /tmp/previa-oss-v1-smoke status
target/release/previa --home /tmp/previa-oss-v1-smoke down
```

Expected:

- `doctor` reports Docker Compose available or gives actionable Docker guidance.
- `up -d` starts the Docker-backed context.
- `status` reports main and runner state.
- `down` removes the context.

- [ ] **Step 6: Commit final fixes if needed**

Only if validation required edits:

```bash
git add <changed-files>
git commit -m "fix: stabilize oss v1 smoke validation"
```

---

## Self-review

- Scope coverage: Docker startup and diagnostics are covered by Tasks 1, 2, and 8. Minimal onboarding is covered by Tasks 3 and 4. Improved errors are covered by Tasks 5 and 6. OSS positioning docs are covered by Task 7.
- Placeholder scan: no unfinished-marker or unspecified implementation blocks remain.
- Type consistency: Rust command names use `DoctorArgs` and `Commands::Doctor`; UI error parsing uses `ParsedApiError`, `parseApiErrorText`, and `userFacingApiErrorMessage`; onboarding component props are `onCreateStack` and `onImportStack`.
- Validation: final checks include Rust workspace tests, app tests, OpenAPI/client drift checks, release build, and Docker smoke.
