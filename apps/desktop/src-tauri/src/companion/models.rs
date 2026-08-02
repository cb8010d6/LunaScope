use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use super::{
    CompanionModelKind, MAX_MODEL_FILE_BYTES, MAX_MODEL_TOTAL_BYTES,
    catalog::{CatalogDocument, CatalogFile, CatalogModel},
    companion_root, display_error, read_settings, require_main_window, set_window_visibility,
    write_settings,
};

const CATALOGS: [(&str, &str); 3] = [
    (
        "ark-models",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/resources/companion/catalog/operators.json"
        )),
    ),
    (
        "ark-illustrations",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/resources/companion/catalog/illustrations.json"
        )),
    ),
    (
        "ark-enemies",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/resources/companion/catalog/enemies.json"
        )),
    ),
];

const CATALOG_PAGE_SIZE: usize = 18;
const MAX_PRELOAD_MODELS: usize = 4;
const MAX_PRELOAD_SESSION_BYTES: u64 = 128 * 1024 * 1024;
const LIVE2D_CATALOG: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/companion/catalog/live2d-official.json"
));

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Live2dCatalogDocument {
    schema_version: u32,
    models: Vec<Live2dCatalogModel>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Live2dCatalogModel {
    id: String,
    name: String,
    model_kind: CompanionModelKind,
    entry_file: String,
    source: String,
    author: String,
    license: String,
    license_warning: String,
    license_note: String,
    repository_url: String,
    license_url: String,
    terms_url: String,
    description: String,
    category: String,
    compatibility_profile: String,
    files: Vec<CatalogFile>,
}

#[derive(Clone, Copy)]
enum CatalogCandidate<'a> {
    Spine(&'a CatalogModel),
    Live2d(&'a Live2dCatalogModel),
}

impl<'a> CatalogCandidate<'a> {
    fn id(self) -> &'a str {
        match self {
            Self::Spine(model) => &model.id,
            Self::Live2d(model) => &model.id,
        }
    }

    fn name(self) -> &'a str {
        match self {
            Self::Spine(model) => &model.name,
            Self::Live2d(model) => &model.name,
        }
    }

    fn source(self) -> &'a str {
        match self {
            Self::Spine(model) => &model.source,
            Self::Live2d(model) => &model.source,
        }
    }

    fn category(self) -> &'a str {
        match self {
            Self::Spine(model) => &model.category,
            Self::Live2d(model) => &model.category,
        }
    }

    fn description(self) -> &'a str {
        match self {
            Self::Spine(model) => &model.description,
            Self::Live2d(model) => &model.description,
        }
    }

    fn matches_query(self, query: &str) -> bool {
        query.is_empty()
            || [self.name(), self.id(), self.source(), self.description()]
                .iter()
                .any(|value| value.to_lowercase().contains(query))
            || match self {
                Self::Spine(model) => model
                    .tags
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(query)),
                Self::Live2d(_) => ["live2d", "cubism", "official", "sample"]
                    .iter()
                    .any(|tag| tag.contains(query)),
            }
    }

    fn to_view(self, installed_ids: &BTreeSet<&str>, active: Option<&str>) -> CatalogModelView {
        match self {
            Self::Spine(model) => CatalogModelView {
                id: model.id.clone(),
                name: model.name.clone(),
                source: model.source.clone(),
                author: model.author.clone(),
                license: model.license.clone(),
                license_warning: model.license_warning.clone(),
                license_note: model.license_note.clone(),
                repository_url: model.repository_url.clone(),
                description: model.description.clone(),
                category: model.category.clone(),
                compatibility_profile: model.compatibility_profile.clone(),
                model_kind: CompanionModelKind::Spine38,
                license_url: None,
                terms_url: None,
                installed: installed_ids.contains(model.id.as_str()),
                active: active == Some(model.id.as_str()),
            },
            Self::Live2d(model) => CatalogModelView {
                id: model.id.clone(),
                name: model.name.clone(),
                source: model.source.clone(),
                author: model.author.clone(),
                license: model.license.clone(),
                license_warning: model.license_warning.clone(),
                license_note: model.license_note.clone(),
                repository_url: model.repository_url.clone(),
                description: model.description.clone(),
                category: model.category.clone(),
                compatibility_profile: model.compatibility_profile.clone(),
                model_kind: CompanionModelKind::Live2d,
                license_url: Some(model.license_url.clone()),
                terms_url: Some(model.terms_url.clone()),
                installed: installed_ids.contains(model.id.as_str()),
                active: active == Some(model.id.as_str()),
            },
        }
    }
}

