use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{PermissionContext, PermissionKind, PermissionRequest, PolicyOutcome, ToolManifest};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum SkillSourceKind {
    Agents,
    Codex,
    Claude,
    OpenCode,
    LunaScopeGlobal,
    LunaScopeProject,
    LunaScopeSystem,
    LunaScopeUser,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum SkillCompatibility {
    Native,
    Compatible,
    BridgeRequired,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct SkillSummary {
    pub catalog_id: String,
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub required_tools: Vec<String>,
    pub required_permissions: Vec<PermissionKind>,
    #[ts(type = "number")]
    pub approximate_context_tokens: u64,
    pub source: SkillSourceKind,
    pub path: String,
    pub compatibility: SkillCompatibility,
    pub warnings: Vec<String>,
    pub content_sha256: String,
    pub user_invocable: bool,
    pub model_invocable: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct LoadedSkill {
    pub summary: SkillSummary,
    pub instructions: String,
    pub supporting_files: Vec<String>,
    pub preserved_frontmatter: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
#[ts(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum McpConfigValue {
    Literal(String),
    CredentialReference(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct McpHeader {
    pub name: String,
    pub value: McpConfigValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", content = "config", rename_all = "snake_case")]
#[ts(tag = "kind", content = "config", rename_all = "snake_case")]
pub enum McpTransportConfig {
    Stdio {
        program: String,
        args: Vec<String>,
        cwd: Option<String>,
        environment: BTreeMap<String, McpConfigValue>,
    },
    StreamableHttp {
        url: String,
        headers: Vec<McpHeader>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    pub transport: McpTransportConfig,
    #[ts(type = "number")]
    pub timeout_ms: u64,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct McpToolDescriptor {
    pub server_config_id: String,
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
    pub output_schema: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct McpServerStatus {
    pub server_config_id: String,
    pub healthy: bool,
    pub server_name: Option<String>,
    pub server_version: Option<String>,
    pub tools: Vec<McpToolDescriptor>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct McpInvocationResult {
    pub content: serde_json::Value,
    pub is_error: bool,
    #[ts(type = "number")]
    pub duration_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ToolRoutingRequest {
    pub role: String,
    pub objective: String,
    pub preferred_tool_ids: Vec<String>,
    pub required_skill_catalog_ids: Vec<String>,
    pub max_tools: u8,
    pub permission_context: PermissionContext,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ToolRouteRejection {
    pub tool_id: String,
    pub reason: String,
    pub outcomes: Vec<PolicyOutcome>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ToolRoutingDecision {
    pub selected_tools: Vec<ToolManifest>,
    pub approval_requests: Vec<PermissionRequest>,
    pub rejections: Vec<ToolRouteRejection>,
    pub rationale: String,
}
