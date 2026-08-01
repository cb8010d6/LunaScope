use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{Value, json};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use super::{
    CompanionModelKind, MAX_MODEL_FILE_BYTES, MAX_MODEL_TOTAL_BYTES,
    catalog::{CatalogDocument, CatalogModel},
    companion_root, display_error, read_settings, set_window_visibility, write_settings,
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
    installed: bool,
    active: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogSearchView {
    models: Vec<CatalogModelView>,
    sources: Vec<String>,
    total: usize,
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

fn models_root() -> Result<PathBuf, String> {
    Ok(companion_root()?.join("models"))
}

fn safe_model_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
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

pub(super) fn write_local_model_metadata(
    directory: &Path,
    id: &str,
    name: &str,
    kind: CompanionModelKind,
    entry_file: &str,
) -> Result<(), String> {
    let metadata = json!({
        "id": id,
        "name": name,
        "modelKind": kind,
        "entryFile": entry_file,
        "source": "local",
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
        let id = metadata
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let entry_file = metadata
            .get("entryFile")
            .or_else(|| metadata.get("skel"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !safe_model_id(id) || entry_file.is_empty() {
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
        installed.push(InstalledModelView {
            id: id.to_owned(),
            name: metadata
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(id)
                .to_owned(),
            model_kind,
            source: metadata
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or("local")
                .to_owned(),
            license: metadata
                .get("license")
                .and_then(Value::as_str)
                .unwrap_or("NOASSERTION")
                .to_owned(),
            path: model_path.to_string_lossy().into_owned(),
            active: active_path.is_some_and(|active| active == model_path),
        });
    }
    installed.sort_by_key(|item| item.name.to_lowercase());
    Ok(installed)
}

#[tauri::command]
pub(crate) fn companion_search_catalog(
    query: Option<String>,
    source: Option<String>,
) -> Result<CatalogSearchView, String> {
    let query = query.unwrap_or_default().trim().to_lowercase();
    let source_filter = source.filter(|value| !value.is_empty());
    let installed = installed_models()?;
    let active = installed
        .iter()
        .find(|item| item.active)
        .map(|item| item.id.as_str());
    let installed_ids = installed
        .iter()
        .map(|item| item.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let mut sources = catalog_models()?
        .iter()
        .map(|model| model.source.clone())
        .collect::<Vec<_>>();
    sources.sort();
    sources.dedup();
    let filtered = catalog_models()?.iter().filter(|model| {
        source_filter
            .as_ref()
            .is_none_or(|source| &model.source == source)
            && (query.is_empty()
                || [
                    model.name.as_str(),
                    model.id.as_str(),
                    model.source.as_str(),
                    model.description.as_str(),
                ]
                .iter()
                .any(|value| value.to_lowercase().contains(&query))
                || model
                    .tags
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(&query)))
    });
    let mut views = filtered
        .take(60)
        .map(|model| CatalogModelView {
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
            installed: installed_ids.contains(model.id.as_str()),
            active: active == Some(model.id.as_str()),
        })
        .collect::<Vec<_>>();
    views.sort_by_key(|item| item.name.to_lowercase());
    let total = views.len();
    Ok(CatalogSearchView {
        models: views,
        sources,
        total,
    })
}

#[tauri::command]
pub(crate) fn companion_list_installed_models() -> Result<Vec<InstalledModelView>, String> {
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
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(display_error)?;
            if bytes.len().saturating_add(chunk.len()) > MAX_MODEL_FILE_BYTES as usize {
                return Err(format!("{} exceeds 64 MiB", file.name));
            }
            bytes.extend_from_slice(&chunk);
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
pub(crate) async fn companion_install_catalog_model(
    app: AppHandle,
    model_id: String,
) -> Result<InstalledModelView, String> {
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
        return Err("model is already installed".into());
    }
    let staging = root.join(format!(".{}.{}.download", model.id, Uuid::new_v4()));
    fs::create_dir(&staging).map_err(display_error)?;
    let result = async {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(5))
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
            fs::write(staging.join(&file.name), bytes).map_err(display_error)?;
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
        let mut metadata = serde_json::to_value(&model).map_err(display_error)?;
        if let Some(object) = metadata.as_object_mut() {
            object.insert("modelKind".into(), json!("spine38"));
            object.insert("entryFile".into(), json!(model.skel));
        }
        fs::write(
            metadata_path(&staging),
            serde_json::to_vec_pretty(&metadata).map_err(display_error)?,
        )
        .map_err(display_error)?;
        fs::rename(&staging, &destination).map_err(display_error)?;
        Ok::<(), String>(())
    }
    .await;
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&staging);
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

#[tauri::command]
pub(crate) fn companion_activate_installed_model(
    app: AppHandle,
    model_id: String,
) -> Result<InstalledModelView, String> {
    let model = installed_models()?
        .into_iter()
        .find(|item| item.id == model_id)
        .ok_or("installed model was not found")?;
    let mut settings = read_settings()?;
    settings.enabled = true;
    settings.model_path = Some(model.path.clone());
    settings.model_name = Some(model.name.clone());
    settings.model_kind = model.model_kind;
    if !matches!(model.model_kind, CompanionModelKind::Live2d) {
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
pub(crate) fn companion_remove_installed_model(model_id: String) -> Result<(), String> {
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
    fn model_ids_cannot_escape_the_cache() {
        assert!(safe_model_id("ark-models-002-amiya"));
        assert!(!safe_model_id("../escape"));
        assert!(!safe_model_id("UPPER"));
    }
}
