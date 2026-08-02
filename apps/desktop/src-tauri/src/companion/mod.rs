use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use crate::{data_root, display_error};

mod avatar;
#[allow(dead_code)]
mod catalog;
pub(crate) mod models;

const MAX_MODEL_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MODEL_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_LIVE2D_CORE_BYTES: usize = 4 * 1024 * 1024;
const LIVE2D_CORE_URL: &str =
    "https://cubism.live2d.com/sdk-web/cubismcore/live2dcubismcore.min.js";
const LIVE2D_CORE_SHA256: &str = "25ae938cb4fe282ce189b357bcc97e603d1e1f7ec78bf04150d401c23cdc792f";

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CompanionModelKind {
    #[default]
    Spine38,
    Live2d,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompanionPreferencesInput {
    enabled: bool,
    scale: f64,
    bubble_visible: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompanionSettings {
    enabled: bool,
    model_path: Option<String>,
    atlas_path: Option<String>,
    model_name: Option<String>,
    model_kind: CompanionModelKind,
    live2d_core_path: Option<String>,
    scale: f64,
    bubble_visible: bool,
}

impl Default for CompanionSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            model_path: None,
            atlas_path: None,
            model_name: None,
            model_kind: CompanionModelKind::Spine38,
            live2d_core_path: None,
            scale: 0.86,
            bubble_visible: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompanionActivity {
    phase: String,
    title: String,
    detail: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompanionRendererStatus {
    state: String,
    message: String,
}

fn companion_root() -> Result<PathBuf, String> {
    Ok(data_root()?.join("companion"))
}

fn settings_path() -> Result<PathBuf, String> {
    Ok(companion_root()?.join("settings.json"))
}

fn normalize_settings(mut settings: CompanionSettings) -> CompanionSettings {
    if !settings.scale.is_finite() {
        settings.scale = CompanionSettings::default().scale;
    }
    settings.scale = settings.scale.clamp(0.35, 1.55);
    settings
}

fn read_settings() -> Result<CompanionSettings, String> {
    let path = settings_path()?;
    if !path.exists() {
        return Ok(CompanionSettings::default());
    }
    let settings =
        serde_json::from_slice::<CompanionSettings>(&fs::read(path).map_err(display_error)?)
            .map_err(display_error)?;
    let mut settings = normalize_settings(settings);
    if matches!(settings.model_kind, CompanionModelKind::Live2d)
        && settings
            .live2d_core_path
            .as_deref()
            .is_some_and(|path| !live2d_core_is_pinned(Path::new(path)))
    {
        settings.enabled = false;
        settings.live2d_core_path = None;
    }
    Ok(settings)
}

fn write_settings(settings: &CompanionSettings) -> Result<(), String> {
    let path = settings_path()?;
    let parent = path
        .parent()
        .ok_or("companion settings path has no parent")?;
    fs::create_dir_all(parent).map_err(display_error)?;
    let bytes = serde_json::to_vec_pretty(settings).map_err(display_error)?;
    fs::write(path, bytes).map_err(display_error)
}

fn require_window(window: &tauri::WebviewWindow, allowed: &[&str]) -> Result<(), String> {
    if allowed.contains(&window.label()) {
        Ok(())
    } else {
        Err("this companion command is not available to the current window".into())
    }
}

pub(super) fn require_main_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    require_window(window, &["main"])
}

#[tauri::command]
pub(crate) fn companion_report_renderer_status(
    app: AppHandle,
    window: tauri::WebviewWindow,
    status: CompanionRendererStatus,
) -> Result<(), String> {
    require_window(&window, &["companion"])?;
    if !matches!(status.state.as_str(), "disabled" | "ready" | "error") {
        return Err("invalid companion renderer state".into());
    }
    if status.message.len() > 2_000 {
        return Err("companion renderer message is too long".into());
    }
    app.emit_to("main", "companion:renderer-status", status)
        .map_err(display_error)
}

fn set_window_visibility(app: &AppHandle, settings: &CompanionSettings) {
    let Some(window) = app.get_webview_window("companion") else {
        return;
    };
    if settings.enabled && settings.model_path.is_some() {
        let _ = window.show();
    } else {
        let _ = window.hide();
    }
}

pub(crate) fn restore_window_visibility(app: &AppHandle) {
    if let Ok(settings) = read_settings() {
        set_window_visibility(app, &settings);
    }
}

#[tauri::command]
pub(crate) fn companion_get_settings(
    window: tauri::WebviewWindow,
) -> Result<CompanionSettings, String> {
    require_window(&window, &["main", "companion"])?;
    read_settings()
}

#[tauri::command]
pub(crate) fn companion_save_preferences(
    app: AppHandle,
    window: tauri::WebviewWindow,
    preferences: CompanionPreferencesInput,
) -> Result<CompanionSettings, String> {
    require_window(&window, &["main", "companion"])?;
    let current = read_settings()?;
    let settings = normalize_settings(CompanionSettings {
        enabled: preferences.enabled,
        scale: preferences.scale,
        bubble_visible: preferences.bubble_visible,
        ..current
    });
    write_settings(&settings)?;
    set_window_visibility(&app, &settings);
    let _ = app.emit_to("companion", "companion:settings", &settings);
    Ok(settings)
}

fn is_spine_model_asset(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "skel" | "atlas" | "png" | "jpg" | "jpeg" | "webp"
            )
        })
        .unwrap_or(false)
}

