use std::path::{Component, PathBuf};

use lunascope_core::{
    PermissionKind, PermissionRequest, PolicyDecision, PolicyOutcome, PolicyRule, PolicyScope,
};

#[derive(Clone, Debug, Default)]
pub struct PolicyEngine {
    rules: Vec<PolicyRule>,
}

impl PolicyEngine {
    pub fn new(mut rules: Vec<PolicyRule>) -> Self {
        rules.sort_by(|left, right| {
            right
                .locked
                .cmp(&left.locked)
                .then_with(|| right.priority.cmp(&left.priority))
                .then_with(|| decision_rank(right.decision).cmp(&decision_rank(left.decision)))
                .then_with(|| left.id.cmp(&right.id))
        });
        Self { rules }
    }

    pub fn evaluate(&self, request: &PermissionRequest) -> PolicyOutcome {
        if let Some(reason) = hard_deny_reason(request) {
            return PolicyOutcome {
                decision: PolicyDecision::Deny,
                matched_rule_id: None,
                reason,
                hard_constraint: true,
            };
        }

        if let Some(rule) = self.rules.iter().find(|rule| {
            rule.locked
                && rule.decision == PolicyDecision::Deny
                && rule.permission == request.permission
                && scope_matches(&rule.scope, request)
        }) {
            return PolicyOutcome {
                decision: PolicyDecision::Deny,
                matched_rule_id: Some(rule.id.clone()),
                reason: rule.reason.clone(),
                hard_constraint: true,
            };
        }

        if let Some(rule) = self.rules.iter().find(|rule| {
            rule.permission == request.permission && scope_matches(&rule.scope, request)
        }) {
            return PolicyOutcome {
                decision: rule.decision,
                matched_rule_id: Some(rule.id.clone()),
                reason: rule.reason.clone(),
                hard_constraint: rule.locked,
            };
        }

        default_outcome(request)
    }
}

/*
 * Hard constraints are evaluated before configured rules. Locked deny rules
 * are then evaluated as a separate tier, so a high-priority allow cannot
 * weaken an administrator or parent-scope prohibition.
 */

fn hard_deny_reason(request: &PermissionRequest) -> Option<String> {
    if request.permission == PermissionKind::ExtensionExecute {
        return Some("imported extension code cannot execute before audit and approval".into());
    }

    if request.permission == PermissionKind::FilesystemRead
        && request
            .context
            .target_path
            .as_deref()
            .is_some_and(is_sensitive_path)
    {
        return Some("credential and environment files are never readable by default".into());
    }

    if request.permission == PermissionKind::GitPush
        && request
            .context
            .arguments
            .iter()
            .any(|argument| argument.eq_ignore_ascii_case("--force") || argument == "-f")
    {
        return Some("force push is prohibited".into());
    }

    None
}

fn default_outcome(request: &PermissionRequest) -> PolicyOutcome {
    let (decision, reason) = match request.permission {
        PermissionKind::FilesystemRead if target_is_inside_workspace(request) => (
            PolicyDecision::Allow,
            "read is inside the approved workspace",
        ),
        PermissionKind::SkillLoad if target_is_inside_workspace(request) => (
            PolicyDecision::Allow,
            "skill instructions are inside the approved workspace",
        ),
        PermissionKind::FilesystemRead => (
            PolicyDecision::Ask,
            "read target is outside or cannot be proven inside the workspace",
        ),
        PermissionKind::SkillLoad => (
            PolicyDecision::Ask,
            "skill instructions are outside the workspace and require approval",
        ),
        PermissionKind::FilesystemWrite
        | PermissionKind::FilesystemDelete
        | PermissionKind::ShellExecute
        | PermissionKind::ProcessSpawn
        | PermissionKind::NetworkConnect
        | PermissionKind::BrowserControl
        | PermissionKind::SecretsUse
        | PermissionKind::GitCommit
        | PermissionKind::GitPush
        | PermissionKind::McpInvoke
        | PermissionKind::WorkerSpawn => (
            PolicyDecision::Ask,
            "the requested capability requires explicit approval at this scope",
        ),
        PermissionKind::ExtensionExecute => unreachable!("handled by hard deny"),
    };
    PolicyOutcome {
        decision,
        matched_rule_id: None,
        reason: reason.into(),
        hard_constraint: false,
    }
}

fn scope_matches(scope: &PolicyScope, request: &PermissionRequest) -> bool {
    match scope {
        PolicyScope::Global => true,
        PolicyScope::Workspace(root) => same_path(root, &request.context.workspace_root),
        PolicyScope::Project(id) => request.context.project_id.as_deref() == Some(id),
        PolicyScope::Run(id) => request.context.run_id.as_ref() == Some(id),
        PolicyScope::Worker(id) => request.context.worker_id.as_ref() == Some(id),
        PolicyScope::Tool(id) => request.context.tool_id.as_deref() == Some(id),
        PolicyScope::Path(root) => request
            .context
            .target_path
            .as_deref()
            .is_some_and(|target| is_path_within(target, root)),
        PolicyScope::Command(program) => request
            .context
            .program
            .as_deref()
            .is_some_and(|actual| actual.eq_ignore_ascii_case(program)),
        PolicyScope::NetworkDomain(domain) => request
            .context
            .network_domain
            .as_deref()
            .is_some_and(|actual| domain_matches(actual, domain)),
    }
}

