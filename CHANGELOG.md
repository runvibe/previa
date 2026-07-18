# Changelog

All notable changes to Previa are documented in this file.

## [v1.0.0-alpha.43] - 2026-07-18

### Bug Fixes
- Redact database credentials from startup logs (fcd232b)

### Maintenance
- Update release metadata for v1.0.0-alpha.42 (519aa92)

Full Changelog: https://github.com/runvibe/previa/compare/v1.0.0-alpha.42...v1.0.0-alpha.43

## [v1.0.0-alpha.42] - 2026-07-18

### Features
- Deploy postgres execution queue (1ff63cb)
- Project durable queued executions (803dcc4)
- Run postgres queue worker (53572ad)
- Add fenced postgres job claims (fe5b60a)
- Add postgres execution queue schema (0b13bf5)
- Add local json project transfer (f1e6695)

### Bug Fixes
- Build app before rust validation (bd33989)

### Documentation
- Add postgres queue implementation plan (e4575e9)
- Redesign runner transport around postgres queue (0fb4926)
- Specify postgres load telemetry queue (7bab233)

### Refactors
- Remove runner execution http api (4783593)

### Testing
- Migrate postgres queue fixtures (d3e6fe7)

### Maintenance
- Bump version to v1.0.0-alpha.42 (8868c04)
- Update release metadata for v1.0.0-alpha.41 (4f5211f)

Full Changelog: https://github.com/runvibe/previa/compare/v1.0.0-alpha.41...v1.0.0-alpha.42

## Unreleased

### Breaking changes

- Require Postgres for all `previa-main` operational state; SQLite is now only
  a project import/export format.
- Replace main-to-runner HTTP execution and telemetry transport with a durable,
  fenced Postgres job/event queue.
- Reduce runner HTTP routes to health, readiness, info, and OpenAPI.

### Features

- Add queue leases, retries, cancellation, recovery, projection, retention,
  runner heartbeats, load shards, diagnostics, Compose provisioning, Helm
  secret injection, and real-Postgres CI coverage.
- Add validated environment defaults for all queue timing and buffer settings.

## [v1.0.0-alpha.41] - 2026-05-29

### Features
- Parse structured api errors in app (33d5611)
- Add agent runtime onboarding (841b7e5)
- Add previa doctor diagnostics (d13905c)

### Bug Fixes
- Keep docker fake compatible with preflight (ce81ebf)
- Surface actionable api errors (ce0374a)
- Improve docker startup diagnostics (55d9ff9)
- Return ok for doctor json reports (9621e94)
- Harden previa doctor diagnostics (9e336f6)

### Documentation
- Document oss agent runtime path (6179075)
- Reposition onboarding for ai agents (f7e9e67)
- Plan oss v1 docker onboarding errors (ed04b95)

### Maintenance
- Bump version to v1.0.0-alpha.41 (a2d79fa)
- Align project contracts and test hygiene (a418b28)
- Update release metadata for v1.0.0-alpha.40 (3685452)

Full Changelog: https://github.com/runvibe/previa/compare/v1.0.0-alpha.40...v1.0.0-alpha.41

## [v1.0.0-alpha.40] - 2026-05-26

### Features
- Add production helm and durable k8s plugin state (e4edefd)

### Bug Fixes
- Allow helm service ports to differ from container ports (2ce16f2)
- App: hide empty runner rps series (5efbfde)

### Documentation
- Design previa link sync (4171c4d)

### Testing
- Rename previa workload toleration fixtures to runvibe (a4cd8ca)

### Other Changes
- Fix e2e queue load runner test mock (e51eb3c)
- Bump app version to alpha 40 (da975a1)
- Add share access levels (c30f7d8)
- Cascade stack share revocation to pipelines (f2b9caa)
- Add account settings profile controls (c62f758)
- Apply forbidden API error message broadly (fb2ee75)
- Fix forbidden stack delete handling (66837c7)
- Fix sharing API base normalization (2d6fbae)
- Add stack sharing access controls (a77fcb2)
- Merge codex/load-runner-polling-telemetry (6551025)
- Fix leng (fcef6a8)
- Add pipeline sharing access controls (2d53306)
- Replace runner load SSE with polling telemetry (e8ce711)
- Polish load test results layout (632b245)
- Add load status code timeline (ee4f61c)
- Fix stale runner reuse and load startup recovery (6487705)

