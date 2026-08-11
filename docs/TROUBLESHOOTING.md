# LunaScope Troubleshooting

Open **Settings → General → Environment** to retry checks, view advanced paths and versions, or copy redacted diagnostics. Copied diagnostics omit local data/workspace/tool paths and remove API keys, Authorization/Bearer values, passwords, tokens, and sensitive headers.

## Windows SmartScreen

The public 0.3.1 binaries are unsigned. Download only from the project Release, compare the file against `SHA256SUMS.txt`, then use **More info → Run anyway** only if the source and digest match. A missing code-signing certificate keeps the Stable release gate closed.

## Git was not found (`GIT_NOT_FOUND`)

LunaScope needs Git to isolate and safely integrate Agent changes.

1. Install [Git for Windows](https://git-scm.com/download/win).
2. Close and reopen LunaScope so the process receives the updated `PATH`.
3. Select **Retry check**.

LunaScope does not silently fall back to direct writes.

## WebView2 or the window does not open

Install or repair Microsoft Edge WebView2 Runtime, then restart Windows and LunaScope. If the packaged app still does not open, include the Windows version and the redacted diagnostic output in a report.

## Provider authentication failed (`PROVIDER_AUTH_FAILED`)

Review the Provider configuration in **Settings → Models & Providers**, re-enter the credential if needed, and use **Test connection**. Do not paste a credential into an issue or diagnostic attachment.

## Model or reasoning setting is unsupported

For `PROVIDER_MODEL_UNSUPPORTED` or `MODEL_REASONING_TEST_REQUIRED`:

1. Confirm the exact model name in the Provider account.
2. Keep reasoning on `Auto` unless the Provider documents another value.
3. Confirm the endpoint and protocol in the Provider configuration.
4. Use the per-model **Run real test** control before saving the routing change.

The test result is scoped to the exact Provider configuration, model, endpoint, credential reference, and reasoning value. It remains valid across restarts until one of those values changes.

## Data directory unavailable or not writable

LunaScope normally uses the saved data root, an existing detected legacy data root, or the Windows per-user default in that order.

- Reconnect the original drive and select **Retry check**; or
- select **Choose data directory**, choose a writable location, and restart LunaScope.

Recovery mode allows only diagnosis, directory replacement, restart, and the read-only language preference. Persistent Agent, Provider, project, extension, Companion, and UltraNote writes remain blocked to prevent data from splitting across roots. LunaScope does not copy or delete the old root.

## Workspace is invalid or Windows denied access

For `WORKSPACE_INVALID` or `WORKSPACE_PERMISSION_DENIED`, select a real folder owned by your Windows account. If Controlled Folder Access is enabled, allow LunaScope in Windows Security or choose another workspace. Exactly one project folder must be marked as the workspace.

## Browser is unavailable (`BROWSER_NOT_AVAILABLE`)

Install or repair Microsoft Edge. This check appears only for a plan that requires real browser verification; unrelated tasks remain usable.

## A task-specific tool is missing (`TOOL_DEPENDENCY_MISSING`)

Open the advanced environment details to see the missing capability. Install the named Node/npm, Python, or Cargo tool, restart LunaScope, and retry. The requirement comes from the current plan and workspace, not from a global installation checklist.

## Run interrupted or recovery needs intervention

- `RUN_INTERRUPTED`: review the preserved Changes and diagnostics, then retry only unfinished work.
- `RECOVERY_NEEDS_INTERVENTION`: LunaScope found a mutating tool request whose real side effect cannot be proven. It will not replay that tool automatically. Inspect the preserved workspace and start an explicit repair task after confirming the current state.

Known idempotent file operations are reconciled against the real worker worktree after restart. Ambiguous mutations fail closed.

## Collect diagnostics safely

1. Open **Settings → General → Environment**.
2. Expand **Advanced details** if you need to inspect local paths and versions.
3. Select **Copy redacted diagnostics**.
4. Review the copied text before sharing it.

Never share API keys, Windows Credential Manager contents, Authorization headers, cookies, signing certificates, or private source files. Security-sensitive reports should use the private process in [SECURITY.md](../SECURITY.md).
