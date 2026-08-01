use std::collections::{BTreeMap, BTreeSet, VecDeque};

use lunascope_core::{
    AllowedWorkerModel, DelegationDecision, DelegationKind, ModelSelection, OrchestrationId,
    OrchestrationPatch, OrchestrationPatchApplyMode, OrchestrationPatchOperation,
    OrchestrationPlan, OrchestrationValidation, RetryPolicy, UserOverrideRecord, WorkerBudget,
    WorkerCheckpointPolicy, WorkerField, WorkerId, WorkerInputContext, WorkerOutputSchema,
    WorkerPatch, WorkerSpec,
};
use thiserror::Error;
use uuid::Uuid;

pub const MAX_PARALLEL_WORKERS: u32 = 4;
pub const MAX_WORKER_DEPTH: u32 = 2;
pub const MAX_WORKERS: usize = 12;
const MAX_WORKER_TIMEOUT_MS: u64 = 30 * 60 * 1000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskAnalysis {
    pub independent_workstreams: u32,
    pub crosses_frontend_backend: bool,
    pub combines_research_and_implementation: bool,
    pub needs_independent_verification: bool,
    pub benefits_from_context_isolation: bool,
    pub appears_small_and_linear: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelWorkerDraft {
    pub role: String,
    pub task: String,
    pub prompt: String,
    pub expected_output: String,
    pub dependency_indices: Vec<usize>,
    pub tools: Vec<String>,
    pub write_scopes: Vec<String>,
    pub completion_criteria: Vec<String>,
    pub skills: Vec<String>,
}

pub fn analyze_objective(objective: &str) -> TaskAnalysis {
    let lowered = objective.to_lowercase();
    let has = |terms: &[&str]| terms.iter().any(|term| lowered.contains(term));
    let frontend = has(&["frontend", "front-end", "ui", "前端", "界面"]);
    let backend = has(&["backend", "back-end", "runtime", "api", "后端", "运行时"]);
    let research = has(&["research", "investigate", "compare", "研究", "调研", "比较"]);
    let implementation = has(&["implement", "build", "fix", "code", "实现", "构建", "修复"]);
    let verification = has(&[
        "verify", "review", "test", "audit", "验证", "审查", "测试", "审核",
    ]);
    let multi_module = has(&[
        "multiple modules",
        "multi-module",
        "cross-module",
        "多模块",
        "多个模块",
        "跨模块",
    ]);
    let documentation = has(&["documentation", "docs", "文档"]);
    let small = has(&[
        "single file",
        "one file",
        "small config",
        "short answer",
        "单文件",
        "小配置",
        "简短问答",
    ]);
    let mut streams = 1;
    streams += u32::from(frontend && backend);
    streams += u32::from(research && implementation);
    streams += u32::from(verification);
    streams += u32::from(multi_module);
    streams += u32::from(documentation && implementation);
    TaskAnalysis {
        independent_workstreams: streams.min(MAX_PARALLEL_WORKERS),
        crosses_frontend_backend: frontend && backend,
        combines_research_and_implementation: research && implementation,
        needs_independent_verification: verification,
        benefits_from_context_isolation: multi_module || research || documentation,
        appears_small_and_linear: small,
    }
}

pub fn decide_delegation(objective: &str, analysis: &TaskAnalysis) -> DelegationDecision {
    let useful_signals = [
        analysis.independent_workstreams >= 2,
        analysis.crosses_frontend_backend,
        analysis.combines_research_and_implementation,
        analysis.needs_independent_verification,
        analysis.benefits_from_context_isolation,
    ]
    .into_iter()
    .filter(|signal| *signal)
    .count();
    let multi =
        !objective.trim().is_empty() && !analysis.appears_small_and_linear && useful_signals >= 2;
    if multi {
        DelegationDecision {
            kind: DelegationKind::MultiAgent,
            rationale: format!(
                "{} independent workstream signal(s), with isolated context or verification value",
                analysis.independent_workstreams
            ),
            expected_benefit: "parallel or isolated work plus an independent verification boundary"
                .into(),
            estimated_duration: "medium".into(),
            estimated_cost: "medium".into(),
        }
    } else {
        DelegationDecision {
            kind: DelegationKind::SingleAgent,
            rationale: if analysis.appears_small_and_linear {
                "the task appears small and linear; delegation overhead would dominate".into()
            } else {
                "no clear independent workstreams or verification benefit were detected".into()
            },
            expected_benefit: "lower coordination cost and one continuous context".into(),
            estimated_duration: "short".into(),
            estimated_cost: "low".into(),
        }
    }
}

pub fn draft_orchestration(
    objective: &str,
    allowed_worker_models: Vec<AllowedWorkerModel>,
    parent_permissions: Vec<String>,
) -> Result<OrchestrationPlan, OrchestrationError> {
    if objective.trim().is_empty() {
        return Err(OrchestrationError::ObjectiveRequired);
    }
    let model = allowed_worker_models
        .first()
        .ok_or(OrchestrationError::WorkerModelRequired)?;
    let analysis = analyze_objective(objective);
    let decision = decide_delegation(objective, &analysis);
    let workers = if decision.kind == DelegationKind::MultiAgent {
        let researcher = worker(
            "researcher",
            objective,
            "Inspect relevant sources and isolate evidence, constraints, and risks.",
            "Produce a concise evidence artifact for downstream workers.",
            model,
            Vec::new(),
            (
                vec!["filesystem.read".into()],
                vec!["filesystem_read".into()],
            ),
        );
        let builder = if analysis.combines_research_and_implementation {
            worker(
                "builder",
                objective,
                "Implement the scoped change using the evidence handoff. Stay within declared ownership.",
                "Produce a reviewable patch artifact and implementation summary.",
                model,
                vec![researcher.worker_id.clone()],
                (
                    vec![
                        "filesystem.read".into(),
                        "filesystem.patch".into(),
                        "process.run".into(),
                    ],
                    vec![
                        "filesystem_read".into(),
                        "filesystem_write".into(),
                        "process_spawn".into(),
                    ],
                ),
            )
        } else {
            worker(
                "reviewer",
                objective,
                "Review the evidence handoff independently, challenge unsupported claims, and identify gaps.",
                "Produce an independent review artifact for the Verifier.",
                model,
                vec![researcher.worker_id.clone()],
                (
                    vec!["filesystem.read".into()],
                    vec!["filesystem_read".into()],
                ),
            )
        };
        let verifier = worker(
            "verifier",
            objective,
            "Review all upstream artifacts independently and verify completion criteria.",
            "Produce verification evidence, remaining risks, and an explicit verdict.",
            model,
            vec![builder.worker_id.clone()],
            (
                vec!["filesystem.read".into(), "process.run".into()],
                vec!["filesystem_read".into(), "process_spawn".into()],
            ),
        );
        vec![researcher, builder, verifier]
    } else {
        vec![worker(
            "builder",
            objective,
            "Complete the task directly and verify the result.",
            "Produce the requested result with verification evidence.",
            model,
            Vec::new(),
            (
                vec![
                    "filesystem.read".into(),
                    "filesystem.patch".into(),
                    "process.run".into(),
                ],
                vec![
                    "filesystem_read".into(),
                    "filesystem_write".into(),
                    "process_spawn".into(),
                ],
            ),
        )]
    };
    let plan = OrchestrationPlan {
        orchestration_id: OrchestrationId::new(format!("orc-{}", Uuid::new_v4())),
        version: 1,
        objective: objective.trim().to_owned(),
        project_id: None,
        thread_id: None,
        conversation_title: None,
        domain_pack: None,
        decision,
        maximum_parallel_workers: MAX_PARALLEL_WORKERS,
        maximum_worker_depth: MAX_WORKER_DEPTH,
        parent_permissions,
        allowed_worker_models,
        user_hard_constraints: Vec::new(),
        workers,
        user_overrides: Vec::new(),
    };
    ensure_valid(&plan)?;
    Ok(plan)
}

pub fn draft_orchestration_from_model(
    objective: &str,
    allowed_worker_models: Vec<AllowedWorkerModel>,
    parent_permissions: Vec<String>,
    mut decision: DelegationDecision,
    mut drafts: Vec<ModelWorkerDraft>,
) -> Result<OrchestrationPlan, OrchestrationError> {
    if objective.trim().is_empty() {
        return Err(OrchestrationError::ObjectiveRequired);
    }
    let model = allowed_worker_models
        .first()
        .ok_or(OrchestrationError::WorkerModelRequired)?;
    drafts.retain(|draft| !draft.role.trim().is_empty() && !draft.task.trim().is_empty());
    // The concurrency limit controls how many Workers may run at once, not how
    // many sequential stages an orchestration may contain. Truncating here used
    // to silently drop tail stages such as repair and independent verification.
    drafts.truncate(MAX_WORKERS);
    if drafts.is_empty() {
        drafts.push(ModelWorkerDraft {
            role: "builder".into(),
            task: "Complete the user request directly and verify the result.".into(),
            prompt: "Complete the user request in the assigned workspace. Inspect before editing, use the available tools, make the requested changes, and verify the result.".into(),
            expected_output: "The requested result with explicit verification evidence.".into(),
            dependency_indices: Vec::new(),
            tools: vec![
                "filesystem.read".into(),
                "filesystem.patch".into(),
                "process.run".into(),
            ],
            write_scopes: vec![".".into()],
            completion_criteria: vec!["the requested workspace change exists".into()],
            skills: Vec::new(),
        });
    }
    if decision.kind == DelegationKind::SingleAgent {
        drafts.truncate(1);
    } else if drafts.len() == 1 {
        drafts.push(ModelWorkerDraft {
            role: "verifier".into(),
            task: "Independently verify the first Worker's output against the user request.".into(),
            prompt: "Independently inspect the completed work and run the most relevant available checks. Report only evidence you actually observed.".into(),
            expected_output: "A typed verification verdict with evidence and remaining risks."
                .into(),
            dependency_indices: vec![0],
            tools: vec!["filesystem.read".into(), "process.run".into()],
            write_scopes: Vec::new(),
            completion_criteria: vec!["verification cites observed evidence".into()],
            skills: Vec::new(),
        });
    }
    if drafts.len() > 1 {
        order_repair_and_verification_stages_last(&mut drafts);
    }
    decision.kind = if drafts.len() == 1 {
        DelegationKind::SingleAgent
    } else {
        DelegationKind::MultiAgent
    };
    for later_index in 0..drafts.len() {
        if !model_worker_draft_is_writable(&drafts[later_index]) {
            continue;
        }
        let dependencies = (0..later_index)
            .filter(|earlier_index| {
                model_worker_draft_is_writable(&drafts[*earlier_index])
                    && draft_write_scopes_overlap(&drafts[*earlier_index], &drafts[later_index])
            })
            .collect::<Vec<_>>();
        for dependency in dependencies {
            if !drafts[later_index].dependency_indices.contains(&dependency) {
                drafts[later_index].dependency_indices.push(dependency);
            }
        }
    }
    for verifier_index in 0..drafts.len() {
        if !drafts[verifier_index].role.eq_ignore_ascii_case("verifier") {
            continue;
        }
        for dependency in 0..verifier_index {
            if model_worker_draft_is_writable(&drafts[dependency])
                && !drafts[verifier_index]
                    .dependency_indices
                    .contains(&dependency)
            {
                drafts[verifier_index].dependency_indices.push(dependency);
            }
        }
    }
    let worker_ids = (0..drafts.len())
        .map(|_| WorkerId::new(format!("worker-{}", Uuid::new_v4())))
        .collect::<Vec<_>>();
    let workers = drafts
        .iter()
        .enumerate()
        .map(|(index, draft)| {
            let dependencies = draft
                .dependency_indices
                .iter()
                .copied()
                .filter(|dependency| *dependency < index)
                .map(|dependency| worker_ids[dependency].clone())
                .collect::<Vec<_>>();
            let read_only = role_defaults_to_read_only(&draft.role);
            let capabilities = if read_only {
                (
                    vec!["filesystem.read".into()],
                    vec!["filesystem_read".into()],
                )
            } else {
                (
                    vec![
                        "filesystem.read".into(),
                        "filesystem.patch".into(),
                        "process.run".into(),
                    ],
                    vec![
                        "filesystem_read".into(),
                        "filesystem_write".into(),
                        "process_spawn".into(),
                    ],
                )
            };
            let mut spec = worker(
                &draft.role,
                objective,
                &draft.task,
                if draft.expected_output.trim().is_empty() {
                    "A structured artifact with explicit evidence."
                } else {
                    &draft.expected_output
                },
                model,
                dependencies,
                capabilities,
            );
            spec.worker_id = worker_ids[index].clone();
            if !draft.prompt.trim().is_empty() {
                spec.prompt = draft.prompt.trim().to_owned();
            }
            if !draft.tools.is_empty() {
                spec.tools = draft.tools.clone();
                let mut permissions = Vec::new();
                for tool in &spec.tools {
                    let permission = match tool.as_str() {
                        "filesystem.read" => Some("filesystem_read"),
                        "filesystem.patch" => Some("filesystem_write"),
                        "process.run" => Some("process_spawn"),
                        "network.search" => Some("network_connect"),
                        "dependency.install" => Some("network_connect"),
                        _ => None,
                    };
                    if let Some(permission) = permission
                        && parent_permissions
                            .iter()
                            .any(|allowed| allowed == permission)
                        && !permissions.iter().any(|existing| existing == permission)
                    {
                        permissions.push(permission.to_owned());
                    }
                }
                spec.permissions = permissions;
            }
            if spec.tools.iter().any(|tool| tool == "filesystem.patch") {
                spec.write_scopes = if draft.write_scopes.is_empty() {
                    vec![".".into()]
                } else {
                    draft.write_scopes.clone()
                };
            } else {
                spec.write_scopes.clear();
            }
            for criterion in &draft.completion_criteria {
                if !criterion.trim().is_empty() && !spec.completion_criteria.contains(criterion) {
                    spec.completion_criteria.push(criterion.clone());
                }
            }
            spec.skills = draft.skills.clone();
            spec.model.reason = format!(
                "selected from the Worker Model Pool by the Orchestration model for {}",
                draft.role
            );
            spec
        })
        .collect();
    let plan = OrchestrationPlan {
        orchestration_id: OrchestrationId::new(format!("orc-{}", Uuid::new_v4())),
        version: 1,
        objective: objective.trim().to_owned(),
        project_id: None,
        thread_id: None,
        conversation_title: None,
        domain_pack: None,
        decision,
        maximum_parallel_workers: MAX_PARALLEL_WORKERS,
        maximum_worker_depth: MAX_WORKER_DEPTH,
        parent_permissions,
        allowed_worker_models,
        user_hard_constraints: Vec::new(),
        workers,
        user_overrides: Vec::new(),
    };
    ensure_valid(&plan)?;
    Ok(plan)
}

fn worker(
    role: &str,
    objective: &str,
    task: &str,
    expected_output: &str,
    allowed_model: &AllowedWorkerModel,
    dependencies: Vec<WorkerId>,
    capabilities: (Vec<String>, Vec<String>),
) -> WorkerSpec {
    let (tools, permissions) = capabilities;
    let worker_id = WorkerId::new(format!("worker-{}", Uuid::new_v4()));
    WorkerSpec {
        worker_id,
        role: role.into(),
        tags: default_role_tags(role),
        objective: objective.into(),
        task: task.into(),
        prompt: format!(
            "Role: {role}\nObjective: {objective}\nTask: {task}\n\
             Return only the requested structured artifact. Respect every permission and hard constraint."
        ),
        input_context: WorkerInputContext {
            summary:
                "Use only the workspace snapshot and dependency artifacts supplied at runtime."
                    .into(),
            artifact_ids: Vec::new(),
            include_workspace_snapshot: true,
        },
        expected_output: expected_output.into(),
        output_schema: WorkerOutputSchema {
            media_type: "application/json".into(),
            schema: if role == "verifier" {
                serde_json::json!({
                    "type": "object",
                    "required": ["summary", "evidence", "verification"],
                    "properties": {
                        "summary": {"type": "string"},
                        "evidence": {"type": "array", "items": {"type": "string"}},
                        "verification": {
                            "type": "object",
                            "required": ["status", "summary", "evidence", "remainingRisks", "criterionResults", "findings"],
                            "properties": {
                                "status": {"enum": ["verified", "partially_verified", "unverified", "unable_to_verify", "failed_verification"]},
                                "summary": {"type": "string"},
                                "evidence": {"type": "array", "items": {"type": "string"}},
                                "remainingRisks": {"type": "array", "items": {"type": "string"}},
                                "criterionResults": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "required": ["criterionId", "status", "evidence", "note"],
                                        "properties": {
                                            "criterionId": {"type": "string"},
                                            "status": {"enum": ["passed", "failed", "unverified"]},
                                            "evidence": {"type": "array", "items": {"type": "string"}},
                                            "note": {"type": "string"}
                                        },
                                        "additionalProperties": false
                                    }
                                },
                                "findings": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "required": ["severity", "title", "description", "affectedPaths", "repairHint"],
                                        "properties": {
                                            "severity": {"enum": ["fatal", "major", "minor"]},
                                            "title": {"type": "string"},
                                            "description": {"type": "string"},
                                            "affectedPaths": {"type": "array", "items": {"type": "string"}},
                                            "repairHint": {"type": "string"}
                                        },
                                        "additionalProperties": false
                                    }
                                }
                            }
                        }
                    },
                    "additionalProperties": true
                })
            } else {
                serde_json::json!({
                    "type": "object",
                    "required": ["summary", "evidence"],
                    "properties": {
                        "summary": {"type": "string"},
                        "evidence": {"type": "array", "items": {"type": "string"}}
                    },
                    "additionalProperties": true
                })
            },
        },
        completion_criteria: vec![
            "output matches the declared schema".into(),
            "claims are supported by explicit evidence".into(),
        ],
        model: ModelSelection {
            provider: allowed_model.provider.clone(),
            model: allowed_model.model.clone(),
            reason: format!("selected from the allowed Worker Model Pool for role {role}"),
            fallback: false,
        },
        skills: Vec::new(),
        tools,
        permissions,
        budget: WorkerBudget {
            maximum_input_tokens: None,
            maximum_output_tokens: Some(4_096),
            maximum_cost_microusd: None,
        },
        dependencies,
        timeout_ms: 5 * 60 * 1000,
        retry_policy: RetryPolicy {
            maximum_attempts: 2,
            backoff_ms: 250,
            retryable_error_codes: vec!["timeout".into(), "provider_unavailable".into()],
        },
        checkpoint_policy: WorkerCheckpointPolicy {
            on_start: true,
            on_artifact: true,
            on_completion: true,
        },
        write_scopes: if role == "builder" {
            vec![".".into()]
        } else {
            Vec::new()
        },
        locked_fields: Vec::new(),
        parent_worker_id: None,
        depth: 0,
    }
}

