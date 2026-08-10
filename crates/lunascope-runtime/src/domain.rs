use std::path::Path;

use lunascope_core::{
    DomainDetection, DomainFlowEvidence, DomainPackDescriptor, DomainPackId, GameEngine,
    OrchestrationPlan, VerificationRecord, VerificationStatus,
};
use thiserror::Error;

use crate::validate_orchestration;

macro_rules! pack {
    (
        $id:expr,
        $label:expr,
        $core_roles:expr,
        $capabilities:expr,
        $workflow:expr,
        $rules:expr,
        $default_skills:expr,
        $default_tools:expr,
        $required_evidence:expr $(,)?
    ) => {
        DomainPackDescriptor {
            id: $id,
            label: $label.into(),
            core_roles: strings($core_roles),
            capabilities: strings($capabilities),
            workflow: strings($workflow),
            rules: strings($rules),
            default_skills: strings($default_skills),
            default_tools: strings($default_tools),
            required_evidence: strings($required_evidence),
        }
    };
}

pub fn domain_pack_catalog() -> Vec<DomainPackDescriptor> {
    vec![
        pack!(
            DomainPackId::Programming,
            "Programming",
            &["Planner", "Builder", "Reviewer", "Verifier"],
            &[
                "repository exploration",
                "architecture analysis",
                "bug diagnosis",
                "safe patching",
                "refactoring",
                "tests",
                "LSP or AST search",
                "Git diff",
                "verification",
            ],
            &[
                "Inspect",
                "Scope",
                "Plan",
                "Checkpoint",
                "Implement",
                "Test",
                "Review",
                "Evidence",
            ],
            &[
                "Use guarded patches and declared write ownership.",
                "Do not claim a test passed without captured output.",
            ],
            &[
                "repository-exploration",
                "architecture-analysis",
                "safe-patching",
            ],
            &["filesystem.read", "filesystem.patch", "process.run"],
            &["reviewed Git diff", "test output", "verification verdict"],
        ),
        pack!(
            DomainPackId::GameDevelopment,
            "Game Development",
            &["Game Designer", "Builder", "Reviewer", "Verifier"],
            &[
                "engine detection",
                "game mechanics",
                "systems",
                "level design",
                "narrative",
                "AI NPC",
                "performance",
                "build logs",
            ],
            &[
                "Detect engine",
                "Inspect assets",
                "Plan safe ownership",
                "Implement",
                "Build or cook with approval",
                "Review logs",
                "Evidence",
            ],
            &[
                "Never modify binary assets without an explicit safe handling plan.",
                "Scene, Prefab, and Blueprint changes require a reviewable representation.",
                "Build, cook, and editor launch require approval.",
                "Large assets belong under the configured LunaScope data root.",
            ],
            &["engine-detection", "game-systems", "build-log-analysis"],
            &["filesystem.read", "filesystem.patch", "process.run"],
            &[
                "detected engine",
                "asset safety decision",
                "build or static verification log",
            ],
        ),
        pack!(
            DomainPackId::Research,
            "Research",
            &["Researcher", "Reviewer", "Verifier"],
            &[
                "literature search",
                "source evaluation",
                "PDF",
                "dataset",
                "experiment design",
                "Python analysis",
                "statistics",
                "reproducibility",
                "citation validation",
            ],
            &[
                "Define question",
                "Record query and date",
                "Evaluate sources",
                "Design analysis",
                "Reproduce",
                "Validate citations",
                "Evidence",
            ],
            &[
                "Separate facts, inferences, and hypotheses.",
                "Never fabricate a citation.",
                "Verify DOI, author, and title before relying on a source.",
                "Record environment and random seed for experiments.",
                "Large datasets belong under the configured LunaScope data root.",
            ],
            &[
                "literature-search",
                "source-evaluation",
                "reproducible-analysis",
            ],
            &["filesystem.read", "process.run", "network.search"],
            &[
                "search query and retrieval date",
                "source ledger",
                "reproduction record",
            ],
        ),
        pack!(
            DomainPackId::AcademicWriting,
            "Academic Writing",
            &["Academic Writer", "Reviewer", "Verifier"],
            &[
                "rubric extraction",
                "evidence matrix",
                "outline",
                "draft",
                "revision",
                "APA 7",
                "DOCX",
                "LaTeX",
                "citation checking",
                "bilingual writing",
            ],
            &[
                "Extract rubric",
                "Build evidence matrix",
                "Outline",
                "Draft",
                "Review claims",
                "Render",
                "Verify citations",
            ],
            &[
                "Use clear Chinese explanations and university-level English.",
                "Do not invent sources.",
                "Mark unsupported claims as Citation needed.",
            ],
            &["rubric-extraction", "evidence-matrix", "citation-checking"],
            &["filesystem.read", "document.render", "network.search"],
            &[
                "rubric coverage matrix",
                "citation audit",
                "rendered document review",
            ],
        ),
        pack!(
            DomainPackId::FrontendDesign,
            "Frontend Design",
            &["Frontend Worker", "Reviewer", "Verifier"],
            &[
                "information architecture",
                "responsive design",
                "accessibility",
                "browser validation",
                "screenshot comparison",
                "visual regression",
                "performance",
                "anti-AI aesthetic audit",
            ],
            &[
                "Inspect behavior",
                "Fix interaction",
                "Implement responsive layout",
                "Browser test",
                "Compare screenshots",
                "Accessibility review",
                "Evidence",
            ],
            &[
                "Fix function and interaction before decoration.",
                "Test multiple viewport sizes in a real browser.",
                "Every clickable element must perform a real action.",
                "Avoid card stacking, excessive gradients, and gratuitous saturation.",
            ],
            &["responsive-design", "accessibility", "browser-validation"],
            &["filesystem.read", "filesystem.patch", "process.run"],
            &[
                "desktop screenshot",
                "mobile screenshot",
                "interaction and accessibility checks",
            ],
        ),
    ]
}

