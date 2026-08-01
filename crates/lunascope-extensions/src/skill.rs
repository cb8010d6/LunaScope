use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use lunascope_core::{
    LoadedSkill, PermissionKind, SkillCompatibility, SkillSourceKind, SkillSummary,
};
use serde_yaml_ng::{Mapping, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

const MAX_SKILL_BYTES: u64 = 1024 * 1024;
const MAX_RESOURCE_BYTES: u64 = 512 * 1024;
const MAX_FRONTMATTER_BYTES: u64 = 64 * 1024;
const MAX_DEPTH: usize = 8;
const MAX_SUPPORTING_FILES: usize = 100;

#[derive(Clone, Debug)]
pub struct SkillRoot {
    pub source: SkillSourceKind,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct SkillCatalog {
    entries: BTreeMap<String, SkillSummary>,
    roots: Vec<PathBuf>,
}

impl SkillCatalog {
    pub fn discover(roots: &[SkillRoot]) -> Result<Self, SkillError> {
        let mut entries = BTreeMap::new();
        let mut canonical_roots = Vec::new();
        for root in roots {
            if !root.path.exists() {
                continue;
            }
            let canonical_root = root.path.canonicalize()?;
            if !canonical_roots.contains(&canonical_root) {
                canonical_roots.push(canonical_root.clone());
            }
            discover_directory(
                root.source,
                &canonical_root,
                &canonical_root,
                0,
                &mut entries,
            )?;
        }
        Ok(Self {
            entries,
            roots: canonical_roots,
        })
    }

    pub fn summaries(&self) -> Vec<SkillSummary> {
        self.entries.values().cloned().collect()
    }

    pub fn summary(&self, catalog_id: &str) -> Option<&SkillSummary> {
        self.entries.get(catalog_id)
    }

    pub fn load(&self, catalog_id: &str) -> Result<LoadedSkill, SkillError> {
        let summary = self
            .entries
            .get(catalog_id)
            .cloned()
            .ok_or_else(|| SkillError::UnknownSkill(catalog_id.into()))?;
        if summary.compatibility == SkillCompatibility::Unsupported {
            return Err(SkillError::Unsupported(summary.warnings.join("; ")));
        }

        let path = PathBuf::from(&summary.path);
        let bytes = read_bounded(&path, MAX_SKILL_BYTES)?;
        let actual_hash = hex_hash(&bytes);
        if actual_hash != summary.content_sha256 {
            return Err(SkillError::ChangedSinceDiscovery {
                expected: summary.content_sha256,
                actual: actual_hash,
            });
        }
        let content = String::from_utf8(bytes).map_err(|_| SkillError::NotUtf8(path.clone()))?;
        let (frontmatter, instructions) = split_frontmatter(&content)?;
        let yaml: Value = serde_yaml_ng::from_str(frontmatter)?;
        let preserved_frontmatter = serde_json::to_value(yaml)?;
        let base = path
            .parent()
            .ok_or_else(|| SkillError::InvalidLayout(path.clone()))?;
        let mut supporting_files = Vec::new();
        collect_supporting_files(base, base, 0, &mut supporting_files)?;

        Ok(LoadedSkill {
            summary,
            instructions: instructions.trim().to_owned(),
            supporting_files,
            preserved_frontmatter,
        })
    }

    /// Reads a UTF-8 reference, template, example, schema, or script source
    /// belonging to a discovered Skill without executing it.
    ///
    /// Claude Code and Codex Skills commonly link both to files beside
    /// `SKILL.md` and to repository-level shared resources. Resolution
    /// therefore checks the Skill directory first and then its discovery root.
    /// In both cases the final path must remain inside that root.
    pub fn read_resource(
        &self,
        catalog_id: &str,
        relative_path: &str,
    ) -> Result<String, SkillError> {
        self.read_resource_from(catalog_id, relative_path, None)
    }

    pub fn read_resource_from(
        &self,
        catalog_id: &str,
        relative_path: &str,
        base_path: Option<&str>,
    ) -> Result<String, SkillError> {
        let summary = self
            .entries
            .get(catalog_id)
            .ok_or_else(|| SkillError::UnknownSkill(catalog_id.into()))?;
        let skill_path = PathBuf::from(&summary.path);
        let skill_directory = skill_path
            .parent()
            .ok_or_else(|| SkillError::InvalidLayout(skill_path.clone()))?;
        let root = self
            .roots
            .iter()
            .filter(|root| skill_path.starts_with(root))
            .max_by_key(|root| root.components().count())
            .ok_or_else(|| SkillError::OutsideRoot(skill_path.clone()))?;

        let requested = Path::new(relative_path.trim());
        if relative_path.trim().is_empty()
            || requested.is_absolute()
            || requested.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::Prefix(_) | std::path::Component::RootDir
                )
            })
        {
            return Err(SkillError::InvalidResourcePath(relative_path.into()));
        }

        let mut candidates = Vec::new();
        if let Some(base_path) = base_path.filter(|path| !path.trim().is_empty()) {
            let base = resolve_skill_resource_path(root, skill_directory, Path::new(base_path))?;
            let base_directory = if base.is_dir() {
                base
            } else {
                base.parent()
                    .ok_or_else(|| SkillError::InvalidLayout(base.clone()))?
                    .to_path_buf()
            };
            candidates.push(base_directory.join(requested));
        }
        candidates.push(skill_directory.join(requested));
        let root_candidate = root.join(requested);
        if !candidates.contains(&root_candidate) {
            candidates.push(root_candidate);
        }
        for candidate in candidates {
            let Ok(canonical) = resolve_candidate(root, &candidate) else {
                continue;
            };
            if !canonical.is_file() {
                return Err(SkillError::ResourceIsNotFile(canonical));
            }
            let bytes = read_bounded_resource(&canonical, MAX_RESOURCE_BYTES)?;
            return String::from_utf8(bytes).map_err(|_| SkillError::NotUtf8(canonical));
        }
        Err(SkillError::ResourceNotFound(relative_path.into()))
    }
}

