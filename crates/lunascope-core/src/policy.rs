use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{RiskLevel, RunId, WorkerId};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum PermissionKind {
    FilesystemRead,
    FilesystemWrite,
    FilesystemDelete,
    ShellExecute,
    ProcessSpawn,
    NetworkConnect,
    BrowserControl,
    SecretsUse,
    GitCommit,
    GitPush,
    McpInvoke,
    SkillLoad,
    WorkerSpawn,
    ExtensionExecute,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum PolicyDecision {
    Allow,
    Ask,
    Deny,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
#[ts(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum PolicyScope {
    Global,
    Workspace(String),
    Project(String),
    Run(RunId),
    Worker(WorkerId),
    Tool(String),
    Path(String),
    Command(String),
    NetworkDomain(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct PolicyRule {
    pub id: String,
    pub permission: PermissionKind,
    pub decision: PolicyDecision,
    pub scope: PolicyScope,
    pub priority: i32,
    pub locked: bool,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct PermissionContext {
    pub workspace_root: String,
    pub project_id: Option<String>,
    pub run_id: Option<RunId>,
    pub worker_id: Option<WorkerId>,
    pub tool_id: Option<String>,
    pub target_path: Option<String>,
    pub program: Option<String>,
    pub network_domain: Option<String>,
    pub arguments: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct PermissionRequest {
    pub permission: PermissionKind,
    pub context: PermissionContext,
    pub risk: RiskLevel,
    pub action: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct PolicyOutcome {
    pub decision: PolicyDecision,
    pub matched_rule_id: Option<String>,
    pub reason: String,
    pub hard_constraint: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ToolManifest {
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    pub risk: RiskLevel,
    pub permissions: Vec<PermissionKind>,
    #[ts(type = "number")]
    pub timeout_ms: u64,
    pub cancellable: bool,
    pub deterministic: bool,
    pub source: String,
    pub version: String,
}
