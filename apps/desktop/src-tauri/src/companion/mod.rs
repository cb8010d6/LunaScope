use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
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
    Ok(normalize_settings(settings))
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
pub(crate) fn companion_get_settings() -> Result<CompanionSettings, String> {
    read_settings()
}

#[tauri::command]
pub(crate) fn companion_save_preferences(
    app: AppHandle,
    preferences: CompanionPreferencesInput,
) -> Result<CompanionSettings, String> {
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

fn live2d_model_files(source: &Path) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let name = source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !name.ends_with(".model3.json") {
        return Err("select a Cubism 3/4/5 .model3.json file".into());
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(source).map_err(display_error)?)
            .map_err(|error| format!("invalid Live2D model manifest: {error}"))?;
    let references = manifest
        .get("FileReferences")
        .and_then(serde_json::Value::as_object)
        .ok_or("Live2D manifest is missing FileReferences")?;
    if references
        .get("Moc")
        .and_then(serde_json::Value::as_str)
        .is_none_or(|value| value.is_empty())
        || references
            .get("Textures")
            .and_then(serde_json::Value::as_array)
            .is_none_or(|value| value.is_empty())
    {
        return Err("Live2D manifest needs a Moc and at least one texture".into());
    }
    let root = source
        .parent()
        .ok_or("selected Live2D manifest has no parent directory")?
        .to_path_buf();
    let mut files = Vec::new();
    let mut total_bytes = 0_u64;
    collect_live2d_files(&root, &root, 0, &mut files, &mut total_bytes)?;
    if !files.iter().any(|path| path == source)
        || !files.iter().any(|path| {
            path.extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("moc3"))
        })
    {
        return Err("Live2D model manifest or .moc3 file is missing".into());
    }
    Ok((root, files))
}

async fn ensure_live2d_core() -> Result<PathBuf, String> {
    let destination = companion_root()?
        .join("runtime")
        .join("live2dcubismcore.min.js");
    if let Ok(bytes) = fs::read(&destination) {
        if bytes
            .windows(b"Live2DCubismCore".len())
            .any(|value| value == b"Live2DCubismCore")
        {
            return Ok(destination);
        }
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
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
    let bytes = response.bytes().await.map_err(display_error)?;
    if bytes.len() > MAX_LIVE2D_CORE_BYTES
        || !bytes
            .windows(b"Live2DCubismCore".len())
            .any(|value| value == b"Live2DCubismCore")
    {
        return Err("Live2D Cubism Core response failed validation".into());
    }
    fs::create_dir_all(destination.parent().ok_or("invalid Live2D runtime path")?)
        .map_err(display_error)?;
    fs::write(&destination, bytes).map_err(display_error)?;
    Ok(destination)
}

#[tauri::command]
pub(crate) async fn companion_import_model(
    app: AppHandle,
    source_path: String,
) -> Result<CompanionSettings, String> {
    let source = PathBuf::from(source_path)
        .canonicalize()
        .map_err(display_error)?;
    let is_spine = source
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("skel"));
    let (model_kind, source_root, files, model_name, live2d_core_path) = if is_spine {
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
        (
            CompanionModelKind::Spine38,
            root,
            spine_model_files(&source)?,
            name,
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
            Some(core.to_string_lossy().into_owned()),
        )
    };
    let destination =
        companion_root()?
            .join("models")
            .join(format!("{}-{}", model_name, Uuid::new_v4()));
    fs::create_dir_all(&destination).map_err(display_error)?;
    for file in files {
        let relative = file
            .strip_prefix(&source_root)
            .map_err(|_| "model asset escaped its selected directory")?;
        let target = destination.join(relative);
        fs::create_dir_all(target.parent().ok_or("model asset has no parent")?)
            .map_err(display_error)?;
        fs::copy(&file, target).map_err(display_error)?;
    }
    let imported_model = destination.join(
        source
            .file_name()
            .ok_or("selected Spine file has no file name")?,
    );
    let local_model_id = format!("local-{}", Uuid::new_v4().simple());
    models::write_local_model_metadata(
        &destination,
        &local_model_id,
        &model_name,
        model_kind,
        source
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("selected model has an invalid file name")?,
    )?;
    let mut settings = read_settings()?;
    settings.enabled = true;
    settings.model_path = Some(imported_model.to_string_lossy().into_owned());
    settings.model_name = Some(model_name);
    settings.model_kind = model_kind;
    settings.live2d_core_path = live2d_core_path;
    write_settings(&settings)?;
    set_window_visibility(&app, &settings);
    let _ = app.emit_to("companion", "companion:settings", &settings);
    Ok(settings)
}

