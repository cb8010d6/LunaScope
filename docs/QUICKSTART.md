# LunaScope Quickstart

This guide takes a new Windows user from a release download to the first Agent run. Advanced routing, Worker pools, Vision, MCP, UltraNote, and Companion are not required.

## Before you start

- Windows 10 or 11 with Microsoft Edge WebView2 Runtime.
- [Git for Windows](https://git-scm.com/download/win). LunaScope uses Git worktrees to isolate Agent changes; it does not fall back to editing a project directly when Git is missing.
- An API key for a supported AI Provider.
- A project folder your Windows account can read and write.

## 1. Download and verify

Download the installer or portable executable from the GitHub Release. Compare its SHA-256 digest with the release's `SHA256SUMS.txt` before running it.

The current public 0.3.1 binaries are unsigned. If SmartScreen appears, verify the digest and source before choosing **More info → Run anyway**. Unsigned artifacts are release candidates, not Stable releases.

## 2. Start LunaScope

LunaScope no longer requires a particular drive. On a first launch it uses the Windows per-user application-data location. If an older installation already has a detected legacy data root, LunaScope continues to reference that directory without copying it.

If a previously selected drive is disconnected or read-only, LunaScope opens in recovery mode. Choose a writable replacement in **Settings → General → Environment**, then restart. Existing data is not deleted or copied automatically.

## 3. Configure a Provider

Open **Settings → Models & Providers / 模型与提供商**, enter the Provider configuration, credential reference, model, and any required protocol or capability settings, then save it. The API key is stored in Windows Credential Manager and is not written to the database or WebView.

Use the existing **Test connection** and per-model **Run real test** controls when you need to verify an endpoint or reasoning value before changing routing settings. Keep the full configuration visible in the settings page; advanced routing, Vision, and Worker assignments are optional.

## 4. Choose a folder

Create a project, add one or more folders, and mark exactly one folder as the workspace. The workspace is the boundary in which Agents may create isolated worktrees, run checks, and integrate verified changes.

## 5. Start a task

Enter a concrete request. Before planning, LunaScope checks:

- data directory access;
- workspace access;
- Git;
- an enabled Provider and a valid orchestration model selection.

Node/npm, Python, Cargo, or a browser are checked only when the generated task plan actually requires them. A Markdown task is not blocked because an unrelated development tool is absent.

The result is either **Ready** or an actionable issue with a retry action. During execution, inspect the conversation, Agent graph, tool activity, changes, and independent verification evidence.

For installation, upgrade, portable behavior, and full removal, see [Installation, upgrade, and uninstall](INSTALLATION.md). For common failures, see [Troubleshooting](TROUBLESHOOTING.md).