Full Changelog: https://github.com/runvibe/previa/compare/k8s-validation-01de45b...v1.0.0-alpha.40

## [v1.0.0-alpha.30] - 2026-05-14

### Bug Fixes
- Publish busy runner dns endpoints (a0476b9)
- Keep kubernetes runner selectors stable (d0bcf32)

### Documentation
- Plan kubernetes runner selector fix (71e713e)
- Changelog: update for v1.0.0-alpha.23 (b8b2504)

### Maintenance
- Bump version to alpha 30 (15a36bd)
- Update release metadata for v1.0.0-alpha.23 (179899f)

### Other Changes
- Polish Previa Codex plugin (212f833)
- Surface Kubernetes cleanup failures (f5d64e9)
- Delete runner pods during reservation cleanup (c9330d3)
- Shorten Kubernetes runner resource names (c485da1)
- Install rustls provider for plugin startup (25d4d5f)
- Use newer runtime for artifact images (fb05365)
- Keep settings action on access page (eb8c695)
- Document access management modes (3eb227d)
- Prevent users from changing own role (b7e3534)
- Simplify access page header (7f7cfa6)
- Bump alpha version to 1.0.0-alpha.23 (5e0719a)
- Add access type help tooltips (b15f982)
- Move access creation into dialogs (a96335e)
- Polish access management layout (b6b5f74)
- Complete protected access workflows (4ad2d16)
- Add access management auth (b0d21b7)

Full Changelog: https://github.com/runvibe/previa/compare/k8s-validation-e7fb6eb...v1.0.0-alpha.30

## [v1.0.0-alpha.23] - 2026-05-14

### Documentation
- Plan kubernetes runner stabilization (acec8e3)
- Clarify runner access permissions (b65bf45)
- Add api token access design (5eba102)
- Define access management auth design (f897603)

### Maintenance
- Update release metadata for v1.0.0-alpha.22 (3e12182)

### Other Changes
- Prevent users from changing own role (b7e3534)
- Simplify access page header (7f7cfa6)
- Bump alpha version to 1.0.0-alpha.23 (5e0719a)
- Add access type help tooltips (b15f982)
- Move access creation into dialogs (a96335e)
- Polish access management layout (b6b5f74)
- Complete protected access workflows (4ad2d16)
- Add access management auth (b0d21b7)
- Clarify runner throttle label (d80285f)
- Send global load target RPS from UI (2997b10)
- Optimize kubernetes plugin image workflow (37e385b)
- Publish kubernetes plugin image via CI (64161fa)
- Implement kubernetes runner reservations (7bc8c08)
- Implement runner reservation foundation (fc65bac)
- Resolve runner reservation spec divergences (172b113)
- Decide AWS Karpenter v0 scope (871277e)
- Define AWS-first runner provisioning scope (bb1d286)
- Require Karpenter for dynamic runner provisioning (c57bfa9)
- Clarify portable runner node provisioning (2de2daf)
- Document Kubernetes runner reservations (71e1c17)
- Document wave load tests (ad96b8f)
- Add Previa Codex plugin scaffold (85ccc82)

Full Changelog: https://github.com/runvibe/previa/compare/v1.0.0-alpha.22...v1.0.0-alpha.23

## [v1.0.0-alpha.22] - 2026-05-11

### Features
- Expand mcp capabilities (f247e5e)

### Documentation
- Plan mcp capability upgrades (bfc7132)

### Maintenance
- Bump version to alpha 22 (51287e4)
- Update release metadata for v1.0.0-alpha.21 (cbf9380)

Full Changelog: https://github.com/runvibe/previa/compare/v1.0.0-alpha.21...v1.0.0-alpha.22

## [v1.0.0-alpha.21] - 2026-05-11

