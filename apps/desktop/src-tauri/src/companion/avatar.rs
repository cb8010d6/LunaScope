#[path = "spine_assets.rs"]
pub(crate) mod spine_assets;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const AVATAR_PACK_VERSION: u32 = 1;

const STANDARD_LAYERS: &[&str] = &[
    "hair_back",
    "body",
    "leg_left",
    "leg_right",
    "arm_left",
    "arm_right",
    "head",
];
const STANDARD_MOTIONS: &[&str] = &[
    "idle",
    "working",
    "reviewing",
    "running",
    "success",
    "failed",
    "waiting",
    "sleeping",
    "reminder",
    "interact",
];
const MAX_RUNTIME_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RUNTIME_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_RUNTIME_FILES: usize = 512;
const TRANSPARENT_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarPackInput {
    pub path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarPackCreateInput {
    pub path: String,
    pub id: String,
    pub name: String,
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default = "default_license_note")]
    pub license_note: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarPackDuplicateInput {
    pub path: String,
    pub destination_parent: String,
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AvatarPoint {
    pub x: f64,
    pub y: f64,
}

impl Default for AvatarPoint {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AvatarScale {
    pub x: f64,
    pub y: f64,
}

impl Default for AvatarScale {
    fn default() -> Self {
        Self { x: 1.0, y: 1.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AvatarCrop {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AvatarLayer {
    pub id: String,
    pub file: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default)]
    pub order: i32,
    #[serde(default = "default_anchor")]
    pub anchor: AvatarPoint,
    #[serde(default)]
    pub offset: AvatarPoint,
    #[serde(default)]
    pub scale: AvatarScale,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<AvatarCrop>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AvatarPackManifest {
    #[serde(default = "avatar_pack_version", alias = "manifestVersion")]
    pub version: u32,
    pub id: String,
    pub name: String,
    pub source: String,
    pub license_note: String,
    #[serde(default = "default_preview")]
    pub preview: String,
    #[serde(default)]
    pub layers: Vec<AvatarLayer>,
    #[serde(default)]
    pub motions: BTreeMap<String, String>,
    #[serde(default)]
    pub states: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exports: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spine_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_skel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_atlas: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_ready: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AvatarValidationIssue {
    pub severity: String,
    pub code: String,
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarPackLifecycleResult {
    pub path: String,
    pub id: String,
    pub created: bool,
    pub duplicated: bool,
    pub deleted: bool,
    pub repacked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<AvatarValidation>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarValidation {
    pub ok: bool,
    pub path: String,
    pub id: String,
    pub name: String,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub runtime_errors: Vec<String>,
    pub has_preview: bool,
    pub has_layers_dir: bool,
    pub has_exports_dir: bool,
    pub has_runtime_export: bool,
    pub runtime_ready: bool,
    pub draft: bool,
    pub runtime_skel: String,
    pub runtime_atlas: String,
    pub spine_version: String,
    pub atlas_pages: Vec<String>,
    pub textures: Vec<String>,
    pub missing_attachments: Vec<String>,
    pub issues: Vec<AvatarValidationIssue>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarImportResult {
    pub imported: bool,
    pub validation: AvatarValidation,
    pub registry_path: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarRuntimeInstallResult {
    pub installed: bool,
    pub model_id: String,
    pub validation: AvatarValidation,
    pub registry_path: String,
    pub runtime_path: String,
    pub skel: String,
    pub atlas: String,
}

pub fn requirements() -> Value {
    json!({
        "manifestVersion": AVATAR_PACK_VERSION,
        "layout": ["avatar-pack.json", "preview.png", "layers/", "exports/"],
        "requiredManifestFields": ["version", "id", "name", "source", "licenseNote"],
        "layerFields": ["id", "file", "name", "visible", "order", "anchor", "offset", "scale", "crop"],
        "recommendedLayers": STANDARD_LAYERS,
        "motions": STANDARD_MOTIONS,
        "stateMappings": STANDARD_MOTIONS,
        "runtimeExports": [".skel", ".atlas", ".png"],
        "runtimeFields": ["spineVersion", "runtimeSkel", "runtimeAtlas", "runtimeReady", "attachments"],
        "runtimeSkel": "Required when exports/ contains more than one .skel file; it is relative to exports/.",
        "limits": "Without Spine Editor or another legal export path, Spine Companion can validate and manage an intermediate avatar pack but cannot claim a finished runtime rig."
    })
}

fn avatar_pack_version() -> u32 {
    AVATAR_PACK_VERSION
}

fn default_true() -> bool {
    true
}

fn default_anchor() -> AvatarPoint {
    AvatarPoint { x: 0.5, y: 0.5 }
}

fn default_preview() -> String {
    "preview.png".to_string()
}

fn default_source() -> String {
    "local".to_string()
}

fn default_license_note() -> String {
    "User-owned local content".to_string()
}

fn standard_manifest(
    id: String,
    name: String,
    source: String,
    license_note: String,
) -> AvatarPackManifest {
    let layers = STANDARD_LAYERS
        .iter()
        .enumerate()
        .map(|(order, id)| AvatarLayer {
            id: (*id).to_string(),
            file: format!("layers/{id}.png"),
            name: id.replace('_', " "),
            visible: true,
            order: order as i32,
            anchor: default_anchor(),
            offset: AvatarPoint::default(),
            scale: AvatarScale::default(),
            crop: None,
        })
        .collect();
    let motions = STANDARD_MOTIONS
        .iter()
        .map(|motion| ((*motion).to_string(), (*motion).to_string()))
        .collect();
    let states = STANDARD_MOTIONS
        .iter()
        .map(|state| ((*state).to_string(), (*state).to_string()))
        .collect();
    AvatarPackManifest {
        version: AVATAR_PACK_VERSION,
        id,
        name,
        source,
        license_note,
        preview: default_preview(),
        layers,
        motions,
        states,
        exports: Vec::new(),
        attachments: Vec::new(),
        spine_version: None,
        runtime_skel: None,
        runtime_atlas: None,
        runtime_ready: None,
        extra: Map::new(),
    }
}

pub fn avatar_registry_path(config_dir: &Path) -> PathBuf {
    config_dir.join("avatar-packs.json")
}

pub fn avatar_runtime_dir(config_dir: &Path, id: &str) -> Result<PathBuf, String> {
    if !is_safe_avatar_id(id) {
        return Err("Avatar ID must use 1-64 ASCII letters, digits, underscores, or hyphens and start with a letter or digit.".to_string());
    }
    Ok(config_dir.join("models").join(format!("avatar--{id}")))
}

pub fn is_safe_avatar_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

pub fn load_manifest(path: &Path) -> Result<AvatarPackManifest, String> {
    let manifest = path.join("avatar-pack.json");
    let text = fs::read_to_string(&manifest)
        .map_err(|error| format!("Cannot read avatar-pack.json: {error}"))?;
    serde_json::from_str(&text).map_err(|error| format!("Invalid avatar-pack.json: {error}"))
}

pub fn save_manifest(
    path: &Path,
    mut manifest: AvatarPackManifest,
) -> Result<AvatarValidation, String> {
    ensure_existing_pack_root(path)?;
    manifest.version = AVATAR_PACK_VERSION;
    write_json_atomically(&path.join("avatar-pack.json"), &manifest)?;
    Ok(validate_pack(path))
}

pub fn create_standard_pack(
    input: AvatarPackCreateInput,
) -> Result<AvatarPackLifecycleResult, String> {
    let root = PathBuf::from(&input.path);
    if root.exists() {
        return Err("Avatar pack destination already exists.".to_string());
    }
    if !is_safe_avatar_id(input.id.trim()) {
        return Err("Avatar ID must use 1-64 ASCII letters, digits, underscores, or hyphens and start with a letter or digit.".to_string());
    }
    if input.name.trim().is_empty() {
        return Err("Avatar name must not be empty.".to_string());
    }
    let parent = root
        .parent()
        .ok_or_else(|| "Avatar pack destination must have a parent directory.".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let staging = sibling_staging_path(&root, "create")?;
    let manifest = standard_manifest(
        input.id.trim().to_string(),
        input.name.trim().to_string(),
        non_empty_or(input.source, default_source()),
        non_empty_or(input.license_note, default_license_note()),
    );
    let result = (|| {
        fs::create_dir(&staging).map_err(|error| error.to_string())?;
        fs::create_dir(staging.join("layers")).map_err(|error| error.to_string())?;
        fs::create_dir(staging.join("exports")).map_err(|error| error.to_string())?;
        fs::write(staging.join("preview.png"), TRANSPARENT_PNG)
            .map_err(|error| error.to_string())?;
        for layer in &manifest.layers {
            fs::write(staging.join(&layer.file), TRANSPARENT_PNG)
                .map_err(|error| error.to_string())?;
        }
        write_json_atomically(&staging.join("avatar-pack.json"), &manifest)?;
        let validation = validate_pack(&staging);
        if !validation.ok {
            return Err(format!(
                "Generated avatar pack failed validation: {}",
                validation.errors.join("; ")
            ));
        }
        fs::rename(&staging, &root).map_err(|error| error.to_string())?;
        Ok(validate_pack(&root))
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    let validation = result?;
    Ok(AvatarPackLifecycleResult {
        path: root.to_string_lossy().to_string(),
        id: validation.id.clone(),
        created: true,
        duplicated: false,
        deleted: false,
        repacked: false,
        validation: Some(validation),
    })
}

pub fn duplicate_pack(
    input: AvatarPackDuplicateInput,
) -> Result<AvatarPackLifecycleResult, String> {
    let source = PathBuf::from(&input.path);
    let mut manifest = load_manifest(&source)?;
    if !is_safe_avatar_id(input.id.trim()) {
        return Err("Avatar ID must use 1-64 ASCII letters, digits, underscores, or hyphens and start with a letter or digit.".to_string());
    }
    if input.name.trim().is_empty() {
        return Err("Avatar name must not be empty.".to_string());
    }
    let destination_parent = PathBuf::from(&input.destination_parent);
    fs::create_dir_all(&destination_parent).map_err(|error| error.to_string())?;
    let destination = destination_parent.join(input.id.trim());
    if destination.exists() {
        return Err("Avatar pack destination already exists.".to_string());
    }
    let staging = sibling_staging_path(&destination, "duplicate")?;
    let result = (|| {
        copy_tree(&source, &staging)?;
        manifest.version = AVATAR_PACK_VERSION;
        manifest.id = input.id.trim().to_string();
        manifest.name = input.name.trim().to_string();
        write_json_atomically(&staging.join("avatar-pack.json"), &manifest)?;
        let validation = validate_pack(&staging);
        if !validation.ok {
            return Err(format!(
                "Duplicated avatar pack failed validation: {}",
                validation.errors.join("; ")
            ));
        }
        fs::rename(&staging, &destination).map_err(|error| error.to_string())?;
        Ok(validate_pack(&destination))
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    let validation = result?;
    Ok(AvatarPackLifecycleResult {
        path: destination.to_string_lossy().to_string(),
        id: validation.id.clone(),
        created: false,
        duplicated: true,
        deleted: false,
        repacked: false,
        validation: Some(validation),
    })
}

pub fn delete_pack(path: &Path) -> Result<AvatarPackLifecycleResult, String> {
    let manifest = load_manifest(path)?;
    let root = path
        .canonicalize()
        .map_err(|error| format!("Cannot read avatar pack: {error}"))?;
    if !root.is_dir() {
        return Err("Avatar pack path must be a directory.".to_string());
    }
    let tombstone = sibling_staging_path(&root, "delete")?;
    fs::rename(&root, &tombstone).map_err(|error| error.to_string())?;
    if let Err(error) = fs::remove_dir_all(&tombstone) {
        let _ = fs::rename(&tombstone, &root);
        return Err(error.to_string());
    }
    Ok(AvatarPackLifecycleResult {
        path: root.to_string_lossy().to_string(),
        id: manifest.id,
        created: false,
        duplicated: false,
        deleted: true,
        repacked: false,
        validation: None,
    })
}

pub fn repack_pack(path: &Path) -> Result<AvatarPackLifecycleResult, String> {
    let root = path
        .canonicalize()
        .map_err(|error| format!("Cannot read avatar pack: {error}"))?;
    let mut manifest = load_manifest(&root)?;
    let staging = sibling_staging_path(&root, "repack")?;
    let result = (|| {
        copy_tree(&root, &staging)?;
        manifest.version = AVATAR_PACK_VERSION;
        write_json_atomically(&staging.join("avatar-pack.json"), &manifest)?;
        let validation = validate_pack(&staging);
        if !validation.ok {
            return Err(format!(
                "Repacked avatar pack failed validation: {}",
                validation.errors.join("; ")
            ));
        }
        let backup = replace_directory_atomically(&staging, &root)?;
        if let Some(backup) = backup {
            fs::remove_dir_all(backup).map_err(|error| error.to_string())?;
        }
        Ok(validate_pack(&root))
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    let validation = result?;
    Ok(AvatarPackLifecycleResult {
        path: root.to_string_lossy().to_string(),
        id: validation.id.clone(),
        created: false,
        duplicated: false,
        deleted: false,
        repacked: true,
        validation: Some(validation),
    })
}

pub fn load_registry(config_dir: &Path) -> Result<Vec<Value>, String> {
    let path = avatar_registry_path(config_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("Cannot read avatar registry: {error}"))?;
    serde_json::from_str::<Vec<Value>>(&text)
        .map_err(|error| format!("Invalid avatar registry: {error}"))
}

pub fn validate_pack(path: &Path) -> AvatarValidation {
    let root = path;
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    let mut runtime_errors = Vec::new();
    let mut issues = Vec::new();
    let mut id = String::new();
    let mut name = String::new();
    let manifest = match load_manifest(root) {
        Ok(value) => Some(value),
        Err(error) => {
            push_issue(
                &mut issues,
                &mut errors,
                "error",
                "manifest.invalid",
                "avatar-pack.json",
                error,
            );
            None
        }
    };

    if let Some(manifest) = &manifest {
        id = manifest.id.trim().to_string();
        name = manifest.name.trim().to_string();
        validate_manifest_model(root, manifest, &mut issues, &mut errors, &mut warnings);
        if !id.is_empty() && !is_safe_avatar_id(&id) {
            push_issue(
                &mut issues,
                &mut errors,
                "error",
                "manifest.id.unsafe",
                "id",
                "Manifest id is unsafe. Use 1-64 ASCII letters, digits, underscores, or hyphens and start with a letter or digit.",
            );
        }
    }

    let preview = manifest
        .as_ref()
        .map(|value| value.preview.as_str())
        .unwrap_or("preview.png");
    let has_preview = contained_pack_file(root, preview).is_ok();
    let has_layers_dir = root.join("layers").is_dir();
    let has_exports_dir = root.join("exports").is_dir();
    if !has_preview {
        push_issue(
            &mut issues,
            &mut errors,
            "error",
            "preview.missing",
            "preview",
            format!("Missing {preview}"),
        );
    }
    if !has_layers_dir {
        push_issue(
            &mut issues,
            &mut errors,
            "error",
            "layers.directoryMissing",
            "layers",
            "Missing layers/ directory",
        );
    }

    if has_exports_dir {
        if let Some(manifest) = &manifest {
            validate_declared_exports(
                &manifest.exports,
                &root.join("exports"),
                &mut issues,
                &mut errors,
            );
        }
    } else if manifest
        .as_ref()
        .is_some_and(|value| !value.exports.is_empty())
    {
        push_issue(
            &mut issues,
            &mut errors,
            "error",
            "exports.directoryMissing",
            "exports",
            "Manifest declares exports but exports/ directory is missing.",
        );
    }

    let mut runtime_skel = String::new();
    let mut runtime_atlas = String::new();
    let mut spine_version = manifest
        .as_ref()
        .and_then(|value| value.spine_version.clone())
        .unwrap_or_default();
    let mut atlas_pages = Vec::new();
    let mut textures = Vec::new();
    let mut missing_attachments = Vec::new();
    let mut runtime_ready = false;
    if !has_exports_dir {
        push_issue(
            &mut issues,
            &mut warnings,
            "warning",
            "runtime.exportsMissing",
            "exports",
            "Missing exports/ directory. This pack is a draft and runtime installation is unavailable.",
        );
    } else if let Some(manifest) = &manifest {
        let exports = root.join("exports");
        let manifest_value = serde_json::to_value(manifest).unwrap_or(Value::Null);
        match validate_runtime_fields(&manifest_value, &exports) {
            Ok(fields) => {
                let skeletons = spine_assets::list_skeletons(&exports).unwrap_or_else(|error| {
                    runtime_errors.push(error);
                    Vec::new()
                });
                match choose_runtime_skel(&skeletons, fields.runtime_skel.as_deref()) {
                    Ok(Some(selected)) => {
                        match spine_assets::validate_spine_asset_dir(&exports, &selected) {
                            Ok(asset_set) => {
                                runtime_skel = selected;
                                runtime_atlas = choose_runtime_atlas(
                                    &asset_set.atlas_files,
                                    fields.runtime_atlas.as_deref(),
                                )
                                .unwrap_or_default();
                                atlas_pages = asset_set.atlas_texture_refs.clone();
                                textures = asset_set.texture_files.clone();
                                if spine_version.is_empty() {
                                    spine_version =
                                        read_spine_binary_version(&exports.join(&runtime_skel))
                                            .unwrap_or_default();
                                }
                                missing_attachments = find_missing_attachments(
                                    &exports,
                                    &asset_set.atlas_files,
                                    &manifest.attachments,
                                );
                                runtime_ready = true;
                                if !missing_attachments.is_empty() {
                                    runtime_errors.push(format!(
                                        "Missing atlas attachment region(s): {}",
                                        missing_attachments.join(", ")
                                    ));
                                    runtime_ready = false;
                                }
                                if !spine_version.is_empty() && !spine_version.starts_with("3.8") {
                                    runtime_errors.push(format!(
                                        "Unsupported Spine runtime version {spine_version}; expected Spine 3.8."
                                    ));
                                    runtime_ready = false;
                                }
                            }
                            Err(error) => runtime_errors.push(error),
                        }
                    }
                    Ok(None) => push_issue(
                        &mut issues,
                        &mut warnings,
                        "warning",
                        "runtime.skeletonMissing",
                        "runtimeSkel",
                        "exports/ has no .skel runtime export. This pack remains a draft.",
                    ),
                    Err(error) => push_issue(
                        &mut issues,
                        &mut errors,
                        "error",
                        "runtime.skeletonSelection",
                        "runtimeSkel",
                        error,
                    ),
                }
                if fields.runtime_ready == Some(true) && !runtime_ready {
                    push_issue(
                        &mut issues,
                        &mut errors,
                        "error",
                        "runtime.readyMismatch",
                        "runtimeReady",
                        "Manifest runtimeReady is true, but exports/ is not a valid Spine runtime set.",
                    );
                }
            }
            Err(field_errors) => {
                for error in field_errors {
                    push_issue(
                        &mut issues,
                        &mut errors,
                        "error",
                        "runtime.fieldInvalid",
                        runtime_error_path(&error),
                        error,
                    );
                }
            }
        }
    }

    if !runtime_errors.is_empty() {
        runtime_errors.sort();
        runtime_errors.dedup();
        let message = format!(
            "Runtime exports are not installable yet: {}",
            runtime_errors.join("; ")
        );
        push_issue(
            &mut issues,
            &mut warnings,
            "runtime",
            "runtime.invalid",
            "exports",
            message,
        );
    }
    let has_runtime_export = runtime_ready;
    AvatarValidation {
        ok: errors.is_empty(),
        path: root.to_string_lossy().to_string(),
        id,
        name,
        warnings,
        errors,
        runtime_errors,
        has_preview,
        has_layers_dir,
        has_exports_dir,
        has_runtime_export,
        runtime_ready,
        draft: !runtime_ready,
        runtime_skel,
        runtime_atlas,
        spine_version,
        atlas_pages,
        textures,
        missing_attachments,
        issues,
    }
}

/// Adds a valid draft or runtime-ready pack to the local registry without copying assets.
pub fn register_pack(path: &Path, config_dir: &Path) -> Result<AvatarImportResult, String> {
    let validation = validate_pack(path);
    if !validation.ok {
        return Err(format!(
            "Avatar pack is invalid: {}",
            validation.errors.join("; ")
        ));
    }
    let registry_path = update_registry(config_dir, &validation, None)?;
    Ok(AvatarImportResult {
        imported: true,
        validation,
        registry_path: registry_path.to_string_lossy().to_string(),
    })
}

#[cfg(test)]
fn import_pack(path: &Path, config_dir: &Path) -> Result<AvatarImportResult, String> {
    register_pack(path, config_dir)
}

/// Copies a runtime-ready pack into models/<id> through a staging directory and atomic replace.
pub fn install_runtime_pack(
    path: &Path,
    config_dir: &Path,
) -> Result<AvatarRuntimeInstallResult, String> {
    let validation = validate_pack(path);
    if !validation.ok {
        return Err(format!(
            "Avatar pack is invalid: {}",
            validation.errors.join("; ")
        ));
    }
    if !validation.runtime_ready {
        let detail = if validation.runtime_errors.is_empty() {
            "No complete Spine runtime export was found.".to_string()
        } else {
            validation.runtime_errors.join("; ")
        };
        return Err(format!(
            "Avatar pack is a draft and cannot be installed: {detail}"
        ));
    }
    let runtime_path = avatar_runtime_dir(config_dir, &validation.id)?;
    let model_id = format!("avatar--{}", validation.id);
    let models_dir = runtime_path
        .parent()
        .ok_or_else(|| "Cannot resolve avatar runtime directory.".to_string())?;
    fs::create_dir_all(models_dir).map_err(|error| error.to_string())?;
    let staging = models_dir.join(format!(".{}.install-{}", validation.id, unique_suffix()));
    let runtime_files = std::iter::once(validation.runtime_skel.clone())
        .chain(std::iter::once(validation.runtime_atlas.clone()))
        .chain(validation.textures.iter().cloned())
        .collect::<Vec<_>>();
    if let Err(error) = copy_runtime_files(&path.join("exports"), &staging, &runtime_files) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Err(error) = spine_assets::validate_spine_asset_dir(&staging, &validation.runtime_skel) {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("Staged avatar runtime failed validation: {error}"));
    }
    let metadata = json!({
        "id": model_id,
        "name": validation.name,
        "modelKind": "spine38",
        "entryFile": validation.runtime_skel,
        "atlasFile": validation.runtime_atlas,
        "source": "avatar-studio",
        "license": "USER_PROVIDED"
    });
    if let Err(error) = fs::write(
        staging.join(".companion-model.json"),
        serde_json::to_vec_pretty(&metadata).map_err(|error| error.to_string())?,
    ) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error.to_string());
    }

    let backup = replace_directory_atomically(&staging, &runtime_path)?;
    let registry_path = match update_registry(config_dir, &validation, Some(&runtime_path)) {
        Ok(path) => path,
        Err(error) => {
            let _ = rollback_directory_replace(&runtime_path, backup.as_deref());
            return Err(error);
        }
    };
    if let Some(backup) = backup {
        let _ = fs::remove_dir_all(backup);
    }
    let skel = validation.runtime_skel.clone();
    let atlas = validation.runtime_atlas.clone();
    Ok(AvatarRuntimeInstallResult {
        installed: true,
        model_id,
        validation,
        registry_path: registry_path.to_string_lossy().to_string(),
        runtime_path: runtime_path.to_string_lossy().to_string(),
        skel,
        atlas,
    })
}

pub fn path_from_input(input: AvatarPackInput) -> PathBuf {
    PathBuf::from(input.path)
}

fn validate_manifest_model(
    root: &Path,
    manifest: &AvatarPackManifest,
    issues: &mut Vec<AvatarValidationIssue>,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    if manifest.version != AVATAR_PACK_VERSION {
        push_issue(
            issues,
            errors,
            "error",
            "manifest.version.unsupported",
            "version",
            format!(
                "Unsupported avatar pack version {}; expected {}.",
                manifest.version, AVATAR_PACK_VERSION
            ),
        );
    }
    for (path, value) in [
        ("id", manifest.id.as_str()),
        ("name", manifest.name.as_str()),
        ("source", manifest.source.as_str()),
        ("licenseNote", manifest.license_note.as_str()),
    ] {
        if value.trim().is_empty() {
            push_issue(
                issues,
                errors,
                "error",
                "manifest.required",
                path,
                format!("Missing required manifest field: {path}"),
            );
        }
    }
    if let Err(error) = validate_pack_relative_file(root, &manifest.preview, None) {
        push_issue(
            issues,
            errors,
            "error",
            "preview.invalid",
            "preview",
            format!("Invalid preview path: {error}"),
        );
    }

    let mut layer_ids = HashSet::new();
    let mut orders = BTreeSet::new();
    for (index, layer) in manifest.layers.iter().enumerate() {
        let prefix = format!("layers[{index}]");
        if layer.id.trim().is_empty() {
            push_issue(
                issues,
                errors,
                "error",
                "layer.id.required",
                format!("{prefix}.id"),
                "Layer id must not be empty.",
            );
        } else if !is_safe_avatar_id(&layer.id) {
            push_issue(
                issues,
                errors,
                "error",
                "layer.id.unsafe",
                format!("{prefix}.id"),
                "Layer id must use safe ASCII letters, digits, underscores, or hyphens.",
            );
        } else if !layer_ids.insert(layer.id.clone()) {
            push_issue(
                issues,
                errors,
                "error",
                "layer.id.duplicate",
                format!("{prefix}.id"),
                format!("Duplicate layer id: {}", layer.id),
            );
        }
        if layer.name.trim().is_empty() {
            push_issue(
                issues,
                errors,
                "error",
                "layer.name.required",
                format!("{prefix}.name"),
                "Layer name must not be empty.",
            );
        }
        if let Err(error) = validate_pack_relative_file(root, &layer.file, Some("layers")) {
            push_issue(
                issues,
                errors,
                "error",
                "layer.file.invalid",
                format!("{prefix}.file"),
                format!("Invalid layer file: {error}"),
            );
        }
        if !orders.insert(layer.order) {
            push_issue(
                issues,
                warnings,
                "warning",
                "layer.order.duplicate",
                format!("{prefix}.order"),
                format!("Multiple layers use order {}.", layer.order),
            );
        }
        validate_layer_geometry(layer, &prefix, issues, errors);
    }

    for (motion, animation) in &manifest.motions {
        if motion.trim().is_empty() || animation.trim().is_empty() {
            push_issue(
                issues,
                errors,
                "error",
                "motion.invalid",
                format!("motions.{motion}"),
                "Motion keys and animation names must not be empty.",
            );
        }
    }
    for (state, motion) in &manifest.states {
        if state.trim().is_empty() || motion.trim().is_empty() {
            push_issue(
                issues,
                errors,
                "error",
                "state.invalid",
                format!("states.{state}"),
                "State keys and motion mappings must not be empty.",
            );
        } else if !manifest.motions.contains_key(motion) {
            push_issue(
                issues,
                warnings,
                "warning",
                "state.motionMissing",
                format!("states.{state}"),
                format!("State {state} maps to undeclared motion {motion}."),
            );
        }
    }
}

fn validate_layer_geometry(
    layer: &AvatarLayer,
    prefix: &str,
    issues: &mut Vec<AvatarValidationIssue>,
    errors: &mut Vec<String>,
) {
    for (path, value) in [
        (format!("{prefix}.anchor.x"), layer.anchor.x),
        (format!("{prefix}.anchor.y"), layer.anchor.y),
    ] {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            push_issue(
                issues,
                errors,
                "error",
                "layer.anchor.invalid",
                path,
                "Layer anchor coordinates must be finite values from 0 to 1.",
            );
        }
    }
    for (path, value) in [
        (format!("{prefix}.offset.x"), layer.offset.x),
        (format!("{prefix}.offset.y"), layer.offset.y),
    ] {
        if !value.is_finite() {
            push_issue(
                issues,
                errors,
                "error",
                "layer.offset.invalid",
                path,
                "Layer offsets must be finite numbers.",
            );
        }
    }
    for (path, value) in [
        (format!("{prefix}.scale.x"), layer.scale.x),
        (format!("{prefix}.scale.y"), layer.scale.y),
    ] {
        if !value.is_finite() || value <= 0.0 {
            push_issue(
                issues,
                errors,
                "error",
                "layer.scale.invalid",
                path,
                "Layer scale values must be finite and greater than zero.",
            );
        }
    }
    if let Some(crop) = &layer.crop {
        if [crop.x, crop.y, crop.width, crop.height]
            .iter()
            .any(|value| !value.is_finite())
            || crop.x < 0.0
            || crop.y < 0.0
            || crop.width <= 0.0
            || crop.height <= 0.0
        {
            push_issue(
                issues,
                errors,
                "error",
                "layer.crop.invalid",
                format!("{prefix}.crop"),
                "Layer crop must use finite non-negative coordinates and positive dimensions.",
            );
        }
    }
}

fn validate_declared_exports(
    entries: &[String],
    exports_dir: &Path,
    issues: &mut Vec<AvatarValidationIssue>,
    errors: &mut Vec<String>,
) {
    for (index, value) in entries.iter().enumerate() {
        match contained_export_file(exports_dir, value) {
            Ok(_) => {}
            Err(error) => push_issue(
                issues,
                errors,
                "error",
                "exports.pathInvalid",
                format!("exports[{index}]"),
                format!("Invalid manifest export {value:?}: {error}"),
            ),
        }
    }
}

struct RuntimeFields {
    runtime_skel: Option<String>,
    runtime_atlas: Option<String>,
    runtime_ready: Option<bool>,
}

fn validate_runtime_fields(
    manifest: &Value,
    exports_dir: &Path,
) -> Result<RuntimeFields, Vec<String>> {
    let mut errors = Vec::new();
    let runtime_skel =
        optional_runtime_path(manifest, "runtimeSkel", "skel", exports_dir, &mut errors);
    let runtime_atlas =
        optional_runtime_path(manifest, "runtimeAtlas", "atlas", exports_dir, &mut errors);
    let runtime_ready = match manifest.get("runtimeReady") {
        Some(Value::Bool(value)) => Some(*value),
        Some(_) => {
            errors.push("Manifest runtimeReady must be a boolean.".to_string());
            None
        }
        None => None,
    };
    if errors.is_empty() {
        Ok(RuntimeFields {
            runtime_skel,
            runtime_atlas,
            runtime_ready,
        })
    } else {
        Err(errors)
    }
}

fn optional_runtime_path(
    manifest: &Value,
    key: &str,
    extension: &str,
    exports_dir: &Path,
    errors: &mut Vec<String>,
) -> Option<String> {
    let value = manifest.get(key)?;
    let Some(value) = value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        errors.push(format!(
            "Manifest {key} must be a non-empty path relative to exports/."
        ));
        return None;
    };
    let path = match spine_assets::safe_relative_path(value) {
        Ok(path) => path,
        Err(error) => {
            errors.push(format!("Invalid manifest {key}: {error}"));
            return None;
        }
    };
    if path
        .extension()
        .and_then(|extension_value| extension_value.to_str())
        .is_none_or(|extension_value| !extension_value.eq_ignore_ascii_case(extension))
    {
        errors.push(format!("Manifest {key} must name a .{extension} file."));
        return None;
    }
    match contained_export_file(exports_dir, value) {
        Ok(_) => Some(normalize_relative(&path)),
        Err(error) => {
            errors.push(format!("Invalid manifest {key}: {error}"));
            None
        }
    }
}

fn choose_runtime_skel(
    skeletons: &[String],
    requested: Option<&str>,
) -> Result<Option<String>, String> {
    match (skeletons, requested) {
        ([], Some(_)) => {
            Err("Manifest runtimeSkel does not name an exported .skel file.".to_string())
        }
        ([], None) => Ok(None),
        (_, Some(requested)) if skeletons.iter().any(|skel| skel == requested) => {
            Ok(Some(requested.to_string()))
        }
        (_, Some(_)) => {
            Err("Manifest runtimeSkel does not name an exported .skel file.".to_string())
        }
        ([only], None) => Ok(Some(only.clone())),
        (_, None) => Err(
            "exports/ contains multiple .skel files; manifest runtimeSkel must select one."
                .to_string(),
        ),
    }
}

fn contained_export_file(exports_dir: &Path, value: &str) -> Result<PathBuf, String> {
    let relative = spine_assets::safe_relative_path(value)?;
    let root = exports_dir
        .canonicalize()
        .map_err(|error| format!("Cannot read exports/: {error}"))?;
    let candidate = root.join(relative);
    let canonical = candidate
        .canonicalize()
        .map_err(|_| "file does not exist under exports/.".to_string())?;
    if !canonical.starts_with(&root) || !canonical.is_file() {
        return Err("file must stay inside exports/.".to_string());
    }
    Ok(canonical)
}

fn contained_pack_file(root: &Path, value: &str) -> Result<PathBuf, String> {
    let relative = spine_assets::safe_relative_path(value)?;
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("Cannot read avatar pack: {error}"))?;
    let candidate = canonical_root.join(relative);
    let canonical = candidate
        .canonicalize()
        .map_err(|_| "file does not exist under the avatar pack.".to_string())?;
    if !canonical.starts_with(&canonical_root) || !canonical.is_file() {
        return Err("file must stay inside the avatar pack.".to_string());
    }
    Ok(canonical)
}

fn validate_pack_relative_file(
    root: &Path,
    value: &str,
    required_directory: Option<&str>,
) -> Result<PathBuf, String> {
    let relative = spine_assets::safe_relative_path(value)?;
    if let Some(required_directory) = required_directory {
        if relative
            .components()
            .next()
            .and_then(|value| value.as_os_str().to_str())
            != Some(required_directory)
        {
            return Err(format!("path must be inside {required_directory}/."));
        }
    }
    contained_pack_file(root, value)
}

fn choose_runtime_atlas(atlases: &[String], requested: Option<&str>) -> Option<String> {
    match requested {
        Some(requested) if atlases.iter().any(|atlas| atlas == requested) => {
            Some(requested.to_string())
        }
        None if atlases.len() == 1 => atlases.first().cloned(),
        _ => None,
    }
}

fn find_missing_attachments(
    exports_dir: &Path,
    atlases: &[String],
    attachments: &[String],
) -> Vec<String> {
    if attachments.is_empty() {
        return Vec::new();
    }
    let mut regions = HashSet::new();
    for atlas in atlases {
        let Ok(text) = fs::read_to_string(exports_dir.join(atlas)) else {
            continue;
        };
        for line in text.lines() {
            let trimmed = line.trim();
            let top_level = !line.chars().next().is_some_and(char::is_whitespace);
            let is_texture = Path::new(trimmed)
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    ["png", "jpg", "jpeg", "webp"]
                        .iter()
                        .any(|allowed| extension.eq_ignore_ascii_case(allowed))
                });
            if top_level && !trimmed.is_empty() && !trimmed.contains(':') && !is_texture {
                regions.insert(trimmed.to_string());
            }
        }
    }
    let mut missing = attachments
        .iter()
        .filter(|attachment| !regions.contains(attachment.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    missing.sort();
    missing.dedup();
    missing
}

fn read_spine_binary_version(path: &Path) -> Option<String> {
    let data = fs::read(path).ok()?;
    let mut offset = 0;
    read_spine_string(&data, &mut offset)?;
    read_spine_string(&data, &mut offset)
}

fn read_spine_string(data: &[u8], offset: &mut usize) -> Option<String> {
    let length = read_spine_varint(data, offset)?;
    if length == 0 {
        return Some(String::new());
    }
    let length = length.checked_sub(1)? as usize;
    let end = offset.checked_add(length)?;
    let value = std::str::from_utf8(data.get(*offset..end)?)
        .ok()?
        .to_string();
    *offset = end;
    Some(value)
}

fn read_spine_varint(data: &[u8], offset: &mut usize) -> Option<u32> {
    let mut value = 0_u32;
    for shift in (0..35).step_by(7) {
        let byte = *data.get(*offset)?;
        *offset += 1;
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

fn push_issue(
    issues: &mut Vec<AvatarValidationIssue>,
    legacy: &mut Vec<String>,
    severity: &str,
    code: &str,
    path: impl Into<String>,
    message: impl Into<String>,
) {
    let message = message.into();
    legacy.push(message.clone());
    issues.push(AvatarValidationIssue {
        severity: severity.to_string(),
        code: code.to_string(),
        path: path.into(),
        message,
    });
}

fn runtime_error_path(error: &str) -> &'static str {
    if error.contains("runtimeAtlas") {
        "runtimeAtlas"
    } else if error.contains("runtimeReady") {
        "runtimeReady"
    } else {
        "runtimeSkel"
    }
}

fn ensure_existing_pack_root(path: &Path) -> Result<(), String> {
    let root = path
        .canonicalize()
        .map_err(|error| format!("Cannot read avatar pack: {error}"))?;
    if !root.is_dir() || !root.join("avatar-pack.json").is_file() {
        return Err(
            "Avatar pack must be an existing directory containing avatar-pack.json.".to_string(),
        );
    }
    Ok(())
}

fn sibling_staging_path(target: &Path, operation: &str) -> Result<PathBuf, String> {
    let parent = target
        .parent()
        .ok_or_else(|| "Avatar pack path must have a parent directory.".to_string())?;
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Avatar pack path must end with a valid directory name.".to_string())?;
    Ok(parent.join(format!(".{name}.{operation}-{}", unique_suffix())))
}

fn non_empty_or(value: String, fallback: String) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback
    } else {
        value.to_string()
    }
}

fn update_registry(
    config_dir: &Path,
    validation: &AvatarValidation,
    runtime_path: Option<&Path>,
) -> Result<PathBuf, String> {
    let registry_path = avatar_registry_path(config_dir);
    if let Some(parent) = registry_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut items = load_registry(config_dir)?;
    items.retain(|item| item.get("id").and_then(Value::as_str) != Some(validation.id.as_str()));
    items.push(json!({
        "id": validation.id,
        "name": validation.name,
        "path": validation.path,
        "hasRuntimeExport": validation.has_runtime_export,
        "runtimeReady": validation.runtime_ready,
        "runtimeSkel": validation.runtime_skel,
        "runtimePath": runtime_path.map(|path| path.to_string_lossy().to_string())
    }));
    write_json_atomically(&registry_path, &items)?;
    Ok(registry_path)
}

fn write_json_atomically<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    let temp = path.with_extension(format!("json.tmp-{}", unique_suffix()));
    let data = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    fs::write(&temp, data).map_err(|error| error.to_string())?;
    replace_file_atomically(&temp, path)
}

fn replace_file_atomically(temp: &Path, target: &Path) -> Result<(), String> {
    let backup = target.with_extension(format!("json.backup-{}", unique_suffix()));
    let had_target = target.exists();
    if had_target {
        fs::rename(target, &backup).map_err(|error| error.to_string())?;
    }
    if let Err(error) = fs::rename(temp, target) {
        if had_target {
            let _ = fs::rename(&backup, target);
        }
        return Err(error.to_string());
    }
    if had_target {
        let _ = fs::remove_file(backup);
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    let root = source
        .canonicalize()
        .map_err(|error| format!("Cannot read runtime exports: {error}"))?;
    if !root.is_dir() {
        return Err("Runtime exports must be a directory.".to_string());
    }
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    copy_tree_inner(&root, &root, destination)
}

fn copy_runtime_files(source: &Path, destination: &Path, files: &[String]) -> Result<(), String> {
    if files.is_empty() || files.len() > MAX_RUNTIME_FILES {
        return Err(format!(
            "Runtime export must contain between 1 and {MAX_RUNTIME_FILES} required files."
        ));
    }
    let root = source
        .canonicalize()
        .map_err(|error| format!("Cannot read runtime exports: {error}"))?;
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    let mut seen = BTreeSet::new();
    let mut total = 0_u64;
    for relative in files {
        let relative = spine_assets::safe_relative_path(relative)?;
        if relative.components().count() > 8 {
            return Err(format!(
                "Runtime export path is nested too deeply: {}",
                relative.display()
            ));
        }
        let normalized = relative.to_string_lossy().replace('\\', "/");
        if !seen.insert(normalized) {
            continue;
        }
        let canonical = root
            .join(&relative)
            .canonicalize()
            .map_err(|error| format!("Cannot read required runtime file: {error}"))?;
        if !canonical.starts_with(&root) || !canonical.is_file() {
            return Err(format!(
                "Runtime export escapes exports/: {}",
                relative.display()
            ));
        }
        let size = canonical
            .metadata()
            .map_err(|error| error.to_string())?
            .len();
        if size > MAX_RUNTIME_FILE_BYTES {
            return Err(format!(
                "Runtime export file exceeds 64 MiB: {}",
                relative.display()
            ));
        }
        total = total
            .checked_add(size)
            .ok_or_else(|| "Runtime export size overflow.".to_string())?;
        if total > MAX_RUNTIME_TOTAL_BYTES {
            return Err("Runtime export exceeds 256 MiB.".to_string());
        }
        let destination_path = destination.join(&relative);
        fs::create_dir_all(
            destination_path
                .parent()
                .ok_or_else(|| "Runtime export has no parent directory.".to_string())?,
        )
        .map_err(|error| error.to_string())?;
        fs::copy(canonical, destination_path).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn copy_tree_inner(root: &Path, source: &Path, destination: &Path) -> Result<(), String> {
    let mut entries = fs::read_dir(source)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if file_type.is_symlink() {
            return Err(format!(
                "Runtime exports cannot contain symlinks: {}",
                source_path.to_string_lossy()
            ));
        }
        if file_type.is_dir() {
            fs::create_dir_all(&destination_path).map_err(|error| error.to_string())?;
            copy_tree_inner(root, &source_path, &destination_path)?;
        } else if file_type.is_file() {
            let canonical = source_path
                .canonicalize()
                .map_err(|error| error.to_string())?;
            if !canonical.starts_with(root) {
                return Err(format!(
                    "Runtime export escapes exports/: {}",
                    source_path.to_string_lossy()
                ));
            }
            fs::copy(&canonical, &destination_path).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn replace_directory_atomically(staging: &Path, target: &Path) -> Result<Option<PathBuf>, String> {
    let backup = target.with_file_name(format!(
        ".{}.backup-{}",
        target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("runtime"),
        unique_suffix()
    ));
    let had_target = target.exists();
    if had_target {
        fs::rename(target, &backup).map_err(|error| error.to_string())?;
    }
    if let Err(error) = fs::rename(staging, target) {
        if had_target {
            let _ = fs::rename(&backup, target);
        }
        return Err(error.to_string());
    }
    Ok(had_target.then_some(backup))
}

fn rollback_directory_replace(target: &Path, backup: Option<&Path>) -> Result<(), String> {
    if target.exists() {
        fs::remove_dir_all(target).map_err(|error| error.to_string())?;
    }
    if let Some(backup) = backup {
        fs::rename(backup, target).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn normalize_relative(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{}-{nanos}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_pack(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("spine-companion-avatar-{name}-{}", unique_suffix()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_draft_pack(root: &Path, id: &str) {
        fs::write(
            root.join("avatar-pack.json"),
            format!(r#"{{"id":"{id}","name":"Test Avatar","source":"local","licenseNote":"User-owned","layers":[],"motions":{{}}}}"#),
        )
        .unwrap();
        fs::write(root.join("preview.png"), []).unwrap();
        fs::create_dir_all(root.join("layers")).unwrap();
    }

    fn write_runtime(root: &Path, skel: &str) {
        let exports = root.join("exports");
        fs::create_dir_all(&exports).unwrap();
        fs::write(exports.join(skel), []).unwrap();
        fs::write(exports.join("avatar.atlas"), "avatar.png\nsize: 1,1\n").unwrap();
        fs::write(exports.join("avatar.png"), []).unwrap();
    }

    #[test]
    fn invalid_pack_reports_missing_files() {
        let root = temp_pack("invalid");
        let result = validate_pack(&root);
        assert!(!result.ok);
        assert!(
            result
                .errors
                .iter()
                .any(|item| item.contains("avatar-pack.json"))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn draft_pack_can_be_registered_without_runtime() {
        let root = temp_pack("draft");
        write_draft_pack(&root, "test_avatar");
        let validation = validate_pack(&root);
        assert!(validation.ok);
        assert!(validation.draft);
        assert!(!validation.runtime_ready);
        let registry = temp_pack("registry");
        let imported = import_pack(&root, &registry).unwrap();
        assert!(imported.imported);
        assert!(Path::new(&imported.registry_path).exists());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(registry);
    }

    #[test]
    fn rejects_unsafe_id_and_export_path() {
        let root = temp_pack("unsafe");
        write_draft_pack(&root, "../escape");
        fs::create_dir_all(root.join("exports")).unwrap();
        fs::write(
            root.join("avatar-pack.json"),
            r#"{"id":"../escape","name":"Test","source":"local","licenseNote":"User-owned","layers":[],"motions":{},"exports":["../outside.skel"]}"#,
        )
        .unwrap();
        let validation = validate_pack(&root);
        assert!(!validation.ok);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("unsafe"))
        );
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("Invalid manifest export"))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn requires_runtime_skel_for_multiple_exports() {
        let root = temp_pack("multiple-runtime");
        write_draft_pack(&root, "test_avatar");
        write_runtime(&root, "a.skel");
        fs::write(root.join("exports").join("b.skel"), []).unwrap();
        let validation = validate_pack(&root);
        assert!(!validation.ok);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("runtimeSkel"))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_skel_selects_one_of_multiple_exports() {
        let root = temp_pack("selected-runtime");
        write_draft_pack(&root, "test_avatar");
        write_runtime(&root, "a.skel");
        fs::write(root.join("exports").join("b.skel"), []).unwrap();
        fs::write(
            root.join("avatar-pack.json"),
            r#"{"id":"test_avatar","name":"Test Avatar","source":"local","licenseNote":"User-owned","layers":[],"motions":{},"runtimeSkel":"b.skel"}"#,
        )
        .unwrap();
        let validation = validate_pack(&root);
        assert!(validation.ok);
        assert!(validation.runtime_ready);
        assert_eq!(validation.runtime_skel, "b.skel");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn installs_runtime_with_atomic_registry_update() {
        let root = temp_pack("runtime");
        write_draft_pack(&root, "test_avatar");
        write_runtime(&root, "avatar.skel");
        assert!(
            spine_assets::validate_spine_asset_selection(&root.join("exports").join("avatar.skel"))
                .is_ok()
        );
        let config = temp_pack("runtime-config");
        let installed = install_runtime_pack(&root, &config).unwrap();
        assert!(installed.installed);
        assert_eq!(installed.model_id, "avatar--test_avatar");
        assert!(
            Path::new(&installed.runtime_path)
                .join("avatar.skel")
                .is_file()
        );
        assert!(
            Path::new(&installed.runtime_path)
                .join(".companion-model.json")
                .is_file()
        );
        let metadata: Value = serde_json::from_slice(
            &fs::read(Path::new(&installed.runtime_path).join(".companion-model.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(metadata["id"], "avatar--test_avatar");
        let registry = load_registry(&config).unwrap();
        assert_eq!(registry.len(), 1);
        assert_eq!(registry[0]["runtimeSkel"], "avatar.skel");
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(config);
    }

    #[test]
    fn missing_atlas_texture_keeps_pack_as_draft() {
        let root = temp_pack("missing-texture");
        write_draft_pack(&root, "test_avatar");
        write_runtime(&root, "avatar.skel");
        fs::remove_file(root.join("exports").join("avatar.png")).unwrap();
        let validation = validate_pack(&root);
        assert!(validation.ok);
        assert!(!validation.runtime_ready);
        assert!(
            validation
                .runtime_errors
                .iter()
                .any(|error| error.contains("texture"))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failed_directory_replace_restores_previous_model() {
        let root = temp_pack("replace-rollback");
        let target = root.join("model");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("previous.txt"), "previous").unwrap();
        let missing_staging = root.join("missing-staging");
        assert!(replace_directory_atomically(&missing_staging, &target).is_err());
        assert_eq!(
            fs::read_to_string(target.join("previous.txt")).unwrap(),
            "previous"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reinstall_replaces_runtime_and_drops_unreferenced_exports() {
        let root = temp_pack("reinstall");
        write_draft_pack(&root, "test_avatar");
        write_runtime(&root, "avatar.skel");
        fs::write(root.join("exports").join("old.txt"), "old").unwrap();
        let config = temp_pack("reinstall-config");
        install_runtime_pack(&root, &config).unwrap();

        fs::remove_file(root.join("exports").join("old.txt")).unwrap();
        fs::write(root.join("exports").join("new.txt"), "new").unwrap();
        let installed = install_runtime_pack(&root, &config).unwrap();
        let runtime = Path::new(&installed.runtime_path);
        assert!(!runtime.join("old.txt").exists());
        assert!(!runtime.join("new.txt").exists());
        assert!(runtime.join("avatar.skel").is_file());
        assert_eq!(load_registry(&config).unwrap().len(), 1);
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(config);
    }

    #[test]
    fn runtime_install_rejects_an_oversized_required_texture() {
        let root = temp_pack("oversized-runtime");
        write_draft_pack(&root, "test_avatar");
        write_runtime(&root, "avatar.skel");
        fs::OpenOptions::new()
            .write(true)
            .open(root.join("exports").join("avatar.png"))
            .unwrap()
            .set_len(MAX_RUNTIME_FILE_BYTES + 1)
            .unwrap();
        let config = temp_pack("oversized-runtime-config");
        let error = install_runtime_pack(&root, &config).unwrap_err();
        assert!(error.contains("64 MiB"));
        assert!(!avatar_runtime_dir(&config, "test_avatar").unwrap().exists());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(config);
    }

    #[test]
    fn avatar_runtime_uses_a_reserved_model_directory() {
        let config = Path::new("C:/companion");
        assert_eq!(
            avatar_runtime_dir(config, "my-avatar").unwrap(),
            config.join("models").join("avatar--my-avatar")
        );
    }
}
