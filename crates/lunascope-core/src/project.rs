use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ProjectFolder {
    pub folder_id: String,
    pub path: String,
    pub display_name: String,
    pub is_workspace: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ProjectKind {
    #[default]
    General,
    UltraNote,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct UltraNoteProjectConfig {
    pub course_id: String,
    pub course_title: String,
    pub course_code: Option<String>,
    pub syllabus_attachment_id: String,
    pub syllabus_display_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct LunaProject {
    pub project_id: String,
    pub name: String,
    pub folders: Vec<ProjectFolder>,
    #[serde(default)]
    pub kind: ProjectKind,
    #[serde(default)]
    pub ultranote: Option<UltraNoteProjectConfig>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum UiLanguage {
    Chinese,
    English,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ModelReplyLanguage {
    #[default]
    FollowUi,
    Chinese,
    English,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct UserPreferences {
    pub language: UiLanguage,
    #[serde(default)]
    pub model_reply_language: ModelReplyLanguage,
    pub ultranote_note_spec: String,
}

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            language: UiLanguage::Chinese,
            model_reply_language: ModelReplyLanguage::FollowUi,
            ultranote_note_spec: "Use clear hierarchical headings, preserve source anchors, explain concepts in plain language, and separate source content from LunaScope explanations. When source and output languages differ, show difficult or technical terms with their English form in parentheses and finish with a glossary. Preserve formulas as defined, checkable derivations and explain every chart or function graph."
                .to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ProjectFileRead {
    pub project_id: String,
    pub path: String,
    pub text: String,
    pub sha256: String,
    #[ts(type = "number")]
    pub file_size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ProjectFileMutation {
    pub project_id: String,
    pub path: String,
    pub sha256: String,
    pub bytes_written: usize,
    pub created: bool,
}
