# LunaScope Agent Runtime Architecture

## Reference-derived invariants

LunaScope independently follows four runtime invariants observed in the current public Codex and OpenCode implementations:

1. A run is a sequence of typed, durable items. A Worker is not one opaque model future.
2. A tool call has an explicit lifecycle: requested before the side effect, then completed or failed exactly once.
3. Tool calls and results retain their Provider-native linkage in the next sampling request.
4. Context pressure is handled by bounded pruning or compaction while recent settled turns and workspace evidence remain available.

The reference projects are acknowledged with their licenses in `docs/THIRD_PARTY_NOTICES.md`. Their code is not copied or vendored.

## Current vertical slice

The native Provider Worker loop now:

- keeps assistant tool calls and tool results as typed conversation items;
- serializes those items as OpenAI Responses `function_call`/`function_call_output`, Chat Completions `tool_calls`/`tool`, or Anthropic `tool_use`/`tool_result`;
- writes `ToolCallRequested` before executing a native filesystem or process tool;
- writes `ToolCallCompleted` immediately after success or failure;
- assigns a stable run/Worker/attempt/call idempotency key;
- bounds durable tool evidence and model-visible tool output separately;
- compacts older settled turns without discarding the latest turn or changing the real workspace.

Three adaptive layers now sit around that loop:

- a model-aware effort profile maps the Rust reasoning contract to each Provider's native wire shape;
- a fail-closed independent vision stage turns bounded visual assets into inert, focus-aware evidence for text-only models;
- the Orchestration model reviews bounded settled-tool checkpoints and can guide or replan existing unfinished Workers at their next safe boundary.

Provider-private reasoning fields are never shown as UI reasoning. DeepSeek's private `reasoning_content` is held in memory only for the directly linked assistant-tool/result continuation required by its protocol. User-visible reasoning remains either a Provider-authored reasoning summary or explicit model commentary with observable evidence.

The desktop projection converts those settled facts into two presentation layers:

- a natural-language conversation narrative for meaningful milestones and concrete operations;
- a compact bottom activity line for the single operation happening now.

Narrative text is deterministic UI prose over runtime events. It is not model chain-of-thought. Raw character-count stream updates are intentionally omitted.

## Restart and side-effect reconciliation

Recovery reads committed `ToolCallRequested`/completion events before changing the run projection. For bounded file operations whose exact result can be proven from the latest Worker worktree, LunaScope records the reconciled completion without executing the side effect again. A deterministic failure-injection test covers the sequence “intent committed → write happened → crash before completion → SQLite reopen → reconcile without replay.”

Tools whose external mutation cannot be proven, including an interrupted generic patch or process, enter `RECOVERY_NEEDS_INTERVENTION`. Automatic retry is rejected until a user inspects the preserved workspace and starts an explicit repair. Ambiguity never becomes a blind replay.

## Remaining migration

The remaining reliability work is deliberately bounded:

1. Expand exact state reconciliation only for tools whose real effect can be proven without replay; all other mutating tools retain the `NeedsIntervention` fallback.
2. Replace deterministic old-turn pruning with a persisted semantic checkpoint when the active Provider approaches its token limit.
3. Move Worker step scheduling out of the desktop adapter and into a runtime-owned turn engine so CLI, desktop, and future headless hosts share one execution contract.
4. Produce reviewed multi-hour endurance and secret-enabled Provider-canary evidence before a Stable release.

No UI, eval manifest, or release note may claim a remaining stage has passed without its deterministic or release evidence.
