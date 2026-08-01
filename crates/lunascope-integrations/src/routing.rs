use std::collections::HashSet;

use lunascope_core::{
    ModelAssignment, ModelProfile, ModelRole, ModelRoutingDecision, ModelRoutingPolicy,
    ModelRoutingRequest, ModelSelectionSettings, ProviderConfig,
};
use thiserror::Error;

pub fn select_model(
    policy: &ModelRoutingPolicy,
    request: &ModelRoutingRequest,
    candidates: &[ModelProfile],
) -> Result<ModelRoutingDecision, RoutingError> {
    validate_routing_policy(policy)?;
    let required = policy
        .required_capabilities
        .iter()
        .chain(&request.required_capabilities)
        .copied()
        .collect::<Vec<_>>();
    let role_preference = policy
        .role_preferences
        .iter()
        .find(|preference| preference.role == request.role);
    let mut eligible = candidates
        .iter()
        .filter(|candidate| candidate.available)
        .filter(|candidate| candidate.roles.contains(&request.role))
        .filter(|candidate| {
            required
                .iter()
                .all(|capability| candidate.capabilities.contains(capability))
        })
        .filter(|candidate| {
            policy.allowed_provider_config_ids.is_empty()
                || policy
                    .allowed_provider_config_ids
                    .contains(&candidate.provider_config_id)
        })
        .filter(|candidate| {
            !policy
                .disallowed_provider_config_ids
                .contains(&candidate.provider_config_id)
        })
        .filter(|candidate| {
            request
                .locked_provider_config_id
                .as_ref()
                .is_none_or(|id| id == &candidate.provider_config_id)
        })
        .filter(|candidate| {
            request
                .locked_model_id
                .as_ref()
                .is_none_or(|id| id == &candidate.model_id)
        })
        .filter(|candidate| {
            role_preference.is_none_or(|preference| {
                if preference.preferred_providers.is_empty() {
                    true
                } else if preference.hard_constraint || !policy.fallback_allowed {
                    let allowed = if policy.fallback_allowed {
                        preference.preferred_providers.as_slice()
                    } else {
                        &preference.preferred_providers[..1]
                    };
                    allowed.contains(&candidate.provider_type)
                } else {
                    true
                }
            })
        })
        .map(|candidate| {
            let estimated_cost = estimate_cost(candidate, request);
            let score = score(candidate, policy, role_preference, estimated_cost);
            (candidate, estimated_cost, score)
        })
        .collect::<Vec<_>>();
    eligible.sort_by(|left, right| {
        right
            .2
            .cmp(&left.2)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.0.provider_config_id.cmp(&right.0.provider_config_id))
            .then_with(|| left.0.model_id.cmp(&right.0.model_id))
    });
    let (selected, estimated_cost, _) = eligible.first().ok_or(RoutingError::NoEligibleModel)?;
    let requires_cost_approval = policy
        .maximum_cost_microusd_per_run
        .is_some_and(|maximum| *estimated_cost > maximum)
        && policy.ask_before_cost_escalation;
    let fallback = role_preference
        .and_then(|preference| preference.preferred_providers.first())
        .is_some_and(|preferred| preferred != &selected.provider_type);
    Ok(ModelRoutingDecision {
        provider_config_id: selected.provider_config_id.clone(),
        model_id: selected.model_id.clone(),
        reason: format!(
            "selected for {:?}: quality {}, speed {}, privacy {}, estimated cost {} microusd",
            request.role,
            selected.quality_score,
            selected.speed_score,
            selected.privacy_score,
            estimated_cost
        ),
        fallback,
        estimated_cost_microusd: *estimated_cost,
        requires_cost_approval,
    })
}