fn spine_model_files(source: &Path) -> Result<Vec<PathBuf>, String> {
    let directory = source
        .parent()
        .ok_or("selected Spine file has no parent directory")?;
    let mut files = Vec::new();
    let mut total_bytes = 0_u64;
    let mut has_atlas = false;
    let mut has_texture = false;
    for entry in fs::read_dir(directory).map_err(display_error)? {
        let path = entry.map_err(display_error)?.path();
        if !path.is_file() || !is_spine_model_asset(&path) {
            continue;
        }
        let size = fs::metadata(&path).map_err(display_error)?.len();
        if size > MAX_MODEL_FILE_BYTES {
            return Err(format!(
                "Spine model file exceeds 64 MiB: {}",
                path.display()
            ));
        }
        total_bytes = total_bytes
            .checked_add(size)
            .ok_or("Spine model size overflow")?;
        if total_bytes > MAX_MODEL_TOTAL_BYTES {
            return Err("Spine model assets exceed 256 MiB".into());
        }
        match path.extension().and_then(|value| value.to_str()) {
            Some(value) if value.eq_ignore_ascii_case("atlas") => has_atlas = true,
            Some(value)
                if ["png", "jpg", "jpeg", "webp"]
                    .iter()
                    .any(|extension| value.eq_ignore_ascii_case(extension)) =>
            {
                has_texture = true;
            }
            _ => {}
        }
        files.push(path);
    }
    if !has_atlas || !has_texture {
        return Err("the selected Spine model needs an atlas and at least one texture".into());
    }
    Ok(files)
}

fn is_live2d_model_asset(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    [
        ".model3.json",
        ".moc3",
        ".png",
        ".jpg",
        ".jpeg",
        ".webp",
        ".wav",
        ".mp3",
        ".ogg",
        ".motion3.json",
        ".exp3.json",
        ".physics3.json",
        ".pose3.json",
        ".userdata3.json",
        ".cdi3.json",
    ]
    .iter()
    .any(|suffix| name.ends_with(suffix))
}

