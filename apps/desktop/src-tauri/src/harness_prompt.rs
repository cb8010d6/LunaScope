use lunascope_core::WorkerSpec;

const BASE: &str = include_str!("../../../../prompts/LUNASCOPE_SYSTEM.md");
const CODING: &str = include_str!("../../../../prompts/HARNESS_CODING.md");
const FRONTEND: &str = include_str!("../../../../prompts/HARNESS_FRONTEND.md");
const VERIFIER: &str = include_str!("../../../../prompts/HARNESS_VERIFIER.md");

pub(crate) fn base_system_prompt() -> &'static str {
    BASE.trim()
}

pub(crate) struct WorkerPromptInput<'a> {
    pub spec: &'a WorkerSpec,
    pub reply_language: &'a str,
    pub selected_skills: &'a str,
    pub environment: &'a str,
    pub self_management_access: bool,
}

pub(crate) fn build_worker_instructions(input: WorkerPromptInput<'_>) -> String {
    let mut sections = vec![BASE.trim(), CODING.trim()];
    if is_frontend_or_graphics(input.spec) {
        sections.push(FRONTEND.trim());
    }
    if input.spec.role.eq_ignore_ascii_case("verifier") {
        sections.push(VERIFIER.trim());
    }
    let self_management = if input.self_management_access {
        "Full Access is active for this conversation. When the user explicitly asks to change LunaScope itself, inspect current settings first and use only dedicated validated LunaScope tools. Never expose credentials or edit the database directly."
    } else {
        "LunaScope self-management tools are unavailable in this access mode. Do not claim that settings or installed Skills were changed."
    };
    format!(
        "{}\n\n# Runtime capability snapshot\n{}\n\n# Worker execution contract\n\
         You are an executing LunaScope Worker operating inside an isolated, write-through copy of the user's selected workspace. \
         Follow the Orchestrator assignment and the complete original objective. For work with more than one meaningful step, call update_plan before the first mutation and keep the checklist synchronized with evidence. \
         Every response that proposes an executable tool action MUST first call report_progress in that same response, unless the Provider has already emitted a public reasoning summary for that response. Use report_progress again after an important discovery or when tool evidence changes the approach; name the concrete observation, decision, and next observable action. \
         Tool paths and cwd values are workspace-relative. Use tools against the real workspace, inspect the resulting state, repair failed checks, and never claim an operation you did not observe. \
         When complete, return one JSON object with no Markdown fence matching this schema: {}\n\n{}\n\n{}\n\n{}",
        sections.join("\n\n"),
        input.environment,
        input.spec.output_schema.schema,
        self_management,
        input.reply_language,
        input.selected_skills,
    )
}

fn is_frontend_or_graphics(spec: &WorkerSpec) -> bool {
    let text = format!(
        "{}\n{}\n{}\n{}",
        spec.role, spec.objective, spec.task, spec.prompt
    )
    .to_ascii_lowercase();
    [
        "frontend",
        "html",
        "css",
        "javascript",
        "three.js",
        "webgl",
        "glsl",
        "shader",
        "browser",
        "responsive",
        "canvas",
        "网页",
        "前端",
        "浏览器",
        "可视化",
    ]
    .iter()
    .any(|term| text.contains(term))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunascope_core::{
        ModelSelection, RetryPolicy, WorkerBudget, WorkerCheckpointPolicy, WorkerId,
        WorkerInputContext, WorkerOutputSchema,
    };

    fn spec(objective: &str, role: &str) -> WorkerSpec {
        WorkerSpec {
            worker_id: WorkerId::new("worker-prompt"),
            display_name: format!("Test {role} assignment"),
            role: role.into(),
            tags: Vec::new(),
            objective: objective.into(),
            task: objective.into(),
            prompt: objective.into(),
            input_context: WorkerInputContext {
                summary: String::new(),
                artifact_ids: Vec::new(),
                include_workspace_snapshot: true,
            },
            expected_output: "result".into(),
            output_schema: WorkerOutputSchema {
                media_type: "application/json".into(),
                schema: serde_json::json!({"type":"object"}),
            },
            completion_criteria: vec!["done".into()],
            owned_acceptance_criteria: Vec::new(),
            parallel_group: None,
            model: ModelSelection {
                provider: "provider".into(),
                model: "model".into(),
                reason: "test".into(),
                fallback: false,
                reasoning_effort: None,
                custom_reasoning_effort: None,
            },
            skills: Vec::new(),
            tools: vec!["filesystem.read".into()],
            permissions: Vec::new(),
            budget: WorkerBudget {
                maximum_input_tokens: None,
                maximum_output_tokens: None,
                maximum_cost_microusd: None,
            },
            dependencies: Vec::new(),
            timeout_ms: 1_000,
            retry_policy: RetryPolicy {
                maximum_attempts: 1,
                backoff_ms: 0,
                retryable_error_codes: Vec::new(),
            },
            checkpoint_policy: WorkerCheckpointPolicy {
                on_start: false,
                on_artifact: false,
                on_completion: false,
            },
            write_scopes: Vec::new(),
            locked_fields: Vec::new(),
            parent_worker_id: None,
            depth: 0,
        }
    }

    #[test]
    fn adds_frontend_harness_only_when_relevant() {
        let frontend = spec("Build a Three.js WebGL shader", "frontend");
        let prompt = build_worker_instructions(WorkerPromptInput {
            spec: &frontend,
            reply_language: "Reply in English.",
            selected_skills: "No skills.",
            environment: "browser: available",
            self_management_access: false,
        });
        assert!(prompt.contains("Frontend and graphics harness"));
        assert!(prompt.contains("check_browser_page"));
        let research = spec("Summarize a paper", "researcher");
        let prompt = build_worker_instructions(WorkerPromptInput {
            spec: &research,
            reply_language: "Reply in English.",
            selected_skills: "No skills.",
            environment: "python: available",
            self_management_access: false,
        });
        assert!(!prompt.contains("Frontend and graphics harness"));
    }
}