impl Live2dCatalogModel {
    fn validate(&self) -> Result<(), String> {
        if !safe_model_id(&self.id)
            || self.name.trim().is_empty()
            || self.source.trim().is_empty()
            || !matches!(self.model_kind, CompanionModelKind::Live2d)
        {
            return Err("Live2D catalog model identity is invalid".into());
        }
        if !self
            .entry_file
            .to_ascii_lowercase()
            .ends_with(".model3.json")
            || Path::new(&self.entry_file).is_absolute()
            || self.entry_file.contains("..")
        {
            return Err(format!("Live2D entry file is unsafe: {}", self.entry_file));
        }
        for url in [&self.repository_url, &self.license_url, &self.terms_url] {
            if !url.starts_with("https://") {
                return Err("Live2D catalog links must use HTTPS".into());
            }
        }
        let mut names = BTreeSet::new();
        let mut has_entry = false;
        let mut has_moc = false;
        let mut has_texture = false;
        for file in &self.files {
            file.validate()?;
            if !names.insert(file.name.as_str()) {
                return Err(format!(
                    "Live2D catalog contains duplicate file: {}",
                    file.name
                ));
            }
            has_entry |= file.name == self.entry_file;
            let lower = file.name.to_ascii_lowercase();
            has_moc |= lower.ends_with(".moc3");
            has_texture |= [".png", ".jpg", ".jpeg", ".webp"]
                .iter()
                .any(|extension| lower.ends_with(extension));
        }
        if !has_entry || !has_moc || !has_texture {
            return Err(format!(
                "Live2D catalog model {} needs its manifest, Moc, and textures",
                self.id
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogModelView {
    id: String,
    name: String,
    source: String,
    author: String,
    license: String,
    license_warning: String,
    license_note: String,
    repository_url: String,
    description: String,
    category: String,
    compatibility_profile: String,
    model_kind: CompanionModelKind,
    license_url: Option<String>,
    terms_url: Option<String>,
    installed: bool,
    active: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogSearchView {
    models: Vec<CatalogModelView>,
    sources: Vec<String>,
    categories: Vec<String>,
    total: usize,
    page: usize,
    page_size: usize,
    total_pages: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstalledModelView {
    id: String,
    name: String,
    model_kind: CompanionModelKind,
    source: String,
    license: String,
    path: String,
    atlas_path: Option<String>,
    live2d_core_path: Option<String>,
    active: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    model_id: String,
    completed_files: usize,
    total_files: usize,
    current_file: String,
    phase: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreloadedModelView {
    model_id: String,
    name: String,
    model_path: String,
    atlas_path: String,
    model_kind: CompanionModelKind,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreloadFailure {
    model_id: String,
    message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogPreloadResult {
    models: Vec<PreloadedModelView>,
    failures: Vec<PreloadFailure>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogLicenseAcceptance {
    accepted: bool,
    model_id: String,
    repository_url: String,
    license_url: String,
    terms_url: String,
}

fn catalog_models() -> Result<&'static Vec<CatalogModel>, String> {
    static MODELS: OnceLock<Result<Vec<CatalogModel>, String>> = OnceLock::new();
    MODELS
        .get_or_init(|| {
            let mut models = Vec::new();
            for (_, raw) in CATALOGS {
                let document: CatalogDocument = serde_json::from_str(raw)
                    .map_err(|error| format!("Bundled companion catalog is invalid: {error}"))?;
                document.validate()?;
                models.extend(document.models);
            }
            Ok(models)
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn live2d_catalog_models() -> Result<&'static Vec<Live2dCatalogModel>, String> {
    static MODELS: OnceLock<Result<Vec<Live2dCatalogModel>, String>> = OnceLock::new();
    MODELS
        .get_or_init(|| {
            let document: Live2dCatalogDocument = serde_json::from_str(LIVE2D_CATALOG)
                .map_err(|error| format!("Bundled Live2D catalog is invalid: {error}"))?;
            if document.schema_version != 1 {
                return Err("Unsupported bundled Live2D catalog schema".into());
            }
            let mut ids = BTreeSet::new();
            for model in &document.models {
                model.validate()?;
                if !ids.insert(model.id.as_str()) {
                    return Err(format!("Duplicate Live2D catalog model: {}", model.id));
                }
            }
            Ok(document.models)
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn models_root() -> Result<PathBuf, String> {
    Ok(companion_root()?.join("models"))
}

fn preload_root() -> Result<PathBuf, String> {
    Ok(companion_root()?.join("preload"))
}

fn catalog_install_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn preload_cache_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn preload_generation() -> &'static AtomicU64 {
    static GENERATION: AtomicU64 = AtomicU64::new(0);
    &GENERATION
}

fn safe_model_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

fn installed_model_id(stored_id: &str, source: &str) -> String {
    if source == "avatar-studio" && !stored_id.starts_with("avatar--") {
        format!("avatar--{stored_id}")
    } else {
        stored_id.to_owned()
    }
}

fn metadata_path(directory: &Path) -> PathBuf {
    directory.join(".companion-model.json")
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn catalog_atlas_file(model: &CatalogModel) -> Option<&super::catalog::CatalogFile> {
    let expected = Path::new(&model.skel)
        .with_extension("atlas")
        .to_string_lossy()
        .into_owned();
    model
        .files
        .iter()
        .filter(|file| {
            Path::new(&file.name)
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("atlas"))
        })
        .find(|file| file.name.eq_ignore_ascii_case(&expected))
        .or_else(|| {
            model.files.iter().find(|file| {
                Path::new(&file.name)
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("atlas"))
            })
        })
}

fn catalog_metadata_bytes(model: &CatalogModel) -> Result<Vec<u8>, String> {
    let mut metadata = serde_json::to_value(model).map_err(display_error)?;
    if let Some(object) = metadata.as_object_mut() {
        object.insert("modelKind".into(), json!("spine38"));
        object.insert("entryFile".into(), json!(model.skel));
        if let Some(atlas) = catalog_atlas_file(model).map(|file| file.name.clone()) {
            object.insert("atlasFile".into(), json!(atlas));
        }
    }
    serde_json::to_vec_pretty(&metadata).map_err(display_error)
}

fn live2d_catalog_metadata_bytes(model: &Live2dCatalogModel) -> Result<Vec<u8>, String> {
    let mut metadata = serde_json::to_value(model).map_err(display_error)?;
    if let Some(object) = metadata.as_object_mut() {
        object.insert("modelKind".into(), json!("live2d"));
        object.insert("entryFile".into(), json!(model.entry_file));
    }
    serde_json::to_vec_pretty(&metadata).map_err(display_error)
}

fn write_catalog_download(
    directory: &Path,
    file: &CatalogFile,
    bytes: &[u8],
) -> Result<(), String> {
    let destination = directory.join(&file.name);
    let parent = destination
        .parent()
        .ok_or("catalog file has no parent directory")?;
    fs::create_dir_all(parent).map_err(display_error)?;
    fs::write(destination, bytes).map_err(display_error)
}

fn checked_directory_size(path: &Path) -> Result<u64, String> {
    if !path.exists() {
        return Ok(0);
    }
    let metadata = fs::symlink_metadata(path).map_err(display_error)?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "preload cache contains an unsupported symbolic link: {}",
            path.display()
        ));
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Err(format!(
            "preload cache contains an unsupported entry: {}",
            path.display()
        ));
    }
    fs::read_dir(path)
        .map_err(display_error)?
        .try_fold(0_u64, |total, entry| {
            let entry = entry.map_err(display_error)?;
            let size = checked_directory_size(&entry.path())?;
            total
                .checked_add(size)
                .ok_or("preload cache size overflow".into())
        })
}

fn ensure_preload_capacity(current: u64, additional: u64) -> Result<u64, String> {
    let total = current
        .checked_add(additional)
        .ok_or("preload cache size overflow")?;
    if total > MAX_PRELOAD_SESSION_BYTES {
        return Err("preload session cache exceeds 128 MiB".into());
    }
    Ok(total)
}

fn file_matches_catalog(path: &Path, file: &super::catalog::CatalogFile) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata,
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(display_error(error)),
    };
    if metadata.len() > MAX_MODEL_FILE_BYTES
        || file
            .size_bytes
            .is_some_and(|expected| expected != metadata.len())
    {
        return Ok(false);
    }
    let bytes = fs::read(path).map_err(display_error)?;
    let valid = if !file.sha256.is_empty() {
        hex_digest(Sha256::digest(&bytes)).eq_ignore_ascii_case(&file.sha256)
    } else {
        let mut digest = Sha1::new();
        digest.update(format!("blob {}\0", bytes.len()));
        digest.update(&bytes);
        hex_digest(digest.finalize()).eq_ignore_ascii_case(&file.github_blob_sha)
    };
    Ok(valid)
}

fn cached_preload_is_complete(directory: &Path, model: &CatalogModel) -> Result<bool, String> {
    let directory_metadata = match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => metadata,
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(display_error(error)),
    };
    if !directory_metadata.is_dir() {
        return Ok(false);
    }
    let metadata_path = metadata_path(directory);
    let metadata: Value = match fs::read(&metadata_path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(metadata) => metadata,
            Err(_) => return Ok(false),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(display_error(error)),
    };
    let atlas = catalog_atlas_file(model)
        .ok_or_else(|| format!("catalog model {} has no atlas", model.id))?;
    if metadata.get("id").and_then(Value::as_str) != Some(model.id.as_str())
        || metadata.get("entryFile").and_then(Value::as_str) != Some(model.skel.as_str())
        || metadata.get("atlasFile").and_then(Value::as_str) != Some(atlas.name.as_str())
        || metadata.get("modelKind").and_then(Value::as_str) != Some("spine38")
    {
        return Ok(false);
    }

    let mut expected = model
        .files
        .iter()
        .map(|file| file.name.as_str())
        .collect::<BTreeSet<_>>();
    expected.insert(".companion-model.json");
    let actual = fs::read_dir(directory)
        .map_err(display_error)?
        .map(|entry| {
            entry
                .map_err(display_error)?
                .file_name()
                .into_string()
                .map_err(|_| "preload cache contains a non-UTF-8 file name".to_string())
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if actual.iter().map(String::as_str).collect::<BTreeSet<_>>() != expected {
        return Ok(false);
    }
    for file in &model.files {
        if !file_matches_catalog(&directory.join(&file.name), file)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn remove_preload_entry(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(display_error)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path).map_err(display_error)
    } else {
        fs::remove_file(path).map_err(display_error)
    }
}

fn prune_preload_cache(root: &Path, keep: &BTreeSet<&str>) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(display_error)? {
        let entry = entry.map_err(display_error)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "preload cache contains a non-UTF-8 entry".to_string())?;
        if !keep.contains(name.as_str()) {
            remove_preload_entry(&entry.path())?;
        }
    }
    Ok(())
}

fn preload_is_current(generation: u64) -> bool {
    preload_generation().load(Ordering::Acquire) == generation
}

fn resolve_preload_models(model_ids: &[String]) -> Result<Vec<CatalogModel>, String> {
    if model_ids.len() > MAX_PRELOAD_MODELS {
        return Err("at most 4 catalog models can be preloaded at once".into());
    }
    let mut unique = BTreeSet::new();
    let catalog = catalog_models()?;
    model_ids
        .iter()
        .map(|model_id| {
            if !safe_model_id(model_id) {
                return Err(format!("invalid catalog model id: {model_id}"));
            }
            if !unique.insert(model_id.as_str()) {
                return Err(format!("catalog model id is repeated: {model_id}"));
            }
            catalog
                .iter()
                .find(|model| model.id == *model_id)
                .cloned()
                .ok_or_else(|| format!("catalog model was not found: {model_id}"))
        })
        .collect()
}

fn validate_live2d_license_acceptance(
    model: &Live2dCatalogModel,
    acceptance: Option<&CatalogLicenseAcceptance>,
) -> Result<(), String> {
    let acceptance = acceptance.ok_or("Live2D license acceptance is required")?;
    if !acceptance.accepted
        || acceptance.model_id != model.id
        || acceptance.repository_url != model.repository_url
        || acceptance.license_url != model.license_url
        || acceptance.terms_url != model.terms_url
    {
        return Err("Live2D license acceptance does not match the pinned catalog entry".into());
    }
    Ok(())
}

fn preloaded_model_view(
    directory: &Path,
    model: &CatalogModel,
) -> Result<PreloadedModelView, String> {
    let atlas = catalog_atlas_file(model)
        .ok_or_else(|| format!("catalog model {} has no atlas", model.id))?;
    Ok(PreloadedModelView {
        model_id: model.id.clone(),
        name: model.name.clone(),
        model_path: directory.join(&model.skel).to_string_lossy().into_owned(),
        atlas_path: directory.join(&atlas.name).to_string_lossy().into_owned(),
        model_kind: CompanionModelKind::Spine38,
    })
}

fn cleanup_preload_cache_at(root: &Path) -> Result<(), String> {
    let preload = root.join("preload");
    if !preload.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(&preload).map_err(display_error)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(preload).map_err(display_error)
    } else {
        fs::remove_file(preload).map_err(display_error)
    }
}

pub(crate) async fn cleanup_preload_cache() -> Result<(), String> {
    let _guard = preload_cache_lock().lock().await;
    cleanup_preload_cache_at(&companion_root()?)
}

pub(super) fn write_local_model_metadata(
    directory: &Path,
    id: &str,
    name: &str,
    kind: CompanionModelKind,
    entry_file: &str,
    atlas_file: Option<&str>,
    source: &str,
) -> Result<(), String> {
    let metadata = json!({
        "id": id,
        "name": name,
        "modelKind": kind,
        "entryFile": entry_file,
        "atlasFile": atlas_file,
        "source": source,
        "license": "USER_PROVIDED"
    });
    fs::write(
        metadata_path(directory),
        serde_json::to_vec_pretty(&metadata).map_err(display_error)?,
    )
    .map_err(display_error)
}

fn installed_models() -> Result<Vec<InstalledModelView>, String> {
    let root = models_root()?;
    fs::create_dir_all(&root).map_err(display_error)?;
    let settings = read_settings()?;
    let active_path = settings.model_path.as_deref().map(Path::new);
    let core_path = companion_root()?
        .join("runtime")
        .join("live2dcubismcore.min.js");
    let live2d_core_path =
        super::live2d_core_is_pinned(&core_path).then(|| core_path.to_string_lossy().into_owned());
    let mut installed = Vec::new();
    for entry in fs::read_dir(&root).map_err(display_error)? {
        let entry = entry.map_err(display_error)?;
        if !entry.file_type().map_err(display_error)?.is_dir() {
            continue;
        }
        let metadata_path = metadata_path(&entry.path());
        if !metadata_path.is_file() {
            continue;
        }
        let metadata: Value =
            serde_json::from_slice(&fs::read(metadata_path).map_err(display_error)?)
                .map_err(display_error)?;
        let stored_id = metadata
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let source = metadata
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("local");
        let id = installed_model_id(stored_id, source);
        let entry_file = metadata
            .get("entryFile")
            .or_else(|| metadata.get("skel"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !safe_model_id(&id) || entry_file.is_empty() {
            continue;
        }
        let model_path = entry.path().join(entry_file);
        if !model_path.is_file() {
            continue;
        }
        let model_kind = match metadata.get("modelKind").and_then(Value::as_str) {
            Some("live2d") => CompanionModelKind::Live2d,
            _ => CompanionModelKind::Spine38,
        };
        let expected_atlas = Path::new(entry_file)
            .with_extension("atlas")
            .to_string_lossy()
            .into_owned();
        let catalog_atlases = metadata
            .get("files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|file| file.get("name").and_then(Value::as_str))
            .filter(|name| {
                Path::new(name)
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("atlas"))
            })
            .collect::<Vec<_>>();
        let atlas_file = metadata
            .get("atlasFile")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                catalog_atlases
                    .iter()
                    .copied()
                    .find(|name| name.eq_ignore_ascii_case(&expected_atlas))
            })
            .or_else(|| catalog_atlases.first().copied());
        let atlas_path = atlas_file
            .map(|value| entry.path().join(value))
            .filter(|path| path.is_file())
            .map(|path| path.to_string_lossy().into_owned());
        installed.push(InstalledModelView {
            id,
            name: metadata
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(stored_id)
                .to_owned(),
            model_kind,
            source: source.to_owned(),
            license: metadata
                .get("license")
                .and_then(Value::as_str)
                .unwrap_or("NOASSERTION")
                .to_owned(),
            path: model_path.to_string_lossy().into_owned(),
            atlas_path,
            live2d_core_path: matches!(model_kind, CompanionModelKind::Live2d)
                .then(|| live2d_core_path.clone())
                .flatten(),
            active: active_path.is_some_and(|active| active == model_path),
        });
    }
    if !installed.iter().any(|item| item.active)
        && let Some(model_path) = active_path.filter(|path| path.is_file())
    {
        let fallback_name = model_path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("local-model");
        let slug = fallback_name
            .to_ascii_lowercase()
            .bytes()
            .map(|byte| {
                if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
                    byte as char
                } else {
                    '-'
                }
            })
            .collect::<String>();
        installed.push(InstalledModelView {
            id: format!("local-{}", slug.trim_matches('-')),
            name: settings
                .model_name
                .clone()
                .unwrap_or_else(|| fallback_name.to_string()),
            model_kind: settings.model_kind,
            source: "local".to_string(),
            license: "USER_PROVIDED".to_string(),
            path: model_path.to_string_lossy().into_owned(),
            atlas_path: settings.atlas_path.clone(),
            live2d_core_path: matches!(settings.model_kind, CompanionModelKind::Live2d)
                .then(|| live2d_core_path.clone())
                .flatten(),
            active: true,
        });
    }
    installed.sort_by_key(|item| item.name.to_lowercase());
    Ok(installed)
}

#[tauri::command]
pub(crate) fn companion_search_catalog(
    window: tauri::WebviewWindow,
    query: Option<String>,
    source: Option<String>,
    category: Option<String>,
    page: Option<usize>,
) -> Result<CatalogSearchView, String> {
    require_main_window(&window)?;
    let query = query.unwrap_or_default().trim().to_lowercase();
    let source_filter = source.filter(|value| !value.is_empty());
    let category_filter = category.filter(|value| !value.is_empty());
    let page = page.unwrap_or(1).max(1);
    let installed = installed_models()?;
    let active = installed
        .iter()
        .find(|item| item.active)
        .map(|item| item.id.as_str());
    let installed_ids = installed
        .iter()
        .map(|item| item.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let mut candidates = catalog_models()?
        .iter()
        .map(CatalogCandidate::Spine)
        .chain(
            live2d_catalog_models()?
                .iter()
                .map(CatalogCandidate::Live2d),
        )
        .collect::<Vec<_>>();
    let mut sources = candidates
        .iter()
        .map(|model| model.source().to_owned())
        .collect::<Vec<_>>();
    sources.sort();
    sources.dedup();
    let mut categories = candidates
        .iter()
        .map(|model| model.category().to_owned())
        .collect::<Vec<_>>();
    categories.sort();
    categories.dedup();
    candidates.retain(|model| {
        source_filter
            .as_ref()
            .is_none_or(|source| model.source() == source)
            && category_filter
                .as_ref()
                .is_none_or(|category| model.category() == category)
            && model.matches_query(&query)
    });
    candidates.sort_by_key(|model| model.name().to_lowercase());
    let total = candidates.len();
    let total_pages = total.div_ceil(CATALOG_PAGE_SIZE).max(1);
    let page = page.min(total_pages);
    let start = (page - 1) * CATALOG_PAGE_SIZE;
    let views = candidates
        .into_iter()
        .skip(start)
        .take(CATALOG_PAGE_SIZE)
        .map(|model| model.to_view(&installed_ids, active))
        .collect::<Vec<_>>();
    Ok(CatalogSearchView {
        models: views,
        sources,
        categories,
        total,
        page,
        page_size: CATALOG_PAGE_SIZE,
        total_pages,
    })
}

#[tauri::command]
pub(crate) fn companion_list_installed_models(
    window: tauri::WebviewWindow,
) -> Result<Vec<InstalledModelView>, String> {
    require_main_window(&window)?;
    installed_models()
}

async fn download_file(
    client: &reqwest::Client,
    file: &super::catalog::CatalogFile,
) -> Result<Vec<u8>, String> {
    let mut last_error = String::new();
    for url in std::iter::once(&file.url).chain(file.fallback_urls.iter()) {
        let response = match client.get(url).send().await {
            Ok(response) if response.status().is_success() => response,
            Ok(response) => {
                last_error = format!("{} returned {}", url, response.status());
                continue;
            }
            Err(error) => {
                last_error = error.to_string();
                continue;
            }
        };
        if response
            .content_length()
            .is_some_and(|size| size > MAX_MODEL_FILE_BYTES)
        {
            last_error = format!("{} exceeds 64 MiB", file.name);
            continue;
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        let mut stream_failed = false;
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    last_error = format!("{} stream failed: {error}", file.name);
                    stream_failed = true;
                    break;
                }
            };
            if bytes.len().saturating_add(chunk.len()) > MAX_MODEL_FILE_BYTES as usize {
                return Err(format!("{} exceeds 64 MiB", file.name));
            }
            bytes.extend_from_slice(&chunk);
        }
        if stream_failed {
            continue;
        }
        if file
            .size_bytes
            .is_some_and(|expected| expected != bytes.len() as u64)
        {
            last_error = format!("{} size does not match catalog metadata", file.name);
            continue;
        }
        let valid = if !file.sha256.is_empty() {
            hex_digest(Sha256::digest(&bytes)).eq_ignore_ascii_case(&file.sha256)
        } else {
            let mut digest = Sha1::new();
            digest.update(format!("blob {}\0", bytes.len()));
            digest.update(&bytes);
            hex_digest(digest.finalize()).eq_ignore_ascii_case(&file.github_blob_sha)
        };
        if valid {
            return Ok(bytes);
        }
        last_error = format!("{} digest verification failed", file.name);
    }
    Err(last_error)
}

#[tauri::command]
pub(crate) async fn companion_preload_catalog_models(
    window: tauri::WebviewWindow,
    model_ids: Vec<String>,
) -> Result<CatalogPreloadResult, String> {
    require_main_window(&window)?;
    let models = resolve_preload_models(&model_ids)?;
    let generation = preload_generation().fetch_add(1, Ordering::AcqRel) + 1;
    let _guard = preload_cache_lock().lock().await;
    if !preload_is_current(generation) {
        return Ok(CatalogPreloadResult {
            models: Vec::new(),
            failures: Vec::new(),
        });
    }
    let root = preload_root()?;
    fs::create_dir_all(&root).map_err(display_error)?;
    let keep = model_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    prune_preload_cache(&root, &keep)?;
    let mut cache_bytes = checked_directory_size(&root)?;
    if cache_bytes > MAX_PRELOAD_SESSION_BYTES {
        return Err("preload session cache exceeds 128 MiB".into());
    }

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(display_error)?;
    let mut preloaded = Vec::with_capacity(models.len());
    let mut failures = Vec::new();
    for model in models {
        if !preload_is_current(generation) {
            break;
        }
        if let Err(message) = model.validate() {
            failures.push(PreloadFailure {
                model_id: model.id.clone(),
                message,
            });
            continue;
        }
        let destination = root.join(&model.id);
        let destination_metadata = fs::symlink_metadata(&destination).ok();
        if destination_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
            && cached_preload_is_complete(&destination, &model)?
        {
            preloaded.push(preloaded_model_view(&destination, &model)?);
            continue;
        }
        if destination_metadata.is_some() {
            let invalid_bytes = checked_directory_size(&destination)?;
            let metadata = fs::symlink_metadata(&destination).map_err(display_error)?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                fs::remove_dir_all(&destination).map_err(display_error)?;
            } else {
                fs::remove_file(&destination).map_err(display_error)?;
            }
            cache_bytes = cache_bytes.saturating_sub(invalid_bytes);
        }

        let staging = root.join(format!(".{}.{}.download", model.id, Uuid::new_v4()));
        fs::create_dir(&staging).map_err(display_error)?;
        let result = async {
            let metadata = catalog_metadata_bytes(&model)?;
            let mut staging_bytes =
                ensure_preload_capacity(cache_bytes, metadata.len() as u64)? - cache_bytes;
            fs::write(metadata_path(&staging), metadata).map_err(display_error)?;
            let mut model_bytes = 0_u64;
            for file in &model.files {
                if !preload_is_current(generation) {
                    return Err("preload request was superseded".into());
                }
                let bytes = download_file(&client, file).await?;
                if !preload_is_current(generation) {
                    return Err("preload request was superseded".into());
                }
                model_bytes = model_bytes
                    .checked_add(bytes.len() as u64)
                    .ok_or("model size overflow")?;
                if model_bytes > MAX_MODEL_TOTAL_BYTES {
                    return Err("model exceeds 256 MiB".into());
                }
                staging_bytes = staging_bytes
                    .checked_add(bytes.len() as u64)
                    .ok_or("preload cache size overflow")?;
                ensure_preload_capacity(cache_bytes, staging_bytes)?;
                write_catalog_download(&staging, file, &bytes)?;
            }
            if !cached_preload_is_complete(&staging, &model)? {
                return Err("downloaded preload cache failed final verification".into());
            }
            fs::rename(&staging, &destination).map_err(display_error)?;
            Ok::<u64, String>(staging_bytes)
        }
        .await;
        let staged_bytes = match result {
            Ok(bytes) => bytes,
            Err(error) => {
                let _ = fs::remove_dir_all(&staging);
                if !preload_is_current(generation) {
                    break;
                }
                failures.push(PreloadFailure {
                    model_id: model.id.clone(),
                    message: error,
                });
                continue;
            }
        };
        cache_bytes = ensure_preload_capacity(cache_bytes, staged_bytes)?;
        preloaded.push(preloaded_model_view(&destination, &model)?);
    }
    if !preload_is_current(generation) {
        return Ok(CatalogPreloadResult {
            models: Vec::new(),
            failures: Vec::new(),
        });
    }
    Ok(CatalogPreloadResult {
        models: preloaded,
        failures,
    })
}

#[tauri::command]
pub(crate) async fn companion_install_catalog_model(
    app: AppHandle,
    window: tauri::WebviewWindow,
    model_id: String,
    license_acceptance: Option<CatalogLicenseAcceptance>,
) -> Result<InstalledModelView, String> {
    require_main_window(&window)?;
    let _install_guard = catalog_install_lock().lock().await;
    if let Some(model) = live2d_catalog_models()?
        .iter()
        .find(|model| model.id == model_id)
        .cloned()
    {
        validate_live2d_license_acceptance(&model, license_acceptance.as_ref())?;
        return install_live2d_catalog_model(&app, model).await;
    }
    let model = catalog_models()?
        .iter()
        .find(|model| model.id == model_id)
        .cloned()
        .ok_or("catalog model was not found")?;
    model.validate()?;
    let root = models_root()?;
    fs::create_dir_all(&root).map_err(display_error)?;
    let destination = root.join(&model.id);
    if destination.exists() {
        return installed_models()?
            .into_iter()
            .find(|item| item.id == model.id)
            .ok_or("model destination exists but is not a valid installation".into());
    }
    let staging = root.join(format!(".{}.{}.download", model.id, Uuid::new_v4()));
    fs::create_dir(&staging).map_err(display_error)?;
    let result = async {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(5))
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(display_error)?;
        let mut total_bytes = 0_u64;
        for (index, file) in model.files.iter().enumerate() {
            let bytes = download_file(&client, file).await?;
            total_bytes = total_bytes
                .checked_add(bytes.len() as u64)
                .ok_or("model size overflow")?;
            if total_bytes > MAX_MODEL_TOTAL_BYTES {
                return Err("model exceeds 256 MiB".into());
            }
            write_catalog_download(&staging, file, &bytes)?;
            let _ = app.emit(
                "companion:model-download-progress",
                DownloadProgress {
                    model_id: model.id.clone(),
                    completed_files: index + 1,
                    total_files: model.files.len(),
                    current_file: file.name.clone(),
                    phase: "downloading".into(),
                },
            );
        }
        fs::write(metadata_path(&staging), catalog_metadata_bytes(&model)?)
            .map_err(display_error)?;
        fs::rename(&staging, &destination).map_err(display_error)?;
        Ok::<(), String>(())
    }
    .await;
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&staging);
        let _ = app.emit(
            "companion:model-download-progress",
            DownloadProgress {
                model_id: model.id.clone(),
                completed_files: 0,
                total_files: model.files.len(),
                current_file: error.clone(),
                phase: "failed".into(),
            },
        );
        return Err(error);
    }
    let _ = app.emit(
        "companion:model-download-progress",
        DownloadProgress {
            model_id: model.id.clone(),
            completed_files: model.files.len(),
            total_files: model.files.len(),
            current_file: model.skel.clone(),
            phase: "installed".into(),
        },
    );
    installed_models()?
        .into_iter()
        .find(|item| item.id == model.id)
        .ok_or("installed model metadata was not found".into())
}