fn collect_live2d_files(
    root: &Path,
    directory: &Path,
    depth: usize,
    files: &mut Vec<PathBuf>,
    total_bytes: &mut u64,
) -> Result<(), String> {
    if depth > 8 {
        return Err("Live2D model directory nesting exceeds 8 levels".into());
    }
    for entry in fs::read_dir(directory).map_err(display_error)? {
        let entry = entry.map_err(display_error)?;
        let file_type = entry.file_type().map_err(display_error)?;
        if file_type.is_symlink() {
            return Err(format!(
                "Live2D model contains a symbolic link: {}",
                entry.path().display()
            ));
        }
        if file_type.is_dir() {
            collect_live2d_files(root, &entry.path(), depth + 1, files, total_bytes)?;
            continue;
        }
        let path = entry.path();
        if !file_type.is_file() || !is_live2d_model_asset(&path) {
            continue;
        }
        path.strip_prefix(root)
            .map_err(|_| "Live2D model asset escaped its selected directory")?;
        let size = entry.metadata().map_err(display_error)?.len();
        if size > MAX_MODEL_FILE_BYTES {
            return Err(format!(
                "Live2D model file exceeds 64 MiB: {}",
                path.display()
            ));
        }
        *total_bytes = total_bytes
            .checked_add(size)
            .ok_or("Live2D model size overflow")?;
        if *total_bytes > MAX_MODEL_TOTAL_BYTES {
            return Err("Live2D model assets exceed 256 MiB".into());
        }
        files.push(path);
    }
    Ok(())
}

fn live2d_reference_path(root: &Path, value: &str, field: &str) -> Result<PathBuf, String> {
    let relative = Path::new(value);
    if value.is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(format!("Live2D {field} contains an unsafe path: {value}"));
    }
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| format!("Live2D {field} references a missing file: {value}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "Live2D {field} must reference a regular file: {value}"
        ));
    }
    if !is_live2d_model_asset(&path) {
        return Err(format!(
            "Live2D {field} references an unsupported file type: {value}"
        ));
    }
    Ok(path)
}

fn validate_live2d_manifest_references(source: &Path) -> Result<Vec<PathBuf>, String> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(source).map_err(display_error)?)
            .map_err(|error| format!("invalid Live2D model manifest: {error}"))?;
    let references = manifest
        .get("FileReferences")
        .and_then(serde_json::Value::as_object)
        .ok_or("Live2D manifest is missing FileReferences")?;
    let root = source
        .parent()
        .ok_or("selected Live2D manifest has no parent directory")?;
    let moc = references
        .get("Moc")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("Live2D manifest needs a Moc")?;
    let textures = references
        .get("Textures")
        .and_then(serde_json::Value::as_array)
        .filter(|value| !value.is_empty())
        .ok_or("Live2D manifest needs at least one texture")?;

    let mut files = vec![live2d_reference_path(root, moc, "Moc")?];
    for texture in textures {
        let value = texture
            .as_str()
            .ok_or("Live2D Textures entries must be paths")?;
        files.push(live2d_reference_path(root, value, "Textures")?);
    }
    for field in ["Physics", "Pose", "UserData", "DisplayInfo"] {
        if let Some(value) = references.get(field) {
            let value = value
                .as_str()
                .ok_or_else(|| format!("Live2D {field} must be a path"))?;
            files.push(live2d_reference_path(root, value, field)?);
        }
    }
    if let Some(expressions) = references.get("Expressions") {
        for expression in expressions
            .as_array()
            .ok_or("Live2D Expressions must be an array")?
        {
            let value = expression
                .get("File")
                .and_then(serde_json::Value::as_str)
                .ok_or("Live2D Expressions entries need a File path")?;
            files.push(live2d_reference_path(root, value, "Expressions")?);
        }
    }
    if let Some(motions) = references.get("Motions") {
        for entries in motions
            .as_object()
            .ok_or("Live2D Motions must be an object")?
            .values()
        {
            for motion in entries
                .as_array()
                .ok_or("Live2D motion groups must be arrays")?
            {
                let value = motion
                    .get("File")
                    .and_then(serde_json::Value::as_str)
                    .ok_or("Live2D motion entries need a File path")?;
                files.push(live2d_reference_path(root, value, "Motions")?);
                if let Some(sound) = motion.get("Sound") {
                    let value = sound.as_str().ok_or("Live2D motion Sound must be a path")?;
                    files.push(live2d_reference_path(root, value, "Motion Sound")?);
                }
            }
        }
    }
    Ok(files)
}

pub(super) fn validate_live2d_manifest(source: &Path) -> Result<(), String> {
    validate_live2d_manifest_references(source).map(|_| ())
}

