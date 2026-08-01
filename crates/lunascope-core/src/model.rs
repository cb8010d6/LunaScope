use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ProviderType {
    OpenAi,
    Anthropic,
    DeepSeek,
    GenericOpenAiCompatible,
    GenericAnthropicCompatible,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ProviderProtocol {
    OpenAiResponses,
    OpenAiChatCompletions,
    AnthropicMessages,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CredentialReference {
    pub id: String,
    pub provider: ProviderType,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
#[ts(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ProviderHeaderValue {
    Literal(String),
    CredentialReference(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ProviderHeader {
    pub name: String,
    pub value: ProviderHeaderValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub id: String,
    pub provider_type: ProviderType,
    pub protocol: ProviderProtocol,
    pub display_name: String,
    pub base_url: String,
    pub credential_reference_id: String,
    #[serde(default)]
    pub default_model_id: String,
    pub custom_headers: Vec<ProviderHeader>,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub context_window_tokens: Option<u64>,
    #[serde(default)]
    pub supports_tools: bool,
    #[serde(default)]
    pub supports_vision: bool,
    #[serde(default)]
    pub supports_structured_output: bool,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ModelCapability {
    Text,
    Vision,
    Tools,
    StructuredOutput,
    LongContext,
    LocalExecution,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ModelRole {
    Orchestration,
    GeneralWorker,
    Programming,
    Research,
    Writing,
    Frontend,
    GameDevelopment,
    Reviewer,
    Verifier,
    FastCheap,
    Vision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ModelAssignment {
    pub role: ModelRole,
    pub provider_config_id: String,
    pub model_id: String,
    pub reasoning_effort: ReasoningEffort,
    #[ts(type = "number | null")]
    pub maximum_context_tokens: Option<u64>,
    #[ts(type = "number | null")]
    pub maximum_budget_microusd: Option<u64>,
    pub fallback_provider_config_id: Option<String>,
    pub fallback_model_id: Option<String>,
    pub locked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ModelSelectionSettings {
    pub orchestration: ModelAssignment,
    pub worker_pool: Vec<ModelAssignment>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ModelProfile {
    pub provider_config_id: String,
    pub provider_type: ProviderType,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: Vec<ModelCapability>,
    pub roles: Vec<ModelRole>,
    pub available: bool,
    pub local: bool,
    pub quality_score: u8,
    pub speed_score: u8,
    pub privacy_score: u8,
    #[ts(type = "number")]
    pub input_cost_microusd_per_million_tokens: u64,
    #[ts(type = "number")]
    pub output_cost_microusd_per_million_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct RoutingPriorities {
    pub quality: u8,
    pub cost: u8,
    pub speed: u8,
    pub privacy: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ModelRoutingPolicy {
    pub priorities: RoutingPriorities,
    pub allowed_provider_config_ids: Vec<String>,
    pub disallowed_provider_config_ids: Vec<String>,
    #[ts(type = "number | null")]
    pub maximum_cost_microusd_per_run: Option<u64>,
    pub prefer_local: bool,
    pub required_capabilities: Vec<ModelCapability>,
    pub fallback_allowed: bool,
    pub ask_before_cost_escalation: bool,
    pub role_preferences: Vec<RoleProviderPreference>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct RoleProviderPreference {
    pub role: ModelRole,
    pub preferred_providers: Vec<ProviderType>,
    pub hard_constraint: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct NaturalLanguageRoutingDraft {
    pub source_text: String,
    pub policy: ModelRoutingPolicy,
    pub matched_rules: Vec<String>,
    pub warnings: Vec<String>,
    pub confirmed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ModelRoutingRequest {
    pub role: ModelRole,
    pub required_capabilities: Vec<ModelCapability>,
    pub locked_provider_config_id: Option<String>,
    pub locked_model_id: Option<String>,
    #[ts(type = "number")]
    pub estimated_input_tokens: u64,
    #[ts(type = "number")]
    pub estimated_output_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ModelRoutingDecision {
    pub provider_config_id: String,
    pub model_id: String,
    pub reason: String,
    pub fallback: bool,
    #[ts(type = "number")]
    pub estimated_cost_microusd: u64,
    pub requires_cost_approval: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ModelUsageRecord {
    pub provider_config_id: String,
    pub model_id: String,
    pub role: ModelRole,
    #[ts(type = "number")]
    pub input_tokens: u64,
    #[ts(type = "number")]
    pub output_tokens: u64,
    #[ts(type = "number | null")]
    pub total_cost_microusd: Option<u64>,
    pub fallback_from: Option<String>,
}
