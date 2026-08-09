use lunascope_core::{
    ConversationMessage, ConversationRole, ConversationThread, ProjectId, RunContinuationSummary,
    RunId, ThreadContextSummary, ThreadId,
};
use lunascope_storage::SqliteEventStore;

fn message(id: &str, role: ConversationRole, content: &str) -> ConversationMessage {
    ConversationMessage {
        message_id: id.to_owned(),
        project_id: ProjectId::from("project-context"),
        thread_id: ThreadId::from("thread-context"),
        run_id: Some(RunId::from("run-context")),
        sequence: 0,
        role,
        content: content.to_owned(),
        context_content: None,
        created_at: "2026-08-01T00:00:00Z".to_owned(),
    }
}

#[test]
fn conversation_identity_and_run_continuation_survive_reopen() {
    let temporary = tempfile::tempdir().expect("temporary database");
    let database = temporary.path().join("conversation.db");
    let store = SqliteEventStore::open(&database).expect("open store");
    let thread = ConversationThread {
        thread_id: ThreadId::from("thread-context"),
        project_id: ProjectId::from("project-context"),
        title: "Long-running project".into(),
        active_run_id: Some(RunId::from("run-context")),
        context_revision: 3,
        created_at: "2026-08-01T00:00:00Z".into(),
        updated_at: "2026-08-01T00:02:00Z".into(),
    };
    store
        .save_conversation_thread(&thread)
        .expect("persist conversation identity");
    let continuation = RunContinuationSummary {
        thread_id: thread.thread_id.clone(),
        run_id: RunId::from("run-context"),
        revision: 4,
        goals: vec!["Complete the same workspace project".into()],
        constraints: vec!["Preserve prior files".into()],
        completed_changes: vec!["Created src/core.rs".into()],
        workspace_state: vec!["src/core.rs · sha256".into()],
        evidence: vec!["cargo test passed".into()],
        unresolved_items: vec!["Add browser verification".into()],
        next_actions: vec!["Run browser verification".into()],
        created_at: "2026-08-01T00:02:00Z".into(),
    };
    store
        .save_run_continuation_summary(&continuation)
        .expect("persist continuation");
    drop(store);

    let reopened = SqliteEventStore::open(&database).expect("reopen store");
    let threads = reopened
        .conversation_threads("project-context")
        .expect("load conversations");
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].title, "Long-running project");
    assert_eq!(threads[0].context_revision, 4);
    assert_eq!(
        reopened
            .latest_run_continuation_summary("thread-context")
            .expect("load continuation"),
        Some(continuation)
    );
}

#[test]
fn conversation_messages_are_ordered_and_idempotent() {
    let store = SqliteEventStore::open_in_memory().expect("open store");
    let first = store
        .append_conversation_message(message(
            "message-user-1",
            ConversationRole::User,
            "Create the project and verify it.",
        ))
        .expect("append user message");
    let second = store
        .append_conversation_message(message(
            "message-assistant-1",
            ConversationRole::Assistant,
            "The project and its verification report are complete.",
        ))
        .expect("append assistant summary");
    let duplicate = store
        .append_conversation_message(message(
            "message-user-1",
            ConversationRole::User,
            "this conflicting retry must not overwrite durable history",
        ))
        .expect("deduplicate message");

    assert_eq!(first.sequence, 1);
    assert_eq!(second.sequence, 2);
    assert_eq!(duplicate, first);
    assert_eq!(
        store
            .conversation_messages("thread-context", 0)
            .expect("load all messages"),
        vec![first.clone(), second.clone()]
    );
    assert_eq!(
        store
            .conversation_messages("thread-context", 1)
            .expect("load messages after checkpoint"),
        vec![second]
    );
}

#[test]
fn compressed_summary_is_a_checkpoint_without_deleting_source_messages() {
    let store = SqliteEventStore::open_in_memory().expect("open store");
    store
        .append_conversation_message(message(
            "message-user-1",
            ConversationRole::User,
            "Preserve every original message.",
        ))
        .expect("append source message");
    let summary = ThreadContextSummary {
        thread_id: ThreadId::from("thread-context"),
        revision: 1,
        covered_through_sequence: 1,
        source_tokens: 128,
        summary_tokens: 24,
        summary: "The user requires immutable source history.".to_owned(),
        updated_at: "2026-08-01T00:01:00Z".to_owned(),
    };
    store
        .save_thread_context_summary(&summary)
        .expect("save context checkpoint");

    assert_eq!(
        store
            .thread_context_summary("thread-context")
            .expect("load context checkpoint"),
        Some(summary)
    );
    assert_eq!(
        store
            .conversation_messages("thread-context", 0)
            .expect("source messages remain")
            .len(),
        1
    );

    store
        .delete_thread("thread-context")
        .expect("delete conversation");
    assert!(
        store
            .conversation_messages("thread-context", 0)
            .expect("load deleted messages")
            .is_empty()
    );
    assert_eq!(
        store
            .thread_context_summary("thread-context")
            .expect("load deleted summary"),
        None
    );
}
