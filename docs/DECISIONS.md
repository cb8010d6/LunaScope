# Decision Ledger

Decisions are append-only. Superseded decisions remain visible and point to their replacement.

## D-0001 - V14 is UX source, not runtime

- Status: Accepted
- Date: 2026-07-27
- Context: The repository initially contained only `indexV14.html`.
- Decision: Preserve V14 as the source of truth for information architecture and product language. Do not reuse its localStorage-backed simulated runtime as production state.
- Consequence: Native integration may change frontend structure only when required by real runtime semantics.

## D-0002 - Runtime is independent of Tauri

- Status: Accepted
- Date: 2026-07-27
- Decision: Domain state, storage, policy, tools, orchestration, recovery, and learning code live in Rust crates without Tauri dependencies. Tauri exposes narrow desktop commands and channels.
- Consequence: Runtime tests do not require a WebView and alternative adapters remain possible.

## D-0003 - Event authority and projection model

- Status: Accepted
- Date: 2026-07-27
- Decision: The append-only SQLite event store is authoritative. Query state is a transactional projection. Periodic snapshots accelerate recovery but never replace committed events as authority.
- Consequence: Commands record accepted state changes before the frontend receives deltas.

## D-0004 - Canonical contract generation

- Status: Accepted
- Date: 2026-07-27
- Decision: Rust domain types are canonical initially. TypeScript declarations and JSON schema are generated/verified in tests so drift fails CI.
- Consequence: Hand-maintained duplicate event unions are prohibited.

## D-0005 - No in-process untrusted extensions

- Status: Accepted
- Date: 2026-07-27
- Decision: External skills are declarative content; external executable capabilities use MCP or a supervised out-of-process bridge. Imported code is never dynamically loaded into the LunaScope Rust process.
- Consequence: Compatibility levels must distinguish native, normalized, bridge-required, and unsupported content.

## D-0006 - Official bridges are process boundaries

- Status: Accepted
- Date: 2026-07-27
- Decision: Codex and Claude Code integration must use documented CLI/app-server/SDK surfaces and the user's own authenticated installation. Never scrape cookies, copy session tokens, or call private web APIs.
- Consequence: Claude Code is not vendored or derived because its published CLI license is proprietary.

## D-0007 - Minimal frontend stack

- Status: Accepted
- Date: 2026-07-27
- Decision: Start with TypeScript, Vite, and the existing semantic HTML/CSS. Add a large UI framework only if measured complexity justifies it.
- Consequence: Migration focuses on real state flow and accessibility, not a new visual prototype.

## D-0008 - Data placement

- Status: Accepted
- Date: 2026-07-27
- Decision: Default large/user data root is `D:\LunaScopeData` when available. If unavailable, first launch requires explicit directory selection. Development fixtures stay small and repository-local.
- Consequence: C: is not polluted with models, course files, or large caches.

## D-0009 - SQLite write concurrency

- Status: Accepted
- Date: 2026-07-27
- Decision: M1 serializes a single SQLite connection behind a process-local mutex and uses `BEGIN IMMEDIATE` for each append/projection transaction. WAL and a five-second busy timeout are enabled.
- Consequence: Ordering is simple and deterministic. Measured contention must justify a writer-task or connection-pool abstraction later.

## D-0010 - Channel owns frontend runtime projection

- Status: Accepted
- Date: 2026-07-27
- Decision: Command responses may acknowledge actions, but the frontend's runtime authority begins from a Snapshot/Delta received through the ordered Tauri channel.
- Consequence: Browser-local state and command-return timing cannot silently become runtime authority.

## D-0011 - Policy precedence has hard and locked-deny tiers

- Status: Accepted
- Date: 2026-07-27
- Decision: Built-in hard constraints run before configured rules. A matching locked deny then wins before priority-ranked ordinary rules. Ordinary ties resolve deterministically as Deny, Ask, then Allow.
- Consequence: A high-priority workspace or tool allow cannot weaken sensitive-path, force-push, imported-extension, administrator, or parent-scope prohibitions.

## D-0012 - File changes use guarded exact patches

- Status: Accepted
- Date: 2026-07-27
- Decision: M2 source modification accepts only an existing in-root, non-final-symlink file plus its expected SHA-256 and exact replacement occurrence counts. Writes use a synchronized temporary file and atomic persistence.
- Consequence: Stale model context fails closed instead of silently patching changed content. Creating files, binary patches, directory mutations, and richer diff application require separate typed tools.

## D-0013 - Native provider transport is independent of routing

- Status: Accepted
- Date: 2026-07-27
- Decision: The first provider is a Rust-native OpenAI Responses HTTP/SSE adapter. It accepts an exact endpoint and caller-supplied credential, requires HTTPS except loopback tests, marks Authorization sensitive, sends `store=false`, and never writes the credential to runtime events.
- Consequence: M2 proves the provider/harness boundary without coupling it to settings or secret storage. M3 owns OS Keyring references, connection configuration, additional providers, model routing, fallback, and usage.

## D-0014 - Related approval events commit atomically

- Status: Accepted
- Date: 2026-07-27
- Decision: Newly generated related events use one `BEGIN IMMEDIATE` batch that allocates contiguous sequences and updates the run projection once. Tool intent, approval request, and `WaitingApproval` transition either all commit or all roll back.
- Consequence: Restart cannot observe a half-created approval boundary. Side-effect/result reconciliation remains a separate tool-runtime responsibility.

## D-0015 - GitHub imports are pinned object extraction, not checkout

- Status: Accepted
- Date: 2026-07-27
- Decision: Accept only canonical public GitHub URLs, resolve a branch/tag/subdirectory to an exact 40-character commit, fetch into a bare quarantine with hooks and local protocols disabled, inspect bounded tree/blob objects, and extract only explicitly selected safe components by object ID.
- Consequence: Imported hooks, package scripts, executables, symlinks, submodules, oversized blobs, traversal, and Windows-reserved paths cannot become executable workspace state. Each approved version retains an immutable manifest/snapshot for comparison and rollback.

## D-0016 - Codex compatibility uses the official CLI-owned boundary

- Status: Accepted
- Date: 2026-07-27
- Decision: Invoke the user's detected official Codex CLI through `codex exec --json --ephemeral --sandbox read-only` with structured arguments, a restricted environment, explicit process/network approval, bounded output/time, one active invocation, and visible cancellation.
- Consequence: LunaScope never reads or copies Codex credentials, never invokes a private service endpoint, and can terminate the owned Windows process tree. A CLI status canary uses only `--version`; real model usage is never hidden inside build tests.

## D-0017 - Secrets cannot cross engineering evidence channels

