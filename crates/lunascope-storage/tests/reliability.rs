use std::{
    env,
    path::Path,
    process::{Command, ExitStatus},
    time::Instant,
};

use lunascope_core::{
    CorrelationId, EventData, EventEnvelope, EventId, EventSource, ProjectId, RunId, RunState,
    ThreadId,
};
use lunascope_storage::SqliteEventStore;
use rusqlite::Connection;

const CRASH_CHILD_ENV: &str = "LUNASCOPE_STORAGE_CRASH_CHILD";
const CRASH_DB_ENV: &str = "LUNASCOPE_STORAGE_CRASH_DB";

fn event(run_id: &str, sequence: u64, suffix: &str, payload: EventData) -> EventEnvelope {
    EventEnvelope::new(
        EventId::new(format!("reliability-{run_id}-{suffix}")),
        sequence,
        "2026-07-27T00:00:00Z",
        ProjectId::from("project-reliability"),
        ThreadId::from("thread-reliability"),
        RunId::from(run_id),
        CorrelationId::from(format!("correlation-{run_id}")),
        EventSource::System,
        payload,
    )
}

fn generated_chunk(run_id: &str, index: u64) -> EventEnvelope {
    let mut value = event(
        run_id,
        1,
        &format!("chunk-{index}"),
        EventData::ModelStreamChunk {
            worker_id: None,
            stream_id: "sustained-stream".to_owned(),
            index,
            text: format!("bounded chunk {index}"),
        },
    );
    value.sequence = 0;
    value
}

fn append_running_fixture(store: &SqliteEventStore, run_id: &str) {
    store
        .append(&event(
            run_id,
            1,
            "created",
            EventData::RunCreated {
                title: "Reliability fixture".to_owned(),
                initial_prompt: "Prove durable recovery".to_owned(),
            },
        ))
        .expect("append run");
    store
        .append(&event(
            run_id,
            2,
            "planning",
            EventData::RunStateChanged {
                from: RunState::Created,
                to: RunState::Planning,
                reason: "plan".to_owned(),
            },
        ))
        .expect("append planning");
    store
        .append(&event(
            run_id,
            3,
            "running",
            EventData::RunStateChanged {
                from: RunState::Planning,
                to: RunState::Running,
                reason: "execute".to_owned(),
            },
        ))
        .expect("append running");
}

#[test]
fn injected_projection_failure_rolls_back_events_and_projection() {
    let directory = tempfile::tempdir().expect("temp directory");
    let path = directory.path().join("failure-injection.db");
    let run_id = RunId::from("run-failure-injection");
    let store = SqliteEventStore::open(&path).expect("open store");
    append_running_fixture(&store, run_id.as_str());

    let injector = Connection::open(&path).expect("open injector");
    injector
        .execute_batch(
            "CREATE TRIGGER fail_projection_update
             BEFORE UPDATE OF sequence ON run_projections
             WHEN NEW.sequence = 5
             BEGIN
               SELECT RAISE(ABORT, 'injected projection failure');
             END;",
        )
        .expect("install failure trigger");

    let result = store.append_batch_next(vec![
        {
            let mut value = event(
                run_id.as_str(),
                1,
                "pausing",
                EventData::RunStateChanged {
                    from: RunState::Running,
                    to: RunState::Pausing,
                    reason: "failure-injection batch".to_owned(),
                },
            );
            value.sequence = 0;
            value
        },
        {
            let mut value = event(
                run_id.as_str(),
                1,
                "paused",
                EventData::RunStateChanged {
                    from: RunState::Pausing,
                    to: RunState::Paused,
                    reason: "must roll back with the batch".to_owned(),
                },
            );
            value.sequence = 0;
            value
        },
    ]);
    assert!(result.is_err());
    assert_eq!(store.events_after(&run_id, 3).unwrap(), Vec::new());
    let projection = store.projection(&run_id).unwrap().expect("projection");
    assert_eq!(projection.sequence, 3);
    assert_eq!(projection.run_state, RunState::Running);

    injector
        .execute_batch("DROP TRIGGER fail_projection_update;")
        .expect("remove failure trigger");
    drop(injector);
    drop(store);

    let reopened = SqliteEventStore::open(&path).expect("reopen after failure");
    let recovered = reopened.recover(&run_id).unwrap().expect("recover run");
    assert_eq!(recovered.sequence, 3);
    assert_eq!(recovered.run_state, RunState::Running);
}