fn live2d_model_files(source: &Path) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let name = source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !name.ends_with(".model3.json") {
        return Err("select a Cubism 3/4/5 .model3.json file".into());
    }
    let root = source
        .parent()
        .ok_or("selected Live2D manifest has no parent directory")?
        .to_path_buf();
    let referenced = validate_live2d_manifest_references(source)?;
    let mut files = Vec::new();
    let mut total_bytes = 0_u64;
    collect_live2d_files(&root, &root, 0, &mut files, &mut total_bytes)?;
    if !files.iter().any(|path| path == source)
        || referenced
            .iter()
            .any(|reference| !files.iter().any(|path| path == reference))
    {
        return Err("Live2D manifest references an unavailable runtime asset".into());
    }
    Ok((root, files))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn live2d_core_is_pinned(path: &Path) -> bool {
    fs::read(path)
        .map(|bytes| sha256_hex(&bytes) == LIVE2D_CORE_SHA256)
        .unwrap_or(false)
}

async fn ensure_live2d_core() -> Result<PathBuf, String> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    let _guard = LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let destination = companion_root()?
        .join("runtime")
        .join("live2dcubismcore.min.js");
    if live2d_core_is_pinned(&destination) {
        return Ok(destination);
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(display_error)?;
    let response = client
        .get(LIVE2D_CORE_URL)
        .header("User-Agent", "LunaScope desktop companion")
        .send()
        .await
        .map_err(display_error)?;
    if !response.status().is_success() {
        return Err(format!(
            "Live2D official Cubism Core download failed with {}",
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_LIVE2D_CORE_BYTES as u64)
    {
        return Err("Live2D Cubism Core exceeds 4 MiB".into());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(display_error)?;
        if bytes.len().saturating_add(chunk.len()) > MAX_LIVE2D_CORE_BYTES {
            return Err("Live2D Cubism Core exceeds 4 MiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if sha256_hex(&bytes) != LIVE2D_CORE_SHA256 {
        return Err("Live2D Cubism Core digest does not match the pinned official build".into());
    }
    fs::create_dir_all(destination.parent().ok_or("invalid Live2D runtime path")?)
        .map_err(display_error)?;
    let staging = destination.with_extension(format!("{}.download", Uuid::new_v4()));
    let result = (|| {
        fs::write(&staging, bytes).map_err(display_error)?;
        if destination.exists() {
            fs::remove_file(&destination).map_err(display_error)?;
        }
        fs::rename(&staging, &destination).map_err(display_error)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(staging);
        return Err(error);
    }
    Ok(destination)
}

#[tauri::command]
pub(crate) async fn companion_import_model(
    app: AppHandle,
    window: tauri::WebviewWindow,
    source_path: String,
) -> Result<CompanionSettings, String> {
    require_main_window(&window)?;
    let source = PathBuf::from(source_path)
        .canonicalize()
        .map_err(display_error)?;
    let is_spine = source
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("skel"));
    let (model_kind, source_root, files, model_name, atlas_file, live2d_core_path) = if is_spine {
        let root = source
            .parent()
            .ok_or("selected Spine file has no parent directory")?
            .to_path_buf();
        let name = source
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .ok_or("selected Spine file has an invalid name")?
            .to_owned();
        let asset_set = avatar::spine_assets::validate_spine_asset_dir(
            &root,
            source
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or("selected Spine file has an invalid name")?,
        )?;
        let expected_atlas = format!("{name}.atlas");
        let atlas = asset_set
            .atlas_files
            .iter()
            .find(|value| value.eq_ignore_ascii_case(&expected_atlas))
            .or_else(|| asset_set.atlas_files.first())
            .cloned()
            .ok_or("selected Spine model has no atlas")?;
        (
            CompanionModelKind::Spine38,
            root,
            spine_model_files(&source)?,
            name,
            Some(atlas),
            None,
        )
    } else {
        let (root, files) = live2d_model_files(&source)?;
        let name = source
            .file_name()
            .and_then(|value| value.to_str())
            .and_then(|value| value.strip_suffix(".model3.json"))
            .filter(|value| !value.is_empty())
            .ok_or("selected Live2D manifest has an invalid name")?
            .to_owned();
        let core = ensure_live2d_core().await?;
        (
            CompanionModelKind::Live2d,
            root,
            files,
            name,
            None,
            Some(core.to_string_lossy().into_owned()),
        )
    };
    let models_root = companion_root()?.join("models");
    fs::create_dir_all(&models_root).map_err(display_error)?;
    let import_id = Uuid::new_v4();
    let destination = models_root.join(format!("{}-{import_id}", model_name));
    let staging = models_root.join(format!(".{import_id}.import"));
    fs::create_dir(&staging).map_err(display_error)?;
    let imported_model = destination.join(
        source
            .file_name()
            .ok_or("selected Spine file has no file name")?,
    );
    let local_model_id = format!("local-{}", Uuid::new_v4().simple());
    let install_result = (|| {
        for file in files {
            let relative = file
                .strip_prefix(&source_root)
                .map_err(|_| "model asset escaped its selected directory")?;
            let target = staging.join(relative);
            fs::create_dir_all(target.parent().ok_or("model asset has no parent")?)
                .map_err(display_error)?;
            fs::copy(&file, target).map_err(display_error)?;
        }
        models::write_local_model_metadata(
            &staging,
            &local_model_id,
            &model_name,
            model_kind,
            source
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or("selected model has an invalid file name")?,
            atlas_file.as_deref(),
            "local",
        )?;
        fs::rename(&staging, &destination).map_err(display_error)
    })();
    if let Err(error) = install_result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    let mut settings = match read_settings() {
        Ok(settings) => settings,
        Err(error) => {
            let _ = fs::remove_dir_all(&destination);
            return Err(error);
        }
    };
    settings.enabled = true;
    settings.model_path = Some(imported_model.to_string_lossy().into_owned());
    settings.atlas_path =
        atlas_file.map(|value| destination.join(value).to_string_lossy().into_owned());
    settings.model_name = Some(model_name);
    settings.model_kind = model_kind;
    settings.live2d_core_path = live2d_core_path;
    if let Err(error) = write_settings(&settings) {
        let _ = fs::remove_dir_all(&destination);
        return Err(error);
    }
    set_window_visibility(&app, &settings);
    let _ = app.emit_to("companion", "companion:settings", &settings);
    Ok(settings)
}

#[tauri::command]
pub(crate) fn companion_set_activity(
    app: AppHandle,
    window: tauri::WebviewWindow,
    activity: CompanionActivity,
) -> Result<(), String> {
    require_main_window(&window)?;
    let phase = match activity.phase.as_str() {
        "idle" | "working" | "reviewing" | "running" | "success" | "failed" | "waiting" => {
            activity.phase
        }
        _ => "idle".into(),
    };
    let payload = CompanionActivity { phase, ..activity };
    let _ = app.emit_to("companion", "companion:activity", payload);
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AvatarAssetBytes {
    bytes: Vec<u8>,
    mime: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AvatarLayerImportInput {
    pack_path: String,
    files: Vec<String>,
}

#[tauri::command]
pub(crate) fn companion_avatar_requirements(
    window: tauri::WebviewWindow,
) -> Result<serde_json::Value, String> {
    require_main_window(&window)?;
    Ok(avatar::requirements())
}

#[tauri::command]
pub(crate) fn companion_list_avatar_packs(
    window: tauri::WebviewWindow,
) -> Result<Vec<serde_json::Value>, String> {
    require_main_window(&window)?;
    avatar::load_registry(&companion_root()?)
}

#[tauri::command]
pub(crate) fn companion_create_avatar_pack(
    window: tauri::WebviewWindow,
    input: avatar::AvatarPackCreateInput,
) -> Result<avatar::AvatarPackLifecycleResult, String> {
    require_main_window(&window)?;
    avatar::create_standard_pack(input)
}

#[tauri::command]
pub(crate) fn companion_duplicate_avatar_pack(
    window: tauri::WebviewWindow,
    input: avatar::AvatarPackDuplicateInput,
) -> Result<avatar::AvatarPackLifecycleResult, String> {
    require_main_window(&window)?;
    avatar::duplicate_pack(input)
}

#[tauri::command]
pub(crate) fn companion_delete_avatar_pack(
    window: tauri::WebviewWindow,
    input: avatar::AvatarPackInput,
) -> Result<avatar::AvatarPackLifecycleResult, String> {
    require_main_window(&window)?;
    avatar::delete_pack(&avatar::path_from_input(input))
}

#[tauri::command]
pub(crate) fn companion_repack_avatar_pack(
    window: tauri::WebviewWindow,
    input: avatar::AvatarPackInput,
) -> Result<avatar::AvatarPackLifecycleResult, String> {
    require_main_window(&window)?;
    avatar::repack_pack(&avatar::path_from_input(input))
}

#[tauri::command]
pub(crate) fn companion_load_avatar_manifest(
    window: tauri::WebviewWindow,
    input: avatar::AvatarPackInput,
) -> Result<avatar::AvatarPackManifest, String> {
    require_main_window(&window)?;
    avatar::load_manifest(&avatar::path_from_input(input))
}

#[tauri::command]
pub(crate) fn companion_save_avatar_manifest(
    window: tauri::WebviewWindow,
    input: avatar::AvatarPackInput,
    manifest: avatar::AvatarPackManifest,
) -> Result<avatar::AvatarValidation, String> {
    require_main_window(&window)?;
    avatar::save_manifest(&avatar::path_from_input(input), manifest)
}

#[tauri::command]
pub(crate) fn companion_validate_avatar_pack(
    window: tauri::WebviewWindow,
    input: avatar::AvatarPackInput,
) -> Result<avatar::AvatarValidation, String> {
    require_main_window(&window)?;
    Ok(avatar::validate_pack(&avatar::path_from_input(input)))
}

#[tauri::command]
pub(crate) fn companion_register_avatar_pack(
    window: tauri::WebviewWindow,
    input: avatar::AvatarPackInput,
) -> Result<avatar::AvatarImportResult, String> {
    require_main_window(&window)?;
    avatar::register_pack(&avatar::path_from_input(input), &companion_root()?)
}

#[tauri::command]
pub(crate) fn companion_install_avatar_pack(
    window: tauri::WebviewWindow,
    input: avatar::AvatarPackInput,
) -> Result<avatar::AvatarRuntimeInstallResult, String> {
    require_main_window(&window)?;
    let source = avatar::path_from_input(input);
    avatar::install_runtime_pack(&source, &companion_root()?)
}

#[tauri::command]
pub(crate) fn companion_import_avatar_layers(
    window: tauri::WebviewWindow,
    input: AvatarLayerImportInput,
) -> Result<Vec<String>, String> {
    require_main_window(&window)?;
    let root = PathBuf::from(&input.pack_path)
        .canonicalize()
        .map_err(|error| format!("Cannot open avatar pack: {error}"))?;
    if !root.join("avatar-pack.json").is_file() {
        return Err("Avatar pack must contain avatar-pack.json".into());
    }
    let layers = root.join("layers");
    fs::create_dir_all(&layers).map_err(display_error)?;
    let mut imported = Vec::new();
    for source in input.files {
        let source = PathBuf::from(source)
            .canonicalize()
            .map_err(display_error)?;
        let metadata = fs::metadata(&source).map_err(display_error)?;
        if !metadata.is_file() || metadata.len() > MAX_MODEL_FILE_BYTES {
            return Err("Avatar layer must be an image no larger than 64 MiB".into());
        }
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp") {
            return Err("Avatar layers support PNG, JPEG, or WebP files".into());
        }
        let base = source
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("layer")
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let mut name = format!("{base}.{extension}");
        let mut index = 2;
        while layers.join(&name).exists() {
            name = format!("{base}_{index}.{extension}");
            index += 1;
        }
        fs::copy(&source, layers.join(&name)).map_err(display_error)?;
        imported.push(format!("layers/{name}"));
    }
    Ok(imported)
}

#[tauri::command]
pub(crate) fn companion_read_avatar_asset(
    window: tauri::WebviewWindow,
    input: avatar::AvatarPackInput,
    relative_path: String,
) -> Result<AvatarAssetBytes, String> {
    require_main_window(&window)?;
    let root = avatar::path_from_input(input)
        .canonicalize()
        .map_err(display_error)?;
    let relative = avatar::spine_assets::safe_relative_path(&relative_path)?;
    let path = root.join(relative).canonicalize().map_err(display_error)?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err("Avatar asset must stay inside the pack directory".into());
    }
    let mime = match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => return Err("Avatar preview only supports PNG, JPEG, or WebP".into()),
    };
    Ok(AvatarAssetBytes {
        bytes: fs::read(path).map_err(display_error)?,
        mime: mime.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_scale_is_finite_and_bounded() {
        let mut settings = CompanionSettings {
            scale: f64::NAN,
            ..Default::default()
        };
        settings = normalize_settings(settings);
        assert_eq!(settings.scale, 0.86);
        settings.scale = 8.0;
        assert_eq!(normalize_settings(settings).scale, 1.55);
    }

    #[test]
    fn sha256_helper_matches_a_known_digest() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn only_spine_runtime_assets_are_imported() {
        assert!(is_spine_model_asset(Path::new("character.skel")));
        assert!(is_spine_model_asset(Path::new("character.ATLAS")));
        assert!(is_spine_model_asset(Path::new("character.png")));
        assert!(!is_spine_model_asset(Path::new("install.exe")));
        assert!(!is_spine_model_asset(Path::new("notes.txt")));
        assert!(is_live2d_model_asset(Path::new("character.model3.json")));
        assert!(is_live2d_model_asset(Path::new("idle.motion3.json")));
        assert!(is_live2d_model_asset(Path::new("character.moc3")));
        assert!(is_live2d_model_asset(Path::new("voice.ogg")));
        assert!(!is_live2d_model_asset(Path::new("setup.js")));
    }

    #[test]
    fn live2d_import_collects_only_allowlisted_runtime_files() {
        let temporary = tempfile::tempdir().expect("temporary Live2D model");
        let root = temporary.path();
        let textures = root.join("textures");
        fs::create_dir_all(&textures).expect("texture directory");
        let manifest = root.join("avatar.model3.json");
        fs::write(
            &manifest,
            r#"{"FileReferences":{"Moc":"avatar.moc3","Textures":["textures/00.png"]}}"#,
        )
        .expect("manifest");
        fs::write(root.join("avatar.moc3"), b"MOC3").expect("moc");
        fs::write(textures.join("00.png"), b"png").expect("texture");
        fs::write(root.join("setup.js"), b"alert(1)").expect("blocked script");

        let (collected_root, files) = live2d_model_files(&manifest).expect("valid model");

        assert_eq!(collected_root, root);
        assert_eq!(files.len(), 3);
        assert!(!files.iter().any(|path| path.ends_with("setup.js")));
    }

    #[test]
    fn live2d_manifest_rejects_paths_outside_the_model_root() {
        let temporary = tempfile::tempdir().expect("temporary Live2D model");
        let root = temporary.path().join("model");
        fs::create_dir_all(&root).expect("model directory");
        fs::write(temporary.path().join("outside.moc3"), b"MOC3").expect("outside moc");
        fs::write(root.join("texture.png"), b"png").expect("texture");
        let manifest = root.join("avatar.model3.json");
        fs::write(
            &manifest,
            r#"{"FileReferences":{"Moc":"../outside.moc3","Textures":["texture.png"]}}"#,
        )
        .expect("manifest");

        let error = live2d_model_files(&manifest).expect_err("path traversal must fail");

        assert!(error.contains("unsafe path"));
    }

    #[test]
    fn live2d_manifest_requires_each_referenced_file() {
        let temporary = tempfile::tempdir().expect("temporary Live2D model");
        let root = temporary.path();
        fs::write(root.join("avatar.moc3"), b"MOC3").expect("moc");
        let manifest = root.join("avatar.model3.json");
        fs::write(
            &manifest,
            r#"{"FileReferences":{"Moc":"avatar.moc3","Textures":["missing.png"]}}"#,
        )
        .expect("manifest");

        let error = live2d_model_files(&manifest).expect_err("missing reference must fail");

        assert!(error.contains("missing file"));
    }
}