fn score(
    candidate: &ModelProfile,
    policy: &ModelRoutingPolicy,
    role_preference: Option<&lunascope_core::RoleProviderPreference>,
    estimated_cost: u64,
) -> i128 {
    let priorities = &policy.priorities;
    let quality = i128::from(candidate.quality_score) * i128::from(priorities.quality);
    let speed = i128::from(candidate.speed_score) * i128::from(priorities.speed);
    let privacy = i128::from(candidate.privacy_score) * i128::from(priorities.privacy);
    let local_bonus = if policy.prefer_local && candidate.local {
        10_000
    } else {
        0
    };
    let cost_penalty =
        i128::from(priorities.cost) * i128::from(estimated_cost.min(1_000_000)) / 10_000;
    let preference_bonus = role_preference
        .and_then(|preference| {
            preference
                .preferred_providers
                .iter()
                .position(|provider| provider == &candidate.provider_type)
        })
        .map_or(0, |index| 20_000i128.saturating_sub(index as i128 * 1_000));
    quality + speed + privacy + local_bonus + preference_bonus - cost_penalty
}

fn estimate_cost(candidate: &ModelProfile, request: &ModelRoutingRequest) -> u64 {
    let input = candidate
        .input_cost_microusd_per_million_tokens
        .saturating_mul(request.estimated_input_tokens)
        / 1_000_000;
    let output = candidate
        .output_cost_microusd_per_million_tokens
        .saturating_mul(request.estimated_output_tokens)
        / 1_000_000;
    input.saturating_add(output)
}

pub fn validate_routing_policy(policy: &ModelRoutingPolicy) -> Result<(), RoutingError> {
    let values = [
        policy.priorities.quality,
        policy.priorities.cost,
        policy.priorities.speed,
        policy.priorities.privacy,
    ];
    if values.iter().any(|value| *value > 100) || values.iter().all(|value| *value == 0) {
        return Err(RoutingError::InvalidPriorities);
    }
    if policy.allowed_provider_config_ids.iter().any(|id| {
        policy
            .disallowed_provider_config_ids
            .iter()
            .any(|denied| denied == id)
    }) {
        return Err(RoutingError::ConflictingProviderLists);
    }
    Ok(())
}

pub fn validate_model_selection_settings(
    settings: &ModelSelectionSettings,
    providers: &[ProviderConfig],
) -> Result<(), RoutingError> {
    if settings.orchestration.role != ModelRole::Orchestration {
        return Err(RoutingError::InvalidOrchestrationRole);
    }
    validate_assignment(&settings.orchestration, providers)?;
    let mut roles = HashSet::new();
    for assignment in &settings.worker_pool {
        if assignment.role == ModelRole::Orchestration {
            return Err(RoutingError::OrchestrationInWorkerPool);
        }
        if !roles.insert(assignment.role) {
            return Err(RoutingError::DuplicateWorkerRole(assignment.role));
        }
        validate_assignment(assignment, providers)?;
    }
    Ok(())
}

