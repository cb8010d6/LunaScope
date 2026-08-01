use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fs,
    path::{Component, Path, PathBuf},
    process::{Command, Output, Stdio},
};

use lunascope_core::{
    GithubImportPreview, GithubImportSource, ImportComparison, ImportComponent,
    ImportComponentKind, ImportFileRecord, ImportInstallRequest, ImportRiskKind,
    InstalledImportVersion, LicenseFinding, LicenseStatus, PermissionContext, PermissionKind,
    PermissionRequest, RiskLevel, SkillCompatibility,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

const MAX_REMOTE_OUTPUT: usize = 4 * 1024 * 1024;
const MAX_TREE_OUTPUT: usize = 16 * 1024 * 1024;
const MAX_FILES: usize = 10_000;
const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const MAX_INSPECT_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct GithubImportManager {
    data_root: PathBuf,
    git_program: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallationManifest {
    installation_id: String,
    workspace_root: String,
    installed_path: String,
    active_version_id: String,
    versions: Vec<InstalledVersionDisk>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstalledVersionDisk {
    public: InstalledImportVersion,
    files: BTreeMap<String, String>,
}

impl GithubImportManager {
    pub fn new(data_root: impl AsRef<Path>) -> Result<Self, GithubImportError> {
        let git_program = resolve_git()?;
        Ok(Self {
            data_root: data_root.as_ref().to_path_buf(),
            git_program,
        })
    }

    pub fn parse_source(url: &str) -> Result<GithubImportSource, GithubImportError> {
        let parsed = Url::parse(url)?;
        if parsed.scheme() != "https"
            || parsed.host_str() != Some("github.com")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(GithubImportError::UnsupportedGithubUrl);
        }
        if parsed.path().contains('%') || parsed.path().contains('\\') {
            return Err(GithubImportError::UnsupportedGithubUrl);
        }
        let segments = parsed
            .path_segments()
            .ok_or(GithubImportError::UnsupportedGithubUrl)?
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        if segments.len() < 2 {
            return Err(GithubImportError::UnsupportedGithubUrl);
        }
        let owner = validate_slug(segments[0], "owner")?;
        let repository = validate_slug(segments[1].trim_end_matches(".git"), "repository")?;
        let tree_tail = if segments.len() == 2 {
            Vec::new()
        } else if segments.get(2) == Some(&"tree") && segments.len() >= 4 {
            segments[3..]
                .iter()
                .map(|value| (*value).to_owned())
                .collect()
        } else {
            return Err(GithubImportError::UnsupportedGithubUrl);
        };
        Ok(GithubImportSource {
            repository_url: format!("https://github.com/{owner}/{repository}.git"),
            owner,
            repository,
            requested_ref: (!tree_tail.is_empty()).then(|| tree_tail.join("/")),
            subdirectory: None,
        })
    }

    pub fn preview(&self, url: &str) -> Result<GithubImportPreview, GithubImportError> {
        let mut source = Self::parse_source(url)?;
        let fetch_ref = if let Some(tree_tail) = &source.requested_ref {
            let (resolved_ref, subdirectory) =
                self.resolve_tree_ref(&source.repository_url, tree_tail)?;
            source.requested_ref = Some(resolved_ref.clone());
            source.subdirectory = subdirectory;
            resolved_ref
        } else {
            "HEAD".to_owned()
        };

        let import_id = Uuid::new_v4().to_string();
        let quarantine = self
            .data_root
            .join("quarantine")
            .join("imports")
            .join(&import_id);
        let repository = quarantine.join("repository.git");
        fs::create_dir_all(&quarantine)?;
        self.git(
            None,
            ["init".as_ref(), "--bare".as_ref(), repository.as_os_str()],
            MAX_REMOTE_OUTPUT,
        )?;
        self.git(
            Some(&repository),
            [
                OsStr::new("remote"),
                OsStr::new("add"),
                OsStr::new("origin"),
                OsStr::new(&source.repository_url),
            ],
            MAX_REMOTE_OUTPUT,
        )?;
        self.git(
            Some(&repository),
            [
                OsStr::new("fetch"),
                OsStr::new("--no-tags"),
                OsStr::new("--depth=1"),
                OsStr::new("--filter=blob:limit=16777216"),
                OsStr::new("--"),
                OsStr::new("origin"),
                OsStr::new(&fetch_ref),
            ],
            MAX_REMOTE_OUTPUT,
        )?;
        let commit_sha = self.git_text(
            Some(&repository),
            ["rev-parse", "FETCH_HEAD^{commit}"],
            MAX_REMOTE_OUTPUT,
        )?;
        let commit_sha = commit_sha.trim().to_owned();
        validate_commit_sha(&commit_sha)?;
        let preview =
            self.analyze_repository(&import_id, source, &commit_sha, &quarantine, &repository)?;
        write_json_atomic(&quarantine.join("preview.json"), &preview)?;
        Ok(preview)
    }

    pub fn install(
        &self,
        request: &ImportInstallRequest,
    ) -> Result<InstalledImportVersion, GithubImportError> {
        if request.component_ids.is_empty() {
            return Err(GithubImportError::NoComponentsSelected);
        }
        let quarantine = self
            .data_root
            .join("quarantine")
            .join("imports")
            .join(&request.import_id);
        let preview: GithubImportPreview = read_json(&quarantine.join("preview.json"))?;
        if preview.import_id != request.import_id {
            return Err(GithubImportError::ManifestMismatch);
        }
        if preview.blocked {
            return Err(GithubImportError::BlockedImport);
        }
        let selected = preview
            .components
            .iter()
            .filter(|component| request.component_ids.contains(&component.id))
            .collect::<Vec<_>>();
        if selected.len() != request.component_ids.iter().collect::<BTreeSet<_>>().len() {
            return Err(GithubImportError::UnknownComponent);
        }

        let workspace = canonical_directory(Path::new(&request.workspace_root))?;
        let key = installation_key(&preview.source, &workspace);
        let installation_root = self.data_root.join("imports").join("installed").join(&key);
        let manifest_path = installation_root.join("manifest.json");
        let destination = workspace.join(".lunascope").join("imports").join(format!(
            "{}-{}",
            preview.source.owner, preview.source.repository
        ));
        let installation_id = if manifest_path.exists() {
            read_json::<InstallationManifest>(&manifest_path)?.installation_id
        } else {
            Uuid::new_v4().to_string()
        };
        let version_id = Uuid::new_v4().to_string();
        let snapshot = installation_root
            .join("versions")
            .join(&version_id)
            .join("files");
        fs::create_dir_all(&snapshot)?;

        let repository = quarantine.join("repository.git");
        let selected_files = selected_files(&preview, &selected);
        let mut file_objects = BTreeMap::new();
        for file in selected_files {
            if !file.extractable {
                continue;
            }
            let relative = safe_relative_path(&file.path)?;
            let target = snapshot.join(relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let bytes = self.git_bytes(
                Some(&repository),
                [
                    OsStr::new("cat-file"),
                    OsStr::new("blob"),
                    OsStr::new(&file.git_object_id),
                ],
                usize::try_from(file.size_bytes)
                    .unwrap_or(usize::MAX)
                    .saturating_add(1),
            )?;
            if bytes.len() as u64 != file.size_bytes {
                return Err(GithubImportError::BlobSizeMismatch(file.path.clone()));
            }
            fs::write(&target, bytes)?;
            file_objects.insert(file.path.clone(), file.git_object_id.clone());
        }

        replace_directory_from_snapshot(&snapshot, &destination)?;
        let installed_at = jiff::Timestamp::now().to_string();
        let public = InstalledImportVersion {
            installation_id: installation_id.clone(),
            version_id: version_id.clone(),
            source: preview.source.clone(),
            commit_sha: preview.commit_sha.clone(),
            content_sha256: preview.content_sha256.clone(),
            installed_path: destination.to_string_lossy().into_owned(),
            component_ids: request.component_ids.clone(),
            installed_at,
            active: true,
        };
        let mut manifest = if manifest_path.exists() {
            read_json::<InstallationManifest>(&manifest_path)?
        } else {
            InstallationManifest {
                installation_id,
                workspace_root: workspace.to_string_lossy().into_owned(),
                installed_path: destination.to_string_lossy().into_owned(),
                active_version_id: version_id.clone(),
                versions: Vec::new(),
            }
        };
        for version in &mut manifest.versions {
            version.public.active = false;
        }
        manifest.active_version_id = version_id;
        manifest.versions.push(InstalledVersionDisk {
            public: public.clone(),
            files: file_objects,
        });
        write_json_atomic(&manifest_path, &manifest)?;
        Ok(public)
    }

    pub fn list_installed(&self) -> Result<Vec<InstalledImportVersion>, GithubImportError> {
        let root = self.data_root.join("imports").join("installed");
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut versions = Vec::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let manifest_path = entry.path().join("manifest.json");
            if manifest_path.is_file() {
                let manifest: InstallationManifest = read_json(&manifest_path)?;
                versions.extend(manifest.versions.into_iter().map(|version| version.public));
            }
        }
        versions.sort_by(|left, right| {
            left.installation_id
                .cmp(&right.installation_id)
                .then_with(|| right.installed_at.cmp(&left.installed_at))
        });
        Ok(versions)
    }

    pub fn rollback(
        &self,
        installation_id: &str,
        version_id: &str,
    ) -> Result<InstalledImportVersion, GithubImportError> {
        let (manifest_path, mut manifest) = self.find_installation(installation_id)?;
        let target = manifest
            .versions
            .iter()
            .find(|version| version.public.version_id == version_id)
            .cloned()
            .ok_or_else(|| GithubImportError::VersionNotFound(version_id.to_owned()))?;
        let snapshot = manifest_path
            .parent()
            .expect("manifest has parent")
            .join("versions")
            .join(version_id)
            .join("files");
        replace_directory_from_snapshot(&snapshot, Path::new(&manifest.installed_path))?;
        for version in &mut manifest.versions {
            version.public.active = version.public.version_id == version_id;
        }
        manifest.active_version_id = version_id.to_owned();
        write_json_atomic(&manifest_path, &manifest)?;
        let mut public = target.public;
        public.active = true;
        Ok(public)
    }

    pub fn compare(
        &self,
        installation_id: &str,
        candidate_import_id: &str,
    ) -> Result<ImportComparison, GithubImportError> {
        let (_, manifest) = self.find_installation(installation_id)?;
        let active = manifest
            .versions
            .iter()
            .find(|version| version.public.version_id == manifest.active_version_id)
            .ok_or(GithubImportError::ManifestMismatch)?;
        let candidate: GithubImportPreview = read_json(
            &self
                .data_root
                .join("quarantine")
                .join("imports")
                .join(candidate_import_id)
                .join("preview.json"),
        )?;
        let candidate_files = candidate
            .files
            .iter()
            .filter(|file| file.extractable)
            .map(|file| (file.path.clone(), file.git_object_id.clone()))
            .collect::<BTreeMap<_, _>>();
        let added = candidate_files
            .keys()
            .filter(|path| !active.files.contains_key(*path))
            .cloned()
            .collect();
        let changed = candidate_files
            .iter()
            .filter(|(path, object)| active.files.get(*path).is_some_and(|old| old != *object))
            .map(|(path, _)| path.clone())
            .collect();
        let removed = active
            .files
            .keys()
            .filter(|path| !candidate_files.contains_key(*path))
            .cloned()
            .collect();
        Ok(ImportComparison {
            installation_id: installation_id.to_owned(),
            active_commit_sha: active.public.commit_sha.clone(),
            candidate_commit_sha: candidate.commit_sha,
            added,
            changed,
            removed,
        })
    }

    fn resolve_tree_ref(
        &self,
        repository_url: &str,
        tree_tail: &str,
    ) -> Result<(String, Option<String>), GithubImportError> {
        if let Some((commit, remainder)) = tree_tail.split_once('/')
            && validate_commit_sha(commit).is_ok()
        {
            safe_relative_path(remainder)?;
            return Ok((commit.to_owned(), Some(remainder.to_owned())));
        }
        if validate_commit_sha(tree_tail).is_ok() {
            return Ok((tree_tail.to_owned(), None));
        }
        let output = self.git_text(
            None,
            ["ls-remote", "--heads", "--tags", repository_url],
            MAX_REMOTE_OUTPUT,
        )?;
        let mut names = output
            .lines()
            .filter_map(|line| line.split_once('\t').map(|(_, name)| name))
            .filter_map(|name| {
                name.strip_prefix("refs/heads/")
                    .or_else(|| name.strip_prefix("refs/tags/"))
            })
            .map(|name| name.trim_end_matches("^{}").to_owned())
            .collect::<Vec<_>>();
        names.sort_by_key(|name| std::cmp::Reverse(name.len()));
        let reference = names
            .into_iter()
            .find(|name| tree_tail == name || tree_tail.starts_with(&format!("{name}/")))
            .ok_or_else(|| GithubImportError::RefNotFound(tree_tail.to_owned()))?;
        let subdirectory = tree_tail
            .strip_prefix(&reference)
            .and_then(|value| value.strip_prefix('/'))
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        if let Some(path) = &subdirectory {
            safe_relative_path(path)?;
        }
        Ok((reference, subdirectory))
    }

    fn analyze_repository(
        &self,
        import_id: &str,
        source: GithubImportSource,
        commit_sha: &str,
        quarantine: &Path,
        repository: &Path,
    ) -> Result<GithubImportPreview, GithubImportError> {
        let mut arguments = vec![
            OsStr::new("ls-tree"),
            OsStr::new("-r"),
            OsStr::new("-z"),
            OsStr::new("-l"),
            OsStr::new("--full-tree"),
            OsStr::new(commit_sha),
        ];
        if let Some(subdirectory) = &source.subdirectory {
            arguments.push(OsStr::new("--"));
            arguments.push(OsStr::new(subdirectory));
        }
        let tree = self.git_bytes(Some(repository), arguments, MAX_TREE_OUTPUT)?;
        let records = tree
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
            .collect::<Vec<_>>();
        if records.len() > MAX_FILES {
            return Err(GithubImportError::TooManyFiles(records.len()));
        }

        let mut files = Vec::new();
        let mut total_bytes = 0_u64;
        let mut inspection = BTreeMap::new();
        let mut blocked = false;
        for record in records {
            let record = String::from_utf8(record.to_vec())?;
            let (metadata, path) = record
                .split_once('\t')
                .ok_or_else(|| GithubImportError::MalformedTreeRecord(record.clone()))?;
            let fields = metadata.split_ascii_whitespace().collect::<Vec<_>>();
            if fields.len() != 4 {
                return Err(GithubImportError::MalformedTreeRecord(record));
            }
            let mode = fields[0].to_owned();
            let object_kind = fields[1];
            let object_id = fields[2].to_owned();
            let size_bytes = fields[3].parse::<u64>().unwrap_or(0);
            total_bytes = total_bytes
                .checked_add(size_bytes)
                .ok_or(GithubImportError::ImportTooLarge)?;
            if total_bytes > MAX_TOTAL_BYTES {
                return Err(GithubImportError::ImportTooLarge);
            }
            let mut risks = detect_path_risks(path);
            if mode == "120000" {
                risks.push(ImportRiskKind::Symlink);
            }
            if mode == "160000" || object_kind == "commit" {
                risks.push(ImportRiskKind::Submodule);
            }
            if mode == "100755" {
                risks.push(ImportRiskKind::Executable);
            }
            if is_script_path(path) {
                risks.push(ImportRiskKind::Script);
            }
            if is_binary_path(path) {
                risks.push(ImportRiskKind::Binary);
            }
            if size_bytes > MAX_INSPECT_BYTES {
                risks.push(ImportRiskKind::Binary);
            }
            risks.sort();
            risks.dedup();
            blocked |= risks.contains(&ImportRiskKind::PathTraversal);
            let extractable = object_kind == "blob"
                && size_bytes <= MAX_INSPECT_BYTES
                && !risks.iter().any(|risk| {
                    matches!(
                        risk,
                        ImportRiskKind::Symlink
                            | ImportRiskKind::Submodule
                            | ImportRiskKind::PathTraversal
                    )
                });
            let bytes = if object_kind == "blob" && size_bytes <= MAX_INSPECT_BYTES {
                Some(
                    self.git_bytes(
                        Some(repository),
                        [
                            OsStr::new("cat-file"),
                            OsStr::new("blob"),
                            OsStr::new(&object_id),
                        ],
                        usize::try_from(size_bytes)
                            .unwrap_or(usize::MAX)
                            .saturating_add(1),
                    )?,
                )
            } else {
                None
            };
            let sha256 = bytes.as_ref().map(|bytes| hex_sha256(bytes));
            if let Some(bytes) = bytes {
                if bytes.contains(&0)
                    && !risks.contains(&ImportRiskKind::Binary)
                    && !risks.contains(&ImportRiskKind::Symlink)
                {
                    risks.push(ImportRiskKind::Binary);
                }
                if is_install_hook(path, &bytes) {
                    risks.push(ImportRiskKind::InstallHook);
                }
                inspection.insert(path.to_owned(), bytes);
            }
            risks.sort();
            risks.dedup();
            files.push(ImportFileRecord {
                path: path.to_owned(),
                git_mode: mode,
                git_object_id: object_id,
                size_bytes,
                sha256,
                risks,
                extractable,
            });
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        let components = detect_components(&files, source.subdirectory.as_deref());
        let license = detect_license(&inspection);
        let mut warnings = Vec::new();
        for (risk, message) in [
            (
                ImportRiskKind::Script,
                "Scripts were detected and will never be auto-run.",
            ),
            (
                ImportRiskKind::InstallHook,
                "Package install hooks were detected; package managers are never auto-run.",
            ),
            (
                ImportRiskKind::Binary,
                "Binary content was detected and remains inert.",
            ),
            (
                ImportRiskKind::Symlink,
                "Symlinks were detected and are excluded from installation.",
            ),
            (
                ImportRiskKind::Submodule,
                "Submodules were detected and are not initialized.",
            ),
        ] {
            if files.iter().any(|file| file.risks.contains(&risk)) {
                warnings.push(message.to_owned());
            }
        }
        if license.status != LicenseStatus::Detected {
            warnings
                .push("No recognized SPDX license was detected; review licensing manually.".into());
        }
        if blocked {
            warnings.push("Unsafe path syntax blocks this import.".into());
        }
        let mut digest = Sha256::new();
        digest.update(commit_sha.as_bytes());
        for file in &files {
            digest.update(file.git_mode.as_bytes());
            digest.update([0]);
            digest.update(file.path.as_bytes());
            digest.update([0]);
            digest.update(file.git_object_id.as_bytes());
            digest.update([0]);
        }
        let permission_requests = vec![
            PermissionRequest {
                permission: PermissionKind::NetworkConnect,
                context: PermissionContext {
                    network_domain: Some("github.com".into()),
                    tool_id: Some("github.import.preview".into()),
                    ..PermissionContext::default()
                },
                risk: RiskLevel::Medium,
                action: "download pinned Git objects into quarantine".into(),
            },
            PermissionRequest {
                permission: PermissionKind::FilesystemWrite,
                context: PermissionContext {
                    target_path: Some(quarantine.to_string_lossy().into_owned()),
                    tool_id: Some("github.import.install".into()),
                    ..PermissionContext::default()
                },
                risk: RiskLevel::Medium,
                action: "install only explicitly selected inert files".into(),
            },
        ];
        Ok(GithubImportPreview {
            import_id: import_id.to_owned(),
            source,
            commit_sha: commit_sha.to_owned(),
            content_sha256: hex_digest(digest.finalize().as_slice()),
            quarantine_path: quarantine.to_string_lossy().into_owned(),
            scanned_at: jiff::Timestamp::now().to_string(),
            files,
            components,
            license,
            permission_requests,
            warnings,
            blocked,
        })
    }

    fn find_installation(
        &self,
        installation_id: &str,
    ) -> Result<(PathBuf, InstallationManifest), GithubImportError> {
        let root = self.data_root.join("imports").join("installed");
        if root.exists() {
            for entry in fs::read_dir(root)? {
                let path = entry?.path().join("manifest.json");
                if path.is_file() {
                    let manifest: InstallationManifest = read_json(&path)?;
                    if manifest.installation_id == installation_id {
                        return Ok((path, manifest));
                    }
                }
            }
        }
        Err(GithubImportError::InstallationNotFound(
            installation_id.to_owned(),
        ))
    }

    fn git<I, S>(
        &self,
        repository: Option<&Path>,
        args: I,
        max_output: usize,
    ) -> Result<Output, GithubImportError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = Command::new(&self.git_program);
        command
            .arg("-c")
            .arg("core.hooksPath=NUL")
            .arg("-c")
            .arg("protocol.file.allow=never");
        if let Some(repository) = repository {
            command.arg("-C").arg(repository);
        }
        command
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_NO_LAZY_FETCH", "1")
            .env("GCM_INTERACTIVE", "Never")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "NUL")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = command.output()?;
        if output.stdout.len() > max_output || output.stderr.len() > max_output {
            return Err(GithubImportError::GitOutputTooLarge);
        }
        if !output.status.success() {
            return Err(GithubImportError::GitFailed {
                exit_code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(output)
    }

    fn git_text<I, S>(
        &self,
        repository: Option<&Path>,
        args: I,
        max_output: usize,
    ) -> Result<String, GithubImportError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        String::from_utf8(self.git(repository, args, max_output)?.stdout)
            .map_err(GithubImportError::from)
    }

    fn git_bytes<I, S>(
        &self,
        repository: Option<&Path>,
        args: I,
        max_output: usize,
    ) -> Result<Vec<u8>, GithubImportError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Ok(self.git(repository, args, max_output)?.stdout)
    }
}

fn selected_files<'a>(
    preview: &'a GithubImportPreview,
    selected: &[&ImportComponent],
) -> Vec<&'a ImportFileRecord> {
    preview
        .files
        .iter()
        .filter(|file| {
            selected.iter().any(|component| {
                component.path_prefix.is_empty()
                    || file.path == component.path_prefix
                    || file
                        .path
                        .starts_with(&format!("{}/", component.path_prefix.trim_end_matches('/')))
            })
        })
        .collect()
}