### Features
- Expose updated load MCP resources (3c1bb5d)
- Add load duration presets (a01fdf2)
- Add planned request markers to load wave (40aeacb)
- Add runner rps slider (aeb316d)
- Paginate stacks list (679ebd2)
- Gate experimental api and ai features (02fed26)
- Rerun e2e from selected step (bf36fdd)
- Add stack tags and filters (894ffb8)
- Add wave lifecycle buckets (d7344bd)
- Add wave lifecycle chart (b731dbd)
- Isolate wave sender runtime (9e63bd4)
- Add wave dispatch diagnostics (0122bd3)
- Decouple wave dispatch from response flow (8699e5b)
- Dispatch wave load by clock (014a88c)
- Chart http rps by runner (211b8f2)
- App: chart target load wave rps (4c832e5)
- App: show configured wave in load results (bfc35f1)
- App: reflect interpolation in wave graph (60ece22)
- App: add interactive wave editor (09b9431)
- App: add wave load editor (bef3dac)
- Main: forward wave load profiles (f3ecafb)
- Runner: execute wave load profiles (2d78e03)
- App: show active api version in header (ff5a91e)
- App: show collapsed test mode tooltips (5ab4ba2)
- App: add project env group workflow (12d152a)
- Main: add project env groups (56ded75)
- Add runtime env groups to engine and runner (4c7e046)

### Bug Fixes
- Combine wave graph and point controls card (f2eb17b)
- Label wave point request values (0708c45)
- Enlarge wave point markers (5082f86)
- Keep wave point markers round (2d8373a)
- Align wave chart to full width (5d2ce35)
- Keep wave edge points inside chart (b069810)
- Show wave request totals at points (095a302)
- Move wave marker values below chart (510d2eb)
- Remove request suffix from wave markers (2ecc4d9)
- Reduce wave request marker clutter (3c4001d)
- Show planned request markers in wave editor (6f2c720)
- Drain load grace period early (bce077c)
- Make stacks page scrollable (6082d9b)
- Move experimental feature toggle to settings footer (4fb8e21)
- Set e2e step play action to 16px (a594877)
- Set e2e step play action to 18px (7da725e)
- Shrink e2e step play action (6882596)
- Remove e2e step play button background (008fc40)
- Refine e2e step play icon style (d3d39e2)
- Align e2e step play action with ai action (876fd58)
- Use play icon for e2e step rerun (14f3f38)
- Make e2e step rerun action visible (e798c8a)
- Bound wave live telemetry (8b1f576)
- Rebuild load rps history from dispatch buckets (8634636)
- Sample closed dispatch rps buckets (824386a)
- Align dispatch buckets to wave time (1a8a9ef)
- Chart dispatch rps from runner buckets (20d4602)
- Bucket load rps chart by second (9428c16)
- Align load rps history with wave time (583ecd4)
- Isolate wave dispatcher runtime (1383d00)
- Run wave clock on dedicated thread (f85f7c7)
- Isolate wave scheduler with channels (ad0123a)
- Enforce open loop wave dispatch (6764acf)
- Remove wave in-flight limit (212b326)
- Surface wave load diagnostics (5a037a9)
- Reuse http client during wave load (4875e74)
- Account for delayed wave dispatch ticks (469e2a0)
- Smooth wave dispatch cadence (8155ee7)
- Load: chart wave dispatch rps (7d9f6d2)
- App: move interpolation before wave editor (a47351f)
- App: create wave points with single click (cb5e1a0)
- App: restore load config scrolling (436fe0c)
- App: clarify wave load units (e3f257d)
- Main: pin openapi version to package version (bf21a3a)
- App: hide env group entry count (9233106)
- App: reuse sidebar item action menu (719763d)
- App: move dashboard action to stack menu (4238474)
- App: enforce env menu scroll height (829cc12)
- App: make collapsed mode tooltips solid (c8f769e)
- App: limit env group menu height (f3cf13f)
- App: make modal inputs visible (7f0d35e)
- Import legacy sqlite projects without env groups (c423e8e)

