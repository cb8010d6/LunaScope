use lunascope_core::{
    ModelRole, ModelRoutingPolicy, NaturalLanguageRoutingDraft, ProviderType,
    RoleProviderPreference,
};

pub fn parse_routing_preference(
    source: impl Into<String>,
    mut base_policy: ModelRoutingPolicy,
) -> NaturalLanguageRoutingDraft {
    let source = source.into();
    let normalized = source.to_lowercase();
    let mut matched_rules = Vec::new();
    let mut warnings = Vec::new();

    detect_role_provider(
        &normalized,
        &mut base_policy,
        ModelRole::Programming,
        &["代码", "编程", "开发", "coding", "programming"],
        &[
            (ProviderType::OpenAi, &["gpt", "openai"][..]),
            (ProviderType::Anthropic, &["claude", "anthropic"][..]),
            (ProviderType::DeepSeek, &["deepseek", "深度求索"][..]),
        ],
        &mut matched_rules,
    );
    detect_role_provider(
        &normalized,
        &mut base_policy,
        ModelRole::Writing,
        &["论文", "写作", "长文档", "writing", "academic"],
        &[
            (ProviderType::Anthropic, &["claude", "anthropic"][..]),
            (ProviderType::OpenAi, &["gpt", "openai"][..]),
        ],
        &mut matched_rules,
    );
    detect_role_provider(
        &normalized,
        &mut base_policy,
        ModelRole::FastCheap,
        &[
            "低风险",
            "重复任务",
            "日常",
            "常规",
            "简单任务",
            "batch",
            "routine",
            "daily",
        ],
        &[
            (ProviderType::DeepSeek, &["deepseek", "深度求索"][..]),
            (ProviderType::OpenAi, &["gpt", "openai"][..]),
        ],
        &mut matched_rules,
    );
    if contains_any(
        &normalized,
        &[
            "成本过高",
            "提高成本",
            "增加成本",
            "cost too high",
            "higher cost",
            "above budget",
        ],
    ) && contains_any(&normalized, &["询问", "问我", "先问", "ask", "confirm"])
    {
        base_policy.ask_before_cost_escalation = true;
        matched_rules.push("Cost escalation requires approval".into());
    }
    if contains_any(&normalized, &["只允许", "only allow", "must use"]) {
        for preference in &mut base_policy.role_preferences {
            preference.hard_constraint = true;
        }
        matched_rules.push("Detected hard provider constraint".into());
    }
    if matched_rules.is_empty() {
        warnings.push(
            "No supported routing clause was recognized; review the unchanged policy.".into(),
        );
    }
    NaturalLanguageRoutingDraft {
        source_text: source,
        policy: base_policy,
        matched_rules,
        warnings,
        confirmed: false,
    }
}

fn detect_role_provider(
    source: &str,
    policy: &mut ModelRoutingPolicy,
    role: ModelRole,
    role_terms: &[&str],
    providers: &[(ProviderType, &[&str])],
    matched: &mut Vec<String>,
) {
    let preferred = source
        .split(['。', '，', '；', ',', ';', '.', '!', '！', '?', '？', '\n'])
        .filter(|clause| contains_any(clause, role_terms))
        .flat_map(|clause| {
            providers
                .iter()
                .filter(move |(_, terms)| contains_any(clause, terms))
                .map(|(provider, _)| *provider)
        })
        .fold(Vec::new(), |mut accumulated, provider| {
            if !accumulated.contains(&provider) {
                accumulated.push(provider);
            }
            accumulated
        });
    if preferred.is_empty() {
        return;
    }
    policy.role_preferences.retain(|item| item.role != role);
    policy.role_preferences.push(RoleProviderPreference {
        role,
        preferred_providers: preferred.clone(),
        hard_constraint: false,
    });
    matched.push(format!("{role:?}: prefer {preferred:?}"));
}

fn contains_any(source: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| source.contains(term))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunascope_core::{ModelCapability, RoutingPriorities};

    fn base_policy() -> ModelRoutingPolicy {
        ModelRoutingPolicy {
            priorities: RoutingPriorities {
                quality: 80,
                cost: 50,
                speed: 50,
                privacy: 50,
            },
            allowed_provider_config_ids: Vec::new(),
            disallowed_provider_config_ids: Vec::new(),
            maximum_cost_microusd_per_run: Some(10_000),
            prefer_local: false,
            required_capabilities: vec![ModelCapability::Text],
            fallback_allowed: true,
            ask_before_cost_escalation: false,
            role_preferences: Vec::new(),
        }
    }

    #[test]
    fn parses_contract_examples_into_reviewable_unconfirmed_draft() {
        let draft = parse_routing_preference(
            "普通代码实现优先 GPT。论文写作和长文档优先 Claude。\
             低风险重复任务优先 DeepSeek。当预计成本过高时先询问。",
            base_policy(),
        );
        assert!(!draft.confirmed);
        assert!(draft.warnings.is_empty());
        assert!(draft.policy.ask_before_cost_escalation);
        assert_eq!(draft.policy.role_preferences.len(), 3);
        assert!(
            draft
                .policy
                .role_preferences
                .iter()
                .any(|item| item.role == ModelRole::Writing
                    && item.preferred_providers == vec![ProviderType::Anthropic])
        );
    }

    #[test]
    fn parses_compact_chinese_preference_without_cross_clause_provider_leakage() {
        let draft = parse_routing_preference(
            "编程任务优先 OpenAI，写作优先 Claude，日常任务优先 DeepSeek；如果提高成本先问我。",
            base_policy(),
        );
        assert!(draft.warnings.is_empty());
        assert!(draft.policy.ask_before_cost_escalation);
        assert_eq!(draft.policy.role_preferences.len(), 3);
        assert!(draft.policy.role_preferences.iter().any(|item| {
            item.role == ModelRole::Programming
                && item.preferred_providers == vec![ProviderType::OpenAi]
        }));
        assert!(draft.policy.role_preferences.iter().any(|item| {
            item.role == ModelRole::Writing
                && item.preferred_providers == vec![ProviderType::Anthropic]
        }));
        assert!(draft.policy.role_preferences.iter().any(|item| {
            item.role == ModelRole::FastCheap
                && item.preferred_providers == vec![ProviderType::DeepSeek]
        }));
    }

    #[test]
    fn unknown_text_does_not_silently_change_policy() {
        let original = base_policy();
        let draft = parse_routing_preference("Use something nice.", original.clone());
        assert_eq!(draft.policy, original);
        assert_eq!(draft.warnings.len(), 1);
    }
}