fn detect_components(
    files: &[ImportFileRecord],
    subdirectory: Option<&str>,
) -> Vec<ImportComponent> {
    let mut components = BTreeMap::<String, ImportComponent>::new();
    for file in files {
        let lower = file.path.to_ascii_lowercase();
        let candidates = [
            (
                lower.ends_with("/skill.md") || lower == "skill.md",
                ImportComponentKind::Skill,
                parent_prefix(&file.path),
                "Skill",
                SkillCompatibility::Compatible,
            ),
            (
                lower.ends_with("/agents.md") || lower == "agents.md",
                ImportComponentKind::Agent,
                file.path.clone(),
                "Agent instructions",
                SkillCompatibility::Compatible,
            ),
            (
                lower.ends_with(".codex-plugin/plugin.json"),
                ImportComponentKind::Plugin,
                plugin_prefix(&file.path),
                "Codex plugin",
                SkillCompatibility::Compatible,
            ),
            (
                path_has_segment(&lower, "commands"),
                ImportComponentKind::Command,
                segment_prefix(&file.path, "commands"),
                "Commands",
                SkillCompatibility::BridgeRequired,
            ),
            (
                path_has_segment(&lower, "hooks"),
                ImportComponentKind::Hook,
                segment_prefix(&file.path, "hooks"),
                "Hooks",
                SkillCompatibility::BridgeRequired,
            ),
            (
                lower.ends_with("mcp.json") || lower.ends_with(".mcp.json"),
                ImportComponentKind::Mcp,
                file.path.clone(),
                "MCP configuration",
                SkillCompatibility::Compatible,
            ),
            (
                path_has_segment(&lower, "tools"),
                ImportComponentKind::Tool,
                segment_prefix(&file.path, "tools"),
                "Tools",
                SkillCompatibility::BridgeRequired,
            ),
            (
                path_has_segment(&lower, "assets"),
                ImportComponentKind::Asset,
                segment_prefix(&file.path, "assets"),
                "Assets",
                SkillCompatibility::Native,
            ),
            (
                file.risks.contains(&ImportRiskKind::Script),
                ImportComponentKind::Script,
                file.path.clone(),
                "Script",
                SkillCompatibility::BridgeRequired,
            ),
            (
                is_runtime_manifest(&lower),
                ImportComponentKind::RuntimeDependency,
                file.path.clone(),
                "Runtime dependency",
                SkillCompatibility::BridgeRequired,
            ),
        ];
        for (matches, kind, prefix, name, compatibility) in candidates {
            if !matches {
                continue;
            }
            let id = format!("{}:{}", kind_slug(kind), prefix);
            components
                .entry(id.clone())
                .or_insert_with(|| ImportComponent {
                    id,
                    kind,
                    display_name: format!("{name} · {prefix}"),
                    path_prefix: prefix,
                    compatibility,
                    requested_permissions: component_permissions(kind),
                    warnings: component_warnings(kind),
                });
        }
    }
    let root = subdirectory.unwrap_or("").trim_matches('/').to_owned();
    components
        .entry(format!(
            "assets:{}",
            if root.is_empty() { "repository" } else { &root }
        ))
        .or_insert_with(|| ImportComponent {
            id: format!(
                "assets:{}",
                if root.is_empty() { "repository" } else { &root }
            ),
            kind: ImportComponentKind::Asset,
            display_name: "Pinned repository content".into(),
            path_prefix: root,
            compatibility: SkillCompatibility::Native,
            requested_permissions: vec!["filesystem_write".into()],
            warnings: vec![
                "Content is installed as inert files; scripts and binaries are never run.".into(),
            ],
        });
    components.into_values().collect()
}

