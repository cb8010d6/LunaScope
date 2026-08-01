use std::path::{Path, PathBuf};

use lunascope_core::{
    LunaProject, PermissionContext, PermissionKind, PermissionRequest, ProjectFileMutation,
    ProjectFileRead, ProjectFolder, ProjectKind, RiskLevel, UserPreferences,
};
use lunascope_runtime::{FilesystemTools, TextPatch, TextReplacement};
use serde::Deserialize;
use tauri::State;
use uuid::Uuid;

use crate::{AppState, display_error, enforce_permissions};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectFolderInput {
    path: String,
    is_workspace: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveProjectRequest {
    project_id: Option<String>,
    name: String,
    folders: Vec<ProjectFolderInput>,
    #[serde(default)]
    kind: ProjectKind,
    ultranote: Option<UltraNoteProjectInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UltraNoteProjectInput {
    course_title: String,
    course_code: Option<String>,
    syllabus_attachment_id: String,
    syllabus_display_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectFileRequest {
    project_id: String,
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateProjectFileRequest {
    project_id: String,
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PatchProjectFileRequest {
    project_id: String,
    path: String,
    expected_sha256: String,
    old: String,
    new: String,
    expected_occurrences: usize,
}

#[tauri::command]
pub(crate) fn list_projects(state: State<'_, AppState>) -> Result<Vec<LunaProject>, String> {
    state.store.projects().map_err(display_error)
}

#[tauri::command]
pub(crate) async fn save_project(
    state: State<'_, AppState>,
    request: SaveProjectRequest,
    allow_once: bool,
) -> Result<LunaProject, String> {
    let name = request.name.trim();
    if name.is_empty() {
        return Err("project name is required".to_owned());
    }
    if request.folders.is_empty() {
        return Err("select at least one project folder".to_owned());
    }
    if request
        .folders
        .iter()
        .filter(|folder| folder.is_workspace)
        .count()
        != 1
    {
        return Err("select exactly one folder as the workspace".to_owned());
    }
    let mut folders = Vec::new();
    for input in request.folders {
        let canonical = Path::new(input.path.trim())
            .canonicalize()
            .map_err(display_error)?;
        if !canonical.is_dir() {
            return Err(format!(
                "project folder is not a directory: {}",
                canonical.display()
            ));
        }
        let canonical_text = canonical.to_string_lossy().into_owned();
        if folders
            .iter()
            .any(|folder: &ProjectFolder| folder.path.eq_ignore_ascii_case(&canonical_text))
        {
            return Err(format!("duplicate project folder: {canonical_text}"));
        }
        folders.push(ProjectFolder {
            folder_id: format!("folder-{}", Uuid::new_v4()),
            display_name: canonical
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| canonical_text.clone()),
            path: canonical_text,
            is_workspace: input.is_workspace,
        });
    }
    let now = jiff::Timestamp::now().to_string();
    let existing = request
        .project_id
        .as_deref()
        .map(|id| state.store.project(id).map_err(display_error))
        .transpose()?
        .flatten();
    let ultranote = match request.kind {
        ProjectKind::General => None,
        ProjectKind::UltraNote => {
            let input = request
                .ultranote
                .as_ref()
                .ok_or_else(|| "UltraNote project settings are required".to_owned())?;
            let unchanged = existing
                .as_ref()
                .and_then(|project| project.ultranote.clone())
                .filter(|config| {
                    config.syllabus_attachment_id == input.syllabus_attachment_id
                        && config.course_title == input.course_title.trim()
                        && config.course_code.as_deref()
                            == input
                                .course_code
                                .as_deref()
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                });
            if let Some(config) = unchanged {
                Some(config)
            } else {
                let providers = state.store.provider_configs().map_err(display_error)?;
                let (provider, _) =
                    crate::orchestration::selected_orchestration_model(&state, &providers)?;
                enforce_permissions(
                    &[
                        PermissionRequest {
                            permission: PermissionKind::NetworkConnect,
                            context: PermissionContext {
                                workspace_root: folders
                                    .iter()
                                    .find(|folder| folder.is_workspace)
                                    .map(|folder| folder.path.clone())
                                    .unwrap_or_default(),
                                network_domain: url::Url::parse(&provider.base_url)
                                    .ok()
                                    .and_then(|url| url.host_str().map(str::to_owned)),
                                tool_id: Some("ultranote.syllabus.parse".to_owned()),
                                ..PermissionContext::default()
                            },
                            risk: RiskLevel::Medium,
                            action: "parse the selected syllabus with the Orchestration model"
                                .to_owned(),
                        },
                        PermissionRequest {
                            permission: PermissionKind::SecretsUse,
                            context: PermissionContext {
                                workspace_root: folders
                                    .iter()
                                    .find(|folder| folder.is_workspace)
                                    .map(|folder| folder.path.clone())
                                    .unwrap_or_default(),
                                tool_id: Some("ultranote.syllabus.parse".to_owned()),
                                ..PermissionContext::default()
                            },
                            risk: RiskLevel::High,
                            action:
                                "use the Orchestration provider credential for syllabus parsing"
                                    .to_owned(),
                        },
                    ],
                    allow_once,
                )?;
                Some(
                    crate::ultranote::initialize_project_course(
                        &state,
                        &input.course_title,
                        input.course_code.as_deref(),
                        &input.syllabus_attachment_id,
                        &input.syllabus_display_name,
                    )
                    .await?,
                )
            }
        }
    };
    let project = LunaProject {
        project_id: existing
            .as_ref()
            .map(|project| project.project_id.clone())
            .unwrap_or_else(|| format!("project-{}", Uuid::new_v4())),
        name: name.to_owned(),
        folders,
        kind: request.kind,
        ultranote,
        created_at: existing
            .as_ref()
            .map(|project| project.created_at.clone())
            .unwrap_or_else(|| now.clone()),
        updated_at: now,
    };
    state.store.save_project(&project).map_err(display_error)?;
    Ok(project)
}

#[tauri::command]
pub(crate) fn delete_project(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<bool, String> {
    state
        .store
        .delete_project(&project_id)
        .map_err(display_error)
}

#[tauri::command]
pub(crate) fn get_user_preferences(state: State<'_, AppState>) -> Result<UserPreferences, String> {
    state.store.user_preferences().map_err(display_error)
}

#[tauri::command]
pub(crate) fn save_user_preferences(
    state: State<'_, AppState>,
    preferences: UserPreferences,
) -> Result<UserPreferences, String> {
    if preferences.ultranote_note_spec.len() > 16 * 1024 {
        return Err("UltraNote specification must not exceed 16 KiB".to_owned());
    }
    state
        .store
        .save_user_preferences(&preferences)
        .map_err(display_error)?;
    Ok(preferences)
}

#[tauri::command]
pub(crate) fn read_project_file(
    state: State<'_, AppState>,
    request: ProjectFileRequest,
    allow_once: bool,
) -> Result<ProjectFileRead, String> {
    let (project, workspace) = workspace_for_project(&state, &request.project_id)?;
    enforce_file_permission(
        &project,
        &workspace,
        &request.path,
        PermissionKind::FilesystemRead,
        RiskLevel::Low,
        "Read a file inside the selected project workspace",
        allow_once,
    )?;
    let read = FilesystemTools::new(&workspace)
        .map_err(display_error)?
        .read_range(&request.path, 0, 1024 * 1024)
        .map_err(display_error)?;
    Ok(ProjectFileRead {
        project_id: project.project_id,
        path: request.path,
        text: read.text,
        sha256: read.sha256,
        file_size: read.file_size,
    })
}

#[tauri::command]
pub(crate) fn create_project_file(
    state: State<'_, AppState>,
    request: CreateProjectFileRequest,
    allow_once: bool,
) -> Result<ProjectFileMutation, String> {
    let (project, workspace) = workspace_for_project(&state, &request.project_id)?;
    enforce_file_permission(
        &project,
        &workspace,
        &request.path,
        PermissionKind::FilesystemWrite,
        RiskLevel::Medium,
        "Create a new file inside the selected project workspace",
        allow_once,
    )?;
    let result = FilesystemTools::new(&workspace)
        .map_err(display_error)?
        .create_text_file(&request.path, &request.content)
        .map_err(display_error)?;
    Ok(ProjectFileMutation {
        project_id: project.project_id,
        path: result.path.to_string_lossy().into_owned(),
        sha256: result.sha256,
        bytes_written: result.bytes_written,
        created: true,
    })
}

#[tauri::command]
pub(crate) fn patch_project_file(
    state: State<'_, AppState>,
    request: PatchProjectFileRequest,
    allow_once: bool,
) -> Result<ProjectFileMutation, String> {
    let (project, workspace) = workspace_for_project(&state, &request.project_id)?;
    enforce_file_permission(
        &project,
        &workspace,
        &request.path,
        PermissionKind::FilesystemWrite,
        RiskLevel::Medium,
        "Modify an existing file with hash and occurrence guards",
        allow_once,
    )?;
    let result = FilesystemTools::new(&workspace)
        .map_err(display_error)?
        .apply_text_patch(&TextPatch {
            path: request.path,
            expected_sha256: request.expected_sha256,
            replacements: vec![TextReplacement {
                old: request.old,
                new: request.new,
                expected_occurrences: request.expected_occurrences,
            }],
        })
        .map_err(display_error)?;
    Ok(ProjectFileMutation {
        project_id: project.project_id,
        path: result.path.to_string_lossy().into_owned(),
        sha256: result.sha256,
        bytes_written: result.bytes_written,
        created: false,
    })
}

fn workspace_for_project(
    state: &State<'_, AppState>,
    project_id: &str,
) -> Result<(LunaProject, PathBuf), String> {
    let project = state
        .store
        .project(project_id)
        .map_err(display_error)?
        .ok_or_else(|| format!("project not found: {project_id}"))?;
    let workspace = project
        .folders
        .iter()
        .find(|folder| folder.is_workspace)
        .ok_or_else(|| "project has no workspace folder".to_owned())?
        .path
        .clone();
    Ok((project, PathBuf::from(workspace)))
}

fn enforce_file_permission(
    project: &LunaProject,
    workspace: &Path,
    relative_path: &str,
    permission: PermissionKind,
    risk: RiskLevel,
    action: &str,
    allow_once: bool,
) -> Result<(), String> {
    let target = workspace.join(relative_path);
    enforce_permissions(
        &[PermissionRequest {
            permission,
            context: PermissionContext {
                workspace_root: workspace.to_string_lossy().into_owned(),
                project_id: Some(project.project_id.clone()),
                target_path: Some(target.to_string_lossy().into_owned()),
                tool_id: Some(
                    match permission {
                        PermissionKind::FilesystemRead => "filesystem.read",
                        _ => "filesystem.write",
                    }
                    .to_owned(),
                ),
                ..PermissionContext::default()
            },
            risk,
            action: action.to_owned(),
        }],
        allow_once,
    )
}