async fn install_live2d_catalog_model(
    app: &AppHandle,
    model: Live2dCatalogModel,
) -> Result<InstalledModelView, String> {
    model.validate()?;
    let root = models_root()?;
    fs::create_dir_all(&root).map_err(display_error)?;
    let destination = root.join(&model.id);
    if destination.exists() {
        return installed_models()?
            .into_iter()
            .find(|item| item.id == model.id)
            .ok_or("Live2D model destination exists but is not a valid installation".into());
    }
    let staging = root.join(format!(".{}.{}.download", model.id, Uuid::new_v4()));
    fs::create_dir(&staging).map_err(display_error)?;
    let result = async {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(5))
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(display_error)?;
        let mut total_bytes = 0_u64;
        for (index, file) in model.files.iter().enumerate() {
            let bytes = download_file(&client, file).await?;
            total_bytes = total_bytes
                .checked_add(bytes.len() as u64)
                .ok_or("Live2D model size overflow")?;
            if total_bytes > MAX_MODEL_TOTAL_BYTES {
                return Err("Live2D model exceeds 256 MiB".into());
            }
            write_catalog_download(&staging, file, &bytes)?;
            let _ = app.emit(
                "companion:model-download-progress",
                DownloadProgress {
                    model_id: model.id.clone(),
                    completed_files: index + 1,
                    total_files: model.files.len(),
                    current_file: file.name.clone(),
                    phase: "downloading".into(),
                },
            );
        }
        super::validate_live2d_manifest(&staging.join(&model.entry_file))?;
        fs::write(
            metadata_path(&staging),
            live2d_catalog_metadata_bytes(&model)?,
        )
        .map_err(display_error)?;
        fs::rename(&staging, &destination).map_err(display_error)?;
        Ok::<(), String>(())
    }
    .await;
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&staging);
        let _ = app.emit(
            "companion:model-download-progress",
            DownloadProgress {
                model_id: model.id.clone(),
                completed_files: 0,
                total_files: model.files.len(),
                current_file: error.clone(),
                phase: "failed".into(),
            },
        );
        return Err(error);
    }
    let _ = app.emit(
        "companion:model-download-progress",
        DownloadProgress {
            model_id: model.id.clone(),
            completed_files: model.files.len(),
            total_files: model.files.len(),
            current_file: model.entry_file.clone(),
            phase: "installed".into(),
        },
    );
    installed_models()?
        .into_iter()
        .find(|item| item.id == model.id)
        .ok_or("installed Live2D model metadata was not found".into())
}