fn component_permissions(kind: ImportComponentKind) -> Vec<String> {
    let mut permissions = vec!["filesystem_write".into()];
    if matches!(
        kind,
        ImportComponentKind::Script
            | ImportComponentKind::Hook
            | ImportComponentKind::RuntimeDependency
            | ImportComponentKind::Tool
    ) {
        permissions.push("process_spawn_if_later_approved".into());
    }
    permissions
}

fn component_warnings(kind: ImportComponentKind) -> Vec<String> {
    match kind {
        ImportComponentKind::Script
        | ImportComponentKind::Hook
        | ImportComponentKind::RuntimeDependency => vec![
            "Installation stores this content only; execution requires a separate audited bridge and approval."
                .into(),
        ],
        _ => Vec::new(),
    }
}

fn detect_license(contents: &BTreeMap<String, Vec<u8>>) -> LicenseFinding {
    let candidate = contents.iter().find(|(path, _)| {
        Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                let name = name.to_ascii_lowercase();
                name.starts_with("license") || name == "copying"
            })
    });
    let Some((path, bytes)) = candidate else {
        return LicenseFinding {
            status: LicenseStatus::Missing,
            spdx_id: None,
            name: None,
            file_path: None,
            warning: Some("No license file was found in the pinned tree.".into()),
        };
    };
    let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    let detected = [
        (
            "MIT",
            "MIT License",
            "permission is hereby granted, free of charge",
        ),
        ("Apache-2.0", "Apache License 2.0", "apache license"),
        (
            "BSD-3-Clause",
            "BSD 3-Clause License",
            "redistribution and use in source and binary forms",
        ),
        ("GPL-3.0-only", "GNU GPL 3.0", "gnu general public license"),
        (
            "MPL-2.0",
            "Mozilla Public License 2.0",
            "mozilla public license",
        ),
    ]
    .into_iter()
    .find(|(_, _, marker)| text.contains(marker));
    match detected {
        Some((spdx, name, _)) => LicenseFinding {
            status: LicenseStatus::Detected,
            spdx_id: Some(spdx.into()),
            name: Some(name.into()),
            file_path: Some(path.clone()),
            warning: None,
        },
        None => LicenseFinding {
            status: LicenseStatus::Unknown,
            spdx_id: None,
            name: None,
            file_path: Some(path.clone()),
            warning: Some("A license file exists but its SPDX identity was not recognized.".into()),
        },
    }
}