### Documentation
- Plan stack tags and filters (a217400)
- Plan isolated wave http fire runtime (8e57d95)
- Plan wave open-loop sender correction (62657b7)
- Plan wave fire observer split (6fe6fde)
- Plan deterministic wave sender (d72ae43)
- Plan wave live telemetry backpressure fix (6ea62d6)
- Plan wave lifecycle buckets (c7835ef)
- Plan wave rps jitter diagnostics (98bf679)
- Plan dedicated wave dispatcher (e3c87a6)
- Plan dedicated wave clock (dfa6588)
- Plan wave scheduler channel isolation (efd9e37)
- Plan open loop wave correctness (f900bec)
- Plan wave load diagnostics corrections (c1337e6)
- Plan fire-and-observe wave dispatch (6dc2f36)
- Plan wave dispatch clock (ddddebe)
- Design http rps load chart (6be1c31)
- Document wave load tests (4213b8a)
- Plan wave load test implementation (55e8fda)
- Design wave load tests (998b40e)

### Refactors
- Decouple wave request sender (621e251)

### Maintenance
- Bump release to alpha 21 (c09f51c)
- App: update dev service worker assets (c75862a)
- App: remove inactive test tab backgrounds (a4c2b5a)
- App: improve light input fill (a46ebf9)
- App: remove input borders (1055c95)
- App: move env entry add action (b7351a4)
- Update release metadata for v1.0.0-alpha.20 (da1d8e0)

### Other Changes
- Persist test mode sidebar collapse state (fa269ad)
- Keep test mode navigation expanded on mobile (fffa564)
- Keep test mode sidebar icons when collapsed (a0ee47a)
- Add runner RPS limit to load test config (bfcf1fb)
- Improve load results layout priority (58f53de)
- Plan load results layout priority (620da0c)
- Clarify wave load diagnostics UI (40ec8fd)
- Plan wave diagnostics UI semantics (5a3aa39)
- Update lockfile for local load target (b3d6e86)
- Document local wave load target workflow (e7f65d2)
- Add local load target stack script (e52a3d2)
- Add local load target pipeline fixture (e2a322b)
- Add deterministic local load target endpoints (1fc875a)
- Add local load target crate (912247c)
- Plan local load target reference (5ff5fd0)
- Fix load test lifecycle chart axes (e115c81)
- Make mobile menus fullscreen (a8666f9)
- Schedule wave starts by deadline (5721518)
- Plan wave deadline scheduler (283d164)
- Improve wave sender start accuracy (58750be)
- Isolate wave HTTP fire path (b5e102c)
- Fix wave open-loop sender late starts (0523544)
- Refactor wave sender fire observer split (38c0781)
- Implement deterministic wave sender (4fabaf3)
- Persist test history collapse state (3e350ef)
- Toggle mobile history from bar (e15952f)
- Use history icon for collapsed mobile history (3802b12)
- Collapse mobile history downward (d426ef8)
- Allow collapsing test history panels (2747f14)
- Fix stack card duplicate and sqlite export (05124f2)
- Add stack dashboard to card menu (18c255e)
- Render test mode tooltips in portal (91c4017)
- Replace API indicator with offline toast (855053f)
- Disable stale service worker by default (98a6889)
- Load orchestrator info from app shell (7f372e7)
- Simplify app API base resolution (bae1adc)
- App: rename OpenAPI specs section (7930fec)
- Increase load request progress height (5dbb621)
- Move load elapsed time into metrics (1c77289)
- Remove load test top chart (d747562)
- Refine test mode sidebar collapse (821e08d)
- Move test mode selector to sidebar (932b958)
- Fix local default context prompt (f2502cb)
- Restore runner resource charts from load history (6f7121f)
- Add runner network metrics to load tests (920b704)
- Distribute load tests across active runners (37b46fd)
- Show runner resource charts with single sample (5836972)
- Document runner requirement for tests (3fbada4)
- Warn when no runners are available (1accbdd)
- Add runner resource metrics to load tests (70e697a)
- Allow clicking local context prompt (9e19deb)
- Fix local context prompt background (2c9d836)
- Render local context prompt in portal (7ed2ab3)
- Move runners header action next to settings (0ac621b)
- Align runners table columns (547f479)
- Fix local context prompt stacking (077a86b)
- Auto-save runner names on edit (114dc26)
- Highlight runner row while editing (74ccfd7)
- Make runner name editing explicit (72924e4)
- Simplify runners table details (3134c4e)
- Keep settings action on runners page (aa28e30)
- Add runners management page (36b6ca4)