- Status: Accepted
- Date: 2026-07-27
- Decision: Provider credentials may enter LunaScope only through the masked native credential form and Windows Credential Manager. They are prohibited from source, command arguments, shell environment setup captured by tools, test fixtures, screenshots, and evidence files.
- Consequence: A user-supplied chat credential is not copied into automation transcripts. Live provider verification pauses only at the manual secret-entry boundary while all non-secret implementation and tests continue.

## D-0018 - Worker graphs are canonical, versioned runtime state

- Status: Accepted
- Date: 2026-07-27
- Decision: `OrchestrationPlan`, complete `WorkerSpec` records, user-selected roles and bounded labels, typed user patches, user field locks, handoffs, Worker state changes, and verification records are Rust-canonical event data. Every accepted user graph change increments the graph version and is durably appended before the UI treats it as authoritative.
- Consequence: The browser preview cannot create or mutate a real graph. Recovery rebuilds Graph vN and every user override from committed events, and an Orchestrator revision cannot silently replace a locked field.

## D-0019 - Parallel writers never share a checkout

- Status: Accepted
- Date: 2026-07-27
- Decision: Each Worker attempt receives a new Git worktree pinned to the canonical repository HEAD. Workers may change only declared, non-overlapping write scopes. Their output is captured as a bounded immutable binary patch artifact; downstream integration applies dependency patches in topological order to another isolated worktree.
- Consequence: The user's checkout remains unchanged, retries cannot inherit dirty attempt state, and a Verifier can inspect the exact combination of upstream patches without Workers concurrently editing one file.

## D-0020 - Provider Workers start read-only

- Status: Accepted
- Date: 2026-07-27
- Decision: The first native multi-agent Provider Worker receives only a bounded workspace snapshot and immutable dependency artifacts, must return schema-valid JSON, and exposes no mutating tool. Code-writing collaboration is proven through the scheduler's structured executor boundary and isolated worktree integration tests until the native tool loop is connected to the same ownership checks.
- Consequence: A model response cannot bypass the worktree or policy boundaries. The desktop rejects a Provider graph containing write scopes or patch/write tools instead of simulating that capability.

## D-0021 - Active graph patches bind to explicit scheduler boundaries

- Status: Accepted
- Date: 2026-07-27
- Decision: Apply now may replace a queued or dependency-waiting WorkerSpec immediately before dispatch; Apply after current step may replace a not-yet-dispatched dependent Worker at its next dispatch; Apply on retry is consumed only before the next provider attempt. Adding/removing Workers or changing dependencies during execution requires Clone revision, which creates a separate durable Planning run. Cancel persists the user Patch before signalling the exact active run token.
- Consequence: A Worker already inside a model call is not silently killed and described as successfully hot-patched. The user must choose Apply on retry, Clone revision, or Cancel. Every accepted revision is validated against parent permissions, the model pool, user locks, and graph version before it becomes a durable Graph vN.

## D-0022 - Domain Packs are bounded plan overlays with strict evidence gates

- Status: Accepted
- Date: 2026-07-27
- Decision: Programming, Game Development, Research, Academic Writing, and Frontend Design are typed descriptors that augment a canonical `OrchestrationPlan` with a small role set, workflow, rules, skills/tools, labels, and required evidence. A separate typed evidence gate fails when required artifacts or metadata are absent.
- Consequence: A pack can improve planning without granting authority beyond parent permissions or inventing a successful flow. Declared pack tools are routing intent until the native tool loop exposes them through the approved scheduler boundary.

## D-0023 - Native orchestration preserves the V14 Blueprint shell

- Status: Accepted
- Date: 2026-07-27
- Decision: Keep the V14 Blueprint/Runtime/Queue/Handoffs toolbar, graph-canvas grammar, phase regions, Orchestrator/Worker/synthesis nodes, dependency lines, minimap, and compact controls as the native orchestration presentation. Replace browser-local graph authority with Rust `OrchestrationSession` events and typed Patch commands.
- Consequence: Existing interaction and visual identity survive the native migration, while the UI cannot claim state that is absent from the event store.

## D-0024 - Projects own multiple folders and one explicit workspace

- Status: Accepted
- Date: 2026-07-27
- Decision: A LunaScope project persists one or more canonical existing folders and requires exactly one to be marked as the Agent workspace. Project CRUD may edit the set and switch the workspace. Native read/create/patch commands remain narrowly scoped to that workspace and pass through the shared permission policy.
- Consequence: Additional project folders are context references, not an implicit expansion of write authority. File creation uses create-new semantics; modification requires the expected SHA-256 and exact replacement occurrence counts.

## D-0025 - UltraNote is a slash-command workflow with a retained note specification

- Status: Accepted
- Date: 2026-07-27
- Decision: `/` is the LunaScope feature trigger and `/ultranote` activates the course-note workflow for the current thread. UltraNote has no permanent side-navigation mode. Its Settings page remains only for the user's note-format specification.
- Consequence: A user specification can influence presentation but cannot weaken course isolation, source provenance, citation anchors, unresolved-policy handling, or academic-integrity boundaries.

## D-0026 - Essential preferences are global, typed, and bilingual

- Status: Accepted
- Date: 2026-07-27
- Decision: Persist only essential global preferences now: interface language and the UltraNote note specification. The Settings navigation exposes only implemented pages: General, Models & Providers, Skills & MCP, Projects & Folders, and UltraNote.
- Consequence: Placeholder and duplicated setting categories are removed. Chinese/English switching is immediate for the shell and newly native surfaces and persists through SQLite in the desktop app.

## D-0027 - Conversation is the only orchestration intake

- Status: Accepted
- Date: 2026-07-28
- Decision: A normal project conversation message invokes the configured Orchestration Model, which returns a bounded single/multi decision and Worker drafts that become the canonical durable graph. The Orchestration route contains only the V14 Blueprint graph and opens node details in the V14 right Inspector.
- Consequence: Users do not fill in a second objective form or manually design the first graph. The configured model owns the decision, while Rust validates worker count, dependencies, permissions, model pool, depth, and persistence.

## D-0028 - Extensions are global and system/user Skills are isolated

- Status: Accepted
- Date: 2026-07-28
- Decision: Skills, Tools, and MCP use the fixed `D:\LunaScopeData\extensions` root. System Skills and user Skills are discovered from separate `skills\system` and `skills\user` directories; neither is discovered from a project workspace.
- Consequence: Switching projects cannot silently change executable capability discovery. Project workspaces remain task context and file scope, not extension installation roots.

## D-0029 - Multiple Providers coexist by stable configuration ID