fn default_role_tags(role: &str) -> Vec<String> {
    match role {
        "planner" => vec!["planning".into(), "coordination".into()],
        "builder" => vec!["implementation".into()],
        "frontend" => vec!["frontend".into(), "ui".into()],
        "backend" => vec!["backend".into(), "runtime".into()],
        "researcher" => vec!["research".into(), "evidence".into()],
        "reviewer" => vec!["review".into(), "read-only".into()],
        "verifier" => vec!["verification".into(), "independent".into()],
        "game_designer" => vec!["game-development".into(), "design".into()],
        "academic_writer" => vec!["academic".into(), "writing".into()],
        "documentation" => vec!["documentation".into(), "writing".into()],
        custom => vec![custom.replace('_', "-")],
    }
}

pub fn validate_orchestration(plan: &OrchestrationPlan) -> OrchestrationValidation {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    if plan.objective.trim().is_empty() {
        errors.push("objective is required".into());
    }
    if plan.version == 0 {
        errors.push("orchestration version must be positive".into());
    }
    if !(1..=MAX_PARALLEL_WORKERS).contains(&plan.maximum_parallel_workers) {
        errors.push(format!(
            "maximum parallel workers must be between 1 and {MAX_PARALLEL_WORKERS}"
        ));
    }
    if plan.maximum_worker_depth > MAX_WORKER_DEPTH {
        errors.push(format!(
            "maximum worker depth cannot exceed {MAX_WORKER_DEPTH}"
        ));
    }
    if plan.workers.is_empty() || plan.workers.len() > MAX_WORKERS {
        errors.push(format!("worker count must be between 1 and {MAX_WORKERS}"));
    }
    let worker_ids = plan
        .workers
        .iter()
        .map(|worker| worker.worker_id.clone())
        .collect::<BTreeSet<_>>();
    if worker_ids.len() != plan.workers.len() {
        errors.push("worker IDs must be unique".into());
    }
    let allowed_models = plan
        .allowed_worker_models
        .iter()
        .map(|model| (&model.provider, &model.model))
        .collect::<BTreeSet<_>>();
    let parent_permissions = plan.parent_permissions.iter().collect::<BTreeSet<_>>();
    let by_id = plan
        .workers
        .iter()
        .map(|worker| (&worker.worker_id, worker))
        .collect::<BTreeMap<_, _>>();
    for worker in &plan.workers {
        if worker.role.trim().is_empty()
            || worker.objective.trim().is_empty()
            || worker.task.trim().is_empty()
            || worker.prompt.trim().is_empty()
            || worker.expected_output.trim().is_empty()
            || worker.completion_criteria.is_empty()
        {
            errors.push(format!(
                "{} is missing a required role/task/prompt/output field",
                worker.worker_id
            ));
        }
        if worker.tags.len() > 8
            || worker.tags.iter().any(|tag| {
                tag.trim().is_empty()
                    || tag.chars().count() > 32
                    || tag.chars().any(char::is_control)
            })
        {
            errors.push(format!(
                "{} tags must contain at most eight non-empty labels of 32 characters or fewer",
                worker.worker_id
            ));
        }
        if worker.tags.iter().collect::<BTreeSet<_>>().len() != worker.tags.len() {
            warnings.push(format!("{} contains duplicate tags", worker.worker_id));
        }
        if !allowed_models.contains(&(&worker.model.provider, &worker.model.model)) {
            errors.push(format!(
                "{} selected model {}/{} outside the allowed Worker Model Pool",
                worker.worker_id, worker.model.provider, worker.model.model
            ));
        }
        if worker
            .permissions
            .iter()
            .any(|permission| !parent_permissions.contains(permission))
        {
            errors.push(format!(
                "{} requests a permission outside the parent capability set",
                worker.worker_id
            ));
        }
        if worker.tools.len() > 4 && !worker.role.eq_ignore_ascii_case("orchestrator") {
            errors.push(format!(
                "{} routes more than four ordinary tool classes",
                worker.worker_id
            ));
        }
        if worker.timeout_ms == 0 || worker.timeout_ms > MAX_WORKER_TIMEOUT_MS {
            errors.push(format!("{} has an invalid timeout", worker.worker_id));
        }
        if worker.retry_policy.maximum_attempts == 0 || worker.retry_policy.maximum_attempts > 3 {
            errors.push(format!(
                "{} maximum attempts must be between 1 and 3",
                worker.worker_id
            ));
        }
        if worker.depth > plan.maximum_worker_depth {
            errors.push(format!(
                "{} exceeds the maximum worker depth",
                worker.worker_id
            ));
        }
        match &worker.parent_worker_id {
            Some(parent_id) => match by_id.get(parent_id) {
                Some(parent) if parent.depth + 1 == worker.depth => {}
                Some(_) => errors.push(format!(
                    "{} depth is inconsistent with its parent",
                    worker.worker_id
                )),
                None => errors.push(format!(
                    "{} references unknown parent {}",
                    worker.worker_id, parent_id
                )),
            },
            None if worker.depth != 0 => errors.push(format!(
                "{} has non-zero depth without a parent",
                worker.worker_id
            )),
            None => {}
        }
        for dependency in &worker.dependencies {
            if dependency == &worker.worker_id {
                errors.push(format!("{} depends on itself", worker.worker_id));
            } else if !worker_ids.contains(dependency) {
                errors.push(format!(
                    "{} references unknown dependency {}",
                    worker.worker_id, dependency
                ));
            }
        }
        if worker.locked_fields.iter().collect::<BTreeSet<_>>().len() != worker.locked_fields.len()
        {
            warnings.push(format!(
                "{} contains duplicate locked fields",
                worker.worker_id
            ));
        }
    }
    let topological_order = topological_order(plan).unwrap_or_else(|| {
        errors.push("worker dependency graph contains a cycle".into());
        Vec::new()
    });
    validate_write_ownership(plan, &mut errors);
    OrchestrationValidation {
        valid: errors.is_empty(),
        errors,
        warnings,
        topological_order,
    }
}