Full Changelog: https://github.com/runvibe/previa/compare/v1.0.0-alpha.20...v1.0.0-alpha.21

## [v1.0.0-alpha.20] - 2026-04-30

### Documentation
- Changelog: update for v1.0.0-alpha.19 (37a68f1)

### Maintenance
- Bump version to 1.0.0-alpha.20 (de6892c)
- Update release metadata for v1.0.0-alpha.19 (ecbb387)
- Update release metadata for v1.0.0-alpha.19 (268d139)

### Other Changes
- Add selected projects sqlite export in app (c1e7834)
- Detect same-origin Previa API on startup (b17aeab)
- Revert "Check local main on port 5056" (eac6755)
- Check local main on port 5056 (3f13c43)
- Support SQLite project imports in app (262aa1b)
- Update repository references to runvibe (191d3ee)
- Add SQLite import export e2e test plan (9a844df)
- Add SQLite project import export (d7398d8)

Full Changelog: https://github.com/runvibe/previa/compare/v1.0.0-alpha.19...v1.0.0-alpha.20

## [v1.0.0-alpha.19] - 2026-04-30

### Documentation
- Changelog: update for v1.0.0-alpha.19 (a94bb19)

### Maintenance
- Publish docker images under repository owner (d91b707)
- Update release metadata for v1.0.0-alpha.18 (8f5fcc8)

Full Changelog: https://github.com/runvibe/previa/compare/v1.0.0-alpha.18...v1.0.0-alpha.19

## [v1.0.0-alpha.18] - 2026-04-30

### Maintenance
- Bump version to 1.0.0-alpha.18 (054e6ba)
- Pass app build skip env to cross (8da7946)

Full Changelog: https://github.com/runvibe/previa/compare/v1.0.0-alpha.17...v1.0.0-alpha.18

## [v1.0.0-alpha.17] - 2026-04-30

### Features
- Serve embedded app from previa-main (484a64d)

### Documentation
- Clarify runner enabled flag (5cd2614)
- Document dynamic runner registry (05e4b4b)

### Maintenance
- Bump version to 1.0.0-alpha.17 (d8a8d1f)
- Ignore container build target (ceb23db)
- Vendor app snapshot (41ee7c0)
- Ignore app subrepo (c48061a)
- Ignore embedded app git metadata (f829fd1)
- Update release metadata for v1.0.0-alpha.16 (1d32b67)

Full Changelog: https://github.com/runvibe/previa/compare/v1.0.0-alpha.16...v1.0.0-alpha.17

## [v1.0.0-alpha.16] - 2026-04-29

### Features
- Add dynamic runner registry (939b7b3)
- Support postgres orchestrator database (73458fd)
- Add local push command (df1091d)
- Add local cli workflow (d1e293e)

### Documentation
- Changelog: update for v1.0.0-alpha.14 (65fed53)
- Changelog: update for v1.0.0-alpha.14 (6158aee)
- Changelog: update for v1.0.0-alpha.14 (7b1c03a)

### Maintenance
- Bump version to 1.0.0-alpha.16 (0a7d9b3)
- Bump version to 1.0.0-alpha.15 (be9aab7)
- Update release metadata for v1.0.0-alpha.14 (bc19b70)
- Publish native macos arm64 cli (d6faa07)
- Update release metadata for v1.0.0-alpha.14 (8f40110)

Full Changelog: https://github.com/runvibe/previa/compare/v1.0.0-alpha.14...v1.0.0-alpha.16

## [v1.0.0-alpha.14] - 2026-04-29

### Features
- Add MCP client install commands (433737b)

### Documentation
- Changelog: update for v1.0.0-alpha.14 (a90f383)
- Expand helper and template variable guidance (f21c293)
- Changelog: update for v1.0.0-alpha.12 (c9d17a3)

### Maintenance
- Bump version to 1.0.0-alpha.14 in Cargo.lock and Cargo.toml (fbafaa1)
- Move release metadata into repo (baf5f50)
- Bump version to 1.0.0-alpha.13 in Cargo.lock and Cargo.toml (06d65b0)

Full Changelog: https://github.com/runvibe/previa/compare/v1.0.0-alpha.12...v1.0.0-alpha.14

## [v1.0.0-alpha.12] - 2026-03-30