fn resolve_skill_resource_path(
    root: &Path,
    skill_directory: &Path,
    requested: &Path,
) -> Result<PathBuf, SkillError> {
    if requested.as_os_str().is_empty()
        || requested.is_absolute()
        || requested.components().any(|component| {
            matches!(
                component,
                std::path::Component::Prefix(_) | std::path::Component::RootDir
            )
        })
    {
        return Err(SkillError::InvalidResourcePath(
            requested.to_string_lossy().into_owned(),
        ));
    }
    for candidate in [skill_directory.join(requested), root.join(requested)] {
        if let Ok(canonical) = resolve_candidate(root, &candidate) {
            return Ok(canonical);
        }
    }
    Err(SkillError::ResourceNotFound(
        requested.to_string_lossy().into_owned(),
    ))
}

fn resolve_candidate(root: &Path, candidate: &Path) -> Result<PathBuf, SkillError> {
    let canonical = candidate
        .canonicalize()
        .map_err(|_| SkillError::ResourceNotFound(candidate.to_string_lossy().into_owned()))?;
    if !canonical.starts_with(root) {
        return Err(SkillError::OutsideRoot(canonical));
    }
    reject_symlink_components(root, candidate)?;
    Ok(canonical)
}

fn reject_symlink_components(root: &Path, candidate: &Path) -> Result<(), SkillError> {
    let relative = candidate
        .strip_prefix(root)
        .map_err(|_| SkillError::OutsideRoot(candidate.to_owned()))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if current == root {
                    return Err(SkillError::OutsideRoot(candidate.to_owned()));
                }
                current.pop();
            }
            std::path::Component::Normal(part) => {
                current.push(part);
                if let Ok(metadata) = fs::symlink_metadata(&current) {
                    if metadata.file_type().is_symlink() {
                        return Err(SkillError::SymlinkResource(current));
                    }
                }
            }
            std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                return Err(SkillError::InvalidResourcePath(
                    candidate.to_string_lossy().into_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn discover_directory(
    source: SkillSourceKind,
    canonical_root: &Path,
    current: &Path,
    depth: usize,
    entries: &mut BTreeMap<String, SkillSummary>,
) -> Result<(), SkillError> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    for child in fs::read_dir(current)? {
        let child = child?;
        let file_type = child.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let path = child.path();
        if file_type.is_dir() {
            discover_directory(source, canonical_root, &path, depth + 1, entries)?;
        } else if file_type.is_file() && child.file_name() == "SKILL.md" {
            let summary = summarize_skill(source, canonical_root, &path)?;
            if entries
                .insert(summary.catalog_id.clone(), summary)
                .is_some()
            {
                return Err(SkillError::DuplicateCatalogId(path));
            }
        }
    }
    Ok(())
}

fn summarize_skill(
    source: SkillSourceKind,
    canonical_root: &Path,
    path: &Path,
) -> Result<SkillSummary, SkillError> {
    let canonical_path = path.canonicalize()?;
    if !canonical_path.starts_with(canonical_root) {
        return Err(SkillError::OutsideRoot(canonical_path));
    }
    let metadata = fs::metadata(&canonical_path)?;
    let bytes = read_bounded(&canonical_path, MAX_SKILL_BYTES)?;
    let content_hash = hex_hash(&bytes);
    let content =
        String::from_utf8(bytes).map_err(|_| SkillError::NotUtf8(canonical_path.clone()))?;
    let (frontmatter_text, instructions) = split_frontmatter(&content)?;
    if frontmatter_text.len() as u64 > MAX_FRONTMATTER_BYTES {
        return Err(SkillError::FrontmatterTooLarge(canonical_path));
    }
    let yaml: Value = serde_yaml_ng::from_str(frontmatter_text)?;
    let mapping = yaml
        .as_mapping()
        .ok_or_else(|| SkillError::FrontmatterNotMapping(canonical_path.clone()))?;

    let directory_id = canonical_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .ok_or_else(|| SkillError::InvalidLayout(canonical_path.clone()))?
        .to_owned();
    let id = portable_id(&directory_id);
    let name = string_value(mapping, "name").unwrap_or_else(|| directory_id.clone());
    let description = string_value(mapping, "description").unwrap_or_default();
    let relative = canonical_path
        .strip_prefix(canonical_root)
        .map_err(|_| SkillError::OutsideRoot(canonical_path.clone()))?;
    let catalog_id = format!(
        "{}:{}",
        source_label(source),
        relative
            .to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches("/SKILL.md")
    );

    let metadata_map = mapping_value(mapping, "metadata");
    let tags = metadata_list(metadata_map, "lunascope/tags");
    let required_tools = metadata_list(metadata_map, "lunascope/required-tools");
    let (required_permissions, mut warnings) = parse_permissions(metadata_list(
        metadata_map,
        "lunascope/required-permissions",
    ));

    let mut compatibility = SkillCompatibility::Native;
    if description.trim().is_empty() {
        compatibility = SkillCompatibility::Unsupported;
        warnings.push("description is required for safe discovery".into());
    }
    if !valid_portable_id(&id) || name.trim().is_empty() {
        compatibility = SkillCompatibility::Unsupported;
        warnings.push("skill name or path-derived ID is invalid".into());
    }

    let bridge_fields = ["hooks", "context", "agent", "background", "shell"];
    let compatible_fields = [
        "allowed-tools",
        "disallowed-tools",
        "model",
        "effort",
        "paths",
        "disable-model-invocation",
        "user-invocable",
    ];
    if bridge_fields.iter().any(|field| has_key(mapping, field))
        || contains_dynamic_shell(instructions)
        || has_scripts(canonical_path.parent().expect("skill path has parent"))?
    {
        compatibility = SkillCompatibility::BridgeRequired;
        warnings.push(
            "runtime-specific execution semantics are preserved but not executed in-process".into(),
        );
    } else if compatible_fields
        .iter()
        .any(|field| has_key(mapping, field))
        || matches!(
            source,
            SkillSourceKind::Claude | SkillSourceKind::OpenCode | SkillSourceKind::Codex
        )
    {
        compatibility = SkillCompatibility::Compatible;
    }

    let user_invocable = bool_value(mapping, "user-invocable").unwrap_or(true);
    let model_invocable = !bool_value(mapping, "disable-model-invocation").unwrap_or(false)
        && metadata_bool(metadata_map, "opencode/autoinvoke").unwrap_or(true);

    Ok(SkillSummary {
        catalog_id,
        id,
        name,
        description,
        tags,
        required_tools,
        required_permissions,
        approximate_context_tokens: metadata.len().div_ceil(4),
        source,
        path: canonical_path.to_string_lossy().into_owned(),
        compatibility,
        warnings,
        content_sha256: content_hash,
        user_invocable,
        model_invocable,
    })
}

fn split_frontmatter(content: &str) -> Result<(&str, &str), SkillError> {
    let normalized = content.strip_prefix('\u{feff}').unwrap_or(content);
    let first_end = normalized
        .find('\n')
        .ok_or(SkillError::MissingFrontmatter)?;
    if normalized[..first_end].trim_end_matches('\r') != "---" {
        return Err(SkillError::MissingFrontmatter);
    }
    let rest = &normalized[first_end + 1..];
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_matches(['\r', '\n']) == "---" {
            return Ok((&rest[..offset], &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    Err(SkillError::MissingFrontmatterTerminator)
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, SkillError> {
    let mut file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    if metadata.len() > limit {
        return Err(SkillError::SkillTooLarge(path.to_owned()));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.by_ref().take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(SkillError::SkillTooLarge(path.to_owned()));
    }
    Ok(bytes)
}

fn read_bounded_resource(path: &Path, limit: u64) -> Result<Vec<u8>, SkillError> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > limit {
        return Err(SkillError::ResourceTooLarge(path.to_owned()));
    }
    let mut file = fs::File::open(path)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.by_ref().take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(SkillError::ResourceTooLarge(path.to_owned()));
    }
    Ok(bytes)
}

fn collect_supporting_files(
    base: &Path,
    current: &Path,
    depth: usize,
    output: &mut Vec<String>,
) -> Result<(), SkillError> {
    if depth > MAX_DEPTH || output.len() >= MAX_SUPPORTING_FILES {
        return Ok(());
    }
    for child in fs::read_dir(current)? {
        let child = child?;
        let file_type = child.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let path = child.path();
        if file_type.is_dir() {
            collect_supporting_files(base, &path, depth + 1, output)?;
        } else if file_type.is_file() && child.file_name() != "SKILL.md" {
            let relative = path
                .strip_prefix(base)
                .map_err(|_| SkillError::OutsideRoot(path.clone()))?;
            output.push(relative.to_string_lossy().replace('\\', "/"));
            if output.len() >= MAX_SUPPORTING_FILES {
                break;
            }
        }
    }
    output.sort();
    Ok(())
}

fn has_scripts(base: &Path) -> Result<bool, SkillError> {
    let scripts = base.join("scripts");
    Ok(scripts.is_dir() && fs::read_dir(scripts)?.next().transpose()?.is_some())
}

fn contains_dynamic_shell(body: &str) -> bool {
    body.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("!`") || trimmed.starts_with("```!")
    })
}

fn valid_portable_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn portable_id(value: &str) -> String {
    let mut result = String::new();
    let mut previous_hyphen = false;
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            result.push(character);
            previous_hyphen = false;
        } else if !previous_hyphen && !result.is_empty() {
            result.push('-');
            previous_hyphen = true;
        }
    }
    result.trim_matches('-').chars().take(64).collect()
}

