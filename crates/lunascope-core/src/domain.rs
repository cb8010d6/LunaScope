use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum DomainPackId {
    Programming,
    GameDevelopment,
    Research,
    AcademicWriting,
    FrontendDesign,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum GameEngine {
    Unity,
    Unreal,
    Godot,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct DomainPackDescriptor {
    pub id: DomainPackId,
    pub label: String,
    pub core_roles: Vec<String>,
    pub capabilities: Vec<String>,
    pub workflow: Vec<String>,
    pub rules: Vec<String>,
    pub default_skills: Vec<String>,
    pub default_tools: Vec<String>,
    pub required_evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct DomainDetection {
    pub selected: DomainPackId,
    pub reason: String,
    pub game_engine: Option<GameEngine>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ProgrammingFlowEvidence {
    pub repository_inspected: bool,
    pub scope_recorded: bool,
    pub checkpoint_created: bool,
    pub patch_artifact_id: Option<String>,
    pub test_command: Option<String>,
    pub test_exit_code: Option<i32>,
    pub diff_reviewed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct GameDevelopmentFlowEvidence {
    pub engine: GameEngine,
    pub binary_assets_modified: bool,
    pub safe_asset_handling_confirmed: bool,
    pub editor_or_build_invoked: bool,
    pub editor_or_build_approval_id: Option<String>,
    pub verification_log_artifact_id: Option<String>,
    pub large_asset_root: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ResearchSourceEvidence {
    pub title: String,
    pub authors: Vec<String>,
    pub doi: Option<String>,
    pub identifiers_verified: bool,
    pub query: String,
    pub retrieval_date: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ResearchFlowEvidence {
    pub facts_inferences_hypotheses_separated: bool,
    pub sources: Vec<ResearchSourceEvidence>,
    pub environment_artifact_id: Option<String>,
    #[ts(type = "number | null")]
    pub random_seed: Option<u64>,
    pub reproduction_artifact_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct AcademicWritingFlowEvidence {
    pub rubric_criteria: Vec<String>,
    pub covered_rubric_criteria: Vec<String>,
    pub evidence_matrix_artifact_id: Option<String>,
    pub cited_sources_verified: bool,
    pub unsupported_claims: u32,
    pub citation_needed_markers: u32,
    pub rendered_artifact_id: Option<String>,
    pub rendered_layout_reviewed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct FrontendFlowEvidence {
    pub tested_viewport_widths: Vec<u32>,
    pub clickable_elements_tested: bool,
    pub accessibility_checked: bool,
    pub screenshot_artifact_ids: Vec<String>,
    pub performance_checked: bool,
    pub anti_ai_aesthetic_reviewed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "pack", content = "evidence", rename_all = "snake_case")]
#[ts(tag = "pack", content = "evidence", rename_all = "snake_case")]
pub enum DomainFlowEvidence {
    Programming(ProgrammingFlowEvidence),
    GameDevelopment(GameDevelopmentFlowEvidence),
    Research(ResearchFlowEvidence),
    AcademicWriting(AcademicWritingFlowEvidence),
    FrontendDesign(FrontendFlowEvidence),
}
