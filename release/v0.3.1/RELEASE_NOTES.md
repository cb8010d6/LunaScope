# LunaScope 0.3.1

LunaScope 0.3.1 is a Windows public engineering preview of the Moonwatcher Project. This release updates the public source snapshot, desktop binaries, architecture documentation, and bilingual README together.

## Highlights

- Durable planning-to-delivery Run lifecycle with guidance, pause, resume, cancellation, verification, and targeted repair.
- Finer multi-Agent decomposition with up to 24 task-derived Workers, bounded parallel execution, explicit write scopes, dynamic supervision, and criterion-level verification.
- Stable orchestration graph geometry with pan, zoom, minimap, topology-aware layout, incremental state changes, and retained viewport state.
- Typed Agent-session message flow with public model summaries, first-class tool events, Worker traces, and persistent same-thread context.
- OpenAI Responses, OpenAI Chat Completions, Anthropic Messages, and generic compatible Provider transports with explicit model/effort testing controls.
- Independent vision fallback for text-only primary models, bounded local browser acceptance, five escalating transport retries, and progress-sensitive verifier repair without a fixed generation cap.
- Expanded UltraNote project workflow: syllabus-backed course memory, PDF/Office/image ingestion, cited bilingual notes, local Mermaid and math rendering, course retrieval, Markdown archive, interactive HTML, and offline PDF export.
- Optional desktop Companion workflows for pinned Live2D and Spine resources with explicit license handling.
- Windows GUI subsystem and hidden internal child processes prevent an extra PowerShell/console window during normal desktop use.

## Downloads

- `LunaScope_0.3.1_x64-setup.exe` — NSIS installer for 64-bit Windows.
- `lunascope-desktop.exe` — portable desktop executable.
- `SHA256SUMS.txt` — SHA-256 digests for both binaries.

The binaries are unsigned. Windows SmartScreen may display a warning; verify the digest before running them.

## Verification

The release build passed:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo check --workspace`
- `cargo test --workspace`
- `npm run typecheck`
- `npm run contract:check`
- `npm run test:companion-assets`
- `npm run evals:manifest-check` (manifest schema only; not a live eval)
- `npm run build`
- `npm run tauri -- build`

Deterministic tests passed with no failures. Tests requiring paid Provider credentials, a live network, explicit external fixtures, or a persistent user workspace remain ignored by default. The twelve cases in `evals/manifest.json` are evidence specifications and remain marked `defined_not_run`; they are not claimed as completed live evaluations.

## Upgrade notes

- Existing Provider credentials remain in Windows Credential Manager. The release package does not contain credentials or runtime databases.
- Model routing settings now depend on the exact Provider/model/effort capability test. Re-test a tuple after changing its Provider configuration, endpoint, credential reference, model, or effort.
- UltraNote PDF generation requires Microsoft Edge on Windows. Markdown and HTML are retained when PDF rendering cannot complete.
- Review `docs/THIRD_PARTY_NOTICES.md` before redistribution. Some bundled research Skills use CC BY-NC 4.0 and are not suitable for commercial redistribution without separate licensing or removal.

## Current limits

- Windows-first and unsigned.
- The LunaScope project license is not yet selected; repository visibility is not a license grant.
- Compatibility varies across relay endpoints; use the built-in real connection test.
- Real-provider and multi-hour endurance canaries are outside the default validation command.
