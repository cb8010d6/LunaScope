use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{PermissionRequest, SkillCompatibility};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ImportComponentKind {
    Skill,
    Agent,
    Command,
    Hook,
    Mcp,
    Tool,
    Asset,
    Script,
    RuntimeDependency,
    Plugin,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ImportRiskKind {
    Script,
    InstallHook,
    Binary,
    Executable,
    Symlink,
    Submodule,
    PathTraversal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct GithubImportSource {
    pub repository_url: String,
    pub owner: String,
    pub repository: String,
    pub requested_ref: Option<String>,
    pub subdirectory: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ImportFileRecord {
    pub path: String,
    pub git_mode: String,
    pub git_object_id: String,
    #[ts(type = "number")]
    pub size_bytes: u64,
    pub sha256: Option<String>,
    pub risks: Vec<ImportRiskKind>,
    pub extractable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ImportComponent {
    pub id: String,
    pub kind: ImportComponentKind,
    pub display_name: String,
    pub path_prefix: String,
    pub compatibility: SkillCompatibility,
    pub requested_permissions: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum LicenseStatus {
    Detected,
    Unknown,
    Missing,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct LicenseFinding {
    pub status: LicenseStatus,
    pub spdx_id: Option<String>,
    pub name: Option<String>,
    pub file_path: Option<String>,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct GithubImportPreview {
    pub import_id: String,
    pub source: GithubImportSource,
    pub commit_sha: String,
    pub content_sha256: String,
    pub quarantine_path: String,
    pub scanned_at: String,
    pub files: Vec<ImportFileRecord>,
    pub components: Vec<ImportComponent>,
    pub license: LicenseFinding,
    pub permission_requests: Vec<PermissionRequest>,
    pub warnings: Vec<String>,
    pub blocked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ImportInstallRequest {
    pub import_id: String,
    pub workspace_root: String,
    pub component_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct InstalledImportVersion {
    pub installation_id: String,
    pub version_id: String,
    pub source: GithubImportSource,
    pub commit_sha: String,
    pub content_sha256: String,
    pub installed_path: String,
    pub component_ids: Vec<String>,
    pub installed_at: String,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ImportComparison {
    pub installation_id: String,
    pub active_commit_sha: String,
    pub candidate_commit_sha: String,
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
}
