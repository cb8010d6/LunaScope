//! Remote model catalog schema, validation, cache handling, and search.
//!
//! This module deliberately has no Tauri dependency. The application layer owns
//! persistence and commands; it can serialize `CatalogCache` as JSON and call
//! `refresh_catalogs` with its shared `reqwest::Client`.

use chrono::Utc;
use reqwest::{Client, Response, Url, header};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

pub const CATALOG_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_PAGE_SIZE: usize = 24;
pub const MAX_PAGE_SIZE: usize = 100;
const MAX_CATALOG_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CatalogSourceKind {
    #[default]
    Official,
    CustomRaw,
    CustomCdn,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSource {
    pub id: String,
    pub label: String,
    pub catalog_url: String,
    #[serde(default)]
    pub kind: CatalogSourceKind,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

impl CatalogSource {
    pub fn validate(&self) -> Result<(), String> {
        validate_identifier(&self.id, "Catalog source id")?;
        validate_nonempty(&self.label, "Catalog source label")?;
        let url = validate_https_url(&self.catalog_url, "Catalog source URL")?;
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();

        match self.kind {
            CatalogSourceKind::Official => Ok(()),
            CatalogSourceKind::CustomRaw if is_raw_host(&host) => Ok(()),
            CatalogSourceKind::CustomCdn if is_cdn_host(&host) => Ok(()),
            CatalogSourceKind::CustomRaw => Err(format!(
                "Custom Raw catalog source must use a supported HTTPS raw host: {}",
                self.catalog_url
            )),
            CatalogSourceKind::CustomCdn => Err(format!(
                "Custom CDN catalog source must use a supported HTTPS CDN host: {}",
                self.catalog_url
            )),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogDocument {
    pub schema_version: u32,
    pub models: Vec<CatalogModel>,
}

impl CatalogDocument {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != CATALOG_SCHEMA_VERSION {
            return Err(format!(
                "Unsupported catalog schema version {}; expected {}.",
                self.schema_version, CATALOG_SCHEMA_VERSION
            ));
        }
        let mut ids = BTreeSet::new();
        for model in &self.models {
            model.validate()?;
            if !ids.insert(model.id.as_str()) {
                return Err(format!("Catalog contains duplicate model id: {}", model.id));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModel {
    pub id: String,
    pub name: String,
    pub source: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub license_warning: String,
    pub license_note: String,
    pub repository_url: String,
    pub skel: String,
    pub files: Vec<CatalogFile>,
    pub spine: SpineCompatibility,
    #[serde(default = "default_true")]
    pub version_verified: bool,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_model_category")]
    pub category: String,
    #[serde(default = "default_compatibility_profile")]
    pub compatibility_profile: String,
}

fn default_model_category() -> String {
    "operator".to_string()
}

fn default_compatibility_profile() -> String {
    "companion".to_string()
}

fn default_true() -> bool {
    true
}

impl CatalogModel {
    pub fn validate(&self) -> Result<(), String> {
        validate_identifier(&self.id, "Model id")?;
        validate_nonempty(&self.name, "Model name")?;
        validate_nonempty(&self.source, "Model source")?;
        validate_nonempty(&self.license_note, "Model license note")?;
        validate_https_url(&self.repository_url, "Model repository URL")?;
        validate_safe_file_name(&self.skel, "Model skel")?;
        if !has_extension(&self.skel, "skel") {
            return Err(format!("Model skel must be a .skel file: {}", self.skel));
        }
        self.spine.validate()?;
        validate_identifier(&self.category, "Model category")?;
        validate_identifier(&self.compatibility_profile, "Compatibility profile")?;
        if self.files.is_empty() {
            return Err(format!("Model {} has no downloadable files.", self.id));
        }

        let mut names = BTreeSet::new();
        let mut has_skel = false;
        let mut has_atlas = false;
        let mut has_texture = false;
        for file in &self.files {
            file.validate()?;
            if !names.insert(file.name.as_str()) {
                return Err(format!(
                    "Model {} has duplicate file: {}",
                    self.id, file.name
                ));
            }
            has_skel |= file.name == self.skel;
            has_atlas |= has_extension(&file.name, "atlas");
            has_texture |= ["png", "jpg", "jpeg", "webp"]
                .iter()
                .any(|extension| has_extension(&file.name, extension));
        }
        if !has_skel {
            return Err(format!(
                "Model {} files do not include selected skel {}.",
                self.id, self.skel
            ));
        }
        if !has_atlas || !has_texture {
            return Err(format!(
                "Model {} must include at least one .atlas file and texture image.",
                self.id
            ));
        }

        let mut tags = BTreeSet::new();
        for tag in &self.tags {
            validate_nonempty(tag, "Model tag")?;
            if !tags.insert(tag.trim().to_ascii_lowercase()) {
                return Err(format!("Model {} has duplicate tag: {}", self.id, tag));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogFile {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub github_blob_sha: String,
    #[serde(default)]
    pub fallback_urls: Vec<String>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
}

impl CatalogFile {
    pub fn validate(&self) -> Result<(), String> {
        validate_safe_file_name(&self.name, "Model file name")?;
        validate_https_url(&self.url, "Model file URL")?;
        let valid_sha256 = self.sha256.len() == 64
            && self
                .sha256
                .bytes()
                .all(|character| character.is_ascii_hexdigit());
        let valid_blob_sha = self.github_blob_sha.len() == 40
            && self
                .github_blob_sha
                .bytes()
                .all(|character| character.is_ascii_hexdigit());
        if !valid_sha256 && !valid_blob_sha {
            return Err(format!(
                "Model file {} must include a SHA-256 or Git blob digest.",
                self.name
            ));
        }
        let mut urls = BTreeSet::new();
        urls.insert(self.url.as_str());
        for fallback in &self.fallback_urls {
            validate_https_url(fallback, "Model file fallback URL")?;
            if !urls.insert(fallback.as_str()) {
                return Err(format!("Model file {} repeats a download URL.", self.name));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct SpineVersion(pub String);

impl SpineVersion {
    #[cfg(test)]
    pub fn parse(value: impl AsRef<str>) -> Result<Self, String> {
        let value = value.as_ref().trim().trim_start_matches('v');
        parse_spine_version_components(value)?;
        Ok(Self(value.to_string()))
    }

    fn components(&self) -> Result<(u32, u32, u32), String> {
        parse_spine_version_components(&self.0)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpineCompatibility {
    #[serde(alias = "spineVersion", alias = "minVersion")]
    pub min: SpineVersion,
    #[serde(default, alias = "maxVersion")]
    pub max: Option<SpineVersion>,
}

impl SpineCompatibility {
    pub fn validate(&self) -> Result<(), String> {
        let minimum = self.min.components()?;
        if let Some(maximum) = self
            .max
            .as_ref()
            .map(SpineVersion::components)
            .transpose()?
        {
            if minimum > maximum {
                return Err("Spine compatibility maximum cannot precede minimum.".to_string());
            }
            if minimum.0 != maximum.0 || minimum.1 != maximum.1 {
                return Err(
                    "Spine compatibility range must stay within one major.minor runtime line."
                        .to_string(),
                );
            }
        }
        Ok(())
    }

    /// Spine binary exports are only compatible inside the same major.minor
    /// runtime line. Patch versions may vary within the declared range.
    pub fn is_compatible_with(&self, runtime: &SpineVersion) -> bool {
        let Ok(minimum) = self.min.components() else {
            return false;
        };
        let Ok(runtime) = runtime.components() else {
            return false;
        };
        if runtime.0 != minimum.0 || runtime.1 != minimum.1 || runtime < minimum {
            return false;
        }
        match self.max.as_ref().map(SpineVersion::components).transpose() {
            Ok(Some(maximum)) => runtime <= maximum,
            Ok(None) => true,
            Err(_) => false,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogCache {
    #[serde(default)]
    pub entries: BTreeMap<String, CachedCatalog>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CachedCatalog {
    pub metadata: SourceCacheMetadata,
    pub document: CatalogDocument,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceCacheMetadata {
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    pub fetched_at_unix_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogSourceState {
    Fresh,
    NotModified,
    Stale,
    Failed,
    Disabled,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSourceStatus {
    pub source_id: String,
    pub state: CatalogSourceState,
    pub model_count: usize,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModelEntry {
    pub catalog_source_id: String,
    #[serde(flatten)]
    pub model: CatalogModel,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogRefreshResult {
    #[serde(default)]
    pub models: Vec<CatalogModelEntry>,
    #[serde(default)]
    pub sources: Vec<CatalogSourceStatus>,
}

/// Refresh every enabled source. A source failure is contained to that source:
/// when a cache entry exists its last validated document remains available and
/// is marked stale, which makes offline use deterministic.
pub async fn refresh_catalogs(
    client: &Client,
    sources: &[CatalogSource],
    cache: &mut CatalogCache,
) -> CatalogRefreshResult {
    let mut result = CatalogRefreshResult::default();
    let mut source_ids = BTreeSet::new();
    let mut model_ids = BTreeSet::new();

    for source in sources {
        if !source_ids.insert(source.id.clone()) {
            result.sources.push(CatalogSourceStatus {
                source_id: source.id.clone(),
                state: CatalogSourceState::Failed,
                model_count: 0,
                error: Some("Duplicate catalog source id.".to_string()),
            });
            continue;
        }
        if !source.enabled {
            result.sources.push(CatalogSourceStatus {
                source_id: source.id.clone(),
                state: CatalogSourceState::Disabled,
                model_count: 0,
                error: None,
            });
            continue;
        }

        let cached = cache.entries.get(&source.id).cloned();
        let source_result = refresh_source(client, source, cached).await;
        if let Some(entry) = source_result.cache_entry.clone() {
            cache.entries.insert(source.id.clone(), entry.clone());
            for model in entry.document.models {
                if model_ids.insert(model.id.clone()) {
                    result.models.push(CatalogModelEntry {
                        catalog_source_id: source.id.clone(),
                        model,
                    });
                } else {
                    result.sources.push(CatalogSourceStatus {
                        source_id: source.id.clone(),
                        state: CatalogSourceState::Failed,
                        model_count: 0,
                        error: Some("Duplicate model id supplied by another source.".to_string()),
                    });
                }
            }
        }
        result.sources.push(source_result.status);
    }

    sort_catalog_models(&mut result.models);
    result
}

/// Return the cached models belonging to the currently enabled sources.
///
/// This performs no I/O and does not mutate the cache. Sources absent from the
/// current configuration, disabled sources, and repeated source/model ids are
/// ignored. Callers can therefore expose startup/offline catalog data without
/// triggering a refresh.
///
/// `lib.rs` integration requires a manager-only Tauri command that accepts the
/// current sources, deserializes `config_dir/catalog-cache.json` as
/// `CatalogCache`, returns this function's result, and is registered in
/// `tauri::generate_handler!`.
#[allow(dead_code)]
pub fn cached_enabled_models(
    sources: &[CatalogSource],
    cache: &CatalogCache,
) -> Vec<CatalogModelEntry> {
    let mut models = Vec::new();
    let mut source_ids = BTreeSet::new();
    let mut model_ids = BTreeSet::new();

    for source in sources.iter().filter(|source| source.enabled) {
        if !source_ids.insert(source.id.as_str()) {
            continue;
        }
        let Some(entry) = cache.entries.get(&source.id) else {
            continue;
        };
        for model in &entry.document.models {
            if model_ids.insert(model.id.as_str()) {
                models.push(CatalogModelEntry {
                    catalog_source_id: source.id.clone(),
                    model: model.clone(),
                });
            }
        }
    }

    sort_catalog_models(&mut models);
    models
}

/// Resolve one model from the validated cache of one configured, enabled source.
///
/// Unlike [`cached_enabled_models`], this is intentionally strict because the
/// returned URLs and digests may be used for preview or installation. Invalid
/// source configuration, duplicate source ids, and invalid cached documents are
/// therefore reported instead of being skipped.
pub fn resolve_cached_model_entry(
    sources: &[CatalogSource],
    cache: &CatalogCache,
    source_id: &str,
    model_id: &str,
) -> Result<CatalogModelEntry, String> {
    validate_identifier(source_id, "Catalog source id")?;
    validate_identifier(model_id, "Model id")?;

    let mut matching_sources = sources.iter().filter(|source| source.id == source_id);
    let source = matching_sources
        .next()
        .ok_or_else(|| format!("Catalog source is not configured: {source_id}"))?;
    if matching_sources.next().is_some() {
        return Err(format!(
            "Catalog source id is configured more than once and is ambiguous: {source_id}"
        ));
    }
    if !source.enabled {
        return Err(format!("Catalog source is disabled: {source_id}"));
    }
    source
        .validate()
        .map_err(|error| format!("Invalid catalog source {source_id}: {error}"))?;

    let cached = cache.entries.get(source_id).ok_or_else(|| {
        format!("No validated cached catalog is available for source: {source_id}")
    })?;
    cached
        .document
        .validate()
        .map_err(|error| format!("Invalid cached catalog for source {source_id}: {error}"))?;

    let model = cached
        .document
        .models
        .iter()
        .find(|model| model.id == model_id)
        .ok_or_else(|| format!("Catalog model {model_id} was not found in source: {source_id}"))?;
    Ok(CatalogModelEntry {
        catalog_source_id: source_id.to_string(),
        model: model.clone(),
    })
}

fn sort_catalog_models(models: &mut [CatalogModelEntry]) {
    models.sort_by(|left, right| {
        left.model
            .name
            .to_ascii_lowercase()
            .cmp(&right.model.name.to_ascii_lowercase())
            .then(left.model.id.cmp(&right.model.id))
            .then(left.catalog_source_id.cmp(&right.catalog_source_id))
    });
}

#[derive(Clone, Debug)]
struct SourceRefresh {
    status: CatalogSourceStatus,
    cache_entry: Option<CachedCatalog>,
}

async fn refresh_source(
    client: &Client,
    source: &CatalogSource,
    cached: Option<CachedCatalog>,
) -> SourceRefresh {
    if let Err(error) = source.validate() {
        return SourceRefresh {
            status: source_status(source, CatalogSourceState::Failed, 0, Some(error)),
            cache_entry: None,
        };
    }

    let mut request = client.get(&source.catalog_url);
    if let Some(entry) = cached.as_ref() {
        if let Some(etag) = entry.metadata.etag.as_deref() {
            request = request.header(header::IF_NONE_MATCH, etag);
        }
        if let Some(last_modified) = entry.metadata.last_modified.as_deref() {
            request = request.header(header::IF_MODIFIED_SINCE, last_modified);
        }
    }

    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => return stale_or_failed(source, cached, error.to_string()),
    };
    if response.url().as_str() != source.catalog_url {
        let redirected = CatalogSource {
            catalog_url: response.url().to_string(),
            ..source.clone()
        };
        if let Err(error) = redirected.validate() {
            return stale_or_failed(
                source,
                cached,
                format!("Catalog redirect was rejected: {error}"),
            );
        }
    }
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        return match cached {
            Some(mut entry) => {
                entry.metadata = cache_metadata(&response, &entry.metadata);
                let count = entry.document.models.len();
                SourceRefresh {
                    status: source_status(source, CatalogSourceState::NotModified, count, None),
                    cache_entry: Some(entry),
                }
            }
            None => stale_or_failed(
                source,
                None,
                "Catalog server returned 304 without a local cache entry.".to_string(),
            ),
        };
    }
    if !response.status().is_success() {
        return stale_or_failed(
            source,
            cached,
            format!(
                "Catalog request returned HTTP {}.",
                response.status().as_u16()
            ),
        );
    }

    let metadata = cache_metadata(&response, &SourceCacheMetadata::default());
    let body = match read_catalog_response(response).await {
        Ok(body) => body,
        Err(error) => return stale_or_failed(source, cached, error),
    };
    let document = match serde_json::from_slice::<CatalogDocument>(&body)
        .map_err(|error| error.to_string())
        .and_then(|document| document.validate().map(|_| document))
    {
        Ok(document) => document,
        Err(error) => return stale_or_failed(source, cached, format!("Invalid catalog: {error}")),
    };
    let count = document.models.len();
    SourceRefresh {
        status: source_status(source, CatalogSourceState::Fresh, count, None),
        cache_entry: Some(CachedCatalog { metadata, document }),
    }
}

async fn read_catalog_response(response: Response) -> Result<Vec<u8>, String> {
    read_catalog_response_with_limit(response, MAX_CATALOG_RESPONSE_BYTES).await
}

async fn read_catalog_response_with_limit(
    mut response: Response,
    limit: usize,
) -> Result<Vec<u8>, String> {
    if let Some(content_length) = response.content_length() {
        ensure_catalog_size_with_limit(content_length, limit)?;
    }
    let initial_capacity = response.content_length().unwrap_or(0) as usize;
    let mut body = Vec::with_capacity(initial_capacity.min(limit));
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        append_catalog_chunk_with_limit(&mut body, &chunk, limit)?;
    }
    Ok(body)
}

fn ensure_catalog_size_with_limit(size: u64, limit: usize) -> Result<(), String> {
    if size > limit as u64 {
        Err(catalog_size_error(limit))
    } else {
        Ok(())
    }
}

fn append_catalog_chunk_with_limit(
    body: &mut Vec<u8>,
    chunk: &[u8],
    limit: usize,
) -> Result<(), String> {
    let decoded_size = body
        .len()
        .checked_add(chunk.len())
        .ok_or_else(|| catalog_size_error(limit))?;
    ensure_catalog_size_with_limit(decoded_size as u64, limit)?;
    body.extend_from_slice(chunk);
    Ok(())
}

fn catalog_size_error(limit: usize) -> String {
    if limit.is_multiple_of(1024 * 1024) {
        format!(
            "Catalog response exceeds the {} MiB limit.",
            limit / (1024 * 1024)
        )
    } else {
        format!("Catalog response exceeds the {limit}-byte limit.")
    }
}

fn stale_or_failed(
    source: &CatalogSource,
    cached: Option<CachedCatalog>,
    error: String,
) -> SourceRefresh {
    match cached {
        Some(entry) => SourceRefresh {
            status: source_status(
                source,
                CatalogSourceState::Stale,
                entry.document.models.len(),
                Some(error),
            ),
            cache_entry: Some(entry),
        },
        None => SourceRefresh {
            status: source_status(source, CatalogSourceState::Failed, 0, Some(error)),
            cache_entry: None,
        },
    }
}

fn source_status(
    source: &CatalogSource,
    state: CatalogSourceState,
    model_count: usize,
    error: Option<String>,
) -> CatalogSourceStatus {
    CatalogSourceStatus {
        source_id: source.id.clone(),
        state,
        model_count,
        error,
    }
}

fn cache_metadata(
    response: &reqwest::Response,
    previous: &SourceCacheMetadata,
) -> SourceCacheMetadata {
    let header_value = |name: reqwest::header::HeaderName| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    };
    SourceCacheMetadata {
        etag: header_value(header::ETAG).or_else(|| previous.etag.clone()),
        last_modified: header_value(header::LAST_MODIFIED)
            .or_else(|| previous.last_modified.clone()),
        fetched_at_unix_ms: Utc::now().timestamp_millis(),
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSearchRequest {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub source_ids: Vec<String>,
    #[serde(default)]
    pub page: usize,
    #[serde(default)]
    pub page_size: usize,
    #[serde(default)]
    pub runtime_spine_version: Option<SpineVersion>,
    #[serde(default)]
    pub include_incompatible: bool,
    #[serde(default)]
    pub installation_filter: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSearchResult {
    pub models: Vec<CatalogModelEntry>,
    pub page: usize,
    pub page_size: usize,
    pub total: usize,
    pub total_pages: usize,
}

pub fn search_catalog(
    models: &[CatalogModelEntry],
    request: &CatalogSearchRequest,
) -> CatalogSearchResult {
    let query = request.query.trim().to_ascii_lowercase();
    let allowed_sources = request
        .source_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .collect::<BTreeSet<_>>();
    let page_size = if request.page_size == 0 {
        DEFAULT_PAGE_SIZE
    } else {
        request.page_size.clamp(1, MAX_PAGE_SIZE)
    };
    let page = request.page.max(1);

    let filtered = models
        .iter()
        .filter(|entry| {
            (allowed_sources.is_empty()
                || allowed_sources.contains(entry.catalog_source_id.as_str()))
                && (request.include_incompatible
                    || request
                        .runtime_spine_version
                        .as_ref()
                        .is_none_or(|runtime| entry.model.spine.is_compatible_with(runtime)))
                && matches_query(entry, &query)
        })
        .cloned()
        .collect::<Vec<_>>();
    let total = filtered.len();
    let total_pages = total.div_ceil(page_size);
    let start = page.saturating_sub(1).saturating_mul(page_size);
    let models = filtered.into_iter().skip(start).take(page_size).collect();
    CatalogSearchResult {
        models,
        page,
        page_size,
        total,
        total_pages,
    }
}

fn matches_query(entry: &CatalogModelEntry, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    [
        entry.model.id.as_str(),
        entry.model.name.as_str(),
        entry.model.source.as_str(),
        entry.model.description.as_str(),
        entry.catalog_source_id.as_str(),
    ]
    .into_iter()
    .chain(entry.model.tags.iter().map(String::as_str))
    .any(|value| value.to_ascii_lowercase().contains(query))
}

fn validate_identifier(value: &str, label: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 96 {
        return Err(format!("{label} must contain 1 to 96 characters."));
    }
    if !value.bytes().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == b'-'
    }) || value.starts_with('-')
        || value.ends_with('-')
    {
        return Err(format!(
            "{label} must use lowercase letters, digits, and hyphens."
        ));
    }
    Ok(())
}

fn validate_nonempty(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{label} must not be empty."))
    } else {
        Ok(())
    }
}

fn validate_safe_file_name(value: &str, label: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() || value.contains('\0') {
        return Err(format!("{label} must name a relative file."));
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
        || path.file_name().is_none()
    {
        return Err(format!("{label} must stay inside the model directory."));
    }
    Ok(())
}

fn validate_https_url(value: &str, label: &str) -> Result<Url, String> {
    let url =
        Url::parse(value.trim()).map_err(|_| format!("{label} must be a valid HTTPS URL."))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(format!("{label} must be a credential-free HTTPS URL."));
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if host == "localhost" || host == "::1" || host.starts_with("127.") {
        return Err(format!("{label} cannot target a loopback host."));
    }
    Ok(url)
}

fn is_raw_host(host: &str) -> bool {
    host == "raw.githubusercontent.com" || host.ends_with(".githubusercontent.com")
}

fn is_cdn_host(host: &str) -> bool {
    host == "cdn.jsdelivr.net" || host.ends_with(".cdn.jsdelivr.net")
}

fn has_extension(value: &str, extension: &str) -> bool {
    Path::new(value)
        .extension()
        .and_then(|current| current.to_str())
        .is_some_and(|current| current.eq_ignore_ascii_case(extension))
}

fn parse_spine_version_components(value: &str) -> Result<(u32, u32, u32), String> {
    let value = value.trim();
    let pieces = value.split('.').collect::<Vec<_>>();
    if !(2..=3).contains(&pieces.len()) || pieces.iter().any(|piece| piece.is_empty()) {
        return Err(format!("Invalid Spine version: {value}"));
    }
    let major = pieces[0]
        .parse::<u32>()
        .map_err(|_| format!("Invalid Spine version: {value}"))?;
    let minor = pieces[1]
        .parse::<u32>()
        .map_err(|_| format!("Invalid Spine version: {value}"))?;
    let patch = pieces
        .get(2)
        .map(|piece| piece.parse::<u32>())
        .transpose()
        .map_err(|_| format!("Invalid Spine version: {value}"))?
        .unwrap_or(0);
    Ok((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, body::Bytes, routing::get};
    use futures::stream;
    use std::convert::Infallible;

    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn source(kind: CatalogSourceKind) -> CatalogSource {
        CatalogSource {
            id: "community".to_string(),
            label: "Community".to_string(),
            catalog_url: match kind {
                CatalogSourceKind::CustomRaw => {
                    "https://raw.githubusercontent.com/example/models/main/catalog.json"
                }
                CatalogSourceKind::CustomCdn => {
                    "https://cdn.jsdelivr.net/gh/example/models@main/catalog.json"
                }
                CatalogSourceKind::Official => "https://catalog.example.test/models.json",
            }
            .to_string(),
            kind,
            enabled: true,
        }
    }

    fn model(id: &str, name: &str, spine: &str) -> CatalogModel {
        CatalogModel {
            id: id.to_string(),
            name: name.to_string(),
            source: "Example Models".to_string(),
            author: "Example author".to_string(),
            license: "MIT".to_string(),
            license_warning: String::new(),
            license_note: "User-owned model files only.".to_string(),
            repository_url: "https://github.com/example/models".to_string(),
            skel: "model.skel".to_string(),
            files: vec![
                CatalogFile {
                    name: "model.skel".to_string(),
                    url: "https://raw.githubusercontent.com/example/models/main/model.skel"
                        .to_string(),
                    sha256: SHA.to_string(),
                    github_blob_sha: String::new(),
                    fallback_urls: vec![
                        "https://cdn.jsdelivr.net/gh/example/models@main/model.skel".to_string(),
                    ],
                    size_bytes: Some(12),
                },
                CatalogFile {
                    name: "model.atlas".to_string(),
                    url: "https://raw.githubusercontent.com/example/models/main/model.atlas"
                        .to_string(),
                    sha256: SHA.to_string(),
                    github_blob_sha: String::new(),
                    fallback_urls: vec![],
                    size_bytes: Some(12),
                },
                CatalogFile {
                    name: "model.png".to_string(),
                    url: "https://raw.githubusercontent.com/example/models/main/model.png"
                        .to_string(),
                    sha256: SHA.to_string(),
                    github_blob_sha: String::new(),
                    fallback_urls: vec![],
                    size_bytes: Some(12),
                },
            ],
            spine: SpineCompatibility {
                min: SpineVersion::parse(spine).unwrap(),
                max: None,
            },
            version_verified: true,
            description: "A test model".to_string(),
            tags: vec!["test".to_string()],
            category: "operator".to_string(),
            compatibility_profile: "companion".to_string(),
        }
    }

    #[test]
    fn validates_custom_raw_and_cdn_sources_as_https_only() {
        source(CatalogSourceKind::CustomRaw).validate().unwrap();
        source(CatalogSourceKind::CustomCdn).validate().unwrap();

        let mut invalid = source(CatalogSourceKind::CustomRaw);
        invalid.catalog_url =
            "http://raw.githubusercontent.com/example/models/main/catalog.json".to_string();
        assert!(invalid.validate().unwrap_err().contains("HTTPS"));

        let mut mismatched = source(CatalogSourceKind::CustomCdn);
        mismatched.catalog_url =
            "https://raw.githubusercontent.com/example/models/main/catalog.json".to_string();
        assert!(mismatched.validate().unwrap_err().contains("CDN"));
    }

    #[test]
    fn rejects_catalog_models_without_integrity_metadata() {
        let mut invalid = model("example", "Example", "3.8");
        invalid.files[0].sha256.clear();
        assert!(invalid.validate().unwrap_err().contains("SHA-256"));

        invalid = model("example", "Example", "3.8");
        invalid.files[0].name = "../model.skel".to_string();
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .contains("inside the model directory")
        );
    }

    #[test]
    fn enforces_spine_runtime_line_and_declared_range() {
        let compatibility = SpineCompatibility {
            min: SpineVersion::parse("3.8.10").unwrap(),
            max: Some(SpineVersion::parse("3.8.20").unwrap()),
        };
        compatibility.validate().unwrap();
        assert!(compatibility.is_compatible_with(&SpineVersion::parse("3.8.15").unwrap()));
        assert!(!compatibility.is_compatible_with(&SpineVersion::parse("3.8.9").unwrap()));
        assert!(!compatibility.is_compatible_with(&SpineVersion::parse("3.7.99").unwrap()));
        assert!(!compatibility.is_compatible_with(&SpineVersion::parse("4.0").unwrap()));
    }

    #[test]
    fn searches_and_pages_by_source_and_runtime() {
        let entries = vec![
            CatalogModelEntry {
                catalog_source_id: "official".to_string(),
                model: model("alpha", "Alpha", "3.8"),
            },
            CatalogModelEntry {
                catalog_source_id: "community".to_string(),
                model: model("beta", "Beta", "4.0"),
            },
            CatalogModelEntry {
                catalog_source_id: "community".to_string(),
                model: model("gamma", "Gamma", "3.8"),
            },
        ];
        let result = search_catalog(
            &entries,
            &CatalogSearchRequest {
                query: "test".to_string(),
                source_ids: vec!["community".to_string()],
                page: 1,
                page_size: 1,
                runtime_spine_version: Some(SpineVersion::parse("3.8.99").unwrap()),
                include_incompatible: false,
                installation_filter: String::new(),
            },
        );
        assert_eq!(result.total, 1);
        assert_eq!(result.total_pages, 1);
        assert_eq!(result.models[0].model.id, "gamma");
    }

    #[test]
    fn enforces_catalog_response_size_at_the_declared_boundary() {
        assert!(
            ensure_catalog_size_with_limit(
                MAX_CATALOG_RESPONSE_BYTES as u64,
                MAX_CATALOG_RESPONSE_BYTES
            )
            .is_ok()
        );
        let error = ensure_catalog_size_with_limit(
            MAX_CATALOG_RESPONSE_BYTES as u64 + 1,
            MAX_CATALOG_RESPONSE_BYTES,
        )
        .unwrap_err();
        assert!(error.contains("16 MiB"));
    }

    #[test]
    fn rejects_a_chunk_that_pushes_the_decoded_body_over_limit() {
        let mut body = Vec::new();
        append_catalog_chunk_with_limit(&mut body, b"abc", 5).unwrap();
        append_catalog_chunk_with_limit(&mut body, b"de", 5).unwrap();
        assert_eq!(body, b"abcde");

        let error = append_catalog_chunk_with_limit(&mut body, b"f", 5).unwrap_err();
        assert!(error.contains("5-byte"));
        assert_eq!(body, b"abcde");
    }

    #[tokio::test]
    async fn rejects_declared_and_chunked_oversize_responses() {
        let app = Router::new()
            .route("/declared", get(|| async { "abcdef" }))
            .route(
                "/chunked",
                get(|| async {
                    let chunks = stream::iter([
                        Ok::<_, Infallible>(Bytes::from_static(b"abc")),
                        Ok::<_, Infallible>(Bytes::from_static(b"def")),
                    ]);
                    Body::from_stream(chunks)
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = Client::new();
        let response = client
            .get(format!("http://{address}/declared"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.content_length(), Some(6));
        let error = read_catalog_response_with_limit(response, 5)
            .await
            .unwrap_err();
        assert!(error.contains("5-byte"));

        let response = client
            .get(format!("http://{address}/chunked"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.content_length(), None);
        let error = read_catalog_response_with_limit(response, 5)
            .await
            .unwrap_err();
        assert!(error.contains("5-byte"));
        server.abort();
    }

    #[test]
    fn reads_only_enabled_configured_catalogs_without_mutating_cache() {
        let mut official = source(CatalogSourceKind::Official);
        official.id = "official".to_string();
        official.label = "Official".to_string();
        let mut disabled = source(CatalogSourceKind::Official);
        disabled.id = "disabled".to_string();
        disabled.enabled = false;
        let community = source(CatalogSourceKind::Official);

        let cached = |models| CachedCatalog {
            metadata: SourceCacheMetadata::default(),
            document: CatalogDocument {
                schema_version: CATALOG_SCHEMA_VERSION,
                models,
            },
        };
        let cache = CatalogCache {
            entries: BTreeMap::from([
                (
                    official.id.clone(),
                    cached(vec![
                        model("shared", "Zulu", "3.8"),
                        model("beta", "Beta", "3.8"),
                    ]),
                ),
                (
                    community.id.clone(),
                    cached(vec![
                        model("shared", "Duplicate", "3.8"),
                        model("alpha", "Alpha", "3.8"),
                    ]),
                ),
                (
                    disabled.id.clone(),
                    cached(vec![model("hidden", "Hidden", "3.8")]),
                ),
                (
                    "unconfigured".to_string(),
                    cached(vec![model("orphan", "Orphan", "3.8")]),
                ),
            ]),
        };
        let original_cache = cache.clone();
        let models =
            cached_enabled_models(&[official.clone(), disabled, community, official], &cache);

        assert_eq!(
            models
                .iter()
                .map(|entry| entry.model.id.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "beta", "shared"]
        );
        assert_eq!(models[2].catalog_source_id, "official");
        assert_eq!(models[2].model.name, "Zulu");
        assert_eq!(cache, original_cache);
    }

    #[test]
    fn resolves_an_exact_validated_cached_model() {
        let source = source(CatalogSourceKind::Official);
        let expected = model("alpha", "Alpha", "3.8");
        let cache = CatalogCache {
            entries: BTreeMap::from([(
                source.id.clone(),
                CachedCatalog {
                    metadata: SourceCacheMetadata::default(),
                    document: CatalogDocument {
                        schema_version: CATALOG_SCHEMA_VERSION,
                        models: vec![expected.clone(), model("beta", "Beta", "3.8")],
                    },
                },
            )]),
        };

        let resolved =
            resolve_cached_model_entry(std::slice::from_ref(&source), &cache, &source.id, "alpha")
                .unwrap();

        assert_eq!(resolved.catalog_source_id, source.id);
        assert_eq!(resolved.model, expected);
    }

    #[test]
    fn rejects_missing_disabled_and_ambiguous_sources() {
        let enabled = source(CatalogSourceKind::Official);
        let mut disabled = enabled.clone();
        disabled.id = "disabled".to_string();
        disabled.enabled = false;
        let cache = CatalogCache::default();

        let missing =
            resolve_cached_model_entry(std::slice::from_ref(&enabled), &cache, "missing", "alpha")
                .unwrap_err();
        assert!(missing.contains("not configured"));

        let disabled_error =
            resolve_cached_model_entry(&[disabled], &cache, "disabled", "alpha").unwrap_err();
        assert!(disabled_error.contains("disabled"));

        let ambiguous =
            resolve_cached_model_entry(&[enabled.clone(), enabled], &cache, "community", "alpha")
                .unwrap_err();
        assert!(ambiguous.contains("ambiguous"));
    }

    #[test]
    fn rejects_invalid_source_and_missing_cache_or_model() {
        let valid = source(CatalogSourceKind::Official);
        let mut invalid = valid.clone();
        invalid.catalog_url = "http://catalog.example.test/models.json".to_string();
        let invalid_error =
            resolve_cached_model_entry(&[invalid], &CatalogCache::default(), "community", "alpha")
                .unwrap_err();
        assert!(invalid_error.contains("Invalid catalog source"));

        let missing_cache = resolve_cached_model_entry(
            std::slice::from_ref(&valid),
            &CatalogCache::default(),
            "community",
            "alpha",
        )
        .unwrap_err();
        assert!(missing_cache.contains("No validated cached catalog"));

        let cache = CatalogCache {
            entries: BTreeMap::from([(
                valid.id.clone(),
                CachedCatalog {
                    metadata: SourceCacheMetadata::default(),
                    document: CatalogDocument {
                        schema_version: CATALOG_SCHEMA_VERSION,
                        models: vec![model("beta", "Beta", "3.8")],
                    },
                },
            )]),
        };
        let missing_model =
            resolve_cached_model_entry(&[valid], &cache, "community", "alpha").unwrap_err();
        assert!(missing_model.contains("was not found"));
    }

    #[test]
    fn rejects_invalid_cached_documents_and_models() {
        let source = source(CatalogSourceKind::Official);
        let cached = |document| CatalogCache {
            entries: BTreeMap::from([(
                source.id.clone(),
                CachedCatalog {
                    metadata: SourceCacheMetadata::default(),
                    document,
                },
            )]),
        };

        let invalid_schema = cached(CatalogDocument {
            schema_version: CATALOG_SCHEMA_VERSION + 1,
            models: vec![model("alpha", "Alpha", "3.8")],
        });
        let schema_error = resolve_cached_model_entry(
            std::slice::from_ref(&source),
            &invalid_schema,
            "community",
            "alpha",
        )
        .unwrap_err();
        assert!(schema_error.contains("Invalid cached catalog"));
        assert!(schema_error.contains("schema version"));

        let mut invalid_model = model("alpha", "Alpha", "3.8");
        invalid_model.files[0].sha256.clear();
        let invalid_model_cache = cached(CatalogDocument {
            schema_version: CATALOG_SCHEMA_VERSION,
            models: vec![invalid_model],
        });
        let model_error = resolve_cached_model_entry(
            std::slice::from_ref(&source),
            &invalid_model_cache,
            "community",
            "alpha",
        )
        .unwrap_err();
        assert!(model_error.contains("Invalid cached catalog"));
        assert!(model_error.contains("SHA-256"));

        let duplicate_models = cached(CatalogDocument {
            schema_version: CATALOG_SCHEMA_VERSION,
            models: vec![
                model("alpha", "Alpha", "3.8"),
                model("alpha", "Duplicate Alpha", "3.8"),
            ],
        });
        let duplicate_error =
            resolve_cached_model_entry(&[source], &duplicate_models, "community", "alpha")
                .unwrap_err();
        assert!(duplicate_error.contains("duplicate model id"));
    }

    #[test]
    fn preserves_stale_cache_after_a_source_error() {
        let source = source(CatalogSourceKind::Official);
        let cached = CachedCatalog {
            metadata: SourceCacheMetadata {
                etag: Some("catalog-v1".to_string()),
                last_modified: Some("Mon, 01 Jan 2024 00:00:00 GMT".to_string()),
                fetched_at_unix_ms: 1,
            },
            document: CatalogDocument {
                schema_version: CATALOG_SCHEMA_VERSION,
                models: vec![model("cached", "Cached", "3.8")],
            },
        };
        let result = stale_or_failed(&source, Some(cached), "offline".to_string());
        assert_eq!(result.status.state, CatalogSourceState::Stale);
        assert_eq!(result.status.model_count, 1);
        assert_eq!(result.cache_entry.unwrap().document.models[0].id, "cached");
    }
}