#[test]
fn unclean_process_exit_recovers_committed_wal() {
    let directory = tempfile::tempdir().expect("temp directory");
    let path = directory.path().join("unclean-exit.db");
    let status = run_crash_child(&path);
    assert_eq!(status.code(), Some(86));

    let reopened = SqliteEventStore::open(&path).expect("reopen after unclean exit");
    let recovered = reopened
        .recover(&RunId::from("run-unclean-exit"))
        .unwrap()
        .expect("recover committed run");
    assert_eq!(recovered.sequence, 3);
    assert_eq!(recovered.run_state, RunState::Running);
}

fn run_crash_child(path: &Path) -> ExitStatus {
    Command::new(env::current_exe().expect("current test executable"))
        .args([
            "--ignored",
            "--exact",
            "crash_child_writes_then_exits_without_unwinding",
        ])
        .env(CRASH_CHILD_ENV, "1")
        .env(CRASH_DB_ENV, path)
        .status()
        .expect("run crash child")
}

#[test]
#[ignore = "helper executed in a child process by the unclean-exit test"]
fn crash_child_writes_then_exits_without_unwinding() {
    if env::var_os(CRASH_CHILD_ENV).is_none() {
        return;
    }
    let path = env::var_os(CRASH_DB_ENV).expect("crash database path");
    let store = SqliteEventStore::open(path).expect("open child store");
    append_running_fixture(&store, "run-unclean-exit");
    std::process::exit(86);
}

#[test]
#[ignore = "sustained 5,000-event reliability and performance baseline"]
fn sustained_event_ingest_snapshot_and_recovery_baseline() {
    const EVENT_COUNT: u64 = 5_000;
    const BATCH_SIZE: u64 = 100;
    let directory = tempfile::tempdir().expect("temp directory");
    let path = directory.path().join("long-run.db");
    let run_id = RunId::from("run-long-baseline");
    let started = Instant::now();
    {
        let store = SqliteEventStore::open(&path).expect("open store");
        append_running_fixture(&store, run_id.as_str());
        for batch_start in (0..EVENT_COUNT).step_by(BATCH_SIZE as usize) {
            let batch_end = (batch_start + BATCH_SIZE).min(EVENT_COUNT);
            let events = (batch_start..batch_end)
                .map(|index| generated_chunk(run_id.as_str(), index))
                .collect();
            store
                .append_batch_next(events)
                .expect("append stream batch");
            if batch_end % 1_000 == 0 && batch_end < EVENT_COUNT {
                store
                    .create_snapshot(&run_id)
                    .expect("create periodic snapshot");
            }
        }
        assert_eq!(
            store
                .projection(&run_id)
                .unwrap()
                .expect("projection")
                .sequence,
            EVENT_COUNT + 3
        );
    }
    let ingest_elapsed = started.elapsed();
    let recovery_started = Instant::now();
    let reopened = SqliteEventStore::open(&path).expect("reopen long-run store");
    let recovered = reopened.recover(&run_id).unwrap().expect("recover run");
    let recovery_elapsed = recovery_started.elapsed();
    assert_eq!(recovered.sequence, EVENT_COUNT + 3);
    assert_eq!(recovered.run_state, RunState::Running);

    println!("events={EVENT_COUNT}");
    println!("database_bytes={}", path.metadata().unwrap().len());
    println!("ingest_ms={}", ingest_elapsed.as_millis());
    println!("recovery_ms={}", recovery_elapsed.as_millis());
}