pub fn detect_domain(objective: &str, workspace: Option<&Path>) -> DomainDetection {
    let lower = objective.to_lowercase();
    let score = |terms: &[&str]| terms.iter().filter(|term| lower.contains(**term)).count();
    let candidates = [
        (
            DomainPackId::GameDevelopment,
            score(&[
                "unity",
                "unreal",
                "godot",
                "game",
                "prefab",
                "blueprint",
                "游戏",
                "关卡",
            ]),
        ),
        (
            DomainPackId::Research,
            score(&[
                "research",
                "literature",
                "dataset",
                "experiment",
                "statistics",
                "科研",
                "研究",
                "数据集",
            ]),
        ),
        (
            DomainPackId::AcademicWriting,
            score(&[
                "paper",
                "essay",
                "apa",
                "latex",
                "citation",
                "论文",
                "学术写作",
                "引用",
            ]),
        ),
        (
            DomainPackId::FrontendDesign,
            score(&[
                "frontend",
                "responsive",
                "accessibility",
                "browser",
                "ui",
                "前端",
                "响应式",
                "无障碍",
            ]),
        ),
        (
            DomainPackId::Programming,
            score(&[
                "code",
                "implement",
                "fix",
                "refactor",
                "repository",
                "编程",
                "代码",
                "修复",
                "重构",
            ]),
        ),
    ];
    let (selected, points) = candidates
        .into_iter()
        .max_by_key(|(_, points)| *points)
        .unwrap_or((DomainPackId::Programming, 0));
    let selected = if points == 0 {
        DomainPackId::Programming
    } else {
        selected
    };
    let game_engine = (selected == DomainPackId::GameDevelopment).then(|| {
        workspace
            .map(detect_game_engine)
            .unwrap_or(GameEngine::Unknown)
    });
    DomainDetection {
        selected,
        reason: if points == 0 {
            "No stronger domain signal was found; Programming is the conservative default.".into()
        } else {
            format!("{points} objective keyword signal(s) matched {selected:?}.")
        },
        game_engine,
    }
}

