# Runtime Architecture

## Dependency direction

```text
apps/desktop (Tauri adapter + WebView)
        |
        v
lunascope-runtime  ---> lunascope-integrations
        |                     |
        v                     v
lunascope-core <--- lunascope-storage
        ^
        |
lunascope-learning
```

`lunascope-core` has no Tauri, database, network, model, or operating-system dependency.

## Authoritative write path

1. A narrow command is deserialized.
2. Command schema, state preconditions, path/scope, and policy are validated.
3. If approval is required, an approval event is committed and execution stops.
4. The runtime opens a SQLite transaction.
5. It allocates the next run sequence and appends one typed event with a unique event ID.
6. It updates the projection in the same transaction.
7. After commit, the desktop adapter sends an ordered delta through a Tauri channel.
8. The frontend accepts only the next sequence; gaps trigger resubscription.

Tool side effects use a two-event intent/result protocol and an idempotency key. Recovery reconciles committed intents without committed results before retrying.

## Recovery path

1. Open SQLite with WAL and foreign keys enabled.
2. Load the newest compatible snapshot for the run.
3. Replay committed events after the snapshot sequence.
4. Reconcile pending approvals, tool intents, and owned processes.
5. Mark interrupted model streams for retry or partial completion.
6. Publish a complete snapshot, followed by deltas after its sequence.

Snapshots are acceleration artifacts. Deleting every snapshot must not lose committed state.

## IPC

Commands are explicit actions such as `create_run`, `pause_run`, `resume_run`, `cancel_run`, `approve_action`, `create_checkpoint`, and `restore_checkpoint`.

Ordered streams use Tauri channels for:

- snapshot
- delta
- model chunks
- tool output
- worker state
- approval
- artifact
- verification

There is no `execute_any_command(string)` surface.

## Contract versioning

- Every persisted event has `schemaVersion`.
- Event discriminants and field casing are stable public data contracts.
- Rust generates the checked-in TypeScript declarations and JSON Schema.
- CI fails when generated artifacts differ.
- Additive readers tolerate known optional fields; incompatible changes require a migration and fixtures from the prior schema.

