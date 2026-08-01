# Threat Model

## Assets

- Workspace and course files
- Git working trees and history
- Provider credentials and OAuth tokens
- Model prompts, outputs, memory, and citations
- Event store, snapshots, checkpoints, and artifacts
- User approval decisions and permission rules
- Local processes and network identity

## Trust boundaries

1. User <-> LunaScope UI
2. Tauri WebView <-> narrow Rust IPC commands/channels
3. Runtime <-> filesystem/process/Git/browser tools
4. Runtime <-> model providers
5. Runtime <-> MCP servers and official agent bridges
6. Quarantine <-> installed extensions
7. Worker <-> orchestrator and sibling workers
8. Course scope <-> other course/project/user scopes

The WebView, model output, imported content, repository files, tool output, MCP metadata, and external URLs are untrusted inputs.

## Principal threats and controls

### IPC authority escalation

- Threat: Compromised frontend invokes arbitrary shell or reads secrets.
- Controls: No generic execute command; typed narrow commands; origin/capability checks; policy decision before execution; structured audit event.

### Path traversal and link escape

- Threat: Relative paths, junctions, symlinks, UNC paths, or case folding escape the approved root.
- Controls: Canonicalize existing ancestors; reject ambiguous/nonexistent escape chains; compare Windows paths case-insensitively; explicitly handle reparse points and UNC scopes; test TOCTOU-sensitive operations.

### Command injection

- Threat: Model-provided strings gain shell semantics.
- Controls: Prefer `program + argv + cwd + env allowlist`; shell tools are separate high-risk tools; reject implicit PowerShell/CMD/WSL interpolation.
- M2 evidence: Allowlisted executables are resolved from PATH by the absolute trusted `System32\where.exe` and pinned to canonical absolute paths, preventing current-directory executable shadowing.

### Secret disclosure

- Threat: Credentials enter prompts, events, logs, tool outputs, artifacts, or crash reports.
- Controls: Store secrets in OS keyring; pass opaque credential references; redact before persistence; deny `.env` and credential locations by default; test known-secret canaries.
- M2 evidence: The native provider places its caller-supplied credential only in a sensitive HTTP header. A protocol test verifies the canary is absent from all persisted events. OS Keyring references remain an M3 gate.

### Prompt/tool result injection

- Threat: Files or remote results instruct the model to exceed scope.
- Controls: Treat retrieved text as data with provenance; tool authority comes only from policy, never content; approvals show exact action and blast radius.

### Extension supply chain

- Threat: Imported repositories execute installers, binaries, hooks, or traversal payloads.
- Controls: Download into quarantine; pin commit and hashes; inventory licenses/scripts/binaries/symlinks; never auto-run package managers or builds; explicit content selection and approval; rollback record.

### MCP/bridge confused deputy

- Threat: A server or bridge exposes tools broader than user permission.
- Controls: Normalize every discovered tool into a manifest; intersect server, workspace, run, worker, and tool capabilities; apply timeouts/cancellation; record request/result with redaction.

### Worker privilege laundering

- Threat: A worker creates another worker or hands off work to bypass restrictions.
- Controls: Child effective permissions cannot exceed parent plus explicit user grant; orchestrator validates all graph patches; no peer can mint authority.

### Replay, duplication, and sequence gaps

- Threat: Duplicate deltas or crash recovery repeats side effects.
- Controls: Unique event IDs; monotonic per-run sequence; correlation/causation; idempotency keys; frontend sequence-gap detection; tool intent/result reconciliation.
- M2 evidence: Related generated events allocate contiguous sequences in one immediate transaction; invalid batches roll back completely. Guarded patches reject stale hashes on replay.

### Course boundary leakage

- Threat: UltraNote context crosses course boundaries or mixes unsupported inference with class sources.
- Controls: Course-scoped storage keys and queries; explicit source anchors; provenance labels; isolation and academic-integrity tests.

## Security validation gates

- Invalid state transition tests
- Event ordering/deduplication/replay tests
- Secret-redaction canary tests
- Windows path traversal/reparse/UNC tests
- Process cancellation and orphan detection
- Approval persistence/restart tests
- Malicious extension fixtures
- MCP permission intersection tests
- Worker privilege isolation tests
- Course isolation and citation-anchor tests