pub fn detect_game_engine(workspace: &Path) -> GameEngine {
    if workspace
        .join("ProjectSettings/ProjectVersion.txt")
        .is_file()
        && workspace.join("Assets").is_dir()
    {
        return GameEngine::Unity;
    }
    if workspace.join("project.godot").is_file() {
        return GameEngine::Godot;
    }
    if std::fs::read_dir(workspace).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|value| value == "uproject")
        })
    }) {
        return GameEngine::Unreal;
    }
    GameEngine::Unknown
}

pub fn apply_domain_pack(
    mut plan: OrchestrationPlan,
    detection: &DomainDetection,
) -> Result<OrchestrationPlan, DomainPackError> {
    let descriptor = domain_pack_catalog()
        .into_iter()
        .find(|pack| pack.id == detection.selected)
        .ok_or(DomainPackError::UnknownPack)?;
    plan.domain_pack = Some(detection.selected);
    for rule in &descriptor.rules {
        if !plan.user_hard_constraints.contains(rule) {
            plan.user_hard_constraints.push(rule.clone());
        }
    }
    let worker_count = plan.workers.len();
    for (index, worker) in plan.workers.iter_mut().enumerate() {
        let last = index + 1 == worker_count;
        let role = if worker.role.trim().is_empty() {
            domain_role(detection.selected, index, last)
        } else {
            worker.role.as_str()
        };
        worker.tags = domain_tags(detection.selected, role);
        // Domain packs constrain workflow and evidence, but Skill selection is
        // task-scoped. Preserve the Orchestration model's catalog selections;
        // the desktop runtime validates them and performs semantic fallback.
        if worker.tools.is_empty() {
            worker.tools = if matches!(
                role,
                "planner" | "reviewer" | "verifier" | "researcher" | "academic_writer"
            ) {
                descriptor
                    .default_tools
                    .iter()
                    .filter(|tool| !tool.contains("patch"))
                    .take(3)
                    .cloned()
                    .collect()
            } else {
                descriptor.default_tools.iter().take(3).cloned().collect()
            };
        }
        worker.permissions = tool_permissions(&worker.tools)
            .into_iter()
            .filter(|permission| plan.parent_permissions.contains(permission))
            .collect();
        if worker.tools.iter().all(|tool| !tool.contains("patch")) {
            worker.write_scopes.clear();
        }
        let engine = detection
            .game_engine
            .map(|engine| format!("\nDetected game engine: {engine:?}."))
            .unwrap_or_default();
        let operational_prompt = worker
            .prompt
            .replace("filesystem.patch", "write_file or replace_in_file")
            .replace("filesystem.read", "list_files, read_file, or search_text")
            .replace("process.run", "run_process");
        let runtime_tools = worker
            .tools
            .iter()
            .flat_map(|tool| match tool.as_str() {
                "filesystem.read" => vec!["list_files", "read_file", "search_text"],
                "filesystem.patch" => vec!["write_file", "replace_in_file"],
                "process.run" => vec!["run_process"],
                _ => Vec::new(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        worker.prompt = format!(
            "{}\n\nRuntime assignment:\nRole: {role}\nDomain Pack: {}\nObjective: {}\nTask: {}\nWorkflow: {}\nWritable scopes: {}\nCapability policy: {}\nAvailable runtime tool functions: {}.\nRules:\n- {}{}\nUse the assigned runtime tool functions to inspect and, when requested, change the real workspace. Do not merely describe changes. Finish with the declared structured output and explicit evidence.",
            operational_prompt,
            descriptor.label,
            worker.objective,
            worker.task,
            descriptor.workflow.join(" -> "),
            if worker.write_scopes.is_empty() {
                "(read-only)".into()
            } else {
                worker.write_scopes.join(", ")
            },
            worker.tools.join(", "),
            runtime_tools,
            descriptor.rules.join("\n- "),
            engine,
        );
        for evidence in &descriptor.required_evidence {
            let criterion = format!("produce or explicitly mark unavailable: {evidence}");
            if !worker.completion_criteria.contains(&criterion) {
                worker.completion_criteria.push(criterion);
            }
        }
    }
    for criterion in &plan.user_hard_constraints {
        if plan.workers.iter().any(|worker| {
            worker
                .owned_acceptance_criteria
                .iter()
                .any(|owned| owned == criterion)
        }) {
            continue;
        }
        let owner_index = plan
            .workers
            .iter()
            .position(|worker| {
                !worker.role.eq_ignore_ascii_case("verifier")
                    && worker
                        .tools
                        .iter()
                        .any(|tool| tool == "filesystem.patch" || tool == "process.run")
            })
            .or_else(|| {
                plan.workers
                    .iter()
                    .position(|worker| !worker.role.eq_ignore_ascii_case("verifier"))
            });
        if let Some(owner) = owner_index.and_then(|index| plan.workers.get_mut(index)) {
            owner.owned_acceptance_criteria.push(criterion.clone());
        }
    }
    if let Some(verifier) = plan
        .workers
        .iter_mut()
        .rfind(|worker| worker.role.eq_ignore_ascii_case("verifier"))
    {
        verifier.owned_acceptance_criteria = plan.user_hard_constraints.clone();
    }
    let validation = validate_orchestration(&plan);
    if validation.valid {
        Ok(plan)
    } else {
        Err(DomainPackError::InvalidPlan(validation.errors))
    }
}

pub fn verify_domain_evidence(evidence: &DomainFlowEvidence) -> VerificationRecord {
    let mut missing = Vec::new();
    let mut confirmed = Vec::new();
    match evidence {
        DomainFlowEvidence::Programming(item) => {
            require(
                item.repository_inspected,
                "repository inspection",
                &mut missing,
                &mut confirmed,
            );
            require(
                item.scope_recorded,
                "scope record",
                &mut missing,
                &mut confirmed,
            );
            require(
                item.checkpoint_created,
                "checkpoint",
                &mut missing,
                &mut confirmed,
            );
            require(
                item.patch_artifact_id.as_deref().is_some_and(nonempty),
                "patch artifact",
                &mut missing,
                &mut confirmed,
            );
            require(
                item.test_command.as_deref().is_some_and(nonempty)
                    && item.test_exit_code == Some(0),
                "successful test command",
                &mut missing,
                &mut confirmed,
            );
            require(
                item.diff_reviewed,
                "reviewed Git diff",
                &mut missing,
                &mut confirmed,
            );
        }
        DomainFlowEvidence::GameDevelopment(item) => {
            require(
                item.engine != GameEngine::Unknown,
                "detected game engine",
                &mut missing,
                &mut confirmed,
            );
            require(
                !item.binary_assets_modified || item.safe_asset_handling_confirmed,
                "safe binary asset handling",
                &mut missing,
                &mut confirmed,
            );
            require(
                !item.editor_or_build_invoked
                    || item
                        .editor_or_build_approval_id
                        .as_deref()
                        .is_some_and(nonempty),
                "approval for editor/build invocation",
                &mut missing,
                &mut confirmed,
            );
            require(
                item.verification_log_artifact_id
                    .as_deref()
                    .is_some_and(nonempty),
                "build or static verification log",
                &mut missing,
                &mut confirmed,
            );
            require(
                item.large_asset_root
                    .as_deref()
                    .is_none_or(is_absolute_data_root_evidence),
                "absolute configured large-asset root",
                &mut missing,
                &mut confirmed,
            );
        }
        DomainFlowEvidence::Research(item) => {
            require(
                item.facts_inferences_hypotheses_separated,
                "fact/inference/hypothesis separation",
                &mut missing,
                &mut confirmed,
            );
            require(
                !item.sources.is_empty(),
                "source ledger",
                &mut missing,
                &mut confirmed,
            );
            require(
                !item.sources.is_empty()
                    && item.sources.iter().all(|source| {
                        nonempty(&source.title)
                            && !source.authors.is_empty()
                            && source.authors.iter().all(|author| nonempty(author))
                            && nonempty(&source.query)
                            && valid_date(&source.retrieval_date)
                            && source.identifiers_verified
                            && source
                                .doi
                                .as_deref()
                                .is_none_or(|doi| doi.starts_with("10.") && doi.contains('/'))
                    }),
                "verified title/author/DOI/query/date metadata",
                &mut missing,
                &mut confirmed,
            );
            require(
                item.environment_artifact_id
                    .as_deref()
                    .is_some_and(nonempty),
                "experiment environment",
                &mut missing,
                &mut confirmed,
            );
            require(
                item.random_seed.is_some(),
                "random seed",
                &mut missing,
                &mut confirmed,
            );
            require(
                item.reproduction_artifact_id
                    .as_deref()
                    .is_some_and(nonempty),
                "reproduction artifact",
                &mut missing,
                &mut confirmed,
            );
        }
        DomainFlowEvidence::AcademicWriting(item) => {
            require(
                !item.rubric_criteria.is_empty()
                    && item
                        .rubric_criteria
                        .iter()
                        .all(|criterion| item.covered_rubric_criteria.contains(criterion)),
                "complete rubric coverage",
                &mut missing,
                &mut confirmed,
            );
            require(
                item.evidence_matrix_artifact_id
                    .as_deref()
                    .is_some_and(nonempty),
                "evidence matrix",
                &mut missing,
                &mut confirmed,
            );
            require(
                item.cited_sources_verified,
                "citation verification",
                &mut missing,
                &mut confirmed,
            );
            require(
                item.citation_needed_markers >= item.unsupported_claims,
                "Citation needed markers for unsupported claims",
                &mut missing,
                &mut confirmed,
            );
            require(
                item.rendered_artifact_id.as_deref().is_some_and(nonempty)
                    && item.rendered_layout_reviewed,
                "rendered layout review",
                &mut missing,
                &mut confirmed,
            );
        }
        DomainFlowEvidence::FrontendDesign(item) => {
            require(
                item.tested_viewport_widths
                    .iter()
                    .any(|width| *width <= 480)
                    && item
                        .tested_viewport_widths
                        .iter()
                        .any(|width| *width >= 1024),
                "real mobile and desktop viewport tests",
                &mut missing,
                &mut confirmed,
            );
            require(
                item.clickable_elements_tested,
                "click interaction checks",
                &mut missing,
                &mut confirmed,
            );
            require(
                item.accessibility_checked,
                "accessibility check",
                &mut missing,
                &mut confirmed,
            );
            require(
                item.screenshot_artifact_ids.len() >= 2
                    && item.screenshot_artifact_ids.iter().all(|id| nonempty(id)),
                "desktop and mobile screenshots",
                &mut missing,
                &mut confirmed,
            );
            require(
                item.performance_checked,
                "performance check",
                &mut missing,
                &mut confirmed,
            );
            require(
                item.anti_ai_aesthetic_reviewed,
                "anti-AI aesthetic review",
                &mut missing,
                &mut confirmed,
            );
        }
    }
    VerificationRecord {
        status: if missing.is_empty() {
            VerificationStatus::Verified
        } else {
            VerificationStatus::FailedVerification
        },
        summary: if missing.is_empty() {
            "Domain Pack evidence gate passed.".into()
        } else {
            format!("Domain Pack evidence gate failed: {}", missing.join(", "))
        },
        evidence: confirmed,
        remaining_risks: missing,
        criterion_results: Vec::new(),
        findings: Vec::new(),
    }
}

fn require(condition: bool, label: &str, missing: &mut Vec<String>, confirmed: &mut Vec<String>) {
    if condition {
        confirmed.push(label.into());
    } else {
        missing.push(label.into());
    }
}

fn nonempty(value: &str) -> bool {
    !value.trim().is_empty()
}

fn is_absolute_data_root_evidence(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() || value.split(['/', '\\']).any(|component| component == "..") {
        return false;
    }
    let bytes = value.as_bytes();
    Path::new(value).is_absolute()
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'))
        || value.starts_with(r"\\")
}

fn valid_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
    {
        return false;
    }
    let Ok(year) = value[0..4].parse::<u32>() else {
        return false;
    };
    let Ok(month) = value[5..7].parse::<u32>() else {
        return false;
    };
    let Ok(day) = value[8..10].parse::<u32>() else {
        return false;
    };
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let maximum_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year => 29,
        2 => 28,
        _ => return false,
    };
    (1..=maximum_day).contains(&day)
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).into()).collect()
}