fn source_label(source: SkillSourceKind) -> &'static str {
    match source {
        SkillSourceKind::Agents => "agents",
        SkillSourceKind::Codex => "codex",
        SkillSourceKind::Claude => "claude",
        SkillSourceKind::OpenCode => "opencode",
        SkillSourceKind::LunaScopeGlobal => "lunascope-global",
        SkillSourceKind::LunaScopeProject => "lunascope-project",
        SkillSourceKind::LunaScopeSystem => "lunascope-system",
        SkillSourceKind::LunaScopeUser => "lunascope-user",
    }
}

fn string_value(mapping: &Mapping, key: &str) -> Option<String> {
    mapping
        .get(Value::String(key.into()))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn bool_value(mapping: &Mapping, key: &str) -> Option<bool> {
    mapping
        .get(Value::String(key.into()))
        .and_then(Value::as_bool)
}

fn mapping_value<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a Mapping> {
    mapping
        .get(Value::String(key.into()))
        .and_then(Value::as_mapping)
}

fn has_key(mapping: &Mapping, key: &str) -> bool {
    mapping.contains_key(Value::String(key.into()))
}

fn metadata_list(metadata: Option<&Mapping>, key: &str) -> Vec<String> {
    let Some(value) = metadata.and_then(|map| map.get(Value::String(key.into()))) else {
        return Vec::new();
    };
    match value {
        Value::Sequence(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Value::String(value) => value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

fn metadata_bool(metadata: Option<&Mapping>, key: &str) -> Option<bool> {
    let value = metadata?.get(Value::String(key.into()))?;
    value.as_bool().or_else(|| match value.as_str() {
        Some("true") => Some(true),
        Some("false") => Some(false),
        _ => None,
    })
}

fn parse_permissions(values: Vec<String>) -> (Vec<PermissionKind>, Vec<String>) {
    let mut permissions = Vec::new();
    let mut warnings = Vec::new();
    for value in values {
        let permission = match value.as_str() {
            "filesystem_read" => PermissionKind::FilesystemRead,
            "filesystem_write" => PermissionKind::FilesystemWrite,
            "filesystem_delete" => PermissionKind::FilesystemDelete,
            "shell_execute" => PermissionKind::ShellExecute,
            "process_spawn" => PermissionKind::ProcessSpawn,
            "network_connect" => PermissionKind::NetworkConnect,
            "browser_control" => PermissionKind::BrowserControl,
            "secrets_use" => PermissionKind::SecretsUse,
            "git_commit" => PermissionKind::GitCommit,
            "git_push" => PermissionKind::GitPush,
            "mcp_invoke" => PermissionKind::McpInvoke,
            "skill_load" => PermissionKind::SkillLoad,
            "worker_spawn" => PermissionKind::WorkerSpawn,
            "extension_execute" => PermissionKind::ExtensionExecute,
            _ => {
                warnings.push(format!("unknown required permission preserved: {value}"));
                continue;
            }
        };
        if !permissions.contains(&permission) {
            permissions.push(permission);
        }
    }
    (permissions, warnings)
}

fn hex_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Error)]
pub enum SkillError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Yaml(#[from] serde_yaml_ng::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("skill has no YAML frontmatter")]
    MissingFrontmatter,
    #[error("skill YAML frontmatter has no closing delimiter")]
    MissingFrontmatterTerminator,
    #[error("skill frontmatter is not a YAML mapping: {0}")]
    FrontmatterNotMapping(PathBuf),
    #[error("skill frontmatter exceeds the 64 KiB limit: {0}")]
    FrontmatterTooLarge(PathBuf),
    #[error("skill exceeds the 1 MiB limit: {0}")]
    SkillTooLarge(PathBuf),
    #[error("skill is not UTF-8: {0}")]
    NotUtf8(PathBuf),
    #[error("skill layout is invalid: {0}")]
    InvalidLayout(PathBuf),
    #[error("skill path resolves outside its discovery root: {0}")]
    OutsideRoot(PathBuf),
    #[error("duplicate skill catalog ID while scanning {0}")]
    DuplicateCatalogId(PathBuf),
    #[error("skill is not in the catalog: {0}")]
    UnknownSkill(String),
    #[error("skill is unsupported: {0}")]
    Unsupported(String),
    #[error("skill changed after discovery; expected {expected}, found {actual}; rescan required")]
    ChangedSinceDiscovery { expected: String, actual: String },
    #[error("Skill resource path is invalid: {0}")]
    InvalidResourcePath(String),
    #[error("Skill resource was not found in the selected package: {0}")]
    ResourceNotFound(String),
    #[error("Skill resource is not a regular file: {0}")]
    ResourceIsNotFile(PathBuf),
    #[error("Skill resource exceeds the 512 KiB read limit: {0}")]
    ResourceTooLarge(PathBuf),
    #[error("Skill resource contains a symbolic link and was not read: {0}")]
    SymlinkResource(PathBuf),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(root: &Path, name: &str, content: &str) {
        let directory = root.join(name);
        fs::create_dir_all(&directory).expect("skill directory");
        fs::write(directory.join("SKILL.md"), content).expect("skill file");
    }

    #[test]
    fn discovers_metadata_and_loads_full_body_only_on_demand() {
        let temp = tempfile::tempdir().expect("temp");
        write_skill(
            temp.path(),
            "repo-review",
            "---\nname: repo-review\ndescription: Review a repository safely\nmetadata:\n  lunascope/tags: [review, git]\n  lunascope/required-tools: [filesystem.read]\n  lunascope/required-permissions: [filesystem_read]\n---\n\nNever mutate the repository.\n",
        );
        fs::write(
            temp.path().join("repo-review").join("reference.md"),
            "details",
        )
        .expect("reference");

        let catalog = SkillCatalog::discover(&[SkillRoot {
            source: SkillSourceKind::Agents,
            path: temp.path().into(),
        }])
        .expect("discover");
        let summaries = catalog.summaries();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].required_tools, ["filesystem.read"]);
        assert_eq!(
            summaries[0].required_permissions,
            [PermissionKind::FilesystemRead]
        );

        let loaded = catalog.load(&summaries[0].catalog_id).expect("load");
        assert_eq!(loaded.instructions, "Never mutate the repository.");
        assert_eq!(loaded.supporting_files, ["reference.md"]);
    }

    #[test]
    fn classifies_claude_execution_extensions_as_bridge_required() {
        let temp = tempfile::tempdir().expect("temp");
        write_skill(
            temp.path(),
            "dynamic-check",
            "---\nname: dynamic-check\ndescription: Inject a live diff\nshell: powershell\n---\n\n!`git diff`\n",
        );
        let catalog = SkillCatalog::discover(&[SkillRoot {
            source: SkillSourceKind::Claude,
            path: temp.path().into(),
        }])
        .expect("discover");
        assert_eq!(
            catalog.summaries()[0].compatibility,
            SkillCompatibility::BridgeRequired
        );
    }

    #[test]
    fn refuses_changed_content_until_rescan() {
        let temp = tempfile::tempdir().expect("temp");
        write_skill(
            temp.path(),
            "stable",
            "---\nname: stable\ndescription: Stable instructions\n---\n\nFirst version\n",
        );
        let catalog = SkillCatalog::discover(&[SkillRoot {
            source: SkillSourceKind::LunaScopeProject,
            path: temp.path().into(),
        }])
        .expect("discover");
        let summary = catalog.summaries().remove(0);
        fs::write(
            temp.path().join("stable").join("SKILL.md"),
            "---\nname: stable\ndescription: Stable instructions\n---\n\nChanged\n",
        )
        .expect("change");
        assert!(matches!(
            catalog.load(&summary.catalog_id),
            Err(SkillError::ChangedSinceDiscovery { .. })
        ));
    }

    #[test]
    fn reads_local_and_shared_skill_resources_without_executing_them() {
        let temp = tempfile::tempdir().expect("temp");
        write_skill(
            temp.path(),
            "academic-paper",
            "---\nname: academic-paper\ndescription: Write a paper\n---\n\nRead references/checklist.md and shared/schema.json.\n",
        );
        fs::create_dir_all(temp.path().join("academic-paper").join("references"))
            .expect("references");
        fs::write(
            temp.path()
                .join("academic-paper")
                .join("references")
                .join("checklist.md"),
            "local checklist",
        )
        .expect("local resource");
        fs::create_dir_all(temp.path().join("shared")).expect("shared");
        fs::write(temp.path().join("shared").join("schema.json"), "{}").expect("shared resource");
        fs::create_dir_all(
            temp.path()
                .join("academic-paper")
                .join("references")
                .join("nested"),
        )
        .expect("nested references");
        fs::write(
            temp.path()
                .join("academic-paper")
                .join("references")
                .join("nested")
                .join("guide.md"),
            "follow ../../../shared/schema.json",
        )
        .expect("nested guide");

        let catalog = SkillCatalog::discover(&[SkillRoot {
            source: SkillSourceKind::Claude,
            path: temp.path().into(),
        }])
        .expect("discover");
        let id = catalog.summaries()[0].catalog_id.clone();
        assert_eq!(
            catalog
                .read_resource(&id, "references/checklist.md")
                .expect("local"),
            "local checklist"
        );
        assert_eq!(
            catalog
                .read_resource(&id, "shared/schema.json")
                .expect("root fallback"),
            "{}"
        );
        assert_eq!(
            catalog
                .read_resource_from(
                    &id,
                    "../../../shared/schema.json",
                    Some("references/nested/guide.md"),
                )
                .expect("relative linked resource"),
            "{}"
        );
    }

    #[test]
    fn rejects_skill_resource_escape() {
        let parent = tempfile::tempdir().expect("parent");
        let root = parent.path().join("skills");
        fs::create_dir_all(&root).expect("root");
        write_skill(
            &root,
            "safe",
            "---\nname: safe\ndescription: Safe Skill\n---\n\nSafe.\n",
        );
        fs::write(parent.path().join("secret.txt"), "secret").expect("outside file");
        let catalog = SkillCatalog::discover(&[SkillRoot {
            source: SkillSourceKind::Codex,
            path: root,
        }])
        .expect("discover");
        let id = catalog.summaries()[0].catalog_id.clone();
        assert!(matches!(
            catalog.read_resource(&id, "../../secret.txt"),
            Err(SkillError::ResourceNotFound(_)) | Err(SkillError::OutsideRoot(_))
        ));
    }
}
