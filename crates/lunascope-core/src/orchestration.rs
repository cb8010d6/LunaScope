use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    ArtifactId, ModelSelection, OrchestrationId, RunId, RuntimeSnapshot, VerificationRecord,
    WorkerId, WorkerSpec, WorkerState,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum DelegationKind {
    SingleAgent,
    MultiAgent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct DelegationDecision {
    pub kind: DelegationKind,
    pub rationale: String,
    pub expected_benefit: String,
    pub estimated_duration: String,
    pub estimated_cost: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkerInputContext {
    pub summary: String,
    pub artifact_ids: Vec<ArtifactId>,
    pub include_workspace_snapshot: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkerOutputSchema {
    pub media_type: String,
    pub schema: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkerBudget {
    #[ts(type = "number | null")]
    pub maximum_input_tokens: Option<u64>,
    #[ts(type = "number | null")]
    pub maximum_output_tokens: Option<u64>,
    #[ts(type = "number | null")]
    pub maximum_cost_microusd: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct AllowedWorkerModel {
    pub provider: String,
    pub model: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct RetryPolicy {
    pub maximum_attempts: u32,
    #[ts(type = "number")]
    pub backoff_ms: u64,
    pub retryable_error_codes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkerCheckpointPolicy {
    pub on_start: bool,
    pub on_artifact: bool,
    pub on_completion: bool,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum WorkerField {
    Role,
    Tags,
    Objective,
    Task,
    Prompt,
    InputContext,
    ExpectedOutput,
    OutputSchema,
    CompletionCriteria,
    Model,
    Skills,
    Tools,
    Permissions,
    Budget,
    Dependencies,
    Timeout,
    RetryPolicy,
    CheckpointPolicy,
    WriteScopes,
    ParentWorker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum OrchestrationPatchApplyMode {
    ApplyNow,
    ApplyAfterCurrentStep,
    ApplyOnRetry,
    CloneRevision,
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkerPatch {
    pub worker_id: WorkerId,
    pub role: Option<String>,
    pub tags: Option<Vec<String>>,
    pub objective: Option<String>,
    pub task: Option<String>,
    pub prompt: Option<String>,
    pub input_context: Option<WorkerInputContext>,
    pub expected_output: Option<String>,
    pub output_schema: Option<WorkerOutputSchema>,
    pub completion_criteria: Option<Vec<String>>,
    pub model: Option<ModelSelection>,
    pub skills: Option<Vec<String>>,
    pub tools: Option<Vec<String>>,
    pub permissions: Option<Vec<String>>,
    pub budget: Option<WorkerBudget>,
    pub dependencies: Option<Vec<WorkerId>>,
    #[ts(type = "number | null")]
    pub timeout_ms: Option<u64>,
    pub retry_policy: Option<RetryPolicy>,
    pub checkpoint_policy: Option<WorkerCheckpointPolicy>,
    pub write_scopes: Option<Vec<String>>,
    pub parent_worker_id: Option<Option<WorkerId>>,
    pub lock_fields: Vec<WorkerField>,
    pub unlock_fields: Vec<WorkerField>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[ts(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum OrchestrationPatchOperation {
    AddWorker { spec: WorkerSpec },
    UpdateWorker { patch: WorkerPatch },
    RemoveWorker { worker_id: WorkerId },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct OrchestrationPatch {
    pub patch_id: String,
    pub base_version: u32,
    pub apply_mode: OrchestrationPatchApplyMode,
    pub reason: String,
    pub operations: Vec<OrchestrationPatchOperation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct UserOverrideRecord {
    pub patch_id: String,
    pub worker_id: WorkerId,
    pub fields: Vec<WorkerField>,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct OrchestrationPlan {
    pub orchestration_id: OrchestrationId,
    pub version: u32,
    pub objective: String,
    #[serde(default)]
    pub project_id: Option<crate::ProjectId>,
    #[serde(default)]
    pub thread_id: Option<crate::ThreadId>,
    #[serde(default)]
    pub conversation_title: Option<String>,
    #[serde(default)]
    pub domain_pack: Option<crate::DomainPackId>,
    pub decision: DelegationDecision,
    pub maximum_parallel_workers: u32,
    pub maximum_worker_depth: u32,
    pub parent_permissions: Vec<String>,
    pub allowed_worker_models: Vec<AllowedWorkerModel>,
    pub user_hard_constraints: Vec<String>,
    pub workers: Vec<WorkerSpec>,
    pub user_overrides: Vec<UserOverrideRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct OrchestrationSession {
    pub run_id: RunId,
    pub plan: OrchestrationPlan,
    pub snapshot: RuntimeSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct OrchestrationValidation {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub topological_order: Vec<WorkerId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct HandoffRecord {
    pub handoff_id: String,
    pub from_worker_id: WorkerId,
    pub to_worker_id: WorkerId,
    pub artifact_ids: Vec<ArtifactId>,
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkerExecutionRecord {
    pub worker_id: WorkerId,
    pub state: WorkerState,
    pub attempts: u32,
    pub worktree_path: Option<String>,
    pub artifact_ids: Vec<ArtifactId>,
    pub summary: String,
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkerStateChange {
    pub worker_id: WorkerId,
    pub from: WorkerState,
    pub to: WorkerState,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct OrchestrationRunResult {
    pub orchestration_id: OrchestrationId,
    pub version: u32,
    pub workers: BTreeMap<WorkerId, WorkerExecutionRecord>,
    pub state_changes: Vec<WorkerStateChange>,
    pub artifacts: Vec<crate::ArtifactRecord>,
    pub handoffs: Vec<HandoffRecord>,
    pub synthesis: String,
    pub verification: VerificationRecord,
}
