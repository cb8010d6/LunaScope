use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum UltraNoteMode {
    Full,
    Limited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum CourseSourceKind {
    Syllabus,
    Slides,
    LectureNotes,
    Transcript,
    Reading,
    TextbookPages,
    BoardPhoto,
    LabMaterial,
    Code,
    Dataset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum NoteProvenance {
    FromClass,
    AgentExplanation,
    Inference,
    ExternalSource,
    Unresolved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum HomeworkKind {
    Practice,
    Ungraded,
    Graded,
    Exam,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum HomeworkAssistanceMode {
    Explain,
    Hint,
    Socratic,
    ReviewAttempt,
    WorkedExample,
    Debug,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct Course {
    pub course_id: String,
    pub title: String,
    pub code: Option<String>,
    pub mode: UltraNoteMode,
    pub current_syllabus_revision: Option<u32>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CourseThreadBinding {
    pub thread_id: String,
    pub course_id: String,
    pub bound_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CourseSource {
    pub source_id: String,
    pub course_id: String,
    pub kind: CourseSourceKind,
    pub display_name: String,
    pub content_sha256: String,
    pub imported_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CitationAnchor {
    pub anchor_id: String,
    pub course_id: String,
    pub source_id: String,
    pub page: Option<u32>,
    pub slide: Option<u32>,
    pub timestamp: Option<String>,
    pub text_range: Option<String>,
    pub excerpt: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CourseSearchHit {
    pub course_id: String,
    pub source_id: String,
    pub excerpt: String,
    pub rank: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct SyllabusStructure {
    pub course_information: Vec<String>,
    pub learning_objectives: Vec<String>,
    pub meeting_times: Vec<String>,
    pub topic_schedule: Vec<String>,
    pub assessments: Vec<String>,
    pub grading: Vec<String>,
    pub textbooks: Vec<String>,
    pub late_policy: Vec<String>,
    pub attendance_policy: Vec<String>,
    pub ai_policy: Vec<String>,
    pub academic_integrity_policy: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct SyllabusRevision {
    pub revision_id: String,
    pub course_id: String,
    pub source_id: String,
    pub revision: u32,
    pub structure: SyllabusStructure,
    pub ambiguities: Vec<String>,
    pub confirmed: bool,
    pub prestudy_plan: Vec<String>,
    pub first_phase_plan: Vec<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct NoteSection {
    pub heading: String,
    pub content: Vec<String>,
    pub provenance: NoteProvenance,
    pub citation_anchor_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct NoteRevision {
    pub note_revision_id: String,
    pub course_id: String,
    pub thread_id: String,
    pub source_id: String,
    pub revision: u32,
    pub title: String,
    pub user_note_spec: Option<String>,
    pub sections: Vec<NoteSection>,
    pub source_map: Vec<CitationAnchor>,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CourseConcept {
    pub concept_id: String,
    pub course_id: String,
    pub term: String,
    pub explanation: String,
    pub confidence: u8,
    pub source_anchor_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CourseAssignment {
    pub assignment_id: String,
    pub course_id: String,
    pub title: String,
    pub kind: HomeworkKind,
    pub due_at: Option<String>,
    pub policy_notes: Vec<String>,
    pub source_anchor_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ReviewItem {
    pub review_item_id: String,
    pub course_id: String,
    pub concept_id: String,
    pub prompt: String,
    pub answer: String,
    pub interval_days: u32,
    pub next_review_at: String,
    pub confidence: u8,
    pub successful_reviews: u32,
    pub lapses: u32,
    pub source_anchor_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CourseMemory {
    pub course_id: String,
    pub objectives: Vec<String>,
    pub concepts: Vec<CourseConcept>,
    pub assignments: Vec<CourseAssignment>,
    pub review_items: Vec<ReviewItem>,
    pub user_corrections: Vec<String>,
    pub exam_scope: Vec<String>,
    pub mastery_evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct HomeworkPolicyDecision {
    pub course_id: String,
    pub kind: HomeworkKind,
    pub clarification_required: bool,
    pub allowed_modes: Vec<HomeworkAssistanceMode>,
    pub direct_submittable_answer_allowed: bool,
    pub rationale: Vec<String>,
    pub prohibited_actions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct UltraNoteWorkspace {
    pub thread_id: String,
    pub binding: Option<CourseThreadBinding>,
    pub course: Option<Course>,
    pub syllabus: Option<SyllabusRevision>,
    pub latest_notes: Vec<NoteRevision>,
    pub memory: Option<CourseMemory>,
    pub requested_action: String,
}