- Status: Accepted
- Date: 2026-07-28
- Decision: Saving a Provider inserts or updates only the matching configuration ID. The UI exposes an explicit blank new-Provider flow and lists all saved enabled/disabled configurations together.
- Consequence: OpenAI, Anthropic, DeepSeek, and compatible endpoints can be bound simultaneously and independently selected for Orchestration and Worker roles.

## D-0030 - Conversation orchestration auto-dispatches executable Workers

- Status: Accepted
- Date: 2026-07-28
- Decision: A graph drafted from a normal conversation is dispatched immediately after durable creation. Each Worker runs a bounded native model/tool loop in an isolated Git worktree or shadow repository, and verified patches are integrated into the selected project workspace. Conversation progress is keyed by Worker ID and updates one visible status row through queued, dependency wait, model, tool, verification, and terminal states.
- Consequence: The graph is no longer a stopping point or a simulated plan. Orchestrator prompts name abstract capabilities, while the Domain Pack converts them into the exact callable runtime functions (`list_files`, `read_file`, `search_text`, `write_file`, `replace_in_file`, and `run_process`). Run completion must persist `Running -> Verifying -> Synthesizing -> Completed/PartiallyCompleted`; a direct invalid terminal jump is rejected.

## D-0031 - Worker model routing follows configured role assignments

- Status: Accepted
- Date: 2026-07-28
- Decision: The Worker Model Pool retains user order and routes each generated Worker role to its closest configured `ModelRole`, with General Worker as the bounded fallback. Domain Packs preserve the Orchestrator-authored Worker role.
- Consequence: Frontend, programming, research, writing, game, review, and verification Workers can use different Providers concurrently without alphabetical Provider order silently selecting the model.

## D-0032 - LunaScope executes Agents through its native runtime only

- Status: Accepted
- Date: 2026-07-28
- Decision: Remove the Official Codex CLI Bridge, its public contracts, commands, dependency, and UI. Provider calls, tool execution, scheduling, worktree isolation, verification, and synthesis are owned by the LunaScope Rust runtime.
- Consequence: A LunaScope run cannot silently delegate execution to Codex CLI. Compatibility with Provider protocols and imported Skills remains independent from runtime delegation.

## D-0033 - Local retry excludes completed Workers

- Status: Accepted
- Date: 2026-07-28
- Decision: A retry run contains only Workers whose durable source state is `Failed` plus unfinished descendants that must resume after their dependency recovers. Completed Workers are excluded, dependencies on excluded completed Workers are removed, and the source run ID is bound to the retry prompt so the behavior survives application restart.
- Consequence: Successful work and already integrated patches are preserved. Dependency-cancelled descendants can resume without repeating completed analysis or implementation, and repeated retry prompts target their own durable run.

## D-0034 - Provider and process boundaries are bounded for executable projects

- Status: Accepted
- Date: 2026-07-28
- Decision: Mutating Workers receive enough per-step output budget for complete file tool arguments, while analysis Workers retain the smaller default. Non-verifier output may be repaired into an artifact, but verifier output remains schema-strict. Windows process timeout or cancellation terminates the complete child process tree, and verifier instructions prohibit persistent services.
- Consequence: Large file writes no longer fail from truncated tool JSON, malformed planning responses fall back to an executable graph instead of stopping the conversation, and launcher verification cannot leave an orphan HTTP server that stalls the run.

## D-0035 - Conversations own orchestration lifecycle and deletion

- Status: Accepted
- Date: 2026-07-28
- Decision: Orchestration is a subview of the active conversation rather than a global project route. Unfinished or failed graphs are kept by thread while the app is running, successful graphs are cleared after final synthesis, and deleting a conversation or project removes its event/projection/snapshot records. Project deletion never deletes files from the folders selected by the user.
- Consequence: Switching conversations cannot display another thread's graph. Failed graphs remain available for precise local retry, while completed or deleted work does not leave stale orchestration UI. An active native run blocks conversation switching and deletion until it stops.

## D-0036 - HarmonyOS Sans is the sole interface typeface

- Status: Accepted
- Date: 2026-07-28
- Decision: All display, body, control, and code-style UI text resolves through locally installed HarmonyOS Sans SC Regular/Bold faces. The repository records the HarmonyOS Sans Fonts License Agreement conditions and does not redistribute font binaries until the unmodified official package and complete agreement are bundled together.
- Consequence: The Windows build has one consistent type system without a CDN. Release environments must install HarmonyOS Sans SC until compliant font bundling is added.

## D-0037 - Sending a message is the sole execution trigger

- Status: Accepted
- Date: 2026-07-28
- Decision: Remove the standalone Run action. Sending a project-conversation message durably creates the Orchestration graph and immediately dispatches its executable Workers. The scheduler limits simultaneous work with the parallelism cap but preserves all bounded graph stages, serializes unordered writers, orders a writable reviewer after ordinary writers, and runs the verifier only after every writer.
- Consequence: A generated graph cannot become an inert stopping point. Non-Git shadow worktrees write verified file mutations through to the selected workspace while execution is in progress. Provider JSON truncation receives one bounded repair attempt, and a final verified verdict is rejected when its own evidence still reports unresolved risks.

## D-0038 - Runtime projections and visible activity replace simulated pages

- Status: Accepted
- Date: 2026-07-29
- Decision: Plan, Changes, Workers, and Artifacts render the active conversation's typed `OrchestrationPlan` and `OrchestrationRunResult`. High-frequency Worker events update node status in place and a compact bottom activity line; the conversation retains bounded progress summaries, tool results, and retry state rather than private hidden reasoning.
- Consequence: The four pages cannot display V14 fixture evidence as live data, graph progress no longer replaces the whole graph DOM for every event, and users can inspect the same durable run through task, worker, file/artifact, and verification views.

## D-0039 - Reply language is independent and Skills use progressive disclosure

- Status: Accepted
- Date: 2026-07-29
- Decision: `ModelReplyLanguage` is a persisted global preference with Follow UI, Simplified Chinese, and English options. LunaScope's first-party runtime prompt requires real tool execution, recovery, verification, evidence, and concise visible activity. Six bundled system Skills are discovered by metadata and only selected Skill bodies are loaded for matching Workers.
- Consequence: UI localization no longer implicitly controls model prose. System and user Skills remain isolated under the fixed global extension root, imported GitHub Skill URLs are pinned and installed inertly below the user Skill root, and system MCP starts with the official OpenAI Developer Docs remote endpoint instead of an empty catalog.

## D-0040 - Fatal verification findings create a bounded autonomous repair chain

