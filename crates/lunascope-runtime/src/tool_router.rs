use std::collections::{BTreeMap, BTreeSet};

use lunascope_core::{
    PermissionRequest, PolicyDecision, RiskLevel, SkillSummary, ToolManifest, ToolRouteRejection,
    ToolRoutingDecision, ToolRoutingRequest,
};

use crate::PolicyEngine;

#[derive(Clone, Debug, Default)]
pub struct ToolRouter;

impl ToolRouter {
    pub fn route(
        &self,
        request: &ToolRoutingRequest,
        manifests: &[ToolManifest],
        skills: &[SkillSummary],
        policy: &PolicyEngine,
    ) -> ToolRoutingDecision {
        let max_tools = request.max_tools.clamp(1, 3) as usize;
        let manifests_by_id = manifests
            .iter()
            .map(|manifest| (manifest.id.as_str(), manifest))
            .collect::<BTreeMap<_, _>>();
        let skills_by_id = skills
            .iter()
            .map(|skill| (skill.catalog_id.as_str(), skill))
            .collect::<BTreeMap<_, _>>();
        let mut candidates = request.preferred_tool_ids.clone();
        let mut rejections = Vec::new();
        let mut skill_approval_requests = Vec::new();

        for skill_id in &request.required_skill_catalog_ids {
            if let Some(skill) = skills_by_id.get(skill_id.as_str()) {
                let mut outcomes = Vec::new();
                let mut asks = Vec::new();
                for permission in &skill.required_permissions {
                    let mut context = request.permission_context.clone();
                    context.tool_id = Some(format!("skill:{}", skill.catalog_id));
                    context.target_path = Some(skill.path.clone());
                    let permission_request = PermissionRequest {
                        permission: *permission,
                        context,
                        risk: RiskLevel::Medium,
                        action: format!("activate skill {} for {}", skill.name, request.role),
                    };
                    let outcome = policy.evaluate(&permission_request);
                    outcomes.push(outcome.clone());
                    if outcome.decision == PolicyDecision::Ask {
                        asks.push(permission_request);
                    }
                }
                if outcomes
                    .iter()
                    .any(|outcome| outcome.decision == PolicyDecision::Deny)
                {
                    rejections.push(ToolRouteRejection {
                        tool_id: format!("skill:{skill_id}"),
                        reason: "one or more skill-required permissions are denied".into(),
                        outcomes,
                    });
                    continue;
                }
                skill_approval_requests.extend(asks);
                candidates.extend(skill.required_tools.iter().cloned());
            } else {
                rejections.push(ToolRouteRejection {
                    tool_id: format!("skill:{skill_id}"),
                    reason: "required skill is not present in the discovered catalog".into(),
                    outcomes: Vec::new(),
                });
            }
        }
        candidates.extend(infer_tools(&request.role, &request.objective));

        let read_only = is_read_only_role(&request.role);
        let mut seen = BTreeSet::new();
        let mut selected_tools = Vec::new();
        let mut approval_requests = skill_approval_requests;
        let mut routed_tool_classes = 0usize;
        for tool_id in candidates {
            if !seen.insert(tool_id.clone()) || routed_tool_classes >= max_tools {
                continue;
            }
            let Some(manifest) = manifests_by_id.get(tool_id.as_str()) else {
                rejections.push(ToolRouteRejection {
                    tool_id,
                    reason: "tool is not registered".into(),
                    outcomes: Vec::new(),
                });
                continue;
            };
            routed_tool_classes += 1;
            if read_only && manifest_is_mutating(manifest) {
                rejections.push(ToolRouteRejection {
                    tool_id,
                    reason: "planner and reviewer roles are read-only by default".into(),
                    outcomes: Vec::new(),
                });
                continue;
            }

            let mut outcomes = Vec::new();
            let mut asks = Vec::new();
            for permission in &manifest.permissions {
                let mut context = request.permission_context.clone();
                context.tool_id = Some(manifest.id.clone());
                let permission_request = PermissionRequest {
                    permission: *permission,
                    context,
                    risk: manifest.risk,
                    action: format!("route tool {} for {}", manifest.id, request.role),
                };
                let outcome = policy.evaluate(&permission_request);
                outcomes.push(outcome.clone());
                if outcome.decision == PolicyDecision::Ask {
                    asks.push(permission_request);
                }
            }
            if outcomes
                .iter()
                .any(|outcome| outcome.decision == PolicyDecision::Deny)
            {
                rejections.push(ToolRouteRejection {
                    tool_id,
                    reason: "one or more required permissions are denied".into(),
                    outcomes,
                });
            } else if !asks.is_empty() {
                approval_requests.extend(asks);
            } else {
                selected_tools.push((*manifest).clone());
            }
        }

        ToolRoutingDecision {
            rationale: format!(
                "{} allowed tool(s), {} approval request(s), {} rejection(s); ordinary workers are capped at {} tool classes",
                selected_tools.len(),
                approval_requests.len(),
                rejections.len(),
                max_tools
            ),
            selected_tools,
            approval_requests,
            rejections,
        }
    }
}