fn topological_order(plan: &OrchestrationPlan) -> Option<Vec<WorkerId>> {
    let mut indegree = plan
        .workers
        .iter()
        .map(|worker| (worker.worker_id.clone(), worker.dependencies.len()))
        .collect::<BTreeMap<_, _>>();
    let mut dependents: BTreeMap<WorkerId, Vec<WorkerId>> = BTreeMap::new();
    for worker in &plan.workers {
        for dependency in &worker.dependencies {
            dependents
                .entry(dependency.clone())
                .or_default()
                .push(worker.worker_id.clone());
        }
    }
    let mut ready = indegree
        .iter()
        .filter_map(|(id, count)| (*count == 0).then_some(id.clone()))
        .collect::<BTreeSet<_>>();
    let mut ordered = Vec::with_capacity(plan.workers.len());
    while let Some(id) = ready.pop_first() {
        ordered.push(id.clone());
        if let Some(children) = dependents.get(&id) {
            for child in children {
                let count = indegree.get_mut(child)?;
                *count = count.checked_sub(1)?;
                if *count == 0 {
                    ready.insert(child.clone());
                }
            }
        }
    }
    (ordered.len() == plan.workers.len()).then_some(ordered)
}

fn validate_write_ownership(plan: &OrchestrationPlan, errors: &mut Vec<String>) {
    for (index, left) in plan.workers.iter().enumerate() {
        for right in plan.workers.iter().skip(index + 1) {
            let overlap = left.write_scopes.iter().any(|left_scope| {
                right
                    .write_scopes
                    .iter()
                    .any(|right_scope| scopes_overlap(left_scope, right_scope))
            });
            if overlap
                && !transitively_depends(plan, &left.worker_id, &right.worker_id)
                && !transitively_depends(plan, &right.worker_id, &left.worker_id)
            {
                errors.push(format!(
                    "{} and {} have overlapping write scopes without dependency ordering",
                    left.worker_id, right.worker_id
                ));
            }
        }
    }
}