- Status: Accepted
- Date: 2026-07-29
- Decision: A Verifier returns typed severity-ranked findings. Any `fatal` finding or failed-verification verdict creates a new durable repair run against the same real workspace, followed by an independent Verifier, for at most two autonomous repair cycles.
- Consequence: LunaScope repairs observed release-blocking defects instead of presenting them as a finished delivery. Every repair remains a separate auditable run with its own graph, events, artifacts, patches, and verdict; the hard cycle bound prevents an unproductive infinite loop.

## D-0041 - Changes is a typed read-only projection of native patch evidence

- Status: Accepted
- Date: 2026-07-29
- Decision: The desktop backend reads only bounded `text/x-diff` artifacts already recorded for the selected run, validates that artifact paths remain below the LunaScope data root, parses unified-diff file/hunk/line metadata, and returns typed change sets to the WebView.
- Consequence: Changes presents Codex-style additions, deletions, line numbers, hunks, and expandable files without granting the WebView arbitrary filesystem access or fabricating diffs from artifact names.

## D-0042 - Reply language is enforced at every model and presentation boundary

- Status: Accepted
- Date: 2026-07-29
- Decision: The configured reply language is a hard contract for Orchestrator drafts, Worker summaries, progress prose, verification findings, risks, and synthesis. Chinese mode performs up to two structure-preserving model rewrites and then applies deterministic localized containment to any remaining untranslated human-facing prose.
- Consequence: Structural identifiers, file paths, tool names, code, and protocol values remain unchanged, while child-Agent roles, tasks, states, evidence summaries, and visible activity follow the selected language even when a Provider ignores the initial instruction.

## D-0043 - Worker execution is a settled item loop, not an opaque final response

- Status: Accepted
- Date: 2026-07-30
- Decision: Model text, assistant tool calls, and tool results remain typed conversation items and are translated into each Provider's native wire format. Every tool intent is durably recorded before execution and is immediately followed by a success or failure settlement with an idempotency key, elapsed time, and bounded evidence. Older settled turns are deterministically compacted while the latest tool turn and real workspace state remain authoritative.
- Consequence: DeepSeek/OpenAI-compatible, OpenAI Responses, and Anthropic models no longer receive tool results disguised as user prose. A process failure leaves an auditable pending tool intent instead of an unknowable opaque Worker failure, repeated model calls retain valid call/result linkage, and long file/command sessions cannot grow the in-memory prompt without a bound.

## D-0044 - Observable work is narrated; private reasoning is not exposed

- Status: Accepted
- Date: 2026-07-30
- Decision: The conversation shows concise natural-language updates derived only from observable runtime facts: Worker start, concrete tool operation, settled tool round, recovery, localization, independent verification, targeted repair, completion, failure, and cancellation. Repetitive token/character counters are filtered. The bottom bar remains a single current-action line, while per-Worker status cards and the V14 graph remain available for detailed inspection. The Orchestration Model also returns a bounded conversation summary title on the first task, replacing the placeholder sidebar label.
- Consequence: Long runs are no longer a black box, but LunaScope still does not expose hidden chain-of-thought or invent progress. Conversation titles follow the configured model language contract, remain bounded for the sidebar, and fall back to a deterministic request excerpt only if two valid model responses omit the title.

## D-0045 - Complex generated graphs converge before dispatch

- Status: Accepted
- Date: 2026-07-30
- Decision: Four Workers is a hard generated-graph maximum for ordinary complex requests. A larger or non-executable model graph is replaced by the bounded planner, builder, writable reviewer, and independent verifier fallback. Any non-read-only Worker whose assignment owns files receives native read/patch tools and a write scope. Near the hard tool-step boundary, a Worker with sufficient workspace evidence is asked to settle; explicit unfinished markers reopen a bounded repair window. Release canaries require `Verified`, not `PartiallyVerified`.
- Consequence: Documentation or test Workers cannot claim proposed text as a created file, overlapping ceremonial nodes cannot cancel verification after exhausting 64 steps, and a partial verdict cannot silently pass the release gate.

## D-0046 - Plans and reasoning summaries are first-class turn items

- Status: Accepted
- Date: 2026-07-30
- Decision: The Orchestrator and every Worker receive bounded `update_plan` and `report_progress` tools. Provider-native reasoning summaries are streamed when the protocol exposes them; model commentary is accepted only through the explicit progress tool. Both become typed, durable runtime records with stable item IDs and start/delta/completed phases. Raw hidden chain-of-thought fields are never projected.
- Consequence: The conversation can distinguish a Provider reasoning summary from a model-authored action update, the Plan view renders the latest per-Agent checklist as a compact table, and the same settled tool-call/tool-result loop continues after every internal or external tool invocation.

## D-0047 - Skill selection is dynamic and upstream packages are pinned

- Status: Accepted
- Date: 2026-07-30
- Decision: Domain packs no longer assign Skills by Worker role. The Orchestration model receives a bounded catalog and selects exact Skill IDs for the current task; LunaScope validates those IDs and uses task-text scoring only as a fallback. Bundled Skills are exact packages from pinned OpenAI, Anthropic, obra/superpowers, Orchestra Research, and Imbad0202 revisions, with source and license manifests.
- Consequence: A builder, reviewer, or custom Agent may load any relevant Skill without a hard-coded role mapping. System-owned Skill files are reconciled on upgrade so obsolete first-party or vendor files cannot survive. Imbad0202 research packages remain CC BY-NC 4.0 and must be removed or separately licensed for a commercial distribution.

## D-0048 - Completed graphs clear without deleting run evidence

- Status: Accepted
- Date: 2026-07-30
- Decision: Successful task completion clears only the V14 graph canvas and active execution activity. The selected thread retains its native plan, run result, change sets, Worker records, and artifacts until the conversation or project is explicitly deleted.
- Consequence: Plan, Changes, Workers, and Artifacts remain inspectable after completion, while the next task begins with an empty graph and no development residue.

## D-0049 - Reasoning, commentary, and tool activity are separate turn items

- Status: Accepted
- Date: 2026-07-30
- Decision: Supersede the synthesized narration portion of D-0044. Provider-native reasoning summaries, model-authored commentary, Agent status, and tool calls/results are separate first-class records. `report_progress` must contain a concrete observation, the decision supported by that evidence, and the next observable action. Tool lifecycle events render as compact tool activity and are never rewritten into assistant prose such as tool-round counters.
- Consequence: The conversation shows what evidence the Agent noticed and what it chose to do next without presenting generic runtime state as reasoning. LunaScope preserves the same safety boundary as Codex: raw hidden chain-of-thought is not projected, while Provider summaries and explicit model commentary remain attributable and durable.

## D-0050 - Conversation recovery binds the real run and Changes includes live worktrees