#[tauri::command]
pub(crate) async fn companion_activate_installed_model(
    app: AppHandle,
    window: tauri::WebviewWindow,
    model_id: String,
) -> Result<InstalledModelView, String> {
    require_main_window(&window)?;
    let model = installed_models()?
        .into_iter()
        .find(|item| item.id == model_id)
        .ok_or("installed model was not found")?;
    let mut settings = read_settings()?;
    settings.enabled = true;
    settings.model_path = Some(model.path.clone());
    settings.atlas_path = model.atlas_path.clone();
    settings.model_name = Some(model.name.clone());
    settings.model_kind = model.model_kind;
    if matches!(model.model_kind, CompanionModelKind::Live2d) {
        settings.live2d_core_path = Some(
            super::ensure_live2d_core()
                .await?
                .to_string_lossy()
                .into_owned(),
        );
    } else {
        settings.live2d_core_path = None;
    }
    write_settings(&settings)?;
    set_window_visibility(&app, &settings);
    let _ = app.emit_to("companion", "companion:settings", &settings);
    installed_models()?
        .into_iter()
        .find(|item| item.id == model_id)
        .ok_or("activated model was not found".into())
}

#[tauri::command]
pub(crate) fn companion_remove_installed_model(
    window: tauri::WebviewWindow,
    model_id: String,
) -> Result<(), String> {
    require_main_window(&window)?;
    let model = installed_models()?
        .into_iter()
        .find(|item| item.id == model_id)
        .ok_or("installed model was not found")?;
    if model.active {
        return Err("the active model cannot be removed".into());
    }
    let root = models_root()?.canonicalize().map_err(display_error)?;
    let directory = Path::new(&model.path)
        .parent()
        .ok_or("installed model has no directory")?
        .canonicalize()
        .map_err(display_error)?;
    if !directory.starts_with(&root) {
        return Err("installed model escaped the companion model directory".into());
    }
    fs::remove_dir_all(directory).map_err(display_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_catalogs_are_valid_and_unique() {
        let models = catalog_models().expect("bundled catalogs");
        assert!(models.len() > 100);
        let unique = models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), models.len());
    }

    #[test]
    fn official_live2d_catalog_is_pinned_and_runtime_complete() {
        let models = live2d_catalog_models().expect("bundled Live2D catalog");
        assert_eq!(models.len(), 1);
        let rice = &models[0];
        assert_eq!(rice.id, "live2d-official-rice");
        assert_eq!(rice.entry_file, "Rice.model3.json");
        assert!(rice.files.iter().any(|file| file.name.ends_with(".moc3")));
        assert!(rice.files.iter().any(|file| file.name.contains('/')));
        assert!(rice.files.iter().all(|file| file.fallback_urls.is_empty()));
    }

    #[test]
    fn live2d_catalog_download_requires_matching_license_acceptance() {
        let rice = &live2d_catalog_models().unwrap()[0];
        assert!(validate_live2d_license_acceptance(rice, None).is_err());
        let mut acceptance = CatalogLicenseAcceptance {
            accepted: true,
            model_id: rice.id.clone(),
            repository_url: rice.repository_url.clone(),
            license_url: rice.license_url.clone(),
            terms_url: rice.terms_url.clone(),
        };
        assert!(validate_live2d_license_acceptance(rice, Some(&acceptance)).is_ok());
        acceptance.terms_url.push_str("?changed");
        assert!(validate_live2d_license_acceptance(rice, Some(&acceptance)).is_err());
    }

    #[test]
    fn catalog_download_writer_preserves_live2d_subdirectories() {
        let temporary = tempfile::tempdir().expect("temporary Live2D install");
        let file = live2d_catalog_models().unwrap()[0]
            .files
            .iter()
            .find(|file| file.name.contains('/'))
            .unwrap();
        write_catalog_download(temporary.path(), file, b"texture").unwrap();
        assert_eq!(
            fs::read(temporary.path().join(&file.name)).unwrap(),
            b"texture"
        );
    }

    #[test]
    fn model_ids_cannot_escape_the_cache() {
        assert!(safe_model_id("ark-models-002-amiya"));
        assert!(!safe_model_id("../escape"));
        assert!(!safe_model_id("UPPER"));
    }

    #[test]
    fn preload_input_is_bounded_unique_and_catalog_backed() {
        let ids = catalog_models()
            .unwrap()
            .iter()
            .take(MAX_PRELOAD_MODELS)
            .map(|model| model.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            resolve_preload_models(&ids).unwrap().len(),
            MAX_PRELOAD_MODELS
        );

        let mut too_many = ids.clone();
        too_many.push(catalog_models().unwrap()[MAX_PRELOAD_MODELS].id.clone());
        assert!(
            resolve_preload_models(&too_many)
                .unwrap_err()
                .contains("at most 4")
        );
        assert!(
            resolve_preload_models(&[ids[0].clone(), ids[0].clone()])
                .unwrap_err()
                .contains("repeated")
        );
        assert!(
            resolve_preload_models(&["../escape".to_string()])
                .unwrap_err()
                .contains("invalid")
        );
        assert!(
            resolve_preload_models(&["missing-model".to_string()])
                .unwrap_err()
                .contains("not found")
        );
    }

    #[test]
    fn preload_session_capacity_is_a_hard_boundary() {
        assert_eq!(
            ensure_preload_capacity(MAX_PRELOAD_SESSION_BYTES - 1, 1).unwrap(),
            MAX_PRELOAD_SESSION_BYTES
        );
        assert!(
            ensure_preload_capacity(MAX_PRELOAD_SESSION_BYTES, 1)
                .unwrap_err()
                .contains("128 MiB")
        );
    }

    #[test]
    fn preload_view_paths_are_scoped_to_the_catalog_model_directory() {
        let root = Path::new("companion").join("preload");
        let model = catalog_models()
            .unwrap()
            .iter()
            .find(|model| model.id == "ark-illustrations-dyn-illust-2014-nian")
            .unwrap();
        let directory = root.join(&model.id);
        let view = preloaded_model_view(&directory, model).unwrap();

        assert_eq!(
            Path::new(&view.model_path).parent(),
            Some(directory.as_path())
        );
        assert_eq!(
            Path::new(&view.atlas_path).parent(),
            Some(directory.as_path())
        );
        assert!(view.model_path.ends_with(&model.skel));
        assert!(view.atlas_path.ends_with("dyn_illust_char_2014_nian.atlas"));
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["modelId"], model.id);
        assert_eq!(json["modelKind"], "spine38");
        assert!(json.get("modelPath").is_some());
        assert!(json.get("atlasPath").is_some());
    }

    #[test]
    fn preload_cleanup_does_not_touch_installed_models() {
        let temporary = tempfile::tempdir().expect("temporary companion root");
        let companion = temporary.path().join("companion");
        let preload_marker = companion.join("preload").join("model").join("cached.skel");
        let installed_marker = companion
            .join("models")
            .join("installed")
            .join("model.skel");
        fs::create_dir_all(preload_marker.parent().unwrap()).unwrap();
        fs::create_dir_all(installed_marker.parent().unwrap()).unwrap();
        fs::write(&preload_marker, b"temporary").unwrap();
        fs::write(&installed_marker, b"installed").unwrap();

        cleanup_preload_cache_at(&companion).unwrap();

        assert!(!companion.join("preload").exists());
        assert_eq!(fs::read(installed_marker).unwrap(), b"installed");
    }

    #[test]
    fn preload_pruning_keeps_only_the_current_page_models() {
        let temporary = tempfile::tempdir().expect("temporary preload cache");
        let keep = temporary.path().join("keep-model");
        let stale = temporary.path().join("stale-model");
        let staging = temporary.path().join(".stale.download");
        fs::create_dir_all(&keep).unwrap();
        fs::create_dir_all(&stale).unwrap();
        fs::create_dir_all(&staging).unwrap();
        let keep_ids = BTreeSet::from(["keep-model"]);

        prune_preload_cache(temporary.path(), &keep_ids).unwrap();

        assert!(keep.exists());
        assert!(!stale.exists());
        assert!(!staging.exists());
    }

    #[test]
    fn complete_preload_cache_is_reusable_and_corruption_is_rejected() {
        let temporary = tempfile::tempdir().expect("temporary preload cache");
        let mut model = catalog_models().unwrap()[0].clone();
        let directory = temporary.path().join(&model.id);
        fs::create_dir(&directory).unwrap();
        for (index, file) in model.files.iter_mut().enumerate() {
            let bytes = format!("verified catalog file {index}").into_bytes();
            file.sha256 = hex_digest(Sha256::digest(&bytes));
            file.github_blob_sha.clear();
            file.size_bytes = Some(bytes.len() as u64);
            fs::write(directory.join(&file.name), bytes).unwrap();
        }
        fs::write(
            metadata_path(&directory),
            catalog_metadata_bytes(&model).unwrap(),
        )
        .unwrap();

        assert!(cached_preload_is_complete(&directory, &model).unwrap());

        fs::write(directory.join(&model.files[0].name), b"corrupt").unwrap();
        assert!(!cached_preload_is_complete(&directory, &model).unwrap());
    }

    #[test]
    fn avatar_installation_ids_are_namespaced_including_legacy_metadata() {
        assert_eq!(
            installed_model_id("ark-models-002-amiya", "avatar-studio"),
            "avatar--ark-models-002-amiya"
        );
        assert_eq!(
            installed_model_id("avatar--my-avatar", "avatar-studio"),
            "avatar--my-avatar"
        );
        assert_eq!(
            installed_model_id("ark-models-002-amiya", "Ark-Models"),
            "ark-models-002-amiya"
        );
    }

    #[test]
    fn catalog_uses_an_explicit_atlas_when_the_stem_differs() {
        let model = catalog_models()
            .unwrap()
            .iter()
            .find(|model| model.id == "ark-illustrations-dyn-illust-2014-nian")
            .expect("known illustration catalog entry");
        assert_eq!(
            catalog_atlas_file(model).map(|file| file.name.as_str()),
            Some("dyn_illust_char_2014_nian.atlas")
        );
        assert_eq!(model.skel, "dyn_illust_char_2014_nian2.skel");
    }
}