fn scopes_overlap(left: &str, right: &str) -> bool {
    let normalize = |value: &str| value.trim().trim_matches('/').replace('\\', "/");
    let left = normalize(left);
    let right = normalize(right);
    left == "."
        || right == "."
        || (!left.is_empty()
            && !right.is_empty()
            && (left == right
                || left.starts_with(&(right.clone() + "/"))
                || right.starts_with(&(left + "/"))))
}

fn draft_write_scopes_overlap(left: &ModelWorkerDraft, right: &ModelWorkerDraft) -> bool {
    match (left.write_scopes.is_empty(), right.write_scopes.is_empty()) {
        (true, true) => true,
        (true, false) => right
            .write_scopes
            .iter()
            .any(|scope| scopes_overlap(".", scope)),
        (false, true) => left
            .write_scopes
            .iter()
            .any(|scope| scopes_overlap(scope, ".")),
        (false, false) => left.write_scopes.iter().any(|left_scope| {
            right
                .write_scopes
                .iter()
                .any(|right_scope| scopes_overlap(left_scope, right_scope))
        }),
    }
}

fn role_defaults_to_read_only(role: &str) -> bool {
    matches!(
        role.trim().to_ascii_lowercase().as_str(),
        "planner" | "researcher" | "reviewer" | "verifier" | "academic_writer"
    )
}

