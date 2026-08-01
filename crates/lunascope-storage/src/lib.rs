use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use lunascope_core::{
    ConversationMessage, Course, CourseAssignment, CourseConcept, CourseSearchHit, CourseSource,
    CourseThreadBinding, EventEnvelope, EventId, LunaProject, McpServerConfig, ModelRoutingPolicy,
    ModelSelectionSettings, NoteRevision, ProjectionError, ProviderConfig, ReviewItem, RunId,
    RuntimeSnapshot, SyllabusRevision, ThreadContextSummary, UserPreferences,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use thiserror::Error;

const MIGRATION_1: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS events (
    run_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    event_id TEXT NOT NULL UNIQUE,
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    timestamp TEXT NOT NULL,
    project_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    correlation_id TEXT NOT NULL,
    causation_id TEXT,
    envelope_json TEXT NOT NULL,
    PRIMARY KEY (run_id, sequence)
);

CREATE INDEX IF NOT EXISTS idx_events_correlation
ON events(correlation_id);

CREATE TABLE IF NOT EXISTS run_projections (
    run_id TEXT PRIMARY KEY,
    sequence INTEGER NOT NULL,
    snapshot_json TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS snapshots (
    run_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    schema_version INTEGER NOT NULL,
    snapshot_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (run_id, sequence)
);
"#;

const MIGRATION_2: &str = r#"
CREATE TABLE IF NOT EXISTS provider_configs (
    id TEXT PRIMARY KEY,
    config_json TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS routing_policies (
    scope_id TEXT PRIMARY KEY,
    policy_json TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
"#;

const MIGRATION_3: &str = r#"
CREATE TABLE IF NOT EXISTS model_selection_settings (
    scope_id TEXT PRIMARY KEY,
    settings_json TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
"#;

const MIGRATION_4: &str = r#"
CREATE TABLE IF NOT EXISTS mcp_server_configs (
    id TEXT PRIMARY KEY,
    config_json TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
"#;

const MIGRATION_5: &str = r#"
CREATE TABLE IF NOT EXISTS courses (
    course_id TEXT PRIMARY KEY,
    course_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS lecture_threads (
    thread_id TEXT PRIMARY KEY,
    course_id TEXT NOT NULL REFERENCES courses(course_id) ON DELETE CASCADE,
    binding_json TEXT NOT NULL,
    bound_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_lecture_threads_course
ON lecture_threads(course_id);

CREATE TABLE IF NOT EXISTS course_sources (
    source_id TEXT PRIMARY KEY,
    course_id TEXT NOT NULL REFERENCES courses(course_id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    source_json TEXT NOT NULL,
    content_text TEXT NOT NULL,
    imported_at TEXT NOT NULL,
    UNIQUE(source_id, course_id)
);

CREATE INDEX IF NOT EXISTS idx_course_sources_course
ON course_sources(course_id, imported_at);

CREATE VIRTUAL TABLE IF NOT EXISTS course_search_fts USING fts5(
    course_id UNINDEXED,
    source_id UNINDEXED,
    content
);

CREATE TABLE IF NOT EXISTS syllabus_revisions (
    revision_id TEXT PRIMARY KEY,
    course_id TEXT NOT NULL REFERENCES courses(course_id) ON DELETE CASCADE,
    source_id TEXT NOT NULL,
    revision_number INTEGER NOT NULL CHECK(revision_number > 0),
    revision_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(course_id, revision_number),
    FOREIGN KEY(source_id, course_id)
        REFERENCES course_sources(source_id, course_id) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS course_learning_objectives (
    objective_id TEXT PRIMARY KEY,
    course_id TEXT NOT NULL REFERENCES courses(course_id) ON DELETE CASCADE,
    revision_id TEXT NOT NULL REFERENCES syllabus_revisions(revision_id) ON DELETE CASCADE,
    objective_text TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
    UNIQUE(revision_id, ordinal)
);

CREATE TABLE IF NOT EXISTS note_revisions (
    note_revision_id TEXT PRIMARY KEY,
    course_id TEXT NOT NULL REFERENCES courses(course_id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL REFERENCES lecture_threads(thread_id) ON DELETE CASCADE,
    source_id TEXT NOT NULL,
    revision_number INTEGER NOT NULL CHECK(revision_number > 0),
    note_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(course_id, thread_id, revision_number),
    FOREIGN KEY(source_id, course_id)
        REFERENCES course_sources(source_id, course_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_course_notes_course
ON note_revisions(course_id, created_at);

CREATE TABLE IF NOT EXISTS citation_anchors (
    anchor_id TEXT PRIMARY KEY,
    course_id TEXT NOT NULL REFERENCES courses(course_id) ON DELETE CASCADE,
    source_id TEXT NOT NULL,
    anchor_json TEXT NOT NULL,
    FOREIGN KEY(source_id, course_id)
        REFERENCES course_sources(source_id, course_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS course_concepts (
    concept_id TEXT PRIMARY KEY,
    course_id TEXT NOT NULL REFERENCES courses(course_id) ON DELETE CASCADE,
    concept_json TEXT NOT NULL,
    UNIQUE(concept_id, course_id)
);

CREATE TABLE IF NOT EXISTS course_assignments (
    assignment_id TEXT PRIMARY KEY,
    course_id TEXT NOT NULL REFERENCES courses(course_id) ON DELETE CASCADE,
    assignment_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS course_review_items (
    review_item_id TEXT PRIMARY KEY,
    course_id TEXT NOT NULL REFERENCES courses(course_id) ON DELETE CASCADE,
    concept_id TEXT NOT NULL,
    next_review_at TEXT NOT NULL,
    review_json TEXT NOT NULL,
    FOREIGN KEY(concept_id, course_id)
        REFERENCES course_concepts(concept_id, course_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_course_review_due
ON course_review_items(course_id, next_review_at);

CREATE TABLE IF NOT EXISTS course_memory_entries (
    entry_id TEXT PRIMARY KEY,
    course_id TEXT NOT NULL REFERENCES courses(course_id) ON DELETE CASCADE,
    category TEXT NOT NULL,
    value TEXT NOT NULL,
    source_anchor_id TEXT,
    user_confirmed INTEGER NOT NULL DEFAULT 0,
    pinned INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
"#;

const MIGRATION_6: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
    project_id TEXT PRIMARY KEY,
    project_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS user_preferences (
    scope_id TEXT PRIMARY KEY,
    preferences_json TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
"#;

const MIGRATION_7: &str = r#"
CREATE TABLE IF NOT EXISTS conversation_messages (
    message_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    run_id TEXT,
    sequence INTEGER NOT NULL CHECK(sequence > 0),
    role TEXT NOT NULL,
    message_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(thread_id, sequence)
);

CREATE INDEX IF NOT EXISTS idx_conversation_messages_thread
ON conversation_messages(thread_id, sequence);

CREATE TABLE IF NOT EXISTS thread_context_summaries (
    thread_id TEXT PRIMARY KEY,
    covered_through_sequence INTEGER NOT NULL CHECK(covered_through_sequence >= 0),
    summary_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppendOutcome {
    Appended { sequence: u64 },
    Duplicate { sequence: u64 },
}

pub struct SqliteEventStore {
    connection: Mutex<Connection>,
}

impl SqliteEventStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let connection = Connection::open(path)?;
        Self::initialize(connection)
    }

    pub fn open_in_memory() -> Result<Self, StorageError> {
        let connection = Connection::open_in_memory()?;
        Self::initialize(connection)
    }

    fn initialize(connection: Connection) -> Result<Self, StorageError> {
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.execute_batch(MIGRATION_1)?;
        connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (1)",
            [],
        )?;
        connection.execute_batch(MIGRATION_2)?;
        connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (2)",
            [],
        )?;
        connection.execute_batch(MIGRATION_3)?;
        connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (3)",
            [],
        )?;
        connection.execute_batch(MIGRATION_4)?;
        connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (4)",
            [],
        )?;
        connection.execute_batch(MIGRATION_5)?;
        connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (5)",
            [],
        )?;
        connection.execute_batch(MIGRATION_6)?;
        connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (6)",
            [],
        )?;
        connection.execute_batch(MIGRATION_7)?;
        connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (7)",
            [],
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn append(&self, event: &EventEnvelope) -> Result<AppendOutcome, StorageError> {
        event.validate()?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

        if let Some((sequence, stored_json)) = transaction
            .query_row(
                "SELECT sequence, envelope_json FROM events WHERE event_id = ?1",
                [event.event_id.as_str()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
        {
            let incoming_json = serde_json::to_string(event)?;
            if stored_json != incoming_json {
                return Err(StorageError::ConflictingEventId(event.event_id.clone()));
            }
            return Ok(AppendOutcome::Duplicate {
                sequence: to_u64(sequence)?,
            });
        }

        let current = load_projection(&transaction, &event.run_id)?;
        let mut projection =
            current.unwrap_or_else(|| RuntimeSnapshot::empty(event.run_id.clone()));
        projection.apply(event)?;

        let event_json = serde_json::to_string(event)?;
        let event_type = serde_json::to_value(event.event_type)?
            .as_str()
            .expect("event type serializes as a string")
            .to_owned();
        transaction.execute(
            "INSERT INTO events (
                run_id, sequence, event_id, schema_version, timestamp,
                project_id, thread_id, event_type, correlation_id,
                causation_id, envelope_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                event.run_id.as_str(),
                to_i64(event.sequence)?,
                event.event_id.as_str(),
                event.schema_version,
                event.timestamp.as_str(),
                event.project_id.as_str(),
                event.thread_id.as_str(),
                event_type,
                event.correlation_id.as_str(),
                event.causation_id.as_ref().map(EventId::as_str),
                event_json,
            ],
        )?;
        let projection_json = serde_json::to_string(&projection)?;
        transaction.execute(
            "INSERT INTO run_projections(run_id, sequence, snapshot_json)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(run_id) DO UPDATE SET
                sequence = excluded.sequence,
                snapshot_json = excluded.snapshot_json,
                updated_at = CURRENT_TIMESTAMP",
            params![
                event.run_id.as_str(),
                to_i64(projection.sequence)?,
                projection_json
            ],
        )?;
        transaction.commit()?;
        Ok(AppendOutcome::Appended {
            sequence: event.sequence,
        })
    }

    /// Atomically appends a related group of newly generated events.
    ///
    /// Callers leave each sequence at zero. The store allocates contiguous
    /// per-run sequences while holding an IMMEDIATE SQLite transaction, applies
    /// the projection after every event, and commits the group as one unit.
    pub fn append_batch_next(
        &self,
        mut events: Vec<EventEnvelope>,
    ) -> Result<Vec<EventEnvelope>, StorageError> {
        if events.is_empty() {
            return Ok(events);
        }
        let run_id = events[0].run_id.clone();
        if events.iter().any(|event| event.run_id != run_id) {
            return Err(StorageError::MixedRunBatch);
        }

        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = load_projection(&transaction, &run_id)?;
        let mut projection = current.unwrap_or_else(|| RuntimeSnapshot::empty(run_id.clone()));

        for event in &mut events {
            if event.sequence != 0 {
                return Err(StorageError::GeneratedSequenceMustBeZero(event.sequence));
            }
            let duplicate: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM events WHERE event_id = ?1)",
                [event.event_id.as_str()],
                |row| row.get(0),
            )?;
            if duplicate {
                return Err(StorageError::GeneratedEventIdExists(event.event_id.clone()));
            }

            event.sequence = projection
                .sequence
                .checked_add(1)
                .ok_or(StorageError::SequenceOverflow)?;
            event.validate()?;
            projection.apply(event)?;
            insert_event(&transaction, event)?;
        }

        upsert_projection(&transaction, &projection)?;
        transaction.commit()?;
        Ok(events)
    }

    pub fn projection(&self, run_id: &RunId) -> Result<Option<RuntimeSnapshot>, StorageError> {
        let connection = self.lock()?;
        load_projection(&connection, run_id)
    }

    pub fn events_after(
        &self,
        run_id: &RunId,
        sequence: u64,
    ) -> Result<Vec<EventEnvelope>, StorageError> {
        let connection = self.lock()?;
        load_events_after(&connection, run_id, sequence)
    }

    pub fn create_snapshot(&self, run_id: &RunId) -> Result<Option<RuntimeSnapshot>, StorageError> {
        let connection = self.lock()?;
        let Some(projection) = load_projection(&connection, run_id)? else {
            return Ok(None);
        };
        connection.execute(
            "INSERT OR REPLACE INTO snapshots(
                run_id, sequence, schema_version, snapshot_json
             ) VALUES (?1, ?2, ?3, ?4)",
            params![
                run_id.as_str(),
                to_i64(projection.sequence)?,
                projection.schema_version,
                serde_json::to_string(&projection)?
            ],
        )?;
        Ok(Some(projection))
    }

    pub fn recover(&self, run_id: &RunId) -> Result<Option<RuntimeSnapshot>, StorageError> {
        let connection = self.lock()?;
        let snapshot_json: Option<String> = connection
            .query_row(
                "SELECT snapshot_json FROM snapshots
                 WHERE run_id = ?1 ORDER BY sequence DESC LIMIT 1",
                [run_id.as_str()],
                |row| row.get(0),
            )
            .optional()?;

        let mut projection = match snapshot_json {
            Some(json) => serde_json::from_str::<RuntimeSnapshot>(&json)?,
            None => {
                let has_events: bool = connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM events WHERE run_id = ?1)",
                    [run_id.as_str()],
                    |row| row.get(0),
                )?;
                if !has_events {
                    return Ok(None);
                }
                RuntimeSnapshot::empty(run_id.clone())
            }
        };

        for event in load_events_after(&connection, run_id, projection.sequence)? {
            projection.apply(&event)?;
        }
        Ok(Some(projection))
    }

    pub fn save_provider_config(&self, config: &ProviderConfig) -> Result<(), StorageError> {
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO provider_configs(id, config_json) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET
                config_json = excluded.config_json,
                updated_at = CURRENT_TIMESTAMP",
            params![config.id, serde_json::to_string(config)?],
        )?;
        Ok(())
    }

    pub fn provider_configs(&self) -> Result<Vec<ProviderConfig>, StorageError> {
        let connection = self.lock()?;
        let mut statement =
            connection.prepare("SELECT config_json FROM provider_configs ORDER BY id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut configs = Vec::new();
        for row in rows {
            configs.push(serde_json::from_str(&row?)?);
        }
        Ok(configs)
    }

    pub fn save_mcp_server_config(&self, config: &McpServerConfig) -> Result<(), StorageError> {
        if config.id.trim().is_empty() {
            return Err(StorageError::InvalidConfigId);
        }
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO mcp_server_configs(id, config_json) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET
                config_json = excluded.config_json,
                updated_at = CURRENT_TIMESTAMP",
            params![config.id, serde_json::to_string(config)?],
        )?;
        Ok(())
    }

    pub fn mcp_server_configs(&self) -> Result<Vec<McpServerConfig>, StorageError> {
        let connection = self.lock()?;
        let mut statement =
            connection.prepare("SELECT config_json FROM mcp_server_configs ORDER BY id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut configs = Vec::new();
        for row in rows {
            configs.push(serde_json::from_str(&row?)?);
        }
        Ok(configs)
    }

    pub fn delete_mcp_server_config(&self, config_id: &str) -> Result<bool, StorageError> {
        if config_id.trim().is_empty() {
            return Err(StorageError::InvalidConfigId);
        }
        let connection = self.lock()?;
        Ok(connection.execute("DELETE FROM mcp_server_configs WHERE id = ?1", [config_id])? > 0)
    }

    pub fn save_routing_policy(
        &self,
        scope_id: &str,
        policy: &ModelRoutingPolicy,
    ) -> Result<(), StorageError> {
        if scope_id.trim().is_empty() {
            return Err(StorageError::InvalidScopeId);
        }
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO routing_policies(scope_id, policy_json) VALUES (?1, ?2)
             ON CONFLICT(scope_id) DO UPDATE SET
                policy_json = excluded.policy_json,
                updated_at = CURRENT_TIMESTAMP",
            params![scope_id, serde_json::to_string(policy)?],
        )?;
        Ok(())
    }

    pub fn routing_policy(
        &self,
        scope_id: &str,
    ) -> Result<Option<ModelRoutingPolicy>, StorageError> {
        if scope_id.trim().is_empty() {
            return Err(StorageError::InvalidScopeId);
        }
        let connection = self.lock()?;
        let json: Option<String> = connection
            .query_row(
                "SELECT policy_json FROM routing_policies WHERE scope_id = ?1",
                [scope_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|value| serde_json::from_str(&value).map_err(StorageError::from))
            .transpose()
    }

    pub fn save_model_selection_settings(
        &self,
        scope_id: &str,
        settings: &ModelSelectionSettings,
    ) -> Result<(), StorageError> {
        if scope_id.trim().is_empty() {
            return Err(StorageError::InvalidScopeId);
        }
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO model_selection_settings(scope_id, settings_json) VALUES (?1, ?2)
             ON CONFLICT(scope_id) DO UPDATE SET
                settings_json = excluded.settings_json,
                updated_at = CURRENT_TIMESTAMP",
            params![scope_id, serde_json::to_string(settings)?],
        )?;
        Ok(())
    }

    pub fn model_selection_settings(
        &self,
        scope_id: &str,
    ) -> Result<Option<ModelSelectionSettings>, StorageError> {
        if scope_id.trim().is_empty() {
            return Err(StorageError::InvalidScopeId);
        }
        let connection = self.lock()?;
        let json: Option<String> = connection
            .query_row(
                "SELECT settings_json FROM model_selection_settings WHERE scope_id = ?1",
                [scope_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|value| serde_json::from_str(&value).map_err(StorageError::from))
            .transpose()
    }

    pub fn save_course(&self, course: &Course) -> Result<(), StorageError> {
        validate_scope_id(&course.course_id)?;
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO courses(course_id, course_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(course_id) DO UPDATE SET
                course_json = excluded.course_json,
                updated_at = excluded.updated_at",
            params![
                course.course_id,
                serde_json::to_string(course)?,
                course.created_at,
                course.updated_at
            ],
        )?;
        Ok(())
    }

    pub fn save_project(&self, project: &LunaProject) -> Result<(), StorageError> {
        validate_scope_id(&project.project_id)?;
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO projects(project_id, project_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(project_id) DO UPDATE SET
                project_json = excluded.project_json,
                updated_at = excluded.updated_at",
            params![
                project.project_id,
                serde_json::to_string(project)?,
                project.created_at,
                project.updated_at
            ],
        )?;
        Ok(())
    }

    pub fn projects(&self) -> Result<Vec<LunaProject>, StorageError> {
        let connection = self.lock()?;
        load_json_rows(
            &connection,
            "SELECT project_json FROM projects ORDER BY updated_at DESC, project_id",
            [],
        )
    }

    pub fn project(&self, project_id: &str) -> Result<Option<LunaProject>, StorageError> {
        validate_scope_id(project_id)?;
        let connection = self.lock()?;
        load_optional_json(
            &connection,
            "SELECT project_json FROM projects WHERE project_id = ?1",
            [project_id],
        )
    }

    pub fn append_conversation_message(
        &self,
        mut message: ConversationMessage,
    ) -> Result<ConversationMessage, StorageError> {
        validate_scope_id(message.message_id.as_str())?;
        validate_scope_id(message.project_id.as_str())?;
        validate_scope_id(message.thread_id.as_str())?;
        if message.content.trim().is_empty() {
            return Err(StorageError::EmptyConversationMessage);
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(stored) = load_optional_json(
            &transaction,
            "SELECT message_json FROM conversation_messages WHERE message_id = ?1",
            [message.message_id.as_str()],
        )? {
            return Ok(stored);
        }
        let next = transaction.query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM conversation_messages WHERE thread_id = ?1",
            [message.thread_id.as_str()],
            |row| row.get::<_, i64>(0),
        )?;
        message.sequence = to_u64(next)?;
        let role = serde_json::to_value(message.role)?
            .as_str()
            .unwrap_or("system")
            .to_owned();
        transaction.execute(
            "INSERT INTO conversation_messages(
                message_id, project_id, thread_id, run_id, sequence, role, message_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                message.message_id,
                message.project_id.as_str(),
                message.thread_id.as_str(),
                message.run_id.as_ref().map(RunId::as_str),
                to_i64(message.sequence)?,
                role,
                serde_json::to_string(&message)?,
                message.created_at,
            ],
        )?;
        transaction.commit()?;
        Ok(message)
    }

    pub fn conversation_messages(
        &self,
        thread_id: &str,
        after_sequence: u64,
    ) -> Result<Vec<ConversationMessage>, StorageError> {
        validate_scope_id(thread_id)?;
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT message_json FROM conversation_messages
             WHERE thread_id = ?1 AND sequence > ?2
             ORDER BY sequence ASC",
        )?;
        let rows = statement.query_map(params![thread_id, to_i64(after_sequence)?], |row| {
            row.get::<_, String>(0)
        })?;
        let mut messages = Vec::new();
        for row in rows {
            messages.push(serde_json::from_str(&row?)?);
        }
        Ok(messages)
    }

    pub fn thread_context_summary(
        &self,
        thread_id: &str,
    ) -> Result<Option<ThreadContextSummary>, StorageError> {
        validate_scope_id(thread_id)?;
        let connection = self.lock()?;
        load_optional_json(
            &connection,
            "SELECT summary_json FROM thread_context_summaries WHERE thread_id = ?1",
            [thread_id],
        )
    }

    pub fn save_thread_context_summary(
        &self,
        summary: &ThreadContextSummary,
    ) -> Result<(), StorageError> {
        validate_scope_id(summary.thread_id.as_str())?;
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO thread_context_summaries(
                thread_id, covered_through_sequence, summary_json, updated_at
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(thread_id) DO UPDATE SET
                covered_through_sequence = excluded.covered_through_sequence,
                summary_json = excluded.summary_json,
                updated_at = excluded.updated_at",
            params![
                summary.thread_id.as_str(),
                to_i64(summary.covered_through_sequence)?,
                serde_json::to_string(summary)?,
                summary.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn delete_thread(&self, thread_id: &str) -> Result<usize, StorageError> {
        validate_scope_id(thread_id)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM thread_context_summaries WHERE thread_id = ?1",
            [thread_id],
        )?;
        transaction.execute(
            "DELETE FROM conversation_messages WHERE thread_id = ?1",
            [thread_id],
        )?;
        transaction.execute(
            "DELETE FROM snapshots
             WHERE run_id IN (SELECT run_id FROM events WHERE thread_id = ?1)",
            [thread_id],
        )?;
        transaction.execute(
            "DELETE FROM run_projections
             WHERE run_id IN (SELECT run_id FROM events WHERE thread_id = ?1)",
            [thread_id],
        )?;
        let deleted =
            transaction.execute("DELETE FROM events WHERE thread_id = ?1", [thread_id])?;
        transaction.commit()?;
        Ok(deleted)
    }

    pub fn delete_project(&self, project_id: &str) -> Result<bool, StorageError> {
        validate_scope_id(project_id)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM thread_context_summaries
             WHERE thread_id IN (SELECT DISTINCT thread_id FROM conversation_messages WHERE project_id = ?1)",
            [project_id],
        )?;
        transaction.execute(
            "DELETE FROM conversation_messages WHERE project_id = ?1",
            [project_id],
        )?;
        transaction.execute(
            "DELETE FROM snapshots
             WHERE run_id IN (SELECT run_id FROM events WHERE project_id = ?1)",
            [project_id],
        )?;
        transaction.execute(
            "DELETE FROM run_projections
             WHERE run_id IN (SELECT run_id FROM events WHERE project_id = ?1)",
            [project_id],
        )?;
        transaction.execute("DELETE FROM events WHERE project_id = ?1", [project_id])?;
        let deleted =
            transaction.execute("DELETE FROM projects WHERE project_id = ?1", [project_id])?;
        transaction.commit()?;
        Ok(deleted > 0)
    }

    pub fn save_user_preferences(&self, preferences: &UserPreferences) -> Result<(), StorageError> {
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO user_preferences(scope_id, preferences_json)
             VALUES ('global', ?1)
             ON CONFLICT(scope_id) DO UPDATE SET
                preferences_json = excluded.preferences_json,
                updated_at = CURRENT_TIMESTAMP",
            [serde_json::to_string(preferences)?],
        )?;
        Ok(())
    }

    pub fn user_preferences(&self) -> Result<UserPreferences, StorageError> {
        let connection = self.lock()?;
        Ok(load_optional_json(
            &connection,
            "SELECT preferences_json FROM user_preferences WHERE scope_id = 'global'",
            [],
        )?
        .unwrap_or_default())
    }

    pub fn courses(&self) -> Result<Vec<Course>, StorageError> {
        let connection = self.lock()?;
        load_json_rows(
            &connection,
            "SELECT course_json FROM courses ORDER BY updated_at DESC, course_id",
            [],
        )
    }

    pub fn course(&self, course_id: &str) -> Result<Option<Course>, StorageError> {
        validate_scope_id(course_id)?;
        let connection = self.lock()?;
        load_optional_json(
            &connection,
            "SELECT course_json FROM courses WHERE course_id = ?1",
            [course_id],
        )
    }

    pub fn bind_course_thread(&self, binding: &CourseThreadBinding) -> Result<(), StorageError> {
        validate_scope_id(&binding.thread_id)?;
        validate_scope_id(&binding.course_id)?;
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO lecture_threads(thread_id, course_id, binding_json, bound_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(thread_id) DO UPDATE SET
                course_id = excluded.course_id,
                binding_json = excluded.binding_json,
                bound_at = excluded.bound_at",
            params![
                binding.thread_id,
                binding.course_id,
                serde_json::to_string(binding)?,
                binding.bound_at
            ],
        )?;
        Ok(())
    }

    pub fn course_thread_binding(
        &self,
        thread_id: &str,
    ) -> Result<Option<CourseThreadBinding>, StorageError> {
        validate_scope_id(thread_id)?;
        let connection = self.lock()?;
        load_optional_json(
            &connection,
            "SELECT binding_json FROM lecture_threads WHERE thread_id = ?1",
            [thread_id],
        )
    }

    pub fn save_course_source(
        &self,
        source: &CourseSource,
        content: &str,
    ) -> Result<(), StorageError> {
        validate_scope_id(&source.source_id)?;
        validate_scope_id(&source.course_id)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO course_sources(
                source_id, course_id, kind, source_json, content_text, imported_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                source.source_id,
                source.course_id,
                format!("{:?}", source.kind),
                serde_json::to_string(source)?,
                content,
                source.imported_at
            ],
        )?;
        transaction.execute(
            "INSERT INTO course_search_fts(course_id, source_id, content)
             VALUES (?1, ?2, ?3)",
            params![source.course_id, source.source_id, content],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn save_syllabus_revision(&self, revision: &SyllabusRevision) -> Result<(), StorageError> {
        validate_scope_id(&revision.course_id)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO syllabus_revisions(
                revision_id, course_id, source_id, revision_number, revision_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                revision.revision_id,
                revision.course_id,
                revision.source_id,
                revision.revision,
                serde_json::to_string(revision)?,
                revision.created_at
            ],
        )?;
        for (ordinal, objective) in revision.structure.learning_objectives.iter().enumerate() {
            transaction.execute(
                "INSERT INTO course_learning_objectives(
                    objective_id, course_id, revision_id, objective_text, ordinal
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    format!("{}-objective-{ordinal}", revision.revision_id),
                    revision.course_id,
                    revision.revision_id,
                    objective,
                    i64::try_from(ordinal).map_err(|_| StorageError::SequenceOverflow)?
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn latest_syllabus_revision(
        &self,
        course_id: &str,
    ) -> Result<Option<SyllabusRevision>, StorageError> {
        validate_scope_id(course_id)?;
        let connection = self.lock()?;
        load_optional_json(
            &connection,
            "SELECT revision_json FROM syllabus_revisions
             WHERE course_id = ?1 ORDER BY revision_number DESC LIMIT 1",
            [course_id],
        )
    }

    pub fn search_course_sources(
        &self,
        course_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<CourseSearchHit>, StorageError> {
        validate_scope_id(course_id)?;
        let query = fts_query(query)?;
        let limit =
            i64::try_from(limit.clamp(1, 50)).map_err(|_| StorageError::SequenceOverflow)?;
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT course_id, source_id,
                    snippet(course_search_fts, 2, '[', ']', ' … ', 24),
                    bm25(course_search_fts)
             FROM course_search_fts
             WHERE course_search_fts MATCH ?1 AND course_id = ?2
             ORDER BY bm25(course_search_fts)
             LIMIT ?3",
        )?;
        let rows = statement.query_map(params![query, course_id, limit], |row| {
            Ok(CourseSearchHit {
                course_id: row.get(0)?,
                source_id: row.get(1)?,
                excerpt: row.get(2)?,
                rank: row.get(3)?,
            })
        })?;
        let mut hits = Vec::new();
        for row in rows {
            hits.push(row?);
        }
        Ok(hits)
    }

    pub fn save_note_revision(&self, note: &NoteRevision) -> Result<(), StorageError> {
        validate_scope_id(&note.course_id)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO note_revisions(
                note_revision_id, course_id, thread_id, source_id,
                revision_number, note_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                note.note_revision_id,
                note.course_id,
                note.thread_id,
                note.source_id,
                note.revision,
                serde_json::to_string(note)?,
                note.created_at
            ],
        )?;
        for anchor in &note.source_map {
            transaction.execute(
                "INSERT INTO citation_anchors(anchor_id, course_id, source_id, anchor_json)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    anchor.anchor_id,
                    anchor.course_id,
                    anchor.source_id,
                    serde_json::to_string(anchor)?
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn latest_course_notes(
        &self,
        course_id: &str,
        limit: usize,
    ) -> Result<Vec<NoteRevision>, StorageError> {
        validate_scope_id(course_id)?;
        let limit = i64::try_from(limit.min(100)).map_err(|_| StorageError::SequenceOverflow)?;
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT note_json FROM note_revisions
             WHERE course_id = ?1 ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![course_id, limit], |row| row.get::<_, String>(0))?;
        parse_json_rows(rows)
    }

    pub fn save_course_concept(&self, concept: &CourseConcept) -> Result<(), StorageError> {
        validate_scope_id(&concept.course_id)?;
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO course_concepts(concept_id, course_id, concept_json)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(concept_id) DO UPDATE SET concept_json = excluded.concept_json",
            params![
                concept.concept_id,
                concept.course_id,
                serde_json::to_string(concept)?
            ],
        )?;
        Ok(())
    }

    pub fn save_course_assignment(
        &self,
        assignment: &CourseAssignment,
    ) -> Result<(), StorageError> {
        validate_scope_id(&assignment.course_id)?;
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO course_assignments(assignment_id, course_id, assignment_json)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(assignment_id) DO UPDATE SET assignment_json = excluded.assignment_json",
            params![
                assignment.assignment_id,
                assignment.course_id,
                serde_json::to_string(assignment)?
            ],
        )?;
        Ok(())
    }

    pub fn save_review_item(&self, item: &ReviewItem) -> Result<(), StorageError> {
        validate_scope_id(&item.course_id)?;
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO course_review_items(
                review_item_id, course_id, concept_id, next_review_at, review_json
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(review_item_id) DO UPDATE SET
                concept_id = excluded.concept_id,
                next_review_at = excluded.next_review_at,
                review_json = excluded.review_json",
            params![
                item.review_item_id,
                item.course_id,
                item.concept_id,
                item.next_review_at,
                serde_json::to_string(item)?
            ],
        )?;
        Ok(())
    }

    pub fn course_concepts(&self, course_id: &str) -> Result<Vec<CourseConcept>, StorageError> {
        validate_scope_id(course_id)?;
        let connection = self.lock()?;
        load_json_rows(
            &connection,
            "SELECT concept_json FROM course_concepts WHERE course_id = ?1 ORDER BY concept_id",
            [course_id],
        )
    }

    pub fn course_assignments(
        &self,
        course_id: &str,
    ) -> Result<Vec<CourseAssignment>, StorageError> {
        validate_scope_id(course_id)?;
        let connection = self.lock()?;
        load_json_rows(
            &connection,
            "SELECT assignment_json FROM course_assignments
             WHERE course_id = ?1 ORDER BY assignment_id",
            [course_id],
        )
    }

    pub fn course_review_items(&self, course_id: &str) -> Result<Vec<ReviewItem>, StorageError> {
        validate_scope_id(course_id)?;
        let connection = self.lock()?;
        load_json_rows(
            &connection,
            "SELECT review_json FROM course_review_items
             WHERE course_id = ?1 ORDER BY next_review_at, review_item_id",
            [course_id],
        )
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, StorageError> {
        self.connection
            .lock()
            .map_err(|_| StorageError::LockPoisoned)
    }
}

fn validate_scope_id(value: &str) -> Result<(), StorageError> {
    if value.trim().is_empty() {
        Err(StorageError::InvalidScopeId)
    } else {
        Ok(())
    }
}

fn fts_query(value: &str) -> Result<String, StorageError> {
    let tokens = value
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .take(12)
        .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        Err(StorageError::InvalidSearchQuery)
    } else {
        Ok(tokens.join(" AND "))
    }
}

fn load_optional_json<T, P>(
    connection: &Connection,
    query: &str,
    params: P,
) -> Result<Option<T>, StorageError>
where
    T: serde::de::DeserializeOwned,
    P: rusqlite::Params,
{
    let json: Option<String> = connection
        .query_row(query, params, |row| row.get(0))
        .optional()?;
    json.map(|value| serde_json::from_str(&value).map_err(StorageError::from))
        .transpose()
}

fn load_json_rows<T, P>(
    connection: &Connection,
    query: &str,
    params: P,
) -> Result<Vec<T>, StorageError>
where
    T: serde::de::DeserializeOwned,
    P: rusqlite::Params,
{
    let mut statement = connection.prepare(query)?;
    let rows = statement.query_map(params, |row| row.get::<_, String>(0))?;
    parse_json_rows(rows)
}

fn parse_json_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<String>>,
) -> Result<Vec<T>, StorageError>
where
    T: serde::de::DeserializeOwned,
{
    let mut values = Vec::new();
    for row in rows {
        values.push(serde_json::from_str(&row?)?);
    }
    Ok(values)
}

fn load_projection(
    connection: &Connection,
    run_id: &RunId,
) -> Result<Option<RuntimeSnapshot>, StorageError> {
    let json: Option<String> = connection
        .query_row(
            "SELECT snapshot_json FROM run_projections WHERE run_id = ?1",
            [run_id.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    json.map(|value| serde_json::from_str(&value).map_err(StorageError::from))
        .transpose()
}

fn insert_event(connection: &Connection, event: &EventEnvelope) -> Result<(), StorageError> {
    let event_json = serde_json::to_string(event)?;
    let event_type = serde_json::to_value(event.event_type)?
        .as_str()
        .expect("event type serializes as a string")
        .to_owned();
    connection.execute(
        "INSERT INTO events (
            run_id, sequence, event_id, schema_version, timestamp,
            project_id, thread_id, event_type, correlation_id,
            causation_id, envelope_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            event.run_id.as_str(),
            to_i64(event.sequence)?,
            event.event_id.as_str(),
            event.schema_version,
            event.timestamp.as_str(),
            event.project_id.as_str(),
            event.thread_id.as_str(),
            event_type,
            event.correlation_id.as_str(),
            event.causation_id.as_ref().map(EventId::as_str),
            event_json,
        ],
    )?;
    Ok(())
}

fn upsert_projection(
    connection: &Connection,
    projection: &RuntimeSnapshot,
) -> Result<(), StorageError> {
    connection.execute(
        "INSERT INTO run_projections(run_id, sequence, snapshot_json)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(run_id) DO UPDATE SET
            sequence = excluded.sequence,
            snapshot_json = excluded.snapshot_json,
            updated_at = CURRENT_TIMESTAMP",
        params![
            projection.run_id.as_str(),
            to_i64(projection.sequence)?,
            serde_json::to_string(projection)?
        ],
    )?;
    Ok(())
}

fn load_events_after(
    connection: &Connection,
    run_id: &RunId,
    sequence: u64,
) -> Result<Vec<EventEnvelope>, StorageError> {
    let mut statement = connection.prepare(
        "SELECT envelope_json FROM events
         WHERE run_id = ?1 AND sequence > ?2
         ORDER BY sequence ASC",
    )?;
    let rows = statement.query_map(params![run_id.as_str(), to_i64(sequence)?], |row| {
        row.get::<_, String>(0)
    })?;
    let mut events = Vec::new();
    for row in rows {
        events.push(serde_json::from_str(&row?)?);
    }
    Ok(events)
}

fn to_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::SequenceOverflow)
}

fn to_u64(value: i64) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| StorageError::SequenceOverflow)
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    InvalidEvent(#[from] lunascope_core::EventValidationError),
    #[error(transparent)]
    Projection(#[from] ProjectionError),
    #[error("event id already exists with different content: {0}")]
    ConflictingEventId(EventId),
    #[error("generated event id already exists: {0}")]
    GeneratedEventIdExists(EventId),
    #[error("generated event sequence must be zero, got {0}")]
    GeneratedSequenceMustBeZero(u64),
    #[error("all events in an atomic batch must belong to one run")]
    MixedRunBatch,
    #[error("event sequence exceeds SQLite INTEGER range")]
    SequenceOverflow,
    #[error("event store mutex is poisoned")]
    LockPoisoned,
    #[error("routing policy scope id is required")]
    InvalidScopeId,
    #[error("course search query is required")]
    InvalidSearchQuery,
    #[error("configuration id is required")]
    InvalidConfigId,
    #[error("conversation message content is required")]
    EmptyConversationMessage,
}