- Status: Accepted
- Date: 2026-07-30
- Decision: A conversation persists the latest Orchestration run ID after planning and migrates older local conversations from their stable `orchestration-run-*` event IDs. Plan, Workers, Artifacts, and verification reconstruct from the durable journal. Changes reads bounded recorded patch artifacts and, for Workers without a settled patch, a bounded Git diff from the isolated worktree below the validated LunaScope data root.
- Consequence: Restarting the desktop no longer points a conversation at its initial empty run. Plan and Changes remain usable during an interrupted run, and write-through files become inspectable before Worker finalization without granting the WebView arbitrary filesystem access.

## D-0051 - Harness quality is governed by a persistent acceptance contract and first-class Skill resources

- Status: Accepted
- Date: 2026-07-30
- Decision: Every Orchestration draft carries 3-12 task-wide observable acceptance criteria. Complex drafts pass through a bounded model quality gate before dispatch; the resulting criteria persist in `OrchestrationPlan.user_hard_constraints`, are injected into every Worker, render as a Plan table, and require criterion-mapped Verifier evidence. Worker context compaction preserves a structured evidence checkpoint instead of only a turn count. Selected Skills expose a read-only `read_skill_resource` tool that resolves package-local and repository-level resources, including links relative to an already-read resource, while scripts and hooks remain inert. Workspace `AGENTS.override.md`, `AGENTS.md`, `CLAUDE.md`, and `OPENCODE.md` instructions are discovered in bounded directory order.
- Consequence: Long tasks retain the original definition of done across Worker and context boundaries; a locally complete node cannot silently replace task-wide acceptance. Codex- and Claude-compatible Skills can use referenced templates, schemas, examples, and agent definitions instead of receiving only `SKILL.md`. Imbad0202 Academic Research Skills preserve their upstream UTF-8 layout and shared resources, while traversal, symlinks, oversized resources, implicit script execution, and instruction-precedence escalation remain blocked.

## D-0052 - Attachments are locally normalized before model routing

- Status: Accepted
- Date: 2026-07-30
- Decision: User-selected PDF, Office, text, and image files enter a bounded native attachment store. LunaScope extracts Office/PDF structure locally, extracts only bounded OOXML media entries, and passes normalized Markdown to every assigned Worker. Raw images and PDFs are added only through the selected Provider's native multimodal content schema when that Provider is explicitly marked vision-capable. Imported files and Skill resources are inert and never executed.
- Consequence: UltraNote and general Agent work can use the same real upload path without requiring Python, Java, a CDN, or arbitrary WebView filesystem authority. Text-only models receive extracted structure rather than unsupported binary blocks; visual models can inspect original images and PDF pages. UltraNote adds a non-overridable output-language, glossary, formula, plot, provenance, and offline-interactive-HTML contract on top of the user's presentation preferences.

## D-0053 - UltraNote can be a project-level execution contract

- Status: Accepted
- Date: 2026-07-31
- Decision: A project has a Rust-canonical `ProjectKind`. Creating an UltraNote project requires course identity and an imported syllabus. The configured Orchestration model extracts the persisted syllabus structure, and every conversation in that project binds to the same course before planning. `/ultranote` remains an explicit per-request trigger only for general projects.
- Consequence: Course context is durable and automatic without turning UltraNote into a separate application mode. Historical general projects deserialize as `general`, and course/thread isolation remains enforced by storage bindings.

## D-0054 - UltraNote PDF is an offline rendered note artifact

- Status: Accepted
- Date: 2026-07-31
- Decision: Export first creates a note-only Markdown artifact, then semantic print HTML. Pinned local KaTeX renders supported LaTeX to MathML, pinned local Mermaid renders selected diagrams to SVG, and the installed Microsoft Edge runtime performs a one-shot offline PDF print. The PDF is accepted only after process success and PDF signature validation.
- Consequence: The PDF contains readable notes rather than generation narration, supports formulas and concept maps without a CDN, and preserves Markdown/HTML if the platform renderer is unavailable.

## D-0055 - Full Access enables bounded LunaScope self-management

- Status: Accepted
- Date: 2026-07-31
- Decision: Full Access exposes dedicated Worker tools for inspecting non-secret settings, updating validated preferences/routing/model selection, and installing pinned GitHub Skill components into the fixed global user-Skill root. It does not expose credential values, raw database access, hook execution, or arbitrary extension activation.
- Consequence: A conversation can perform requested LunaScope configuration and Skill installation while preserving the imported-content and secret boundaries. Lower access modes do not receive these tools.

## D-0056 - Conversation facts outlive runs and context windows

- Status: Accepted
- Date: 2026-08-01
- Decision: User messages and task-ending LunaScope summaries are stored in a dedicated SQLite conversation ledger keyed by the real project and thread IDs. Planning reads the latest durable summary plus every message after its checkpoint. At 68% of the configured model context window, a dedicated context-compression child Agent writes a continuation brief while the immutable source messages remain available. Prior attachment text is kept as model-only context without expanding the visible user message.
- Consequence: A new turn in the same conversation can recover earlier requirements, evidence, files, decisions, and outcomes after task completion or desktop restart. Compression cannot erase the source conversation, and deleting a conversation or project removes both its messages and derived summary.

## D-0057 - Live guidance is a cooperative replan, not a restarted run

- Status: Accepted
- Date: 2026-08-01
- Decision: While a run is active, Send becomes Guide. LunaScope durably records the message, pauses new safe-step dispatch, asks the configured Orchestration model to revise only unfinished work, patches queued Workers, queues guidance for active Workers at their next model boundary, and then resumes. Pause/Resume and Cancel use the existing scheduler control and cancellation token. Failed Workers receive at most two automatic local retries; completed Workers never rerun.
- Consequence: Users can steer a long task without losing completed tool effects or duplicating the whole graph. A message racing with natural run completion becomes the next turn in the same conversation instead of disappearing.

## D-0058 - Browser acceptance is a bounded verifier tool

- Status: Accepted
- Date: 2026-08-01
- Decision: A read-only Verifier with process permission receives `check_browser_page`. The backend, not the WebView or model, selects an installed Edge/Chrome executable and opens only a workspace-local HTML file. Optional selector-based click/setValue actions run in a size-limited temporary copy with network proxying disabled, bounded virtual time, bounded output, and a 30-second process timeout.
- Consequence: Browser DOM, test results, console diagnostics, and requested control flows can be observed by the independent Verifier without a long-lived server, arbitrary URL navigation, direct browser executable authority, or mutation of the user workspace.

## D-0059 - Long tasks converge through evidence-bearing phases and a typed acceptance ledger