fn domain_role(pack: DomainPackId, index: usize, last: bool) -> &'static str {
    if last && index > 0 {
        return "verifier";
    }
    match (pack, index) {
        (DomainPackId::Programming, 0) => "planner",
        (DomainPackId::Programming, _) => "builder",
        (DomainPackId::GameDevelopment, 0) => "game_designer",
        (DomainPackId::GameDevelopment, _) => "builder",
        (DomainPackId::Research, 0) => "researcher",
        (DomainPackId::Research, _) => "reviewer",
        (DomainPackId::AcademicWriting, 0) => "academic_writer",
        (DomainPackId::AcademicWriting, _) => "reviewer",
        (DomainPackId::FrontendDesign, 0) => "frontend",
        (DomainPackId::FrontendDesign, _) => "reviewer",
    }
}

fn domain_tags(pack: DomainPackId, role: &str) -> Vec<String> {
    vec![domain_pack_tag(pack).into(), role.replace('_', "-")]
}

fn domain_pack_tag(pack: DomainPackId) -> &'static str {
    match pack {
        DomainPackId::Programming => "programming",
        DomainPackId::GameDevelopment => "game-development",
        DomainPackId::Research => "research",
        DomainPackId::AcademicWriting => "academic-writing",
        DomainPackId::FrontendDesign => "frontend-design",
    }
}

