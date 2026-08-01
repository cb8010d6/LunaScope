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

The desktop projection converts those settled facts into two presentation layers:

- a natural-language conversation narrative for meaningful milestones and concrete operations;
- a compact bottom activity line for the single operation happening now.

Narrative text is deterministic UI prose over runtime events. It is not model chain-of-thought. Raw character-count stream updates are intentionally omitted.

## Remaining migration

The next reliability stages are deliberately separated from the current runnable slice:

1. Project settled model/tool items into a resumable per-Worker conversation journal.
2. Reconcile an unfinished mutating call on restart from its idempotency key and observed workspace state before deciding whether it is safe to retry.
3. Replace deterministic old-turn pruning with a persisted semantic checkpoint when the active Provider approaches its token limit.
4. Move Worker step scheduling out of the desktop adapter and into a runtime-owned turn engine so CLI, desktop, and future headless hosts share one execution contract.

No UI or retry policy may claim these remaining stages are implemented until recovery tests prove them across an unclean process exit.
