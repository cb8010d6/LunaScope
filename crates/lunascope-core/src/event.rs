use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

use crate::{
    AgentSessionId, ArtifactId, CheckpointId, CorrelationId, EventId, OrchestrationId, ProjectId,
    RetryPolicy, RunId, RunState, ThreadId, WorkerBudget, WorkerCheckpointPolicy, WorkerField,
    WorkerId, WorkerInputContext, WorkerOutputSchema, WorkerState,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum RiskLevel {
    None,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum RedactionState {
    NotRequired,
    Redacted,
    ContainsSensitiveData,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
#[ts(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum EventSource {
    User,
    System,
    Orchestrator,
    Worker(WorkerId),
    Tool(String),
    Mcp(String),
    Bridge(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum VerificationStatus {
    Verified,
    PartiallyVerified,
    Unverified,
    UnableToVerify,
    FailedVerification,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum CompletionKind {
    Completed,
    PartiallyCompleted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum AgentPlanStepStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct AgentPlanStep {
    pub step: String,
    pub status: AgentPlanStepStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct AgentPlan {
    pub explanation: Option<String>,
    pub steps: Vec<AgentPlanStep>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ReasoningSummarySource {
    /// A summary item emitted by the Provider's reasoning-summary protocol.
    Provider,
    /// Model-authored commentary or a progress tool call. This is not hidden chain-of-thought.
    ModelCommentary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ReasoningSummaryRecord {
    pub item_id: String,
    pub source: ReasoningSummarySource,
    pub summary: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct TransportRetryRecord {
    pub request_id: String,
    pub attempt: u32,
    pub maximum_retries: u32,
    #[ts(type = "number")]
    pub delay_seconds: u64,
    pub reason: String,
    /// RFC 3339 UTC timestamp.
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum PermissionDecision {
    AllowOnce,
    AllowForRun,
    AllowForScope,
    Reject,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ModelSelection {
    pub provider: String,
    pub model: String,
    pub reason: String,
    pub fallback: bool,
    #[serde(default)]
    pub reasoning_effort: Option<crate::ReasoningEffort>,
    #[serde(default)]
    pub custom_reasoning_effort: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkerSpec {
    pub worker_id: WorkerId,
    #[serde(default)]
    pub display_name: String,
    pub role: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub objective: String,
    pub task: String,
    pub prompt: String,
    pub input_context: WorkerInputContext,
    pub expected_output: String,
    pub output_schema: WorkerOutputSchema,
    pub completion_criteria: Vec<String>,
    #[serde(default)]
    pub owned_acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub parallel_group: Option<String>,
    pub model: ModelSelection,
    pub skills: Vec<String>,
    pub tools: Vec<String>,
    pub permissions: Vec<String>,
    pub budget: WorkerBudget,
    pub dependencies: Vec<WorkerId>,
    #[ts(type = "number")]
    pub timeout_ms: u64,
    pub retry_policy: RetryPolicy,
    pub checkpoint_policy: WorkerCheckpointPolicy,
    pub write_scopes: Vec<String>,
    pub locked_fields: Vec<WorkerField>,
    pub parent_worker_id: Option<WorkerId>,
    pub depth: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ToolCall {
    pub call_id: String,
    pub tool_id: String,
    pub input: serde_json::Value,
    pub idempotency_key: String,
    #[ts(type = "number")]
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ToolResult {
    pub call_id: String,
    pub success: bool,
    pub output: serde_json::Value,
    pub error_code: Option<String>,
    #[ts(type = "number")]
    pub duration_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub approval_id: String,
    pub call_id: Option<String>,
    pub action: String,
    pub reason: String,
    pub permissions: Vec<String>,
    pub scope: String,
    pub blast_radius: String,
    pub rollback: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ApprovalResolution {
    pub approval_id: String,
    pub decision: PermissionDecision,
    pub instructions: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ArtifactRecord {
    pub artifact_id: ArtifactId,
    pub name: String,
    pub media_type: String,
    pub path: String,
    pub sha256: String,
    pub created_by: EventSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CheckpointRecord {
    pub checkpoint_id: CheckpointId,
    #[ts(type = "number")]
    pub event_sequence: u64,
    pub reason: String,
    pub artifact_ids: Vec<ArtifactId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct VerificationRecord {
    pub status: VerificationStatus,
    pub summary: String,
    pub evidence: Vec<String>,
    pub remaining_risks: Vec<String>,
    /// Criterion-level acceptance evidence. Older stored events remain readable,
    /// but a live Verifier must cover the current plan's complete AC contract.
    #[serde(default)]
    pub criterion_results: Vec<CriterionVerification>,
    #[serde(default)]
    pub findings: Vec<VerificationFinding>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum CriterionVerificationStatus {
    Passed,
    Failed,
    Unverified,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CriterionVerification {
    /// Stable identifier from the task-wide contract, for example `AC-3`.
    pub criterion_id: String,
    pub status: CriterionVerificationStatus,
    /// Direct observations supporting this one criterion. A global summary is not
    /// accepted as evidence for an individual passed criterion.
    #[serde(default)]
    pub evidence: Vec<String>,
    pub note: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum VerificationSeverity {
    Fatal,
    Major,
    Minor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct VerificationFinding {
    pub severity: VerificationSeverity,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub affected_paths: Vec<String>,
    pub repair_hint: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
    Binary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum DiffLineKind {
    Context,
    Addition,
    Deletion,
    Metadata,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkspaceFileChange {
    pub path: String,
    pub kind: FileChangeKind,
    pub additions: u32,
    pub deletions: u32,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct OrchestrationChangeSet {
    pub run_id: RunId,
    pub files: Vec<WorkspaceFileChange>,
    pub additions: u32,
    pub deletions: u32,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum RunControlKind {
    Guidance,
    Pause,
    Resume,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum RunControlStatus {
    Requested,
    Queued,
    Applied,
    Rejected,
    Settled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct RunControlRecord {
    pub control_id: String,
    pub control: RunControlKind,
    pub status: RunControlStatus,
    pub summary: String,
    pub affected_worker_ids: Vec<WorkerId>,
    /// RFC 3339 UTC timestamp.
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct AgentActivityItem {
    pub activity_id: String,
    #[serde(default)]
    pub agent_kind: AgentKind,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub display_name: String,
    pub worker_id: Option<WorkerId>,
    pub phase: String,
    #[serde(default)]
    pub waiting_for_model: bool,
    pub observation: String,
    pub decision: String,
    pub next_action: String,
    pub evidence_refs: Vec<String>,
    pub source: ReasoningSummarySource,
    /// RFC 3339 UTC timestamp.
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum AgentKind {
    #[default]
    OrchestrationModel,
    Worker,
    Verifier,
    ContextCompressor,
    Supervisor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum OrchestrationPlanningStage {
    EvaluatingDelegation,
    ExtractingAcceptanceCriteria,
    DecomposingWork,
    AuditingWriteScopes,
    SchedulingParallelism,
    ReviewingPlan,
    CommittingGraph,
    ReplanningGuidance,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct OrchestrationDraftWorker {
    pub draft_id: String,
    pub display_name: String,
    pub task: String,
    pub dependency_draft_ids: Vec<String>,
    pub parallel_group: Option<String>,
    pub write_scopes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct OrchestrationPlanningActivity {
    pub activity_id: String,
    pub stage: OrchestrationPlanningStage,
    pub summary: String,
    pub evidence: Vec<String>,
    pub next_action: String,
    pub draft_version: u32,
    pub draft_workers: Vec<OrchestrationDraftWorker>,
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum SupervisorDecisionKind {
    Continue,
    GuideRunning,
    ReviseQueued,
    HoldDispatch,
    ReassignFailed,
    SpawnRepair,
    RequestVerification,
    StopForFatalPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct SupervisorDecisionRecord {
    pub decision_id: String,
    pub kind: SupervisorDecisionKind,
    pub summary: String,
    pub affected_worker_ids: Vec<WorkerId>,
    pub evidence_refs: Vec<String>,
    /// RFC 3339 UTC timestamp.
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct OrchestrationRevisionRecord {
    pub revision_id: String,
    pub from_version: u32,
    pub to_version: u32,
    pub reason: String,
    pub affected_worker_ids: Vec<WorkerId>,
    pub summary: String,
    /// RFC 3339 UTC timestamp.
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum EventType {
    RunCreated,
    RunStateChanged,
    RunControlRecorded,
    AgentActivityRecorded,
    OrchestrationPlanningActivityRecorded,
    SupervisorDecisionRecorded,
    OrchestrationRevisionRecorded,
    PlanUpdated,
    OrchestrationDecided,
    OrchestrationCreated,
    WorkerCreated,
    WorkerRemoved,
    WorkerStateChanged,
    ModelStreamChunk,
    ReasoningSummaryRecorded,
    AgentPlanUpdated,
    ModelRoutingDecided,
    ModelUsageRecorded,
    TransportRetryScheduled,
    ToolCallRequested,
    ToolCallCompleted,
    ApprovalRequested,
    ApprovalResolved,
    HandoffRecorded,
    OrchestrationPatched,
    ArtifactRecorded,
    CheckpointCreated,
    ContextCompacted,
    VerificationRecorded,
    RunCompleted,
    RunFailed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[ts(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum EventData {
    RunCreated {
        title: String,
        initial_prompt: String,
    },
    RunStateChanged {
        from: RunState,
        to: RunState,
        reason: String,
    },
    RunControlRecorded {
        record: RunControlRecord,
    },
    AgentActivityRecorded {
        item: AgentActivityItem,
    },
    OrchestrationPlanningActivityRecorded {
        activity: OrchestrationPlanningActivity,
    },
    SupervisorDecisionRecorded {
        record: SupervisorDecisionRecord,
    },
    OrchestrationRevisionRecorded {
        record: OrchestrationRevisionRecord,
    },
    PlanUpdated {
        version: u32,
        markdown: String,
    },
    OrchestrationDecided {
        decision: String,
        rationale: String,
        estimated_cost: Option<String>,
    },
    OrchestrationCreated {
        plan: Box<crate::OrchestrationPlan>,
    },
    WorkerCreated {
        spec: Box<WorkerSpec>,
    },
    WorkerRemoved {
        worker_id: WorkerId,
        from: WorkerState,
        reason: String,
    },
    WorkerStateChanged {
        worker_id: WorkerId,
        from: WorkerState,
        to: WorkerState,
        reason: String,
    },
    ModelStreamChunk {
        worker_id: Option<WorkerId>,
        stream_id: String,
        #[ts(type = "number")]
        index: u64,
        text: String,
    },
    ReasoningSummaryRecorded {
        worker_id: Option<WorkerId>,
        record: ReasoningSummaryRecord,
    },
    AgentPlanUpdated {
        worker_id: Option<WorkerId>,
        plan: AgentPlan,
    },
    ModelRoutingDecided {
        decision: crate::ModelRoutingDecision,
    },
    ModelUsageRecorded {
        usage: crate::ModelUsageRecord,
    },
    TransportRetryScheduled {
        worker_id: Option<WorkerId>,
        record: TransportRetryRecord,
    },
    ToolCallRequested {
        worker_id: Option<WorkerId>,
        call: ToolCall,
    },
    ToolCallCompleted {
        worker_id: Option<WorkerId>,
        result: ToolResult,
    },
    ApprovalRequested {
        worker_id: Option<WorkerId>,
        request: ApprovalRequest,
    },
    ApprovalResolved {
        resolution: ApprovalResolution,
    },
    HandoffRecorded {
        from: String,
        to: String,
        artifact_ids: Vec<ArtifactId>,
        summary: String,
    },
    OrchestrationPatched {
        patch: Box<crate::OrchestrationPatch>,
        plan: Box<crate::OrchestrationPlan>,
    },
    ArtifactRecorded {
        artifact: ArtifactRecord,
    },
    CheckpointCreated {
        checkpoint: CheckpointRecord,
    },
    ContextCompacted {
        #[ts(type = "number")]
        previous_tokens: u64,
        #[ts(type = "number")]
        resulting_tokens: u64,
        summary_artifact_id: ArtifactId,
    },
    VerificationRecorded {
        verification: VerificationRecord,
    },
    RunCompleted {
        completion: CompletionKind,
        verification: VerificationStatus,
        summary: String,
    },
    RunFailed {
        code: String,
        message: String,
        recoverable: bool,
    },
}

impl EventData {
    pub fn event_type(&self) -> EventType {
        match self {
            Self::RunCreated { .. } => EventType::RunCreated,
            Self::RunStateChanged { .. } => EventType::RunStateChanged,
            Self::RunControlRecorded { .. } => EventType::RunControlRecorded,
            Self::AgentActivityRecorded { .. } => EventType::AgentActivityRecorded,
            Self::OrchestrationPlanningActivityRecorded { .. } => {
                EventType::OrchestrationPlanningActivityRecorded
            }
            Self::SupervisorDecisionRecorded { .. } => EventType::SupervisorDecisionRecorded,
            Self::OrchestrationRevisionRecorded { .. } => EventType::OrchestrationRevisionRecorded,
            Self::PlanUpdated { .. } => EventType::PlanUpdated,
            Self::OrchestrationDecided { .. } => EventType::OrchestrationDecided,
            Self::OrchestrationCreated { .. } => EventType::OrchestrationCreated,
            Self::WorkerCreated { .. } => EventType::WorkerCreated,
            Self::WorkerRemoved { .. } => EventType::WorkerRemoved,
            Self::WorkerStateChanged { .. } => EventType::WorkerStateChanged,
            Self::ModelStreamChunk { .. } => EventType::ModelStreamChunk,
            Self::ReasoningSummaryRecorded { .. } => EventType::ReasoningSummaryRecorded,
            Self::AgentPlanUpdated { .. } => EventType::AgentPlanUpdated,
            Self::ModelRoutingDecided { .. } => EventType::ModelRoutingDecided,
            Self::ModelUsageRecorded { .. } => EventType::ModelUsageRecorded,
            Self::TransportRetryScheduled { .. } => EventType::TransportRetryScheduled,
            Self::ToolCallRequested { .. } => EventType::ToolCallRequested,
            Self::ToolCallCompleted { .. } => EventType::ToolCallCompleted,
            Self::ApprovalRequested { .. } => EventType::ApprovalRequested,
            Self::ApprovalResolved { .. } => EventType::ApprovalResolved,
            Self::HandoffRecorded { .. } => EventType::HandoffRecorded,
            Self::OrchestrationPatched { .. } => EventType::OrchestrationPatched,
            Self::ArtifactRecorded { .. } => EventType::ArtifactRecorded,
            Self::CheckpointCreated { .. } => EventType::CheckpointCreated,
            Self::ContextCompacted { .. } => EventType::ContextCompacted,
            Self::VerificationRecorded { .. } => EventType::VerificationRecorded,
            Self::RunCompleted { .. } => EventType::RunCompleted,
            Self::RunFailed { .. } => EventType::RunFailed,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct EventEnvelope {
    pub event_id: EventId,
    #[ts(type = "number")]
    pub sequence: u64,
    pub schema_version: u32,
    /// RFC 3339 UTC timestamp.
    pub timestamp: String,
    pub project_id: ProjectId,
    pub thread_id: ThreadId,
    pub run_id: RunId,
    pub orchestration_id: Option<OrchestrationId>,
    pub worker_id: Option<WorkerId>,
    #[serde(default)]
    pub agent_session_id: Option<AgentSessionId>,
    #[serde(default)]
    pub parent_session_id: Option<AgentSessionId>,
    #[serde(default)]
    pub parent_event_id: Option<EventId>,
    #[serde(default)]
    pub related_tool_event_id: Option<EventId>,
    pub correlation_id: CorrelationId,
    pub causation_id: Option<EventId>,
    pub event_type: EventType,
    pub source: EventSource,
    pub payload: EventData,
    pub risk: RiskLevel,
    pub redaction_state: RedactionState,
}

impl EventEnvelope {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        event_id: EventId,
        sequence: u64,
        timestamp: impl Into<String>,
        project_id: ProjectId,
        thread_id: ThreadId,
        run_id: RunId,
        correlation_id: CorrelationId,
        source: EventSource,
        payload: EventData,
    ) -> Self {
        let event_type = payload.event_type();
        Self {
            event_id,
            sequence,
            schema_version: 2,
            timestamp: timestamp.into(),
            project_id,
            thread_id,
            run_id,
            orchestration_id: None,
            worker_id: None,
            agent_session_id: None,
            parent_session_id: None,
            parent_event_id: None,
            related_tool_event_id: None,
            correlation_id,
            causation_id: None,
            event_type,
            source,
            payload,
            risk: RiskLevel::None,
            redaction_state: RedactionState::NotRequired,
        }
    }

    pub fn validate(&self) -> Result<(), EventValidationError> {
        if self.schema_version == 0 {
            return Err(EventValidationError::InvalidSchemaVersion);
        }
        if self.sequence == 0 {
            return Err(EventValidationError::InvalidSequence);
        }
        let payload_type = self.payload.event_type();
        if self.event_type != payload_type {
            return Err(EventValidationError::TypeMismatch {
                envelope: self.event_type,
                payload: payload_type,
            });
        }
        if self.timestamp.trim().is_empty() {
            return Err(EventValidationError::MissingTimestamp);
        }
        match &self.payload {
            EventData::RunStateChanged { from, to, .. } if !from.can_transition_to(*to) => {
                return Err(EventValidationError::InvalidRunTransition {
                    from: *from,
                    to: *to,
                });
            }
            EventData::WorkerStateChanged { from, to, .. } if !from.can_transition_to(*to) => {
                return Err(EventValidationError::InvalidWorkerTransition {
                    from: *from,
                    to: *to,
                });
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum EventValidationError {
    #[error("event schema version must be greater than zero")]
    InvalidSchemaVersion,
    #[error("event sequence must be greater than zero")]
    InvalidSequence,
    #[error("event timestamp is required")]
    MissingTimestamp,
    #[error("event type mismatch: envelope {envelope:?}, payload {payload:?}")]
    TypeMismatch {
        envelope: EventType,
        payload: EventType,
    },
    #[error("invalid run transition in event: {from:?} -> {to:?}")]
    InvalidRunTransition { from: RunState, to: RunState },
    #[error("invalid worker transition in event: {from:?} -> {to:?}")]
    InvalidWorkerTransition { from: WorkerState, to: WorkerState },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct RuntimeSnapshot {
    pub schema_version: u32,
    #[ts(type = "number")]
    pub sequence: u64,
    pub run_id: RunId,
    pub run_state: RunState,
    pub plan_markdown: String,
    #[serde(default)]
    pub orchestration_plan: Option<crate::OrchestrationPlan>,
    pub workers: BTreeMap<String, WorkerState>,
    #[serde(default)]
    pub agent_plans: BTreeMap<String, AgentPlan>,
    #[serde(default)]
    pub reasoning_summaries: BTreeMap<String, Vec<ReasoningSummaryRecord>>,
    #[serde(default)]
    pub activity_items: Vec<AgentActivityItem>,
    #[serde(default)]
    pub orchestration_planning_activities: Vec<OrchestrationPlanningActivity>,
    #[serde(default)]
    pub run_controls: Vec<RunControlRecord>,
    #[serde(default)]
    pub supervisor_decisions: Vec<SupervisorDecisionRecord>,
    #[serde(default)]
    pub orchestration_revisions: Vec<OrchestrationRevisionRecord>,
    pub pending_approvals: Vec<ApprovalRequest>,
    pub artifact_ids: Vec<ArtifactId>,
    pub verification: VerificationStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct RuntimeDelta {
    #[ts(type = "number")]
    pub from_sequence: u64,
    #[ts(type = "number")]
    pub to_sequence: u64,
    pub events: Vec<EventEnvelope>,
}

impl RuntimeSnapshot {
    pub fn empty(run_id: RunId) -> Self {
        Self {
            schema_version: 1,
            sequence: 0,
            run_id,
            run_state: RunState::Created,
            plan_markdown: String::new(),
            orchestration_plan: None,
            workers: BTreeMap::new(),
            agent_plans: BTreeMap::new(),
            reasoning_summaries: BTreeMap::new(),
            activity_items: Vec::new(),
            orchestration_planning_activities: Vec::new(),
            run_controls: Vec::new(),
            supervisor_decisions: Vec::new(),
            orchestration_revisions: Vec::new(),
            pending_approvals: Vec::new(),
            artifact_ids: Vec::new(),
            verification: VerificationStatus::Unverified,
        }
    }

    pub fn apply(&mut self, event: &EventEnvelope) -> Result<(), ProjectionError> {
        if event.run_id != self.run_id {
            return Err(ProjectionError::RunMismatch {
                expected: self.run_id.clone(),
                actual: event.run_id.clone(),
            });
        }
        let expected = self.sequence + 1;
        if event.sequence != expected {
            return Err(ProjectionError::SequenceGap {
                expected,
                actual: event.sequence,
            });
        }
        event.validate()?;

        match &event.payload {
            EventData::RunCreated { .. } if event.sequence != 1 => {
                return Err(ProjectionError::RunCreatedAfterStart);
            }
            EventData::RunCreated { .. } => {}
            EventData::RunStateChanged { from, to, .. } => {
                if self.run_state != *from {
                    return Err(ProjectionError::RunStateMismatch {
                        expected: self.run_state,
                        actual: *from,
                    });
                }
                self.run_state = *to;
            }
            EventData::PlanUpdated { markdown, .. } => {
                self.plan_markdown.clone_from(markdown);
            }
            EventData::OrchestrationCreated { plan } => {
                self.orchestration_plan = Some((**plan).clone());
            }
            EventData::WorkerCreated { spec } => {
                if self.workers.contains_key(spec.worker_id.as_str()) {
                    return Err(ProjectionError::DuplicateWorker(spec.worker_id.clone()));
                }
                self.workers
                    .insert(spec.worker_id.to_string(), WorkerState::Draft);
            }
            EventData::WorkerRemoved {
                worker_id, from, ..
            } => {
                let current = self
                    .workers
                    .get(worker_id.as_str())
                    .ok_or_else(|| ProjectionError::UnknownWorker(worker_id.clone()))?;
                if current != from {
                    return Err(ProjectionError::WorkerStateMismatch {
                        worker_id: worker_id.clone(),
                        expected: *current,
                        actual: *from,
                    });
                }
                self.workers.remove(worker_id.as_str());
            }
            EventData::WorkerStateChanged {
                worker_id,
                from,
                to,
                ..
            } => {
                let current = self
                    .workers
                    .get_mut(worker_id.as_str())
                    .ok_or_else(|| ProjectionError::UnknownWorker(worker_id.clone()))?;
                if *current != *from {
                    return Err(ProjectionError::WorkerStateMismatch {
                        worker_id: worker_id.clone(),
                        expected: *current,
                        actual: *from,
                    });
                }
                *current = *to;
            }
            EventData::AgentPlanUpdated { worker_id, plan } => {
                self.agent_plans
                    .insert(agent_projection_key(worker_id.as_ref()), plan.clone());
            }
            EventData::ReasoningSummaryRecorded { worker_id, record } => {
                let records = self
                    .reasoning_summaries
                    .entry(agent_projection_key(worker_id.as_ref()))
                    .or_default();
                records.push(record.clone());
                if records.len() > 12 {
                    records.drain(..records.len() - 12);
                }
            }
            EventData::AgentActivityRecorded { item } => {
                self.activity_items.push(item.clone());
                if self.activity_items.len() > 160 {
                    self.activity_items.drain(..self.activity_items.len() - 160);
                }
            }
            EventData::OrchestrationPlanningActivityRecorded { activity } => {
                self.orchestration_planning_activities
                    .push(activity.clone());
                if self.orchestration_planning_activities.len() > 48 {
                    self.orchestration_planning_activities
                        .drain(..self.orchestration_planning_activities.len() - 48);
                }
            }
            EventData::RunControlRecorded { record } => {
                self.run_controls.push(record.clone());
                if self.run_controls.len() > 48 {
                    self.run_controls.drain(..self.run_controls.len() - 48);
                }
            }
            EventData::SupervisorDecisionRecorded { record } => {
                self.supervisor_decisions.push(record.clone());
                if self.supervisor_decisions.len() > 48 {
                    self.supervisor_decisions
                        .drain(..self.supervisor_decisions.len() - 48);
                }
            }
            EventData::OrchestrationRevisionRecorded { record } => {
                self.orchestration_revisions.push(record.clone());
                if self.orchestration_revisions.len() > 48 {
                    self.orchestration_revisions
                        .drain(..self.orchestration_revisions.len() - 48);
                }
            }
            EventData::OrchestrationPatched { plan, .. } => {
                self.orchestration_plan = Some((**plan).clone());
            }
            EventData::ApprovalRequested { request, .. } => {
                if self
                    .pending_approvals
                    .iter()
                    .any(|item| item.approval_id == request.approval_id)
                {
                    return Err(ProjectionError::DuplicateApproval(
                        request.approval_id.clone(),
                    ));
                }
                self.pending_approvals.push(request.clone());
            }
            EventData::ApprovalResolved { resolution } => {
                let before = self.pending_approvals.len();
                self.pending_approvals
                    .retain(|item| item.approval_id != resolution.approval_id);
                if before == self.pending_approvals.len() {
                    return Err(ProjectionError::UnknownApproval(
                        resolution.approval_id.clone(),
                    ));
                }
            }
            EventData::ArtifactRecorded { artifact }
                if !self.artifact_ids.contains(&artifact.artifact_id) =>
            {
                self.artifact_ids.push(artifact.artifact_id.clone());
            }
            EventData::ArtifactRecorded { .. } => {}
            EventData::VerificationRecorded { verification } => {
                self.verification = verification.status;
            }
            EventData::RunCompleted {
                completion,
                verification,
                ..
            } => {
                let terminal = match completion {
                    CompletionKind::Completed => RunState::Completed,
                    CompletionKind::PartiallyCompleted => RunState::PartiallyCompleted,
                };
                self.run_state.validate_transition(terminal).map_err(|_| {
                    ProjectionError::InvalidTerminalTransition {
                        from: self.run_state,
                        to: terminal,
                    }
                })?;
                self.run_state = terminal;
                self.verification = *verification;
            }
            EventData::RunFailed { .. } => {
                self.run_state
                    .validate_transition(RunState::Failed)
                    .map_err(|_| ProjectionError::InvalidTerminalTransition {
                        from: self.run_state,
                        to: RunState::Failed,
                    })?;
                self.run_state = RunState::Failed;
            }
            _ => {}
        }

        self.sequence = event.sequence;
        Ok(())
    }
}

fn agent_projection_key(worker_id: Option<&WorkerId>) -> String {
    worker_id
        .map(ToString::to_string)
        .unwrap_or_else(|| "orchestrator".to_owned())
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ProjectionError {
    #[error(transparent)]
    InvalidEvent(#[from] EventValidationError),
    #[error("projection run mismatch: expected {expected}, got {actual}")]
    RunMismatch { expected: RunId, actual: RunId },
    #[error("sequence gap: expected {expected}, got {actual}")]
    SequenceGap { expected: u64, actual: u64 },
    #[error("run_created is only valid as sequence 1")]
    RunCreatedAfterStart,
    #[error("run state mismatch: projection {expected:?}, event starts at {actual:?}")]
    RunStateMismatch {
        expected: RunState,
        actual: RunState,
    },
    #[error("worker already exists: {0}")]
    DuplicateWorker(WorkerId),
    #[error("unknown worker: {0}")]
    UnknownWorker(WorkerId),
    #[error(
        "worker {worker_id} state mismatch: projection {expected:?}, event starts at {actual:?}"
    )]
    WorkerStateMismatch {
        worker_id: WorkerId,
        expected: WorkerState,
        actual: WorkerState,
    },
    #[error("approval already pending: {0}")]
    DuplicateApproval(String),
    #[error("unknown approval: {0}")]
    UnknownApproval(String),
    #[error("invalid terminal transition: {from:?} -> {to:?}")]
    InvalidTerminalTransition { from: RunState, to: RunState },
}
