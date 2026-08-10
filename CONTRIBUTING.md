# Contributing to LunaScope

LunaScope is a Windows-first Rust/Tauri application. Changes to paths, permissions, durable events, tool execution, Provider credentials, worktrees, verification, or recovery are architecture-sensitive and require deterministic tests.

## Prerequisites

- Windows 10 or 11
- Rust `1.95.0` with `rustfmt` and `clippy`
- Node.js 22 and npm
- Microsoft Edge WebView2 Runtime
- Git for Windows
- Tauri 2 Windows build prerequisites

## Setup

```powershell
npm ci
cargo check --workspace
npm run typecheck
```

Do not add real API keys, tokens, cookies, certificates, credential exports, or private workspace fixtures. Live Provider tests must remain ignored or secret-enabled and must redact their evidence.

## Generated contracts

Rust domain types are canonical. After changing exported contracts:

```powershell
cargo run -p lunascope-core --example export_contract
npm run contract:check
```

Commit the generated TypeScript/schema changes with the Rust source. Do not hand-edit generated contracts to conceal drift.

## Required validation

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo check --workspace
cargo test --workspace
npm ci
npm run typecheck
npm run contract:check
npm run test:companion-assets
npm run test:pixi-runtime
npm run test:tauri-csp
npm run test:environment-preflight
npm run evals:manifest-check
npm run evals:tier1
npm run release:gate:test
npm run test:release-evidence-redaction
npm run build
npm audit --omit=dev
```

`evals:manifest-check` validates only the eval manifest schema. It is not a live eval result. Tier 1 uses deterministic fixtures without paid credentials. Release Canary and endurance suites are separate manual release evidence.

## Architecture-sensitive areas

- `crates/lunascope-core`: typed contracts and state machines;
- `crates/lunascope-storage`: SQLite events, projections, migrations, restart behavior;
- `crates/lunascope-runtime`: policy, scheduler, worktree and tool isolation;
- `apps/desktop/src-tauri/src/data_paths.rs`: the single data-root source of truth;
- Tauri capabilities, command manifests, CSP, and dynamic asset scope;
- Provider credential references and diagnostic redaction;
- unfinished mutating-tool reconciliation and `NeedsIntervention` fallback.

Do not weaken permission checks, worktree isolation, Provider compatibility testing, or independent verification to make a test pass.

## Pull requests

- Keep one coherent purpose and explain user-visible behavior.
- State commands actually executed and list skipped live/manual suites honestly.
- Add deterministic regression tests for important behavior.
- Update Quickstart/Troubleshooting/Decision/Risk documentation when behavior changes.
- Use placeholders or repository-relative paths in public text; never expose a contributor's absolute local path.
- Do not claim Stable readiness while the root `LICENSE`, Windows signing, live canary, endurance, or external legal gates remain unsatisfied.