### Features
- Add MCP resource support (bd81d5c)
- Update MCP handlers and add response status logic chore: bump version to 1.0.0-alpha.12 (0e1640a)
- Add previa init command (572390b)

### Bug Fixes
- Prefer workspace binaries for --bin (700db44)
- Support codex MCP protocol version (cafd78e)
- Prefer local matching runtime binaries (673fc73)

### Documentation
- Changelog: update for v1.0.0-alpha.12 (d81de15)
- Clarify --bin binary resolution order (82c4007)
- Document bin runtime resolution order (aa92be6)

### Other Changes
- Enable MCP in default local compose config (85e03a5)

Full Changelog: https://github.com/runvibe/previa/compare/v1.0.0-alpha.11...v1.0.0-alpha.12

## [v1.0.0-alpha.11] - 2026-03-24

### Documentation
- Changelog: update for v1.0.0-alpha.10 (4f4f875)

### Maintenance
- Update version to 1.0.0-alpha.11 in Cargo files (13acf77)

### Other Changes
- Add MCP guide for local pipeline workflows (8a8c96e)
- Fix Windows process handle check (70e1d86)

Full Changelog: https://github.com/runvibe/previa/compare/v1.0.0-alpha.10...v1.0.0-alpha.11

## [v1.0.0-alpha.10] - 2026-03-24

### Features
- Add pipeline export CLI (a8b1fad)

### Documentation
- Changelog: update for v1.0.0-alpha.10 (15fb964)
- Add project repository workflow guide (f37d8ac)
- Changelog: update for v1.0.0-alpha.9 (6cf1d95)

### Maintenance
- Update version to 1.0.0-alpha.10 in Cargo files (097da27)

### Other Changes
- Add Windows PowerShell installer (2e7776b)
- Support macOS detection in installer (99d3663)
- Fix release workflow YAML schema issues (c925953)
- Add release scope selector to release workflow (808ac5d)
- Hide --bin outside Linux (4ee7dd2)

Full Changelog: https://github.com/runvibe/previa/compare/v1.0.0-alpha.9...v1.0.0-alpha.10

## [v1.0.0-alpha.9] - 2026-03-19

### Documentation
- Changelog: update for v1.0.0-alpha.9 (5324594)

### Other Changes
- Atualiza versão para 1.0.0-alpha.9 no Cargo.lock e Cargo.toml (c70f6ed)
- Print manual URL when browser launch fails (babede7)

Full Changelog: https://github.com/runvibe/previa/compare/v1.0.0-alpha.8...v1.0.0-alpha.9

## [v1.0.0-alpha.8] - 2026-03-19

### Other Changes
- Atualiza versão para 1.0.0-alpha.8 no Cargo.lock e Cargo.toml (b668f2e)

Full Changelog: https://github.com/runvibe/previa/compare/v1.0.0-alpha.7...v1.0.0-alpha.8

## [v1.0.0-alpha.7] - 2026-03-19

### Other Changes
- Atualiza versão para 1.0.0-alpha.7 no Cargo.lock e Cargo.toml (53d95bf)
- Use legacy-compatible compose port syntax (6b9a918)
- Refresh README badge URLs (e688c15)

Full Changelog: https://github.com/runvibe/previa/compare/v1.0.0-alpha.6...v1.0.0-alpha.7

## [v1.0.0-alpha.6] - 2026-03-19

### Other Changes
- Atualiza versão para 1.0.0-alpha.6 no Cargo.lock e Cargo.toml (c8d3de2)
- Automate release changelog generation (4acffc3)
- Support docker-compose fallback (ab8b66b)
- Polish public README presentation (dfbc129)
- Add social preview assets (b5338df)
- Add public repo community files (70f2e18)
- Add MIT license (305d878)
- Add README badges (bf173b4)
- Add contributor guide (fcc06b8)
- Align docs with current runtime behavior (d85af81)
- Remove engine mention from README (42b48b8)
- Add walkthrough and reference docs (ebb93bf)
- Clarify onboarding and version alignment docs (3c2018a)

Full Changelog: https://github.com/runvibe/previa/compare/v1.0.0-alpha.5...v1.0.0-alpha.6
