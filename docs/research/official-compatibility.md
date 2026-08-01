# Official Compatibility and License Record

Research date: 2026-07-27

Only official documentation and official repositories are normative. Feature compatibility is implemented independently from public contracts; source code is reused only when its license permits and attribution obligations are satisfied.

## Tauri 2

- Official command documentation: https://v2.tauri.app/develop/calling-rust/
- Official repository: https://github.com/tauri-apps/tauri
- License: dual MIT or Apache-2.0.
- Relevant contract: commands are narrow request/response IPC; Tauri channels are recommended for ordered/high-throughput streaming; async event listeners may process events out of order.
- LunaScope decision: use commands for explicit actions and channels for ordered Snapshot/Delta/model/tool streams. Keep runtime crates independent from Tauri.

## OpenAI Codex

- Official repository: https://github.com/openai/codex
- App-server protocol: https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md
- License: Apache-2.0.
- Relevant contract: the maintained CLI is Rust; it supports MCP client/server behavior. `codex app-server` exposes a bidirectional JSON-RPC interface over stdio and documents lifecycle, events, approvals, skills, apps, and auth endpoints.
- LunaScope decision: implement an out-of-process Codex bridge against the documented app-server/CLI protocol. Reuse is optional and must retain Apache notices. ChatGPT authentication stays inside the official Codex process.

## Anthropic Claude Code

- Official product repository: https://github.com/anthropics/claude-code
- Official license: https://github.com/anthropics/claude-code/blob/main/LICENSE.md
- Official setup: https://docs.anthropic.com/en/docs/claude-code/getting-started
- License: copyright Anthropic; use subject to Anthropic Commercial Terms. It is not an open-source implementation license.
- Relevant contract: current official releases support plugins composed of commands, agents, skills, hooks, and MCP configuration; the product has a supported Windows installation.
- LunaScope decision: do not copy, vendor, modify, or reverse engineer Claude Code implementation. Compatibility means parsing documented declarative formats and optionally supervising the user's official installed CLI as an out-of-process bridge.

## OpenCode

- Official documentation: https://opencode.ai/docs/
- Skills: https://opencode.ai/docs/skills
- Tools and permissions: https://opencode.ai/docs/tools
- Official repository: https://github.com/anomalyco/opencode
- License: MIT.
- Relevant contract: on-demand `SKILL.md` loading supports `.opencode/skills`, `.claude/skills`, and `.agents/skills`; skill access supports allow/ask/deny; tools and MCP are permission-controlled.
- LunaScope decision: normalize the documented `SKILL.md` subset and preserve unknown platform metadata. Any OpenCode runtime bridge remains out-of-process.

## Compatibility rule

LunaScope must report one of:

- Native: declarative contract executes within LunaScope without third-party runtime semantics.
- Compatible: normalized safely; unsupported metadata is preserved.
- Bridge-required: execution is delegated to an official installed third-party runtime.
- Unsupported: inspect/export only.

No compatibility claim implies identical behavior, bundled third-party credentials, subscription-to-API conversion, or permission bypass.