fn detect_path_risks(path: &str) -> Vec<ImportRiskKind> {
    if safe_relative_path(path).is_err() {
        vec![ImportRiskKind::PathTraversal]
    } else {
        Vec::new()
    }
}

fn safe_relative_path(value: &str) -> Result<PathBuf, GithubImportError> {
    if value.is_empty()
        || value.contains('\\')
        || value.contains(':')
        || value.starts_with('/')
        || value.ends_with('/')
    {
        return Err(GithubImportError::UnsafePath(value.to_owned()));
    }
    let path = PathBuf::from(value);
    if path.components().any(|component| {
        !matches!(component, Component::Normal(_))
            || component
                .as_os_str()
                .to_str()
                .is_none_or(is_windows_reserved_name)
    }) {
        return Err(GithubImportError::UnsafePath(value.to_owned()));
    }
    Ok(path)
}

fn is_windows_reserved_name(value: &str) -> bool {
    let stem = value
        .trim_end_matches([' ', '.'])
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    value.ends_with([' ', '.'])
        || matches!(
            stem.as_str(),
            "con"
                | "prn"
                | "aux"
                | "nul"
                | "com1"
                | "com2"
                | "com3"
                | "com4"
                | "com5"
                | "com6"
                | "com7"
                | "com8"
                | "com9"
                | "lpt1"
                | "lpt2"
                | "lpt3"
                | "lpt4"
                | "lpt5"
                | "lpt6"
                | "lpt7"
                | "lpt8"
                | "lpt9"
        )
}