fn target_is_inside_workspace(request: &PermissionRequest) -> bool {
    request
        .context
        .target_path
        .as_deref()
        .is_some_and(|target| is_path_within(target, &request.context.workspace_root))
}

fn is_path_within(target: &str, root: &str) -> bool {
    if PathBuf::from(target)
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return false;
    }
    let target = normalize_path(target);
    let root = normalize_path(root);
    target == root || target.starts_with(&(root + "\\"))
}

fn same_path(left: &str, right: &str) -> bool {
    normalize_path(left) == normalize_path(right)
}

fn normalize_path(value: &str) -> String {
    value
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn domain_matches(actual: &str, configured: &str) -> bool {
    let actual = actual.trim_end_matches('.').to_lowercase();
    let configured = configured.trim_end_matches('.').to_lowercase();
    actual == configured || actual.ends_with(&format!(".{configured}"))
}

fn is_sensitive_path(value: &str) -> bool {
    let path = PathBuf::from(value);
    path.components().any(|component| {
        let component = component.as_os_str().to_string_lossy().to_lowercase();
        matches!(
            component.as_str(),
            ".env"
                | ".ssh"
                | ".aws"
                | ".azure"
                | ".gnupg"
                | "credentials"
                | "id_rsa"
                | "id_ed25519"
        ) || component.starts_with(".env.")
    })
}

fn decision_rank(decision: PolicyDecision) -> u8 {
    match decision {
        PolicyDecision::Deny => 2,
        PolicyDecision::Ask => 1,
        PolicyDecision::Allow => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunascope_core::{PermissionContext, RiskLevel};

    fn request(permission: PermissionKind, target_path: Option<&str>) -> PermissionRequest {
        PermissionRequest {
            permission,
            context: PermissionContext {
                workspace_root: r"D:\work".into(),
                target_path: target_path.map(str::to_owned),
                ..PermissionContext::default()
            },
            risk: RiskLevel::Low,
            action: "test".into(),
        }
    }

    #[test]
    fn allows_workspace_read_but_asks_for_outside_read() {
        let engine = PolicyEngine::default();
        assert_eq!(
            engine
                .evaluate(&request(
                    PermissionKind::FilesystemRead,
                    Some(r"D:\work\src\lib.rs")
                ))
                .decision,
            PolicyDecision::Allow
        );
        assert_eq!(
            engine
                .evaluate(&request(
                    PermissionKind::FilesystemRead,
                    Some(r"D:\other\secret.txt")
                ))
                .decision,
            PolicyDecision::Ask
        );
    }

    #[test]
    fn hard_denies_env_even_when_rule_allows() {
        let engine = PolicyEngine::new(vec![PolicyRule {
            id: "allow-all-reads".into(),
            permission: PermissionKind::FilesystemRead,
            decision: PolicyDecision::Allow,
            scope: PolicyScope::Global,
            priority: 100,
            locked: false,
            reason: "test override".into(),
        }]);
        let outcome = engine.evaluate(&request(
            PermissionKind::FilesystemRead,
            Some(r"D:\work\.env"),
        ));
        assert_eq!(outcome.decision, PolicyDecision::Deny);
        assert!(outcome.hard_constraint);
    }

    #[test]
    fn locked_high_priority_rule_wins_deterministically() {
        let engine = PolicyEngine::new(vec![
            PolicyRule {
                id: "allow".into(),
                permission: PermissionKind::FilesystemWrite,
                decision: PolicyDecision::Allow,
                scope: PolicyScope::Workspace(r"D:\work".into()),
                priority: 10,
                locked: false,
                reason: "allow".into(),
            },
            PolicyRule {
                id: "deny".into(),
                permission: PermissionKind::FilesystemWrite,
                decision: PolicyDecision::Deny,
                scope: PolicyScope::Workspace(r"D:\work".into()),
                priority: 10,
                locked: true,
                reason: "locked deny".into(),
            },
        ]);
        let outcome = engine.evaluate(&request(
            PermissionKind::FilesystemWrite,
            Some(r"D:\work\src\lib.rs"),
        ));
        assert_eq!(outcome.decision, PolicyDecision::Deny);
        assert_eq!(outcome.matched_rule_id.as_deref(), Some("deny"));
    }

    #[test]
    fn locked_deny_beats_higher_priority_allow_and_dotdot_is_not_inside() {
        let engine = PolicyEngine::new(vec![
            PolicyRule {
                id: "allow".into(),
                permission: PermissionKind::FilesystemWrite,
                decision: PolicyDecision::Allow,
                scope: PolicyScope::Global,
                priority: 1_000,
                locked: false,
                reason: "ordinary allow".into(),
            },
            PolicyRule {
                id: "deny".into(),
                permission: PermissionKind::FilesystemWrite,
                decision: PolicyDecision::Deny,
                scope: PolicyScope::Workspace(r"D:\work".into()),
                priority: 1,
                locked: true,
                reason: "parent constraint".into(),
            },
        ]);
        assert_eq!(
            engine
                .evaluate(&request(
                    PermissionKind::FilesystemWrite,
                    Some(r"D:\work\src\lib.rs")
                ))
                .decision,
            PolicyDecision::Deny
        );
        assert_eq!(
            PolicyEngine::default()
                .evaluate(&request(
                    PermissionKind::FilesystemRead,
                    Some(r"D:\work\..\outside.txt")
                ))
                .decision,
            PolicyDecision::Ask
        );
    }
}