- Status: Accepted
- Date: 2026-08-01
- Decision: Implementation, repair, and independent verification use different bounded execution budgets. A writable implementation phase may preserve its patch and latest browser evidence for a directly dependent repair Worker without claiming completion. Retries receive the exact prior terminal failure and resume from checkpointed files. The final Verifier must return exactly one Rust-canonical `CriterionVerification` row for every stable `AC-N`; missing, duplicate, failed, unverified, or evidence-free passed rows become fatal findings in the scheduler and enter the bounded autonomous repair path.
- Consequence: One exhausted model context no longer discards a usable implementation or forces a whole-graph restart. Global prose such as "all requirements passed" cannot satisfy the release gate, while already-passed criteria remain outside the targeted repair scope. Behavioral evidence takes precedence over lexical keyword checks.

## D-0060 - Vision fallback is a native fail-closed perception stage

- Status: Accepted
- Date: 2026-08-02
- Decision: Model selection has an optional independent `Vision` assignment. When the assigned Orchestration or Worker model is text-only, LunaScope sends bounded visual assets only to that vision-capable Provider, converts the response into question-focused inert text, and gives only that text to the main model. Descriptions are cached by Provider, model, media type, bytes, and focus prompt with at most four concurrent requests. The design is based on the pinned MIT-licensed `Anionex/codex-vision-proxy` reference, but is implemented natively in Rust without a Python sidecar.
- Consequence: DeepSeek, GLM, and other text-only models can use screenshots and uploaded images without receiving unsupported multimodal payloads. A vision failure stops that visual step; raw content never silently falls through to the text model, and instructions visible inside an image remain untrusted data.

## D-0061 - Reasoning effort and private reasoning remain Provider-native

- Status: Accepted
- Date: 2026-08-02
- Decision: LunaScope exposes `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, and `max` in the Rust contract, then narrows the UI to a model-aware supported profile. OpenAI Responses uses `reasoning.effort`, OpenAI-compatible Chat uses `reasoning_effort` plus documented DeepSeek/GLM thinking toggles, and Anthropic Messages uses `output_config.effort`. DeepSeek `reasoning_content` is retained only long enough to preserve the next assistant-tool/result protocol turn; it is not persisted or projected as a user-visible reasoning summary.
- Consequence: Extended effort levels no longer depend on a lowest-common-denominator dropdown, while unsupported known aliases are hidden. Provider-authored summaries remain visible; private chain content remains private and still satisfies DeepSeek's multi-turn tool protocol.

## D-0062 - Active graphs receive bounded dynamic supervision

- Status: Accepted
- Date: 2026-08-02
- Decision: The configured Orchestration model reviews bounded tool-result checkpoints during execution, immediately after a failed operation and periodically during long successful runs. It compares observable evidence with the persisted acceptance contract and current Worker states, then chooses `continue`, targeted `guide`, or safe-boundary `replan` guidance for existing unfinished Workers. A run permits at most 24 monitor reviews and at most six per Worker; completed, failed, and cancelled Workers cannot be reopened by the monitor.
- Consequence: Worker prompts and approaches can adapt to drift, repeated failure, weak evidence, or task-wide gaps without replaying successful calls or replacing the active graph. The monitor cannot expand permissions, alter completed work, or expose hidden chain-of-thought.

## D-0063 - Planning is part of the same durable controllable Run

- Status: Accepted
- Date: 2026-08-03
- Decision: Persist and register the Run before the first planning Provider call. One cancellation token, pause gate, guidance queue, and phase projection span Planning, dispatch, execution, verification, repair, and settlement. A repeated Send in any active phase is durable guidance for that Run.
- Consequence: The desktop can show Guide, Pause, and Cancel immediately after message persistence; planning cannot become an unobservable duplicate invocation. User cancellation terminates planning, model calls, tools, retries, and repair chains as `Cancelled`, never as execution failure and never as verifier input.

## D-0064 - Public activity and graph revisions are durable coordination records

- Status: Accepted
- Date: 2026-08-03
- Decision: Persist model/provider reasoning summaries, explicit `observation -> decision -> next action` commentary, tool lifecycle evidence, run-control acknowledgements, supervisor decisions, and graph revisions as distinct records. Dynamic supervision consumes run events asynchronously and may guide active Workers or patch queued Workers only at safe boundaries. The frontend applies sequence-ordered incremental node updates and rejects stale state regressions.
- Consequence: Users can inspect meaningful work and supervision without exposing hidden chain-of-thought. Monitoring no longer adds a synchronous Provider delay to a Worker tool step, and the V14 graph does not flicker or reset running nodes when a stale snapshot arrives.

## D-0065 - Reasoning effort is a model-family capability with an explicit escape hatch

- Status: Accepted
- Date: 2026-08-03
- Decision: `Auto` omits an explicit effort parameter and is distinct from disabling thinking. Built-in profiles cover OpenAI, Anthropic, DeepSeek, Kimi/Moonshot, GLM, Grok/xAI, and Qwen/Alibaba model families using Provider-native request fields. A model not recognized by the capability table remains on `Auto` unless the user supplies a bounded custom ASCII effort value; known models reject unsupported values. Every assignment exposes a Test action that validates the current profile and performs a real bounded Provider call through the configured keyring credential.
- Consequence: LunaScope does not advertise invalid effort choices, yet newly released compatible models remain usable before the embedded catalog is updated. Saving and sending share the same backend validation, old invalid settings migrate to `Auto`, and a green test result proves the selected model and exact effort were accepted by the remote Provider.

## D-0066 - Reasoning effort is a tested Provider-native value

- Status: Accepted; supersedes the selection behavior in D-0065
- Date: 2026-08-04
- Decision: The settings UI exposes one bounded free-form reasoning-effort field per Orchestration, Vision, and Worker assignment. Blank means the Provider default and sends no explicit effort; any nonblank value is passed using the Provider protocol's native field. The legacy enum remains only for wire compatibility and migration, while new assignments persist their value directly and keep that enum on `Auto`. Every distinct Provider configuration, model, and effort tuple must complete a real bounded Provider request in the current desktop session before the backend accepts a settings save. Changing Provider metadata or credentials invalidates all test attestations.
- Consequence: A stale embedded model catalog can no longer collapse the control to `Auto` or falsely reject a newly released Kimi, GLM, OpenAI, Anthropic, DeepSeek, Grok, Qwen, or compatible model. Invalid values fail at the actual Provider before they can become saved configuration, and the save rule cannot be bypassed through the WebView.

## D-0067 - Registered desktop commands must have a matching main-window ACL entry

- Status: Accepted
- Date: 2026-08-04
- Decision: Every application command in the Tauri build manifest must also appear in the composed main-window permission set. A Rust test derives each `allow-*` identifier from the registered command list and fails the build when any entry is missing. Model compatibility tests report progress and results beside the exact assignment instead of relying on a distant page-level status element.
- Consequence: A command cannot ship in the frontend and Rust invoke handler while remaining unreachable from the WebView. ACL faults are identified as LunaScope application-permission errors, and long settings pages provide immediate visible feedback for the control the user actually clicked.

## D-0068 - Worker identity is task-derived and live topology is replaceable only before side effects

- Status: Accepted
- Date: 2026-08-07
- Decision: Each model-authored Worker carries a task-derived display name, owned acceptance criteria, explicit write scopes, an expected deliverable, and a dependency-derived parallel wave. Ordinary assignments are limited to one independently reviewable module, function cluster, test cluster, asset group, migration, or defect. The scheduler continuously refills available slots and accepts graph revisions only for Workers that have not started; running and completed side effects remain immutable. A guidance replan may replace queued topology, while active Workers receive guidance only at their next safe model boundary.
- Consequence: The graph represents concrete employee-sized work instead of role labels. Independent non-conflicting nodes run concurrently, conflicting writers are serialized, and user guidance changes the real unfinished graph without replaying completed work.

## D-0069 - Observable planning must also have a convergence budget

- Status: Accepted
- Date: 2026-08-07
- Decision: Planning publishes model lifecycle, Provider reasoning summaries, explicit progress commentary, and draft graph revisions, but it may perform at most five Provider steps and three planning-tool rounds. After the third tool round the Provider receives no further planning tools and must submit the final graph. Complex planning calls use cancellable 150-second tool-round and 180-second final-response limits. Acceptance ownership is checked only after the normalized global contract is attached, then remains strict.
- Consequence: Users can see a real graph being constructed without allowing an otherwise valid long task to spend unbounded time in planning. Provider latency remains visible and cancellable; malformed ownership cannot fail during the temporary conversion state or bypass validation after contract attachment.

## D-0070 - Conversation continuity is a native durable projection

- Status: Accepted
- Date: 2026-08-07
- Decision: Persist `ConversationThread` identity and `RunContinuationSummary` records in SQLite. A continuation records long-term goals, constraints, completed changes, workspace state, evidence, unresolved items, and next actions. The frontend hydrates native threads and runtime projections from the backend; `localStorage` remains an interface cache rather than the source of conversational truth.
- Consequence: Same-thread follow-up work survives task settlement, context compaction, and application restart. Planner, Worker, verifier, and guidance replan can receive the same durable context version instead of reconstructing history from transient UI state.

## D-0071 - Contour art is procedural and motion is non-linear

- Status: Accepted
- Date: 2026-08-07
- Decision: LunaScope does not ship or request a generated contour image. The WebView derives a stable per-thread seed, builds a five-octave fractal height field locally, and extracts eight SVG isolines with Marching Squares. Contours appear only behind orchestration, waiting, and empty surfaces. Page transitions, new activity, panels, dialogs, lunar phases, streaming indicators, and active graph nodes share explicit non-linear easing tokens; reduced-motion preference collapses these transitions without removing state information.
- Consequence: The visual texture remains offline, naturally varied, reproducible inside a thread, and independent of bundled image assets. Motion communicates navigation or runtime state without adding linear mechanical movement or obscuring readable content.

## D-0072 - Delegation decisions are a visible planning stage

- Status: Accepted
- Date: 2026-08-07
- Decision: Treat the Orchestration model's single-Agent versus multi-Agent decision as a first-class persisted planning stage. Prefer Provider-native reasoning summaries and model-authored `observation -> decision -> next action` commentary. When a model submits its final structured graph without calling the progress tools, derive the public stage only from that same response's rationale, acceptance criteria, Worker topology, dependencies, and write scopes; never manufacture or expose hidden chain-of-thought. Planning, guiding, pausing, and cancelling lock the composer, while stable execution alone accepts guidance.
- Consequence: Every accepted graph has visible evidence, a delegation judgment, and a concrete next action without forcing extra Provider rounds that can degrade planning quality. The global moon remains mounted across routes until the active model lifecycle settles, and the user cannot accidentally start or guide through a critical graph mutation.

## D-0073 - Route motion uses one sequential content stage

- Status: Accepted
- Date: 2026-08-07
- Decision: Do not use browser snapshot cross-fades for workspace navigation. Keep one persistent `#view` stage, complete a short nonlinear exit animation, replace its contents only at the invisible boundary, then run the nonlinear entrance animation. Coalesce rapid requests to the latest destination, suppress nested content-entry effects during the route transaction, lock route controls for the bounded transition, and bypass movement under reduced-motion preference.
- Consequence: Old and new pages can no longer be visible at the same time. Navigation avoids expensive blur and clip-path compositing, does not queue obsolete intermediate destinations, and preserves a single accessibility-busy surface throughout the state change.