fn is_script_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [
        ".ps1", ".psm1", ".bat", ".cmd", ".sh", ".bash", ".zsh", ".fish", ".py", ".rb", ".pl",
        ".js", ".mjs", ".cjs", ".ts",
    ]
    .iter()
    .any(|extension| lower.ends_with(extension))
        || lower.ends_with("build.rs")
}

fn is_binary_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [
        ".exe", ".dll", ".com", ".msi", ".sys", ".so", ".dylib", ".wasm", ".jar", ".class", ".bin",
    ]
    .iter()
    .any(|extension| lower.ends_with(extension))
}

fn is_install_hook(path: &str, bytes: &[u8]) -> bool {
    if !path.to_ascii_lowercase().ends_with("package.json") {
        return false;
    }
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|value| value.get("scripts").cloned())
        .and_then(|scripts| scripts.as_object().cloned())
        .is_some_and(|scripts| {
            ["preinstall", "install", "postinstall", "prepare"]
                .iter()
                .any(|name| scripts.contains_key(*name))
        })
}

fn is_runtime_manifest(path: &str) -> bool {
    matches!(
        Path::new(path).file_name().and_then(|name| name.to_str()),
        Some(
            "package.json"
                | "package-lock.json"
                | "pnpm-lock.yaml"
                | "yarn.lock"
                | "bun.lock"
                | "pyproject.toml"
                | "requirements.txt"
                | "cargo.toml"
                | "cargo.lock"
        )
    )
}

