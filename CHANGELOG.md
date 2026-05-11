# Changelog

All notable changes to Previa are documented in this file.

## [v1.0.0-alpha.21] - 2026-05-11

### Maintenance
- Bump version to 1.0.0-alpha.21.

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