## D-0074 - Planning visibility is replayable and graph submission has a one-response fast path

- Status: Accepted; supersedes the planning-round budget in D-0069
- Date: 2026-08-07
- Decision: Start the conversation projection immediately with an honest runtime-authored intake record, then stream Provider reasoning summaries and model-authored planning tools. The Orchestration model may submit the complete typed graph through `submit_orchestration_graph` in that same Provider response; adapters that cannot combine planning and submission retain a bounded text-JSON fallback. Planning uses at most one observable tool round and three total Provider attempts, and a second quality-review request runs only for a concrete invalid, incomplete, non-executable, or broadly assigned graph. Persisted planning activities are replayed into the conversation after graph creation, thread switching, and desktop recovery. Global model-wait UI is visible only in the active conversation and is hard-disabled while paused, pausing, cancelling, idle, or terminal.
- Consequence: A first message receives visible feedback before the remote Provider responds, compatible Providers avoid a mandatory second graph-generation request, and users can inspect the real delegation rationale after restart. A late model event cannot resurrect the moon while paused or make it float over Settings, Plan, Changes, Workers, Artifacts, or the orchestration canvas.

## D-0075 - Public reasoning is model-authored and required before action

- Status: Accepted; supersedes the runtime-authored intake and derived planning-summary portions of D-0072 and D-0074
- Date: 2026-08-07
- Decision: LunaScope treats public reasoning visibility as a runtime protocol, not presentation copy. The WebView may show a neutral model-wait lifecycle, but every reasoning card must contain either a Provider-native public reasoning summary or text/structured progress authored by the model. Runtime-generated observations, decisions, next actions, round counters, and graph-derived imitation reasoning are prohibited. Before graph generation, the Orchestration model produces a concrete public planning briefing with private thinking disabled; the formal graph call then retains the user's configured reasoning effort. Before a Worker executes any side-effecting or evidence-gathering tool, that response must contain Provider public reasoning, model commentary, or `report_progress`; otherwise the tool is not executed and the model must resubmit the action with a public summary. DeepSeek private `reasoning_content` remains replay-only for protocol continuity and is never projected as public thought.
- Consequence: Users see task-specific model judgment, decomposition tradeoffs, evidence, and next actions for both the Orchestration model and named child Agents. Providers that cannot produce any public summary fail closed before an action instead of receiving synthetic filler. Public summaries are durable, replayable, source-labelled, and visually separated per Agent.