fn path_has_segment(path: &str, segment: &str) -> bool {
    path.split('/').any(|value| value == segment)
}

fn segment_prefix(path: &str, segment: &str) -> String {
    let segments = path.split('/').collect::<Vec<_>>();
    let index = segments
        .iter()
        .position(|value| value.eq_ignore_ascii_case(segment))
        .unwrap_or(segments.len().saturating_sub(1));
    segments[..=index].join("/")
}

fn parent_prefix(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(parent, _)| parent.to_owned())
        .unwrap_or_default()
}

fn plugin_prefix(path: &str) -> String {
    path.to_ascii_lowercase()
        .find(".codex-plugin/plugin.json")
        .map(|index| path[..index].trim_end_matches('/').to_owned())
        .unwrap_or_else(|| parent_prefix(path))
}

fn kind_slug(kind: ImportComponentKind) -> &'static str {
    match kind {
        ImportComponentKind::Skill => "skill",
        ImportComponentKind::Agent => "agent",
        ImportComponentKind::Command => "command",
        ImportComponentKind::Hook => "hook",
        ImportComponentKind::Mcp => "mcp",
        ImportComponentKind::Tool => "tool",
        ImportComponentKind::Asset => "asset",
        ImportComponentKind::Script => "script",
        ImportComponentKind::RuntimeDependency => "runtime",
        ImportComponentKind::Plugin => "plugin",
    }
}

fn validate_slug(value: &str, field: &'static str) -> Result<String, GithubImportError> {
    if value.is_empty()
        || value.len() > 100
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(GithubImportError::InvalidRepositoryField(field));
    }
    Ok(value.to_owned())
}

fn validate_commit_sha(value: &str) -> Result<(), GithubImportError> {
    if value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(GithubImportError::InvalidCommitSha)
    }
}