fn tool_permissions(tools: &[String]) -> Vec<String> {
    let mut permissions = Vec::new();
    for tool in tools {
        let permission = if tool == "filesystem.read" || tool == "document.render" {
            "filesystem_read"
        } else if tool == "filesystem.patch" {
            "filesystem_write"
        } else if tool == "process.run" {
            "process_spawn"
        } else if tool == "network.search" {
            "network_connect"
        } else {
            continue;
        };
        if !permissions.iter().any(|existing| existing == permission) {
            permissions.push(permission.into());
        }
    }
    permissions
}

#[derive(Debug, Error)]
pub enum DomainPackError {
    #[error("domain pack is unavailable")]
    UnknownPack,
    #[error("domain pack produced an invalid Worker graph: {0:?}")]
    InvalidPlan(Vec<String>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft_orchestration;
    use lunascope_core::{
        AcademicWritingFlowEvidence, AllowedWorkerModel, DelegationKind, FrontendFlowEvidence,
        GameDevelopmentFlowEvidence, ProgrammingFlowEvidence, ResearchFlowEvidence,
        ResearchSourceEvidence,
    };

    fn plan(objective: &str) -> OrchestrationPlan {
        draft_orchestration(
            objective,
            vec![AllowedWorkerModel {
                provider: "fixture".into(),
                model: "fixture-model".into(),
            }],
            vec![
                "filesystem_read".into(),
                "filesystem_write".into(),
                "process_spawn".into(),
                "network_connect".into(),
            ],
        )
        .expect("plan")
    }

    #[test]
    fn catalog_contains_exactly_the_five_first_class_domain_packs() {
        let catalog = domain_pack_catalog();
        assert_eq!(catalog.len(), 5);
        for pack in catalog {
            assert!(!pack.workflow.is_empty());
            assert!(!pack.rules.is_empty());
            assert!(!pack.required_evidence.is_empty());
        }
    }

    #[test]
    fn each_pack_produces_a_valid_evidence_bound_graph() {
        let objectives = [
            (
                DomainPackId::Programming,
                "Research and implement multiple repository modules, then verify",
            ),
            (
                DomainPackId::GameDevelopment,
                "Research and implement multiple game systems, then verify",
            ),
            (
                DomainPackId::Research,
                "Research multiple literature sources and independently verify citations",
            ),
            (
                DomainPackId::AcademicWriting,
                "Research and draft multiple paper sections, then verify citations",
            ),
            (
                DomainPackId::FrontendDesign,
                "Research and implement frontend responsive UI, then verify in browser",
            ),
        ];
        for (id, objective) in objectives {
            let base = plan(objective);
            assert_eq!(base.decision.kind, DelegationKind::MultiAgent);
            let packed = apply_domain_pack(
                base,
                &DomainDetection {
                    selected: id,
                    reason: "fixture".into(),
                    game_engine: (id == DomainPackId::GameDevelopment)
                        .then_some(GameEngine::Unknown),
                },
            )
            .expect("pack");
            assert_eq!(packed.domain_pack, Some(id));
            assert!(validate_orchestration(&packed).valid);
            assert!(packed.workers.iter().all(|worker| !worker.tags.is_empty()));
            assert!(
                packed
                    .workers
                    .iter()
                    .any(|worker| worker.role == "verifier")
            );
        }
    }

    #[test]
    fn domain_pack_preserves_model_role_and_names_real_runtime_tools() {
        let mut base = plan("Create and verify hello.txt in the project workspace");
        base.workers[0].role = "builder".into();
        base.workers[0].prompt =
            "Use filesystem.read, filesystem.patch, and process.run against the workspace.".into();
        base.workers[0].tools = vec![
            "filesystem.read".into(),
            "filesystem.patch".into(),
            "process.run".into(),
        ];
        base.workers[0].write_scopes = vec![".".into()];

        let packed = apply_domain_pack(
            base,
            &DomainDetection {
                selected: DomainPackId::Programming,
                reason: "fixture".into(),
                game_engine: None,
            },
        )
        .expect("pack");
        let worker = &packed.workers[0];

        assert_eq!(worker.role, "builder");
        assert!(
            worker
                .prompt
                .contains("Use list_files, read_file, or search_text")
        );
        assert!(worker.prompt.contains("write_file or replace_in_file"));
        assert!(worker.prompt.contains("run_process"));
        assert!(worker.prompt.contains(
            "Available runtime tool functions: list_files, read_file, search_text, write_file, replace_in_file, run_process."
        ));
    }

    #[test]
    fn game_engine_detection_uses_project_markers_without_launching_an_editor() {
        let temp = tempfile::tempdir().expect("temp");
        std::fs::create_dir_all(temp.path().join("Assets")).expect("assets");
        std::fs::create_dir_all(temp.path().join("ProjectSettings")).expect("settings");
        std::fs::write(
            temp.path().join("ProjectSettings/ProjectVersion.txt"),
            "m_EditorVersion: 6000.0",
        )
        .expect("marker");
        assert_eq!(detect_game_engine(temp.path()), GameEngine::Unity);
    }

    #[test]
    fn every_domain_flow_has_a_strict_verified_evidence_fixture() {
        let fixtures = [
            DomainFlowEvidence::Programming(ProgrammingFlowEvidence {
                repository_inspected: true,
                scope_recorded: true,
                checkpoint_created: true,
                patch_artifact_id: Some("patch-1".into()),
                test_command: Some("cargo test".into()),
                test_exit_code: Some(0),
                diff_reviewed: true,
            }),
            DomainFlowEvidence::GameDevelopment(GameDevelopmentFlowEvidence {
                engine: GameEngine::Unity,
                binary_assets_modified: false,
                safe_asset_handling_confirmed: true,
                editor_or_build_invoked: true,
                editor_or_build_approval_id: Some("approval-1".into()),
                verification_log_artifact_id: Some("build-log-1".into()),
                large_asset_root: Some(
                    r"C:\Users\Example\AppData\Local\LunaScopeData\game-assets".into(),
                ),
            }),
            DomainFlowEvidence::Research(ResearchFlowEvidence {
                facts_inferences_hypotheses_separated: true,
                sources: vec![ResearchSourceEvidence {
                    title: "A verified paper".into(),
                    authors: vec!["A. Author".into()],
                    doi: Some("10.1000/example".into()),
                    identifiers_verified: true,
                    query: "verified research query".into(),
                    retrieval_date: "2026-07-27".into(),
                }],
                environment_artifact_id: Some("environment-1".into()),
                random_seed: Some(42),
                reproduction_artifact_id: Some("reproduction-1".into()),
            }),
            DomainFlowEvidence::AcademicWriting(AcademicWritingFlowEvidence {
                rubric_criteria: vec!["argument".into(), "evidence".into()],
                covered_rubric_criteria: vec!["argument".into(), "evidence".into()],
                evidence_matrix_artifact_id: Some("matrix-1".into()),
                cited_sources_verified: true,
                unsupported_claims: 1,
                citation_needed_markers: 1,
                rendered_artifact_id: Some("paper.pdf".into()),
                rendered_layout_reviewed: true,
            }),
            DomainFlowEvidence::FrontendDesign(FrontendFlowEvidence {
                tested_viewport_widths: vec![390, 1440],
                clickable_elements_tested: true,
                accessibility_checked: true,
                screenshot_artifact_ids: vec!["mobile.png".into(), "desktop.png".into()],
                performance_checked: true,
                anti_ai_aesthetic_reviewed: true,
            }),
        ];
        for fixture in fixtures {
            let verification = verify_domain_evidence(&fixture);
            assert_eq!(
                verification.status,
                VerificationStatus::Verified,
                "{}",
                verification.summary
            );
            assert!(
                verification.remaining_risks.is_empty(),
                "missing evidence: {:?}",
                verification.remaining_risks
            );
        }
    }

    #[test]
    fn large_asset_root_evidence_is_drive_agnostic_and_rejects_traversal() {
        assert!(is_absolute_data_root_evidence(
            r"C:\Users\Example\AppData\Local\LunaScopeData\game-assets"
        ));
        assert!(is_absolute_data_root_evidence(
            r"E:\Portable\LunaScopeData\game-assets"
        ));
        assert!(!is_absolute_data_root_evidence(
            r"E:\Portable\LunaScopeData\..\outside"
        ));
        assert!(!is_absolute_data_root_evidence("relative/game-assets"));
    }

    #[test]
    fn evidence_gate_fails_instead_of_inventing_missing_research_metadata() {
        let verification =
            verify_domain_evidence(&DomainFlowEvidence::Research(ResearchFlowEvidence {
                facts_inferences_hypotheses_separated: false,
                sources: Vec::new(),
                environment_artifact_id: None,
                random_seed: None,
                reproduction_artifact_id: None,
            }));
        assert_eq!(verification.status, VerificationStatus::FailedVerification);
        assert!(
            verification
                .remaining_risks
                .contains(&"source ledger".into())
        );
        assert!(verification.remaining_risks.contains(&"random seed".into()));
    }

    #[test]
    fn evidence_gate_rejects_an_impossible_retrieval_date() {
        let verification =
            verify_domain_evidence(&DomainFlowEvidence::Research(ResearchFlowEvidence {
                facts_inferences_hypotheses_separated: true,
                sources: vec![ResearchSourceEvidence {
                    title: "A paper".into(),
                    authors: vec!["A. Author".into()],
                    doi: Some("10.1000/example".into()),
                    identifiers_verified: true,
                    query: "research query".into(),
                    retrieval_date: "2026-02-30".into(),
                }],
                environment_artifact_id: Some("environment-1".into()),
                random_seed: Some(42),
                reproduction_artifact_id: Some("reproduction-1".into()),
            }));
        assert_eq!(verification.status, VerificationStatus::FailedVerification);
        assert!(
            verification
                .remaining_risks
                .contains(&"verified title/author/DOI/query/date metadata".into())
        );
    }
}
