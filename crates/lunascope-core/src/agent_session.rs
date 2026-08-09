use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{AgentSessionId, ProjectId, RunId, ThreadId, WorkerId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum AgentSessionKind {
    Primary,
    Orchestrator,
    Worker,
    Verifier,
    Supervisor,
    ContextCompressor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum AgentSessionState {
    Queued,
    Running,
    WaitingForModel,
    WaitingForTool,
    Paused,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

/// Durable identity for one model-facing Agent loop. Runtime events reference
/// this record instead of inferring Agent ownership from display text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct AgentSessionRecord {
    pub session_id: AgentSessionId,
    pub parent_session_id: Option<AgentSessionId>,
    pub project_id: ProjectId,
    pub thread_id: ThreadId,
    pub run_id: RunId,
    pub worker_id: Option<WorkerId>,
    pub kind: AgentSessionKind,
    pub display_name: String,
    pub state: AgentSessionState,
    /// RFC 3339 UTC timestamp.
    pub created_at: String,
    /// RFC 3339 UTC timestamp.
    pub updated_at: String,
}

/// Persistent liveness record for a long-running orchestration. A stale lease
/// lets desktop recovery distinguish a crashed owner from a live controller.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct RunLeaseRecord {
    pub run_id: RunId,
    pub owner_id: String,
    pub phase: String,
    /// RFC 3339 UTC timestamp.
    pub heartbeat_at: String,
    /// RFC 3339 UTC timestamp.
    pub expires_at: String,
    /// RFC 3339 UTC timestamp of the most recent observable progress.
    pub last_progress_at: String,
}