fn model_worker_draft_is_writable(draft: &ModelWorkerDraft) -> bool {
    if draft.tools.is_empty() {
        !role_defaults_to_read_only(&draft.role)
    } else {
        draft.tools.iter().any(|tool| tool == "filesystem.patch")
    }
}

fn order_repair_and_verification_stages_last(drafts: &mut Vec<ModelWorkerDraft>) {
    let mut ordinary = Vec::new();
    let mut repair = Vec::new();
    let mut verifier = Vec::new();
    for (original_index, draft) in std::mem::take(drafts).into_iter().enumerate() {
        if draft.role.eq_ignore_ascii_case("verifier") {
            verifier.push((original_index, draft));
        } else if draft.role.eq_ignore_ascii_case("reviewer")
            && model_worker_draft_is_writable(&draft)
        {
            repair.push((original_index, draft));
        } else {
            ordinary.push((original_index, draft));
        }
    }
    ordinary.append(&mut repair);
    ordinary.append(&mut verifier);

    let remapped_indices = ordinary
        .iter()
        .enumerate()
        .map(|(new_index, (original_index, _))| (*original_index, new_index))
        .collect::<BTreeMap<_, _>>();
    for (new_index, (_, mut draft)) in ordinary.into_iter().enumerate() {
        draft.dependency_indices = draft
            .dependency_indices
            .into_iter()
            .filter_map(|original_dependency| remapped_indices.get(&original_dependency).copied())
            .filter(|dependency| *dependency < new_index)
            .collect();
        drafts.push(draft);
    }
}

fn transitively_depends(plan: &OrchestrationPlan, worker: &WorkerId, target: &WorkerId) -> bool {
    let by_id = plan
        .workers
        .iter()
        .map(|spec| (&spec.worker_id, spec))
        .collect::<BTreeMap<_, _>>();
    let mut pending = VecDeque::from([worker]);
    let mut seen = BTreeSet::new();
    while let Some(current) = pending.pop_front() {
        if !seen.insert(current) {
            continue;
        }
        let Some(spec) = by_id.get(current) else {
            continue;
        };
        for dependency in &spec.dependencies {
            if dependency == target {
                return true;
            }
            pending.push_back(dependency);
        }
    }
    false
}

