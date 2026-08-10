# Installation, upgrade, and uninstall

## Installer

Download the Windows installer and `SHA256SUMS.txt` from the same GitHub Release. Verify the digest before execution. Until a real Windows signing certificate is configured, artifacts are labeled `unsigned-rc` and must not be presented as Stable.

The installer installs the application. It does not install a Windows service.

## Portable executable

The portable executable skips installation, but it is not a self-contained data mode. It uses the same Windows per-user configuration and data-root selection as the installed application; it does not keep databases, credentials, models, or caches beside the executable.

## Data and credentials

The data root is resolved in this order:

1. a previously saved, valid absolute path;
2. an existing legacy data root from an earlier LunaScope release;
3. the Windows per-user application-data directory (typically `%LOCALAPPDATA%\com.lunascope.workbench`).

The small data-root selection file is stored under the Windows per-user application configuration directory (typically `%APPDATA%\com.lunascope.workbench\data-root.json`). The selected root contains:

```text
<data-root>/
├─ state/                 SQLite state and projections
├─ extensions/            system/user Skills, Tools and MCP metadata
├─ companion/
│  ├─ models/             installed Companion assets
│  ├─ preload/            temporary gallery preload cache
│  └─ runtime/            bounded Companion runtimes
└─ worktrees/             isolated Agent worktrees and artifacts
```

Provider and MCP secret values are stored in Windows Credential Manager, not inside the data root.

## Upgrade

Installing a newer version reuses the saved data root and existing SQLite/provider metadata. A detected legacy root is referenced in place; LunaScope does not automatically copy large model, course, extension, or worktree directories.

Before a major upgrade, close LunaScope and back up the selected data root. Do not merge two active roots manually.

## Uninstall

Uninstalling the application removes installed application files but intentionally preserves user data and Windows credentials. This prevents an application uninstall from silently deleting projects, Provider metadata, extensions, Companion assets, or course data.

## Completely remove LunaScope data

This operation is destructive:

1. Open **Settings → General → Environment** and record the current data root.
2. Close LunaScope.
3. Back up anything you may need.
4. Remove the exact selected data root.
5. Remove the LunaScope per-user configuration directory, including `data-root.json`.
6. Remove LunaScope credential entries from Windows Credential Manager.

If the selected root is a legacy data root, verify the exact directory shown in **Settings → General → Environment** before deleting it. Never delete an entire drive or broad user directory.