## D-0076 - EventEnvelope v2 owns the complete Agent session tree

- Status: Accepted
- Date: 2026-08-09
- Decision: Keep one append-only Rust `EventEnvelope` protocol and add optional `agentSessionId`, `parentSessionId`, `parentEventId`, and `relatedToolEventId`. Persist `AgentSessionRecord` for Primary, Orchestrator, Worker, Supervisor, Verifier, and Context Compressor identities. Store only durable user messages and final task summaries as `ConversationMessage`; operational reasoning, tools, verification, errors, and control records remain typed Agent events.
- Consequence: Tool intent/result linkage, subagent ancestry, restart replay, and UI deduplication no longer depend on inferred role strings or browser storage. Schema v1 remains readable while new events serialize as v2.

## D-0077 - Planner uses three tools and one compatibility continuation

- Status: Accepted; supersedes the dedicated public-briefing call in D-0075 and the three-attempt wording in D-0074
- Date: 2026-08-09
- Decision: The first Orchestrator request exposes exactly `report_progress`, `update_orchestration_draft`, and `submit_orchestration_graph`. Request one concrete public summary, one evidence-linked candidate topology, and the final graph in the same response. Permit one continuation only when the protocol cannot combine those calls. A Provider-native summary or the exact model-authored final rationale/topology is public evidence; private reasoning never is. Local gates review only a concrete omission, broad assignment, write conflict, or missing verifier.
- Consequence: Planning no longer spends a separate model call on explanatory prose or an unrelated `update_plan` checklist. The credential-backed DeepSeek graph canary fell from roughly five minutes in the broken path to 94.57 seconds while retaining eight fine-grained Workers and five-way parallelism.

## D-0078 - Transport retry and verifier repair are separate policies

- Status: Accepted
- Date: 2026-08-09
- Decision: A side-effect-free Provider request may retry five times after the initial attempt at 2, 5, 10, 20, and 40 seconds, respecting longer `Retry-After`; retries stop after any Provider output. Verifier repair has no fixed generation limit. It preserves passed nodes and pauses as `NeedsIntervention` only when three consecutive generations repeat both the fatal-defect fingerprint and the file/test/evidence fingerprint, or at a real user/permission/budget/environment boundary.
- Consequence: Network recovery cannot replay successful tools, while a project is no longer abandoned merely because the fourth or seventh repair generation is needed. A deterministic test proves six progressing failed generations may be followed by a passing seventh.

## D-0079 - Provider brand and wire protocol are independent

- Status: Accepted
- Date: 2026-08-09
- Decision: Treat OpenAI, Anthropic, DeepSeek, GLM, Kimi, Qwen, Grok, and generic relays as configuration identities over OpenAI Responses, OpenAI Chat Completions, or Anthropic Messages. Normalize endpoints without duplicate `/v1`, emit protocol-native effort fields, and validate actual streaming/tool/reasoning behavior through the saved capability test rather than model-name guesses.
- Consequence: OpenAI official and compatible relays can select Responses or Chat, custom base paths and headers remain possible, and normal UI consumes one normalized event stream.

## D-0080 - The desktop shell keeps one viewport and batches event projection

- Status: Accepted
- Date: 2026-08-09
- Decision: Keep one desktop content stage, batch transient event projection for 30-80 ms, patch known event/node IDs in place, cap the live transient timeline at 2,000 events, and keep graph pan/zoom/minimap state per orchestration. Use explicit CSP, custom accessible controls, the Windows GUI subsystem, and `CREATE_NO_WINDOW` for internal child processes.
- Consequence: High-frequency streams no longer repeatedly serialize and rerender the complete conversation, graph updates do not reset the viewport, project switching avoids scrollbar jitter, and release/internal commands do not create a visible PowerShell console.

## D-0081 - Graph geometry and public activity use compact stable projections

- Status: Accepted
- Date: 2026-08-09
- Decision: Render the orchestration graph with fixed-size nodes, topology-level columns, upstream-barycenter ordering, fixed edge anchors, first-open fit, and a 0.12–2.0 zoom range. A committed graph replaces its planning draft instead of sharing the same full-height container. Conversation activity uses the task-derived Agent name and model-authored content directly; frontend taxonomy such as “model reasoning summary”, “observation”, “decision”, and “next” is not repeated in the visible message.
- Consequence: Long task names cannot move edge anchors or overlap adjacent nodes, high-node-count graphs retain a complete overview plus zoomable detail, and the conversation reads as a concise activity stream instead of an explanatory AI dashboard. The underlying typed evidence and activity fields remain durable for inspection and verification.

## D-0082 - Desktop chrome and project management use stable custom surfaces

- Status: Accepted
- Date: 2026-08-09
- Decision: Run the main Tauri window without operating-system decorations and expose only accessible LunaScope window controls. Keep the project-management dialog, its scroll containers, and its column geometry mounted while selection changes patch bounded subregions. Reserve scrollbar gutters for persistent panes and use the shared custom-control layer for visible selects and decision dialogs. Retain semantic text inputs and textareas for IME and accessibility rather than replacing them with content-editable imitations.
- Consequence: Project selection cannot flash because a scrollbar or full dialog shell is removed and reinserted, desktop chrome matches the product, and confirmations no longer fall back to Windows/browser-native prompts. Custom controls must continue to meet keyboard, focus, ARIA, high-DPI, and reduced-motion requirements.

## D-0083 - Orchestration geometry is mounted from typed coordinates

- Status: Accepted
- Date: 2026-08-09
- Decision: Treat generated graph markup as topology and coordinate data, then apply plane, phase, node, and minimap geometry after the packaged WebView has mounted the graph. Select the newest compatible plan projection, key viewports by orchestration version plus a topology fingerprint, rebind the ResizeObserver whenever the graph surface is mounted again, and render an explicit incomplete-data state instead of a synthetic empty graph.
- Consequence: Packaged desktop rendering no longer depends on fragile `innerHTML` geometry attributes, stale plans cannot collapse a revised graph to a synthesis-only view, topology revisions receive a fresh fit without resetting later user pan/zoom, and route or window-size changes keep nodes, edges, phases, and the minimap aligned.