pub fn apply_user_patch(
    plan: &OrchestrationPlan,
    patch: &OrchestrationPatch,
) -> Result<OrchestrationPlan, OrchestrationError> {
    if patch.base_version != plan.version {
        return Err(OrchestrationError::StalePatch {
            expected: plan.version,
            actual: patch.base_version,
        });
    }
    if patch.operations.is_empty() {
        return Err(OrchestrationError::EmptyPatch);
    }
    if patch.apply_mode == OrchestrationPatchApplyMode::Cancel {
        return Err(OrchestrationError::PatchCancelled);
    }
    let mut next = plan.clone();
    if patch.apply_mode == OrchestrationPatchApplyMode::CloneRevision {
        next.orchestration_id = OrchestrationId::new(format!("orc-{}", Uuid::new_v4()));
        next.version = 1;
    } else {
        next.version = next.version.saturating_add(1);
    }
    for operation in &patch.operations {
        match operation {
            OrchestrationPatchOperation::AddWorker { spec } => next.workers.push(spec.clone()),
            OrchestrationPatchOperation::RemoveWorker { worker_id } => {
                let before = next.workers.len();
                next.workers.retain(|worker| &worker.worker_id != worker_id);
                if next.workers.len() == before {
                    return Err(OrchestrationError::WorkerNotFound(worker_id.clone()));
                }
            }
            OrchestrationPatchOperation::UpdateWorker {
                patch: worker_patch,
            } => apply_worker_patch(&mut next, worker_patch, patch)?,
        }
    }
    ensure_valid(&next)?;
    Ok(next)
}