fn infer_tools(role: &str, objective: &str) -> Vec<String> {
    let text = format!("{} {}", role.to_lowercase(), objective.to_lowercase());
    let mut tools = vec!["filesystem.read".to_owned()];
    if contains_any(
        &text,
        &[
            "build",
            "code",
            "implement",
            "fix",
            "refactor",
            "编程",
            "实现",
            "修复",
        ],
    ) {
        tools.extend(["filesystem.apply_text_patch".into(), "process.run".into()]);
    } else if contains_any(
        &text,
        &["review", "verify", "audit", "审核", "验证", "审计"],
    ) {
        tools.extend(["git.diff".into(), "process.run".into()]);
    } else if contains_any(&text, &["research", "search", "研究", "检索"]) {
        tools.extend(["mcp.search".into(), "browser.navigate".into()]);
    }
    tools
}

fn contains_any(text: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| text.contains(term))
}

fn is_read_only_role(role: &str) -> bool {
    matches!(
        role.trim().to_ascii_lowercase().as_str(),
        "planner" | "reviewer"
    )
}

fn manifest_is_mutating(manifest: &ToolManifest) -> bool {
    manifest.permissions.iter().any(|permission| {
        matches!(
            permission,
            lunascope_core::PermissionKind::FilesystemWrite
                | lunascope_core::PermissionKind::FilesystemDelete
                | lunascope_core::PermissionKind::GitCommit
                | lunascope_core::PermissionKind::GitPush
                | lunascope_core::PermissionKind::ShellExecute
                | lunascope_core::PermissionKind::ExtensionExecute
        )
    }) || matches!(manifest.risk, RiskLevel::High | RiskLevel::Critical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunascope_core::{
        PermissionContext, PermissionKind, PolicyRule, PolicyScope, SkillCompatibility,
        SkillSourceKind,
    };

    fn manifest(id: &str, permission: PermissionKind) -> ToolManifest {
        ToolManifest {
            id: id.into(),
            name: id.into(),
            description: id.into(),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: serde_json::json!({"type": "object"}),
            risk: RiskLevel::Low,
            permissions: vec![permission],
            timeout_ms: 30_000,
            cancellable: true,
            deterministic: false,
            source: "native".into(),
            version: "1".into(),
        }
    }

    fn request(role: &str, objective: &str) -> ToolRoutingRequest {
        ToolRoutingRequest {
            role: role.into(),
            objective: objective.into(),
            preferred_tool_ids: Vec::new(),
            required_skill_catalog_ids: Vec::new(),
            max_tools: 3,
            permission_context: PermissionContext {
                workspace_root: r"D:\work".into(),
                target_path: Some(r"D:\work\src\lib.rs".into()),
                ..PermissionContext::default()
            },
        }
    }

    fn manifests() -> Vec<ToolManifest> {
        vec![
            manifest("filesystem.read", PermissionKind::FilesystemRead),
            manifest(
                "filesystem.apply_text_patch",
                PermissionKind::FilesystemWrite,
            ),
            manifest("process.run", PermissionKind::ProcessSpawn),
            manifest("git.diff", PermissionKind::FilesystemRead),
            manifest("mcp.search", PermissionKind::McpInvoke),
            manifest("browser.navigate", PermissionKind::BrowserControl),
        ]
    }

    #[test]
    fn builder_gets_read_and_approval_bound_mutating_tools() {
        let decision = ToolRouter.route(
            &request("builder", "implement a code fix"),
            &manifests(),
            &[],
            &PolicyEngine::default(),
        );
        assert_eq!(decision.selected_tools[0].id, "filesystem.read");
        assert_eq!(decision.approval_requests.len(), 2);
        assert!(decision.selected_tools.len() <= 3);
    }

    #[test]
    fn planner_never_receives_mutating_tool_by_default() {
        let mut request = request("planner", "plan a code refactor");
        request.preferred_tool_ids = vec!["filesystem.apply_text_patch".into()];
        let decision = ToolRouter.route(&request, &manifests(), &[], &PolicyEngine::default());
        assert!(
            decision
                .rejections
                .iter()
                .any(|rejection| rejection.tool_id == "filesystem.apply_text_patch")
        );
    }

    #[test]
    fn skill_requirements_feed_routing_and_locked_deny_wins() {
        let skill = SkillSummary {
            catalog_id: "agents:research".into(),
            id: "research".into(),
            name: "research".into(),
            description: "Research".into(),
            tags: Vec::new(),
            required_tools: vec!["mcp.search".into()],
            required_permissions: vec![PermissionKind::McpInvoke],
            approximate_context_tokens: 10,
            source: SkillSourceKind::Agents,
            path: r"D:\work\.agents\skills\research\SKILL.md".into(),
            compatibility: SkillCompatibility::Native,
            warnings: Vec::new(),
            content_sha256: "hash".into(),
            user_invocable: true,
            model_invocable: true,
        };
        let policy = PolicyEngine::new(vec![PolicyRule {
            id: "deny-mcp".into(),
            permission: PermissionKind::McpInvoke,
            decision: PolicyDecision::Deny,
            scope: PolicyScope::Global,
            priority: 100,
            locked: true,
            reason: "offline project".into(),
        }]);
        let mut request = request("researcher", "research");
        request.required_skill_catalog_ids = vec![skill.catalog_id.clone()];
        let rejected_skill_id = format!("skill:{}", skill.catalog_id);
        let decision = ToolRouter.route(&request, &manifests(), &[skill], &policy);
        assert!(
            decision
                .rejections
                .iter()
                .any(|rejection| rejection.tool_id == rejected_skill_id)
        );
    }

    #[test]
    fn skill_required_permissions_are_approval_bound_before_tool_routing() {
        let mut skill = SkillSummary {
            catalog_id: "agents:remote".into(),
            id: "remote".into(),
            name: "remote".into(),
            description: "Remote research".into(),
            tags: Vec::new(),
            required_tools: vec!["filesystem.read".into()],
            required_permissions: vec![PermissionKind::NetworkConnect],
            approximate_context_tokens: 10,
            source: SkillSourceKind::Agents,
            path: r"D:\work\.agents\skills\remote\SKILL.md".into(),
            compatibility: SkillCompatibility::Native,
            warnings: Vec::new(),
            content_sha256: "hash".into(),
            user_invocable: true,
            model_invocable: true,
        };
        let mut request = request("researcher", "inspect files");
        request.required_skill_catalog_ids = vec![skill.catalog_id.clone()];
        let decision = ToolRouter.route(
            &request,
            &manifests(),
            std::slice::from_ref(&skill),
            &PolicyEngine::default(),
        );
        assert!(decision.approval_requests.iter().any(|request| {
            request.permission == PermissionKind::NetworkConnect
                && request.context.tool_id.as_deref() == Some("skill:agents:remote")
        }));

        skill.required_permissions = vec![PermissionKind::ExtensionExecute];
        let denied = ToolRouter.route(&request, &manifests(), &[skill], &PolicyEngine::default());
        assert!(
            denied
                .rejections
                .iter()
                .any(|rejection| rejection.tool_id == "skill:agents:remote")
        );
    }
}
