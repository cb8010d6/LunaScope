use lunascope_core::{
    CorrelationId, EventData, EventEnvelope, EventId, EventSource, LunaProject, ProjectFolder,
    ProjectId, RunId, ThreadId,
};
use lunascope_storage::SqliteEventStore;

fn append_run(store: &SqliteEventStore, project_id: &str, thread_id: &str, run_id: &str) {
    store
        .append(&EventEnvelope::new(
            EventId::new(format!("event-{run_id}")),
            1,
            "2026-07-28T00:00:00Z",
            ProjectId::from(project_id),
            ThreadId::from(thread_id),
            RunId::from(run_id),
            CorrelationId::from(format!("correlation-{run_id}")),
            EventSource::User,
            EventData::RunCreated {
                title: format!("Conversation {thread_id}"),
                initial_prompt: String::new(),
            },
        ))
        .expect("append run");
}

#[test]
fn deleting_a_thread_removes_only_its_events_and_runtime_state() {
    let store = SqliteEventStore::open_in_memory().expect("open store");
    append_run(&store, "project-a", "thread-a", "run-a");
    append_run(&store, "project-a", "thread-b", "run-b");
    store
        .create_snapshot(&RunId::from("run-a"))
        .expect("create snapshot");

    assert_eq!(store.delete_thread("thread-a").expect("delete thread"), 1);
    assert!(
        store
            .projection(&RunId::from("run-a"))
            .expect("load deleted projection")
            .is_none()
    );
    assert!(
        store
            .recover(&RunId::from("run-a"))
            .expect("recover deleted run")
            .is_none()
    );
    assert!(
        store
            .projection(&RunId::from("run-b"))
            .expect("load retained projection")
            .is_some()
    );
}

#[test]
fn deleting_a_project_removes_project_runs_but_not_local_files() {
    let directory = tempfile::tempdir().expect("temp directory");
    let marker = directory.path().join("keep.txt");
    std::fs::write(&marker, "keep me").expect("write marker");
    let store = SqliteEventStore::open_in_memory().expect("open store");
    let project = LunaProject {
        project_id: "project-a".to_owned(),
        name: "Disposable metadata".to_owned(),
        folders: vec![ProjectFolder {
            folder_id: "folder-a".to_owned(),
            path: directory.path().to_string_lossy().into_owned(),
            display_name: "workspace".to_owned(),
            is_workspace: true,
        }],
        kind: lunascope_core::ProjectKind::General,
        ultranote: None,
        created_at: "2026-07-28T00:00:00Z".to_owned(),
        updated_at: "2026-07-28T00:00:00Z".to_owned(),
    };
    store.save_project(&project).expect("save project");
    append_run(&store, "project-a", "thread-a", "run-a");
    append_run(&store, "project-b", "thread-b", "run-b");

    assert!(store.delete_project("project-a").expect("delete project"));
    assert!(
        store
            .project("project-a")
            .expect("load deleted project")
            .is_none()
    );
    assert!(
        store
            .projection(&RunId::from("run-a"))
            .expect("load deleted project run")
            .is_none()
    );
    assert!(
        store
            .projection(&RunId::from("run-b"))
            .expect("load retained run")
            .is_some()
    );
    assert_eq!(
        std::fs::read_to_string(marker).expect("read retained marker"),
        "keep me"
    );
}