#[tauri::command]
pub(crate) fn companion_set_activity(app: AppHandle, activity: CompanionActivity) {
    let phase = match activity.phase.as_str() {
        "idle" | "working" | "reviewing" | "running" | "success" | "failed" | "waiting" => {
            activity.phase
        }
        _ => "idle".into(),
    };
    let payload = CompanionActivity { phase, ..activity };
    let _ = app.emit_to("companion", "companion:activity", payload);
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
pub(crate) fn companion_avatar_requirements() -> serde_json::Value {
    avatar::requirements()
}

#[tauri::command]
pub(crate) fn companion_list_avatar_packs() -> Result<Vec<serde_json::Value>, String> {
    avatar::load_registry(&companion_root()?)
}

#[tauri::command]
pub(crate) fn companion_create_avatar_pack(
    input: avatar::AvatarPackCreateInput,
) -> Result<avatar::AvatarPackLifecycleResult, String> {
    avatar::create_standard_pack(input)
}

#[tauri::command]
pub(crate) fn companion_duplicate_avatar_pack(
    input: avatar::AvatarPackDuplicateInput,
) -> Result<avatar::AvatarPackLifecycleResult, String> {
    avatar::duplicate_pack(input)
}

#[tauri::command]
pub(crate) fn companion_delete_avatar_pack(
    input: avatar::AvatarPackInput,
) -> Result<avatar::AvatarPackLifecycleResult, String> {
    avatar::delete_pack(&avatar::path_from_input(input))
}

#[tauri::command]
pub(crate) fn companion_repack_avatar_pack(
    input: avatar::AvatarPackInput,
) -> Result<avatar::AvatarPackLifecycleResult, String> {
    avatar::repack_pack(&avatar::path_from_input(input))
}

#[tauri::command]
pub(crate) fn companion_load_avatar_manifest(
    input: avatar::AvatarPackInput,
) -> Result<avatar::AvatarPackManifest, String> {
    avatar::load_manifest(&avatar::path_from_input(input))
}

#[tauri::command]
pub(crate) fn companion_save_avatar_manifest(
    input: avatar::AvatarPackInput,
    manifest: avatar::AvatarPackManifest,
) -> Result<avatar::AvatarValidation, String> {
    avatar::save_manifest(&avatar::path_from_input(input), manifest)
}

#[tauri::command]
pub(crate) fn companion_validate_avatar_pack(
    input: avatar::AvatarPackInput,
) -> avatar::AvatarValidation {
    avatar::validate_pack(&avatar::path_from_input(input))
}

#[tauri::command]
pub(crate) fn companion_register_avatar_pack(
    input: avatar::AvatarPackInput,
) -> Result<avatar::AvatarImportResult, String> {
    avatar::register_pack(&avatar::path_from_input(input), &companion_root()?)
}

#[tauri::command]
pub(crate) fn companion_install_avatar_pack(
    input: avatar::AvatarPackInput,
) -> Result<avatar::AvatarRuntimeInstallResult, String> {
    let source = avatar::path_from_input(input);
    let installed = avatar::install_runtime_pack(&source, &companion_root()?)?;
    models::write_local_model_metadata(
        Path::new(&installed.runtime_path),
        &installed.validation.id,
        &installed.validation.name,
        CompanionModelKind::Spine38,
        &installed.skel,
    )?;
    Ok(installed)
}

#[tauri::command]
pub(crate) fn companion_import_avatar_layers(
    input: AvatarLayerImportInput,
) -> Result<Vec<String>, String> {
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
    input: avatar::AvatarPackInput,
    relative_path: String,
) -> Result<AvatarAssetBytes, String> {
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
    fn only_spine_runtime_assets_are_imported() {
        assert!(is_spine_model_asset(Path::new("character.skel")));
        assert!(is_spine_model_asset(Path::new("character.ATLAS")));
        assert!(is_spine_model_asset(Path::new("character.png")));
        assert!(!is_spine_model_asset(Path::new("install.exe")));
        assert!(!is_spine_model_asset(Path::new("notes.txt")));
        assert!(is_live2d_model_asset(Path::new("character.model3.json")));
        assert!(is_live2d_model_asset(Path::new("idle.motion3.json")));
        assert!(is_live2d_model_asset(Path::new("character.moc3")));
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
}