fn validate_assignment(
    assignment: &ModelAssignment,
    providers: &[ProviderConfig],
) -> Result<(), RoutingError> {
    if assignment.model_id.trim().is_empty()
        || assignment.maximum_context_tokens == Some(0)
        || assignment.maximum_budget_microusd == Some(0)
    {
        return Err(RoutingError::InvalidModelAssignment(assignment.role));
    }
    if !providers
        .iter()
        .any(|provider| provider.id == assignment.provider_config_id && provider.enabled)
    {
        return Err(RoutingError::UnknownProviderConfig(
            assignment.provider_config_id.clone(),
        ));
    }
    match (
        assignment.fallback_provider_config_id.as_deref(),
        assignment.fallback_model_id.as_deref(),
    ) {
        (None, None) => {}
        (Some(provider_id), Some(model_id)) if !model_id.trim().is_empty() => {
            if !providers
                .iter()
                .any(|provider| provider.id == provider_id && provider.enabled)
            {
                return Err(RoutingError::UnknownProviderConfig(provider_id.into()));
            }
        }
        _ => return Err(RoutingError::IncompleteFallback(assignment.role)),
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum RoutingError {
    #[error("routing priorities must be 0..=100 and at least one must be non-zero")]
    InvalidPriorities,
    #[error("a provider config cannot be both allowed and disallowed")]
    ConflictingProviderLists,
    #[error("no model satisfies the routing hard constraints")]
    NoEligibleModel,
    #[error("the orchestration assignment must use the orchestration role")]
    InvalidOrchestrationRole,
    #[error("the worker pool must not contain the orchestration role")]
    OrchestrationInWorkerPool,
    #[error("worker pool contains duplicate role {0:?}")]
    DuplicateWorkerRole(ModelRole),
    #[error("model assignment for {0:?} has an empty model or zero limit")]
    InvalidModelAssignment(ModelRole),
    #[error("model assignment references unavailable provider config {0}")]
    UnknownProviderConfig(String),
    #[error("fallback for {0:?} must provide both provider config and model")]
    IncompleteFallback(ModelRole),
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunascope_core::{
        ModelCapability, ModelRole, ProviderProtocol, ProviderType, ReasoningEffort,
        RoleProviderPreference, RoutingPriorities,
    };

    fn profile(provider: &str, model: &str, quality: u8, cost: u64) -> ModelProfile {
        ModelProfile {
            provider_config_id: provider.into(),
            provider_type: match provider {
                "openai" => ProviderType::OpenAi,
                "deepseek" => ProviderType::DeepSeek,
                _ => ProviderType::Anthropic,
            },
            model_id: model.into(),
            display_name: model.into(),
            capabilities: vec![ModelCapability::Text, ModelCapability::Tools],
            roles: vec![ModelRole::Programming],
            available: true,
            local: false,
            quality_score: quality,
            speed_score: 50,
            privacy_score: 50,
            input_cost_microusd_per_million_tokens: cost,
            output_cost_microusd_per_million_tokens: cost,
        }
    }

    fn policy() -> ModelRoutingPolicy {
        ModelRoutingPolicy {
            priorities: RoutingPriorities {
                quality: 100,
                cost: 20,
                speed: 20,
                privacy: 20,
            },
            allowed_provider_config_ids: Vec::new(),
            disallowed_provider_config_ids: Vec::new(),
            maximum_cost_microusd_per_run: Some(100),
            prefer_local: false,
            required_capabilities: vec![ModelCapability::Tools],
            fallback_allowed: true,
            ask_before_cost_escalation: true,
            role_preferences: Vec::new(),
        }
    }

    fn provider_config(id: &str) -> ProviderConfig {
        ProviderConfig {
            id: id.into(),
            provider_type: ProviderType::OpenAi,
            protocol: ProviderProtocol::OpenAiResponses,
            display_name: id.into(),
            base_url: "https://api.openai.com".into(),
            credential_reference_id: format!("credential-{id}"),
            default_model_id: "gpt-test".into(),
            custom_headers: Vec::new(),
            context_window_tokens: Some(128_000),
            supports_tools: true,
            supports_vision: true,
            supports_structured_output: true,
            enabled: true,
        }
    }

    fn assignment(role: ModelRole) -> ModelAssignment {
        ModelAssignment {
            role,
            provider_config_id: "openai".into(),
            model_id: "gpt-test".into(),
            reasoning_effort: ReasoningEffort::Medium,
            maximum_context_tokens: Some(64_000),
            maximum_budget_microusd: Some(10_000),
            fallback_provider_config_id: None,
            fallback_model_id: None,
            locked: false,
        }
    }

    #[test]
    fn validates_orchestration_and_worker_model_separation() {
        let providers = vec![provider_config("openai")];
        let valid = ModelSelectionSettings {
            orchestration: assignment(ModelRole::Orchestration),
            worker_pool: vec![assignment(ModelRole::Programming)],
        };
        validate_model_selection_settings(&valid, &providers).expect("valid settings");

        let duplicate = ModelSelectionSettings {
            orchestration: assignment(ModelRole::Orchestration),
            worker_pool: vec![
                assignment(ModelRole::Programming),
                assignment(ModelRole::Programming),
            ],
        };
        assert!(matches!(
            validate_model_selection_settings(&duplicate, &providers),
            Err(RoutingError::DuplicateWorkerRole(ModelRole::Programming))
        ));

        let mut unknown = assignment(ModelRole::Writing);
        unknown.provider_config_id = "missing".into();
        assert!(matches!(
            validate_model_selection_settings(
                &ModelSelectionSettings {
                    orchestration: assignment(ModelRole::Orchestration),
                    worker_pool: vec![unknown],
                },
                &providers
            ),
            Err(RoutingError::UnknownProviderConfig(id)) if id == "missing"
        ));
    }

    #[test]
    fn routing_policy_rejects_invalid_weights_and_conflicting_provider_lists() {
        let mut invalid_weights = policy();
        invalid_weights.priorities = RoutingPriorities {
            quality: 0,
            cost: 0,
            speed: 0,
            privacy: 0,
        };
        assert!(matches!(
            validate_routing_policy(&invalid_weights),
            Err(RoutingError::InvalidPriorities)
        ));

        let mut conflicting = policy();
        conflicting.allowed_provider_config_ids = vec!["openai".into()];
        conflicting.disallowed_provider_config_ids = vec!["openai".into()];
        assert!(matches!(
            validate_routing_policy(&conflicting),
            Err(RoutingError::ConflictingProviderLists)
        ));
    }

    #[test]
    fn locked_provider_and_capabilities_are_hard_constraints() {
        let request = ModelRoutingRequest {
            role: ModelRole::Programming,
            required_capabilities: vec![ModelCapability::Vision],
            locked_provider_config_id: Some("openai".into()),
            locked_model_id: None,
            estimated_input_tokens: 1_000,
            estimated_output_tokens: 1_000,
        };
        let result = select_model(
            &policy(),
            &request,
            &[
                profile("openai", "coding", 90, 100),
                profile("anthropic", "writing", 99, 100),
            ],
        );
        assert!(matches!(result, Err(RoutingError::NoEligibleModel)));
    }

    #[test]
    fn selection_is_explainable_and_marks_cost_approval() {
        let request = ModelRoutingRequest {
            role: ModelRole::Programming,
            required_capabilities: Vec::new(),
            locked_provider_config_id: None,
            locked_model_id: None,
            estimated_input_tokens: 1_000_000,
            estimated_output_tokens: 1_000_000,
        };
        let decision = select_model(
            &policy(),
            &request,
            &[
                profile("openai", "strong", 95, 1_000),
                profile("deepseek", "cheap", 60, 10),
            ],
        )
        .expect("selection");
        assert_eq!(decision.model_id, "strong");
        assert!(decision.requires_cost_approval);
        assert!(decision.reason.contains("estimated cost"));
    }

    #[test]
    fn soft_preference_falls_back_but_hard_preference_does_not() {
        let request = ModelRoutingRequest {
            role: ModelRole::Programming,
            required_capabilities: Vec::new(),
            locked_provider_config_id: None,
            locked_model_id: None,
            estimated_input_tokens: 100,
            estimated_output_tokens: 100,
        };
        let mut policy = policy();
        policy.role_preferences = vec![RoleProviderPreference {
            role: ModelRole::Programming,
            preferred_providers: vec![ProviderType::OpenAi, ProviderType::DeepSeek],
            hard_constraint: false,
        }];
        let mut unavailable = profile("openai", "strong", 100, 10);
        unavailable.available = false;
        let decision = select_model(
            &policy,
            &request,
            &[unavailable, profile("deepseek", "fallback", 50, 10)],
        )
        .unwrap();
        assert_eq!(decision.model_id, "fallback");
        assert!(decision.fallback);

        policy.fallback_allowed = false;
        assert!(matches!(
            select_model(
                &policy,
                &request,
                &[profile("deepseek", "fallback", 50, 10)]
            ),
            Err(RoutingError::NoEligibleModel)
        ));
    }
}
