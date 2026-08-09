use std::collections::BTreeMap;

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
    Auto,
    None,
    Minimal,
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    #[ts(rename = "xhigh")]
    XHigh,
    Max,
}

impl ReasoningEffort {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ReasoningEffortProfile {
    pub supported: Vec<ReasoningEffort>,
    pub default_effort: ReasoningEffort,
    pub thinking_supported: bool,
    pub note: String,
}

pub fn reasoning_effort_profile(
    provider_type: ProviderType,
    model_id: &str,
) -> ReasoningEffortProfile {
    use ReasoningEffort::{Auto, High, Low, Max, Medium, None, XHigh};

    let model = model_id.trim().to_ascii_lowercase();
    match provider_type {
        ProviderType::DeepSeek => deepseek_reasoning_profile(&model),
        ProviderType::Anthropic | ProviderType::GenericAnthropicCompatible => {
            anthropic_reasoning_profile(&model)
        }
        ProviderType::OpenAi => openai_reasoning_profile(&model),
        ProviderType::GenericOpenAiCompatible if model.contains("deepseek-v4") => {
            deepseek_reasoning_profile(&model)
        }
        ProviderType::GenericOpenAiCompatible if model.contains("claude") => {
            anthropic_reasoning_profile(&model)
        }
        ProviderType::GenericOpenAiCompatible
            if model.contains("kimi-k3") || model.contains("kimi/kimi-k3") =>
        {
            ReasoningEffortProfile {
                supported: vec![Auto, Max],
                default_effort: Auto,
                thinking_supported: true,
                note: "Kimi K3 accepts max reasoning effort; Auto preserves the provider default."
                    .into(),
            }
        }
        ProviderType::GenericOpenAiCompatible
            if model.starts_with("kimi-")
                || model.starts_with("kimi/")
                || model.starts_with("moonshot-") =>
        {
            ReasoningEffortProfile {
                supported: vec![Auto],
                default_effort: Auto,
                thinking_supported: false,
                note: "This Kimi/Moonshot model has no built-in effort profile in LunaScope. Auto is safe; use a custom effort override if the configured endpoint documents one.".into(),
            }
        }
        ProviderType::GenericOpenAiCompatible if model.contains("glm-5.2") => {
            ReasoningEffortProfile {
                supported: vec![
                    Auto,
                    None,
                    ReasoningEffort::Minimal,
                    Low,
                    Medium,
                    High,
                    XHigh,
                    Max,
                ],
                default_effort: Auto,
                thinking_supported: true,
                note: "GLM-5.2 supports the full none-through-max reasoning effort range on native compatible endpoints.".into(),
            }
        }
        ProviderType::GenericOpenAiCompatible
            if model.starts_with("glm-5.1") || model == "glm-5" =>
        {
            ReasoningEffortProfile {
                supported: vec![
                    Auto,
                    None,
                    ReasoningEffort::Minimal,
                    Low,
                    Medium,
                    High,
                    XHigh,
                ],
                default_effort: Auto,
                thinking_supported: true,
                note: "GLM-5 and GLM-5.1 support none through xhigh; max is not accepted by these releases.".into(),
            }
        }
        ProviderType::GenericOpenAiCompatible if model.starts_with("glm-") => {
            ReasoningEffortProfile {
                supported: vec![Auto, None, High],
                default_effort: Auto,
                thinking_supported: true,
                note: "GLM family compatibility profile: Auto uses dynamic thinking, Off disables it where supported, and High enables it. Endpoint-specific values remain available through the custom override for unrecognized ids.".into(),
            }
        }
        ProviderType::GenericOpenAiCompatible if model.contains("qwen3.8-max") => {
            ReasoningEffortProfile {
                supported: vec![Auto, Low, Medium, XHigh],
                default_effort: Auto,
                thinking_supported: true,
                note: "Qwen3.8 Max supports low, medium, and xhigh through the OpenAI-compatible reasoning_effort field.".into(),
            }
        }
        ProviderType::GenericOpenAiCompatible
            if model.starts_with("qwen3.5")
                || model.starts_with("qwen3.6")
                || model.starts_with("qwen3.7") =>
        {
            ReasoningEffortProfile {
                supported: vec![
                    Auto,
                    None,
                    ReasoningEffort::Minimal,
                    Low,
                    Medium,
                    High,
                    XHigh,
                    Max,
                ],
                default_effort: Auto,
                thinking_supported: true,
                note: "Current Qwen hybrid-thinking families expose the standard none-through-max range on Responses-compatible endpoints.".into(),
            }
        }
        ProviderType::GenericOpenAiCompatible if model.starts_with("qwen") => {
            ReasoningEffortProfile {
                supported: vec![Auto, None, High],
                default_effort: Auto,
                thinking_supported: true,
                note: "Qwen family compatibility profile: Auto preserves endpoint defaults; Off and High map to the broadly supported thinking toggle.".into(),
            }
        }
        ProviderType::GenericOpenAiCompatible if model.starts_with("gemini-3.1-pro") => {
            ReasoningEffortProfile {
                supported: vec![Auto, Low, Medium, High],
                default_effort: Auto,
                thinking_supported: true,
                note: "Gemini 3.1 Pro maps OpenAI-compatible reasoning effort to low, medium, or high thinking levels.".into(),
            }
        }
        ProviderType::GenericOpenAiCompatible if model.starts_with("gemini-3-pro") => {
            ReasoningEffortProfile {
                supported: vec![Auto, Low, High],
                default_effort: Auto,
                thinking_supported: true,
                note: "Gemini 3 Pro maps OpenAI-compatible reasoning effort to low or high thinking levels.".into(),
            }
        }
        ProviderType::GenericOpenAiCompatible if model.starts_with("gemini-3") => {
            ReasoningEffortProfile {
                supported: vec![Auto, ReasoningEffort::Minimal, Low, Medium, High],
                default_effort: Auto,
                thinking_supported: true,
                note: "Gemini 3 OpenAI compatibility maps reasoning_effort to model-specific thinking levels.".into(),
            }
        }
        ProviderType::GenericOpenAiCompatible if model.starts_with("gemini-2.5-flash") => {
            ReasoningEffortProfile {
                supported: vec![Auto, None, Low, Medium, High],
                default_effort: Auto,
                thinking_supported: true,
                note: "Gemini 2.5 Flash maps OpenAI-compatible reasoning effort to a thinking budget and can disable thinking.".into(),
            }
        }
        ProviderType::GenericOpenAiCompatible if model.starts_with("gemini-2.5-pro") => {
            ReasoningEffortProfile {
                supported: vec![Auto, Low, Medium, High],
                default_effort: Auto,
                thinking_supported: true,
                note: "Gemini 2.5 Pro maps OpenAI-compatible reasoning effort to a thinking budget and cannot disable thinking.".into(),
            }
        }
        ProviderType::GenericOpenAiCompatible if model.starts_with("grok-4.20-multi-agent") => {
            ReasoningEffortProfile {
                supported: vec![Auto, Low, Medium, High, XHigh],
                default_effort: Auto,
                thinking_supported: true,
                note: "Grok 4.20 Multi-Agent supports low through xhigh; effort controls the collaborating Agent count.".into(),
            }
        }
        ProviderType::GenericOpenAiCompatible if model.starts_with("grok-4.5") => {
            ReasoningEffortProfile {
                supported: vec![Auto, Low, Medium, High],
                default_effort: Auto,
                thinking_supported: true,
                note: "Grok 4.5 supports low, medium, and high reasoning effort and cannot disable reasoning.".into(),
            }
        }
        ProviderType::GenericOpenAiCompatible if model.starts_with("grok-4.3") => {
            ReasoningEffortProfile {
                supported: vec![Auto, None, Low, Medium, High],
                default_effort: Auto,
                thinking_supported: true,
                note: "Grok 4.3 supports none, low, medium, and high reasoning effort.".into(),
            }
        }
        ProviderType::GenericOpenAiCompatible if model.starts_with("grok-") => {
            ReasoningEffortProfile {
                supported: vec![Auto],
                default_effort: Auto,
                thinking_supported: false,
                note: "This Grok model has no built-in effort profile in LunaScope. Auto is safe; use a custom effort override if the configured endpoint documents one.".into(),
            }
        }
        ProviderType::GenericOpenAiCompatible => ReasoningEffortProfile {
            supported: vec![Auto],
            default_effort: Auto,
            thinking_supported: false,
            note: "No provider-native effort contract is known for this compatible model, so LunaScope omits the field instead of guessing.".into(),
        },
    }
}

pub fn effective_reasoning_effort(
    provider_type: ProviderType,
    model_id: &str,
    requested: ReasoningEffort,
) -> ReasoningEffort {
    let profile = reasoning_effort_profile(provider_type, model_id);
    if profile.supported.contains(&requested) {
        requested
    } else {
        profile.default_effort
    }
}

fn deepseek_reasoning_profile(model: &str) -> ReasoningEffortProfile {
    use ReasoningEffort::{Auto, High, Low, Max};
    if model.contains("v4-flash") {
        ReasoningEffortProfile {
            supported: vec![Auto, High, Max],
            default_effort: Auto,
            thinking_supported: true,
            note: "DeepSeek V4 Flash accepts high and max; Auto preserves the provider default."
                .into(),
        }
    } else if model.contains("v4-pro") {
        ReasoningEffortProfile {
            supported: vec![Auto, High, Max],
            default_effort: Auto,
            thinking_supported: true,
            note: "DeepSeek V4 Pro currently maps low to high and xhigh to max, so duplicate levels are hidden.".into(),
        }
    } else {
        ReasoningEffortProfile {
            supported: vec![Auto, Low, High],
            default_effort: Auto,
            thinking_supported: true,
            note: "DeepSeek compatibility profile; exact effort mapping depends on the selected release.".into(),
        }
    }
}

fn anthropic_reasoning_profile(model: &str) -> ReasoningEffortProfile {
    use ReasoningEffort::{Auto, High, Low, Max, Medium, XHigh};
    let supports_xhigh = model.contains("opus-5")
        || model.contains("sonnet-5")
        || model.contains("fable-5")
        || model.contains("mythos-5")
        || model.contains("opus-4-8")
        || model.contains("opus-4.8")
        || model.contains("opus-4-7")
        || model.contains("opus-4.7");
    let supports_max = supports_xhigh
        || model.contains("opus-4-6")
        || model.contains("opus-4.6")
        || model.contains("sonnet-4-6")
        || model.contains("sonnet-4.6")
        || model.contains("mythos-preview");
    let mut supported = vec![Auto, Low, Medium, High];
    if supports_xhigh {
        supported.push(XHigh);
    }
    if supports_max {
        supported.push(Max);
    }
    ReasoningEffortProfile {
        supported,
        default_effort: Auto,
        thinking_supported: true,
        note: "Claude effort is serialized as output_config.effort; xhigh and max are shown only for known supporting families.".into(),
    }
}

fn openai_reasoning_profile(model: &str) -> ReasoningEffortProfile {
    use ReasoningEffort::{Auto, High, Low, Medium, Minimal, None, XHigh};
    if model.contains("gpt-5-pro") || model.contains("o1-pro") || model.contains("o3-pro") {
        return ReasoningEffortProfile {
            supported: vec![Auto, High],
            default_effort: Auto,
            thinking_supported: true,
            note: "This Pro model has a fixed high reasoning effort.".into(),
        };
    }
    if model == "gpt-5.1" || model.starts_with("gpt-5.1-") && !model.contains("codex-max") {
        return ReasoningEffortProfile {
            supported: vec![Auto, None, Low, Medium, High],
            default_effort: Auto,
            thinking_supported: true,
            note: "GPT-5.1 supports none, low, medium, and high.".into(),
        };
    }
    if model.contains("codex-max") {
        return ReasoningEffortProfile {
            supported: vec![Auto, Low, Medium, High, XHigh],
            default_effort: Auto,
            thinking_supported: true,
            note: "Codex Max supports extended xhigh reasoning.".into(),
        };
    }
    if model.starts_with("gpt-5") {
        return ReasoningEffortProfile {
            supported: vec![Auto, None, Minimal, Low, Medium, High, XHigh],
            default_effort: Auto,
            thinking_supported: true,
            note: "Current GPT-5-family Responses models expose reasoning levels through xhigh; the provider validates snapshot-specific support.".into(),
        };
    }
    if model.starts_with('o')
        && model
            .chars()
            .nth(1)
            .is_some_and(|value| value.is_ascii_digit())
    {
        return ReasoningEffortProfile {
            supported: vec![Auto, Low, Medium, High],
            default_effort: Auto,
            thinking_supported: true,
            note: "OpenAI o-series compatibility profile.".into(),
        };
    }
    ReasoningEffortProfile {
        supported: vec![Auto],
        default_effort: Auto,
        thinking_supported: false,
        note: "No provider-native reasoning effort is known for this OpenAI model.".into(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ModelAssignment {
    pub role: ModelRole,
    pub provider_config_id: String,
    pub model_id: String,
    /// Provider-native reasoning effort. `None` means the Provider default.
    /// This is intentionally free-form because compatible endpoints frequently
    /// add effort levels before LunaScope can update a built-in model catalog.
    #[serde(default)]
    pub custom_reasoning_effort: Option<String>,
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
    #[serde(default)]
    pub vision: Option<ModelAssignment>,
    pub worker_pool: Vec<ModelAssignment>,
    /// Legacy provider/model effort overrides retained only for migration from
    /// LunaScope 0.1.0. New settings store the value on each assignment.
    #[serde(default)]
    pub custom_reasoning_efforts: BTreeMap<String, String>,
}

pub fn reasoning_effort_override_key(provider_config_id: &str, model_id: &str) -> String {
    format!("{}\0{}", provider_config_id.trim(), model_id.trim())
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
    #[serde(default)]
    pub phase: String,
    #[ts(type = "number")]
    pub input_tokens: u64,
    #[ts(type = "number")]
    pub output_tokens: u64,
    #[serde(default)]
    #[ts(type = "number")]
    pub cached_input_tokens: u64,
    #[serde(default)]
    #[ts(type = "number")]
    pub latency_ms: u64,
    #[ts(type = "number | null")]
    pub total_cost_microusd: Option<u64>,
    pub fallback_from: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_v4_profiles_hide_equivalent_effort_levels() {
        let flash = reasoning_effort_profile(ProviderType::DeepSeek, "deepseek-v4-flash");
        assert_eq!(
            flash.supported,
            vec![
                ReasoningEffort::Auto,
                ReasoningEffort::High,
                ReasoningEffort::Max
            ]
        );
        let pro = reasoning_effort_profile(ProviderType::DeepSeek, "deepseek-v4-pro");
        assert_eq!(
            pro.supported,
            vec![
                ReasoningEffort::Auto,
                ReasoningEffort::High,
                ReasoningEffort::Max
            ]
        );
    }

    #[test]
    fn current_frontier_profiles_include_provider_specific_extended_levels() {
        let openai = reasoning_effort_profile(ProviderType::OpenAi, "gpt-5.2-codex-max");
        assert!(openai.supported.contains(&ReasoningEffort::XHigh));
        assert!(!openai.supported.contains(&ReasoningEffort::Max));

        let claude = reasoning_effort_profile(ProviderType::Anthropic, "claude-opus-4.8");
        assert!(claude.supported.contains(&ReasoningEffort::XHigh));
        assert!(claude.supported.contains(&ReasoningEffort::Max));

        let glm = reasoning_effort_profile(ProviderType::GenericOpenAiCompatible, "glm-5.2");
        assert_eq!(
            glm.supported,
            vec![
                ReasoningEffort::Auto,
                ReasoningEffort::None,
                ReasoningEffort::Minimal,
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::XHigh,
                ReasoningEffort::Max
            ]
        );

        let qwen =
            reasoning_effort_profile(ProviderType::GenericOpenAiCompatible, "qwen3.8-max-preview");
        assert_eq!(
            qwen.supported,
            vec![
                ReasoningEffort::Auto,
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::XHigh,
            ]
        );

        let gemini = reasoning_effort_profile(
            ProviderType::GenericOpenAiCompatible,
            "gemini-3.1-pro-preview",
        );
        assert_eq!(
            gemini.supported,
            vec![
                ReasoningEffort::Auto,
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High
            ]
        );
    }

    #[test]
    fn unknown_compatible_models_omit_unverified_effort_fields() {
        assert_eq!(
            effective_reasoning_effort(
                ProviderType::GenericOpenAiCompatible,
                "vendor-model",
                ReasoningEffort::High,
            ),
            ReasoningEffort::Auto
        );
    }

    #[test]
    fn compatible_provider_families_use_model_specific_effort_profiles() {
        let kimi = reasoning_effort_profile(ProviderType::GenericOpenAiCompatible, "kimi/kimi-k3");
        assert_eq!(
            kimi.supported,
            vec![ReasoningEffort::Auto, ReasoningEffort::Max]
        );

        let qwen = reasoning_effort_profile(ProviderType::GenericOpenAiCompatible, "qwen3.7-max");
        assert!(qwen.supported.contains(&ReasoningEffort::None));
        assert!(qwen.supported.contains(&ReasoningEffort::Max));

        let grok = reasoning_effort_profile(
            ProviderType::GenericOpenAiCompatible,
            "grok-4.20-multi-agent",
        );
        assert!(grok.supported.contains(&ReasoningEffort::XHigh));

        let unknown_kimi =
            reasoning_effort_profile(ProviderType::GenericOpenAiCompatible, "moonshot-future");
        assert_eq!(unknown_kimi.supported, vec![ReasoningEffort::Auto]);
        assert!(!unknown_kimi.thinking_supported);
    }
}