fn resolve_git() -> Result<PathBuf, GithubImportError> {
    let system_root = std::env::var_os("SystemRoot").ok_or(GithubImportError::GitNotAvailable)?;
    let output = Command::new(
        PathBuf::from(system_root)
            .join("System32")
            .join("where.exe"),
    )
    .arg("git.exe")
    .stdin(Stdio::null())
    .output()?;
    if !output.status.success() {
        return Err(GithubImportError::GitNotAvailable);
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(PathBuf::from)
        .ok_or(GithubImportError::GitNotAvailable)
}

fn canonical_directory(path: &Path) -> Result<PathBuf, GithubImportError> {
    let path = path.canonicalize()?;
    if path.is_dir() {
        Ok(path)
    } else {
        Err(GithubImportError::WorkspaceNotDirectory(path))
    }
}

fn installation_key(source: &GithubImportSource, workspace: &Path) -> String {
    let mut digest = Sha256::new();
    digest.update(workspace.to_string_lossy().to_lowercase().as_bytes());
    let suffix = hex_digest(digest.finalize().as_slice());
    format!(
        "{}-{}-{}",
        source.owner.to_ascii_lowercase(),
        source.repository.to_ascii_lowercase(),
        &suffix[..12]
    )
}

fn replace_directory_from_snapshot(
    snapshot: &Path,
    destination: &Path,
) -> Result<(), GithubImportError> {
    if !snapshot.is_dir() {
        return Err(GithubImportError::SnapshotNotFound);
    }
    let parent = destination
        .parent()
        .ok_or_else(|| GithubImportError::UnsafePath(destination.display().to_string()))?;
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".lunascope-import-staging-{}", Uuid::new_v4()));
    copy_directory(snapshot, &staging)?;
    let backup = parent.join(format!(".lunascope-import-backup-{}", Uuid::new_v4()));
    if destination.exists() {
        fs::rename(destination, &backup)?;
    }
    if let Err(error) = fs::rename(&staging, destination) {
        if backup.exists() {
            let _ = fs::rename(&backup, destination);
        }
        return Err(GithubImportError::Io(error));
    }
    if backup.exists() {
        fs::remove_dir_all(backup)?;
    }
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), GithubImportError> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_directory(&entry.path(), &target)?;
        } else if entry.file_type()?.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            return Err(GithubImportError::SnapshotContainsSpecialFile);
        }
    }
    Ok(())
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), GithubImportError> {
    let parent = path.parent().ok_or(GithubImportError::ManifestMismatch)?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".manifest-{}.tmp", Uuid::new_v4()));
    fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, GithubImportError> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn hex_sha256(bytes: &[u8]) -> String {
    hex_digest(Sha256::digest(bytes).as_slice())
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

#[derive(Debug, Error)]
pub enum GithubImportError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Url(#[from] url::ParseError),
    #[error(transparent)]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(
        "only canonical public https://github.com/<owner>/<repo>[/tree/<ref>/<path>] URLs are supported"
    )]
    UnsupportedGithubUrl,
    #[error("invalid GitHub repository {0}")]
    InvalidRepositoryField(&'static str),
    #[error("Git is not available from the trusted Windows PATH")]
    GitNotAvailable,
    #[error("Git command failed with exit code {exit_code:?}: {stderr}")]
    GitFailed {
        exit_code: Option<i32>,
        stderr: String,
    },
    #[error("Git output exceeded the quarantine inspection limit")]
    GitOutputTooLarge,
    #[error("Git reference was not found: {0}")]
    RefNotFound(String),
    #[error("resolved commit is not a full 40-character SHA")]
    InvalidCommitSha,
    #[error("malformed Git tree record: {0}")]
    MalformedTreeRecord(String),
    #[error("import contains too many files: {0}")]
    TooManyFiles(usize),
    #[error("import exceeds the 512 MiB quarantine limit")]
    ImportTooLarge,
    #[error("unsafe Windows path in imported tree: {0}")]
    UnsafePath(String),
    #[error("workspace is not a directory: {0}")]
    WorkspaceNotDirectory(PathBuf),
    #[error("no components were selected")]
    NoComponentsSelected,
    #[error("selected component is not present in the pinned preview")]
    UnknownComponent,
    #[error("the import is blocked by a hard quarantine finding")]
    BlockedImport,
    #[error("quarantine or installation manifest does not match")]
    ManifestMismatch,
    #[error("blob size changed during pinned extraction: {0}")]
    BlobSizeMismatch(String),
    #[error("installation was not found: {0}")]
    InstallationNotFound(String),
    #[error("rollback version was not found: {0}")]
    VersionNotFound(String),
    #[error("rollback snapshot was not found")]
    SnapshotNotFound,
    #[error("rollback snapshot contains a special filesystem entry")]
    SnapshotContainsSpecialFile,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn parses_repository_and_rejects_lookalike_or_credential_urls() {
        let source =
            GithubImportManager::parse_source("https://github.com/openai/codex").expect("parse");
        assert_eq!(source.owner, "openai");
        assert_eq!(source.repository, "codex");
        assert_eq!(source.repository_url, "https://github.com/openai/codex.git");
        assert!(
            GithubImportManager::parse_source("https://github.com.evil.test/openai/codex").is_err()
        );
        assert!(
            GithubImportManager::parse_source("https://token@github.com/openai/codex").is_err()
        );
    }

    #[test]
    fn rejects_windows_traversal_and_reserved_names() {
        for path in [
            "../escape",
            "folder\\escape",
            "C:/escape",
            "aux.txt",
            "/root",
        ] {
            assert!(safe_relative_path(path).is_err(), "{path}");
        }
        assert!(safe_relative_path("skills/demo/SKILL.md").is_ok());
    }

    #[test]
    fn detects_install_hooks_without_executing_them() {
        let bytes = br#"{"scripts":{"postinstall":"powershell -File owned.ps1"}}"#;
        assert!(is_install_hook("package.json", bytes));
        assert!(is_script_path("owned.ps1"));
    }

    #[test]
    fn detects_component_categories_and_keeps_scripts_bridge_only() {
        let files = vec![
            fixture_file("skills/demo/SKILL.md", vec![]),
            fixture_file("commands/check.md", vec![]),
            fixture_file("hooks/postinstall.ps1", vec![ImportRiskKind::Script]),
            fixture_file(".codex-plugin/plugin.json", vec![]),
            fixture_file("package.json", vec![ImportRiskKind::InstallHook]),
        ];
        let components = detect_components(&files, None);
        assert!(
            components
                .iter()
                .any(|value| value.kind == ImportComponentKind::Skill)
        );
        assert!(
            components
                .iter()
                .any(|value| value.kind == ImportComponentKind::Plugin)
        );
        assert!(
            components
                .iter()
                .any(|value| value.kind == ImportComponentKind::Hook)
        );
        assert!(components.iter().any(|value| {
            value.kind == ImportComponentKind::Script
                && value.compatibility == SkillCompatibility::BridgeRequired
        }));
    }

    #[test]
    fn pinned_quarantine_install_update_and_rollback_never_execute_install_hook() {
        let data = tempfile::tempdir().expect("data root");
        let workspace = tempfile::tempdir().expect("workspace");
        let manager = GithubImportManager::new(data.path()).expect("manager");
        let import_one = "fixture-import-one";
        let quarantine_one = data
            .path()
            .join("quarantine")
            .join("imports")
            .join(import_one);
        let repository_one = quarantine_one.join("repository.git");
        fs::create_dir_all(repository_one.join("skills/demo")).expect("fixture directories");
        fs::write(
            repository_one.join("skills/demo/SKILL.md"),
            "---\nname: demo\n---\nPinned fixture\n",
        )
        .expect("skill");
        fs::write(
            repository_one.join("package.json"),
            r#"{"scripts":{"postinstall":"powershell -Command \"Set-Content exploited.txt owned\""},"version":"1.0.0"}"#,
        )
        .expect("package");
        fs::write(
            repository_one.join("LICENSE"),
            "MIT License\nPermission is hereby granted, free of charge",
        )
        .expect("license");
        fs::write(repository_one.join("version.txt"), "v1").expect("version one");
        git_fixture(&manager.git_program, &repository_one, ["init"]);
        git_fixture(
            &manager.git_program,
            &repository_one,
            ["config", "user.email", "fixture@example.test"],
        );
        git_fixture(
            &manager.git_program,
            &repository_one,
            ["config", "user.name", "LunaScope Fixture"],
        );
        git_fixture(&manager.git_program, &repository_one, ["add", "."]);
        git_fixture(
            &manager.git_program,
            &repository_one,
            ["commit", "-m", "fixture v1"],
        );
        let commit_one = git_fixture(&manager.git_program, &repository_one, ["rev-parse", "HEAD"]);
        let source = GithubImportSource {
            repository_url: "https://github.com/example/safe-fixture.git".into(),
            owner: "example".into(),
            repository: "safe-fixture".into(),
            requested_ref: Some("main".into()),
            subdirectory: None,
        };
        let preview_one = manager
            .analyze_repository(
                import_one,
                source.clone(),
                commit_one.trim(),
                &quarantine_one,
                &repository_one,
            )
            .expect("preview one");
        assert_eq!(preview_one.commit_sha, commit_one.trim());
        assert_eq!(preview_one.license.spdx_id.as_deref(), Some("MIT"));
        assert!(preview_one.files.iter().any(|file| {
            file.path == "package.json" && file.risks.contains(&ImportRiskKind::InstallHook)
        }));
        write_json_atomic(&quarantine_one.join("preview.json"), &preview_one)
            .expect("persist preview one");
        let selection = preview_one
            .components
            .iter()
            .find(|component| component.id == "assets:repository")
            .expect("repository assets")
            .id
            .clone();
        let first = manager
            .install(&ImportInstallRequest {
                import_id: import_one.into(),
                workspace_root: workspace.path().to_string_lossy().into_owned(),
                component_ids: vec![selection.clone()],
            })
            .expect("install one");
        let destination = PathBuf::from(&first.installed_path);
        assert_eq!(
            fs::read_to_string(destination.join("version.txt")).expect("installed version"),
            "v1"
        );
        assert!(!repository_one.join("exploited.txt").exists());
        assert!(!destination.join("exploited.txt").exists());

        fs::write(repository_one.join("version.txt"), "v2").expect("version two");
        git_fixture(
            &manager.git_program,
            &repository_one,
            ["add", "version.txt"],
        );
        git_fixture(
            &manager.git_program,
            &repository_one,
            ["commit", "-m", "fixture v2"],
        );
        let commit_two = git_fixture(&manager.git_program, &repository_one, ["rev-parse", "HEAD"]);
        let import_two = "fixture-import-two";
        let quarantine_two = data
            .path()
            .join("quarantine")
            .join("imports")
            .join(import_two);
        let repository_two = quarantine_two.join("repository.git");
        copy_directory(&repository_one, &repository_two).expect("copy pinned repository");
        let preview_two = manager
            .analyze_repository(
                import_two,
                source,
                commit_two.trim(),
                &quarantine_two,
                &repository_two,
            )
            .expect("preview two");
        write_json_atomic(&quarantine_two.join("preview.json"), &preview_two)
            .expect("persist preview two");
        let comparison = manager
            .compare(&first.installation_id, import_two)
            .expect("compare update");
        assert_eq!(comparison.active_commit_sha, commit_one.trim());
        assert_eq!(comparison.candidate_commit_sha, commit_two.trim());
        assert!(comparison.changed.contains(&"version.txt".into()));
        let second = manager
            .install(&ImportInstallRequest {
                import_id: import_two.into(),
                workspace_root: workspace.path().to_string_lossy().into_owned(),
                component_ids: vec![selection],
            })
            .expect("install update");
        assert_ne!(first.version_id, second.version_id);
        assert_eq!(
            fs::read_to_string(destination.join("version.txt")).expect("updated version"),
            "v2"
        );
        let restored = manager
            .rollback(&first.installation_id, &first.version_id)
            .expect("rollback");
        assert!(restored.active);
        assert_eq!(
            fs::read_to_string(destination.join("version.txt")).expect("restored version"),
            "v1"
        );
        assert!(!destination.join("exploited.txt").exists());
    }

    #[test]
    #[ignore = "downloads a small public GitHub repository into a temporary quarantine"]
    fn public_github_quarantine_canary() {
        let data = tempfile::tempdir().expect("temporary data root");
        let manager = GithubImportManager::new(data.path()).expect("manager");
        let preview = manager
            .preview("https://github.com/octocat/Hello-World")
            .expect("public GitHub quarantine preview");
        assert_eq!(preview.commit_sha.len(), 40);
        assert!(!preview.files.is_empty());
        assert!(!preview.blocked);
        assert!(
            Path::new(&preview.quarantine_path)
                .join("preview.json")
                .is_file()
        );
    }

    fn git_fixture<const N: usize>(
        git_program: &Path,
        repository: &Path,
        args: [&str; N],
    ) -> String {
        let output = Command::new(git_program)
            .args(["-C", &repository.to_string_lossy()])
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "NUL")
            .output()
            .expect("run fixture git");
        assert!(
            output.status.success(),
            "fixture git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("utf8 fixture git output")
    }

    fn fixture_file(path: &str, risks: Vec<ImportRiskKind>) -> ImportFileRecord {
        ImportFileRecord {
            path: path.into(),
            git_mode: "100644".into(),
            git_object_id: "0".repeat(40),
            size_bytes: 1,
            sha256: None,
            risks,
            extractable: true,
        }
    }
}