fn apply_worker_patch(
    plan: &mut OrchestrationPlan,
    patch: &WorkerPatch,
    orchestration_patch: &OrchestrationPatch,
) -> Result<(), OrchestrationError> {
    let worker = plan
        .workers
        .iter_mut()
        .find(|worker| worker.worker_id == patch.worker_id)
        .ok_or_else(|| OrchestrationError::WorkerNotFound(patch.worker_id.clone()))?;
    let mut changed = Vec::new();
    macro_rules! replace {
        ($field:ident, $kind:expr) => {
            if let Some(value) = &patch.$field {
                worker.$field = value.clone();
                changed.push($kind);
            }
        };
    }
    replace!(role, WorkerField::Role);
    replace!(tags, WorkerField::Tags);
    replace!(objective, WorkerField::Objective);
    replace!(task, WorkerField::Task);
    replace!(prompt, WorkerField::Prompt);
    replace!(input_context, WorkerField::InputContext);
    replace!(expected_output, WorkerField::ExpectedOutput);
    replace!(output_schema, WorkerField::OutputSchema);
    replace!(completion_criteria, WorkerField::CompletionCriteria);
    replace!(model, WorkerField::Model);
    replace!(skills, WorkerField::Skills);
    replace!(tools, WorkerField::Tools);
    replace!(permissions, WorkerField::Permissions);
    replace!(budget, WorkerField::Budget);
    replace!(dependencies, WorkerField::Dependencies);
    replace!(timeout_ms, WorkerField::Timeout);
    replace!(retry_policy, WorkerField::RetryPolicy);
    replace!(checkpoint_policy, WorkerField::CheckpointPolicy);
    replace!(write_scopes, WorkerField::WriteScopes);
    if let Some(parent) = &patch.parent_worker_id {
        worker.parent_worker_id = parent.clone();
        changed.push(WorkerField::ParentWorker);
    }
    for field in &patch.unlock_fields {
        worker.locked_fields.retain(|locked| locked != field);
    }
    for field in &patch.lock_fields {
        if !worker.locked_fields.contains(field) {
            worker.locked_fields.push(*field);
        }
    }
    let override_fields = changed
        .into_iter()
        .chain(patch.lock_fields.iter().copied())
        .chain(patch.unlock_fields.iter().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if !override_fields.is_empty() {
        plan.user_overrides.push(UserOverrideRecord {
            patch_id: orchestration_patch.patch_id.clone(),
            worker_id: patch.worker_id.clone(),
            fields: override_fields,
            reason: orchestration_patch.reason.clone(),
        });
    }
    Ok(())
}

pub fn merge_orchestrator_revision(
    current: &OrchestrationPlan,
    proposed: &OrchestrationPlan,
) -> Result<OrchestrationPlan, OrchestrationError> {
    if current.orchestration_id != proposed.orchestration_id {
        return Err(OrchestrationError::WrongOrchestration);
    }
    let current_by_id = current
        .workers
        .iter()
        .map(|worker| (&worker.worker_id, worker))
        .collect::<BTreeMap<_, _>>();
    let mut merged = proposed.clone();
    merged.version = current.version.saturating_add(1);
    merged.user_overrides = current.user_overrides.clone();
    for worker in &mut merged.workers {
        let Some(existing) = current_by_id.get(&worker.worker_id) else {
            continue;
        };
        preserve_locked(existing, worker);
    }
    ensure_valid(&merged)?;
    Ok(merged)
}

fn preserve_locked(current: &WorkerSpec, proposed: &mut WorkerSpec) {
    for field in &current.locked_fields {
        match field {
            WorkerField::Role => proposed.role = current.role.clone(),
            WorkerField::Tags => proposed.tags = current.tags.clone(),
            WorkerField::Objective => proposed.objective = current.objective.clone(),
            WorkerField::Task => proposed.task = current.task.clone(),
            WorkerField::Prompt => proposed.prompt = current.prompt.clone(),
            WorkerField::InputContext => proposed.input_context = current.input_context.clone(),
            WorkerField::ExpectedOutput => {
                proposed.expected_output = current.expected_output.clone()
            }
            WorkerField::OutputSchema => proposed.output_schema = current.output_schema.clone(),
            WorkerField::CompletionCriteria => {
                proposed.completion_criteria = current.completion_criteria.clone();
            }
            WorkerField::Model => proposed.model = current.model.clone(),
            WorkerField::Skills => proposed.skills = current.skills.clone(),
            WorkerField::Tools => proposed.tools = current.tools.clone(),
            WorkerField::Permissions => proposed.permissions = current.permissions.clone(),
            WorkerField::Budget => proposed.budget = current.budget.clone(),
            WorkerField::Dependencies => proposed.dependencies = current.dependencies.clone(),
            WorkerField::Timeout => proposed.timeout_ms = current.timeout_ms,
            WorkerField::RetryPolicy => proposed.retry_policy = current.retry_policy.clone(),
            WorkerField::CheckpointPolicy => {
                proposed.checkpoint_policy = current.checkpoint_policy.clone();
            }
            WorkerField::WriteScopes => proposed.write_scopes = current.write_scopes.clone(),
            WorkerField::ParentWorker => {
                proposed.parent_worker_id = current.parent_worker_id.clone();
                proposed.depth = current.depth;
            }
        }
    }
    proposed.locked_fields = current.locked_fields.clone();
}

fn ensure_valid(plan: &OrchestrationPlan) -> Result<(), OrchestrationError> {
    let validation = validate_orchestration(plan);
    if validation.valid {
        Ok(())
    } else {
        Err(OrchestrationError::InvalidPlan(validation.errors))
    }
}

#[derive(Debug, Error)]
pub enum OrchestrationError {
    #[error("orchestration objective is required")]
    ObjectiveRequired,
    #[error("at least one allowed Worker model is required")]
    WorkerModelRequired,
    #[error("orchestration plan is invalid: {0:?}")]
    InvalidPlan(Vec<String>),
    #[error("patch contains no operations")]
    EmptyPatch,
    #[error("patch was cancelled")]
    PatchCancelled,
    #[error("stale orchestration patch: expected version {expected}, got {actual}")]
    StalePatch { expected: u32, actual: u32 },
    #[error("worker not found: {0}")]
    WorkerNotFound(WorkerId),
    #[error("proposed revision belongs to another orchestration")]
    WrongOrchestration,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> AllowedWorkerModel {
        AllowedWorkerModel {
            provider: "deepseek-primary".into(),
            model: "deepseek-v4-flash".into(),
        }
    }

    fn permissions() -> Vec<String> {
        vec![
            "filesystem_read".into(),
            "filesystem_write".into(),
            "process_spawn".into(),
        ]
    }

    #[test]
    fn small_linear_task_stays_single_but_cross_domain_review_is_multi() {
        let single = analyze_objective("Fix one file typo");
        assert_eq!(
            decide_delegation("Fix one file typo", &single).kind,
            DelegationKind::SingleAgent
        );
        let multi = analyze_objective(
            "Research and implement frontend and backend modules, then independently verify them",
        );
        let decision = decide_delegation("large task", &multi);
        assert_eq!(decision.kind, DelegationKind::MultiAgent);
        assert!(!decision.rationale.is_empty());
        assert!(!decision.expected_benefit.is_empty());
    }

    #[test]
    fn draft_has_complete_prompts_models_dependencies_and_permission_intersection() {
        let plan = draft_orchestration(
            "Research and implement multiple modules, then verify",
            vec![model()],
            permissions(),
        )
        .expect("draft");
        assert_eq!(plan.decision.kind, DelegationKind::MultiAgent);
        assert_eq!(plan.workers.len(), 3);
        assert!(plan.workers.iter().all(|worker| {
            !worker.prompt.is_empty()
                && !worker.output_schema.media_type.is_empty()
                && !worker.completion_criteria.is_empty()
                && !worker.model.reason.is_empty()
        }));
        assert!(validate_orchestration(&plan).valid);
    }

    #[test]
    fn rejects_cycles_parent_permission_escalation_and_parallel_write_overlap() {
        let mut plan = draft_orchestration(
            "Research and implement multiple modules, then verify",
            vec![model()],
            permissions(),
        )
        .expect("draft");
        let first = plan.workers[0].worker_id.clone();
        let last = plan.workers.last().expect("last").worker_id.clone();
        plan.workers[0].dependencies = vec![last];
        plan.workers[1].permissions.push("administrator".into());
        plan.workers[0].write_scopes = vec!["src".into()];
        plan.workers[2].write_scopes = vec!["src/lib".into()];
        let validation = validate_orchestration(&plan);
        assert!(!validation.valid);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("cycle"))
        );
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("parent capability"))
        );
        assert!(plan.workers.iter().any(|worker| worker.worker_id == first));
    }

    #[test]
    fn model_draft_orders_overlapping_writers_before_validation() {
        let decision = decide_delegation(
            "Implement frontend and backend modules, then verify them",
            &analyze_objective("Implement frontend and backend modules, then verify them"),
        );
        let write_draft = |role: &str| ModelWorkerDraft {
            role: role.into(),
            task: format!("Implement the {role} portion"),
            prompt: format!("Write and verify the {role} files"),
            expected_output: format!("Completed {role} files"),
            dependency_indices: Vec::new(),
            tools: vec!["filesystem.read".into(), "filesystem.patch".into()],
            write_scopes: vec![".".into()],
            completion_criteria: vec![format!("{role} files exist")],
            skills: Vec::new(),
        };
        let plan = draft_orchestration_from_model(
            "Implement frontend and backend modules, then verify them",
            vec![model()],
            permissions(),
            decision,
            vec![write_draft("frontend"), write_draft("backend")],
        )
        .expect("overlapping model writers should be serialized");

        assert_eq!(
            plan.workers[1].dependencies,
            vec![plan.workers[0].worker_id.clone()]
        );
        assert!(validate_orchestration(&plan).valid);
    }

    #[test]
    fn model_draft_orders_default_writers_with_empty_tool_lists() {
        let decision = decide_delegation(
            "Implement frontend and backend modules, then verify them",
            &analyze_objective("Implement frontend and backend modules, then verify them"),
        );
        let default_write_draft = |role: &str| ModelWorkerDraft {
            role: role.into(),
            task: format!("Implement the {role} portion"),
            prompt: format!("Write and verify the {role} files"),
            expected_output: format!("Completed {role} files"),
            dependency_indices: Vec::new(),
            tools: Vec::new(),
            write_scopes: Vec::new(),
            completion_criteria: vec![format!("{role} files exist")],
            skills: Vec::new(),
        };
        let plan = draft_orchestration_from_model(
            "Implement frontend and backend modules, then verify them",
            vec![model()],
            permissions(),
            decision,
            vec![
                default_write_draft("frontend"),
                default_write_draft("backend"),
                default_write_draft("documentation"),
            ],
        )
        .expect("default writable roles with overlapping scopes should be serialized");

        assert_eq!(
            plan.workers[1].dependencies,
            vec![plan.workers[0].worker_id.clone()]
        );
        assert_eq!(
            plan.workers[2].dependencies,
            vec![
                plan.workers[0].worker_id.clone(),
                plan.workers[1].worker_id.clone()
            ]
        );
        assert!(validate_orchestration(&plan).valid);
    }

    #[test]
    fn model_draft_keeps_sequential_repair_and_verifier_beyond_parallel_limit() {
        let decision = DelegationDecision {
            kind: DelegationKind::MultiAgent,
            rationale: "The project needs staged implementation and verification.".into(),
            expected_benefit: "Each stage has a bounded responsibility.".into(),
            estimated_duration: "long".into(),
            estimated_cost: "medium".into(),
        };
        let draft = |role: &str, dependency_indices: Vec<usize>| ModelWorkerDraft {
            role: role.into(),
            task: format!("Complete the {role} stage"),
            prompt: format!("Execute and verify the {role} stage"),
            expected_output: format!("Completed {role} stage"),
            dependency_indices,
            tools: vec!["filesystem.read".into()],
            write_scopes: Vec::new(),
            completion_criteria: vec![format!("{role} stage is complete")],
            skills: Vec::new(),
        };
        let plan = draft_orchestration_from_model(
            "Build a complete project, repair it, and independently verify it.",
            vec![model()],
            permissions(),
            decision,
            vec![
                draft("planner", vec![]),
                draft("frontend", vec![0]),
                draft("backend", vec![1]),
                draft("documentation", vec![2]),
                draft("reviewer", vec![3]),
                draft("verifier", vec![4]),
            ],
        )
        .expect("sequential stages beyond the parallel limit must be preserved");

        assert_eq!(plan.maximum_parallel_workers, MAX_PARALLEL_WORKERS);
        assert_eq!(plan.workers.len(), 6);
        assert_eq!(plan.workers[4].role, "reviewer");
        assert_eq!(plan.workers[5].role, "verifier");
        assert_eq!(
            plan.workers[5].dependencies,
            vec![plan.workers[4].worker_id.clone()]
        );
    }

    #[test]
    fn writable_reviewer_runs_after_all_other_writers_and_before_verifier() {
        let decision = DelegationDecision {
            kind: DelegationKind::MultiAgent,
            rationale: "The project needs implementation, tests, repair, and verification.".into(),
            expected_benefit: "Late repair sees every generated file.".into(),
            estimated_duration: "long".into(),
            estimated_cost: "medium".into(),
        };
        let writable = |role: &str, scope: &str| ModelWorkerDraft {
            role: role.into(),
            task: format!("Complete the {role} stage"),
            prompt: format!("Write and verify the {role} files"),
            expected_output: format!("Completed {role} stage"),
            dependency_indices: Vec::new(),
            tools: vec!["filesystem.read".into(), "filesystem.patch".into()],
            write_scopes: vec![scope.into()],
            completion_criteria: vec![format!("{role} files exist")],
            skills: Vec::new(),
        };
        let read_only = |role: &str| ModelWorkerDraft {
            role: role.into(),
            task: format!("Complete the {role} stage"),
            prompt: format!("Inspect the {role} stage"),
            expected_output: format!("Completed {role} stage"),
            dependency_indices: Vec::new(),
            tools: vec!["filesystem.read".into()],
            write_scopes: Vec::new(),
            completion_criteria: vec![format!("{role} stage is complete")],
            skills: Vec::new(),
        };
        let plan = draft_orchestration_from_model(
            "Build, test, repair, and verify a complete project.",
            vec![model()],
            permissions(),
            decision,
            vec![
                read_only("planner"),
                writable("frontend", "index.html"),
                writable("builder", "app.js"),
                writable("reviewer", "."),
                writable("documentation", "README.md"),
                writable("academic_writer", "tests"),
                read_only("verifier"),
            ],
        )
        .expect("late repair ordering should produce a valid plan");

        let roles = plan
            .workers
            .iter()
            .map(|worker| worker.role.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            roles,
            vec![
                "planner",
                "frontend",
                "builder",
                "documentation",
                "academic_writer",
                "reviewer",
                "verifier"
            ]
        );
        assert!(
            plan.workers[5]
                .dependencies
                .contains(&plan.workers[4].worker_id)
        );
        assert!(
            plan.workers[6]
                .dependencies
                .contains(&plan.workers[5].worker_id)
        );
        assert!(validate_orchestration(&plan).valid);
    }

    #[test]
    fn user_patch_is_versioned_and_locked_prompt_survives_orchestrator_revision() {
        let plan = draft_orchestration(
            "Research and implement multiple modules, then verify",
            vec![model()],
            permissions(),
        )
        .expect("draft");
        let worker_id = plan.workers[0].worker_id.clone();
        let patch = OrchestrationPatch {
            patch_id: "patch-1".into(),
            base_version: plan.version,
            apply_mode: OrchestrationPatchApplyMode::ApplyAfterCurrentStep,
            reason: "User requires a narrower prompt".into(),
            operations: vec![OrchestrationPatchOperation::UpdateWorker {
                patch: WorkerPatch {
                    worker_id: worker_id.clone(),
                    role: None,
                    tags: Some(vec!["user-owned".into(), "research".into()]),
                    objective: None,
                    task: None,
                    prompt: Some("USER LOCKED PROMPT".into()),
                    input_context: None,
                    expected_output: None,
                    output_schema: None,
                    completion_criteria: None,
                    model: None,
                    skills: None,
                    tools: None,
                    permissions: None,
                    budget: None,
                    dependencies: None,
                    timeout_ms: None,
                    retry_policy: None,
                    checkpoint_policy: None,
                    write_scopes: None,
                    parent_worker_id: None,
                    lock_fields: vec![WorkerField::Prompt, WorkerField::Tags],
                    unlock_fields: Vec::new(),
                },
            }],
        };
        let patched = apply_user_patch(&plan, &patch).expect("patch");
        assert_eq!(patched.version, 2);
        assert_eq!(patched.user_overrides.len(), 1);
        let mut proposed = patched.clone();
        proposed
            .workers
            .iter_mut()
            .find(|worker| worker.worker_id == worker_id)
            .expect("worker")
            .prompt = "orchestrator tried to replace it".into();
        let merged = merge_orchestrator_revision(&patched, &proposed).expect("merge");
        assert_eq!(
            merged
                .workers
                .iter()
                .find(|worker| worker.worker_id == worker_id)
                .expect("worker")
                .prompt,
            "USER LOCKED PROMPT"
        );
        assert_eq!(
            merged
                .workers
                .iter()
                .find(|worker| worker.worker_id == worker_id)
                .expect("worker")
                .tags,
            vec!["user-owned", "research"]
        );
    }
}
