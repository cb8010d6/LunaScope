use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use lunascope_core::{ArtifactId, ArtifactRecord, EventSource, OrchestrationId, WorkerId};
use process_wrap::tokio::{CommandWrap, JobObject, KillOnDrop};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const MAX_GIT_OUTPUT: usize = 16 * 1024 * 1024;
const MAX_WORKER_ARTIFACT: usize = 4 * 1024 * 1024;
const MAX_SNAPSHOT_FILES: usize = 200;
const MAX_SNAPSHOT_FILE_BYTES: u64 = 32 * 1024;
const MAX_SNAPSHOT_BYTES: usize = 512 * 1024;
const GIT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Debug)]
pub struct WorktreeManager {
    git: PathBuf,
    workspace_root: PathBuf,
    repository_root: PathBuf,
    orchestration_root: PathBuf,
    git_excludes: PathBuf,
    base_commit: String,
    shadow_repository: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerWorktree {
    pub worker_id: WorkerId,
    pub attempt: u32,
    pub path: PathBuf,
    pub base_commit: String,
}

#[derive(Clone, Debug)]
pub struct PatchArtifact {
    pub record: ArtifactRecord,
    pub worker_id: WorkerId,
    pub bytes: Vec<u8>,
    pub changed_paths: Vec<String>,
    pub base_commit: String,
}

impl WorktreeManager {
    pub fn write_through_workspace_root(&self) -> Option<PathBuf> {
        self.shadow_repository.then(|| self.workspace_root.clone())
    }

    pub async fn open(
        workspace_root: impl AsRef<Path>,
        data_root: impl AsRef<Path>,
        orchestration_id: &OrchestrationId,
        cancellation: CancellationToken,
    ) -> Result<Self, WorktreeError> {
        let workspace_root = workspace_root.as_ref().canonicalize()?;
        if !workspace_root.is_dir() {
            return Err(WorktreeError::WorkspaceNotDirectory(workspace_root));
        }
        let data_root = data_root.as_ref();
        if !data_root.is_absolute() {
            return Err(WorktreeError::DataRootNotAbsolute(data_root.to_path_buf()));
        }
        validate_component(orchestration_id.as_str())?;
        fs::create_dir_all(data_root)?;
        let data_root = data_root.canonicalize()?;
        let orchestration_root = data_root.join("worktrees").join(orchestration_id.as_str());
        fs::create_dir_all(orchestration_root.join("patches"))?;
        let orchestration_root = orchestration_root.canonicalize()?;
        if !orchestration_root.starts_with(&data_root) {
            return Err(WorktreeError::PathEscapedDataRoot(orchestration_root));
        }
        let git_excludes = orchestration_root.join("runtime-git-excludes");
        write_atomic(
            &git_excludes,
            b".lunascope/\n**/.lunascope/\n**/edgprofile/\n**/edge-profile/\n**/chrome-profile/\n**/browser-profile/\n**/user-data-dir/\n**/.playwright/\n**/playwright-report/\n**/test-results/\n",
        )?;
        let git = resolve_git()?;
        let provisional = Self {
            git,
            workspace_root: workspace_root.clone(),
            repository_root: workspace_root.clone(),
            orchestration_root,
            git_excludes,
            base_commit: String::new(),
            shadow_repository: false,
        };
        let top_level = provisional
            .git_text(
                &workspace_root,
                ["rev-parse", "--show-toplevel"],
                None,
                cancellation.clone(),
            )
            .await;
        let (repository_root, shadow_repository) = match top_level {
            Ok(top_level) => (PathBuf::from(top_level.trim()).canonicalize()?, false),
            Err(WorktreeError::GitFailed { .. }) => {
                let shadow = provisional.orchestration_root.join("base-repository");
                fs::create_dir_all(&shadow)?;
                copy_workspace_tree(&workspace_root, &shadow, &provisional.orchestration_root)?;
                provisional
                    .git_bytes(&shadow, ["init"], None, cancellation.clone(), false)
                    .await?;
                provisional
                    .git_bytes(
                        &shadow,
                        ["add", "-A", "--"],
                        None,
                        cancellation.clone(),
                        false,
                    )
                    .await?;
                provisional
                    .git_bytes(
                        &shadow,
                        [
                            "-c",
                            "user.name=LunaScope",
                            "-c",
                            "user.email=lunascope@local.invalid",
                            "commit",
                            "--allow-empty",
                            "--no-gpg-sign",
                            "--no-verify",
                            "-m",
                            "LunaScope shadow workspace baseline",
                        ],
                        None,
                        cancellation.clone(),
                        false,
                    )
                    .await?;
                (shadow.canonicalize()?, true)
            }
            Err(error) => return Err(error),
        };
        let base_commit = provisional
            .git_text(
                &repository_root,
                ["rev-parse", "HEAD^{commit}"],
                None,
                cancellation.clone(),
            )
            .await?
            .trim()
            .to_owned();
        if base_commit.len() != 40 || !base_commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(WorktreeError::InvalidCommit(base_commit));
        }
        Ok(Self {
            git: provisional.git,
            workspace_root,
            repository_root,
            orchestration_root: provisional.orchestration_root,
            git_excludes: provisional.git_excludes,
            base_commit,
            shadow_repository,
        })
    }

    pub fn repository_root(&self) -> &Path {
        &self.repository_root
    }

    pub fn base_commit(&self) -> &str {
        &self.base_commit
    }

    pub fn orchestration_root(&self) -> &Path {
        &self.orchestration_root
    }

    pub fn store_worker_artifact(
        &self,
        worker_id: &WorkerId,
        name: &str,
        media_type: &str,
        bytes: &[u8],
    ) -> Result<ArtifactRecord, WorktreeError> {
        validate_component(worker_id.as_str())?;
        if name.trim().is_empty() || media_type.trim().is_empty() {
            return Err(WorktreeError::InvalidArtifactMetadata);
        }
        if bytes.len() > MAX_WORKER_ARTIFACT {
            return Err(WorktreeError::ArtifactTooLarge(bytes.len()));
        }
        let artifact_id = ArtifactId::new(format!("artifact-{}", Uuid::new_v4()));
        let path = self
            .orchestration_root
            .join("artifacts")
            .join(format!("{}.bin", artifact_id.as_str()));
        write_atomic(&path, bytes)?;
        Ok(ArtifactRecord {
            artifact_id,
            name: name.trim().to_owned(),
            media_type: media_type.trim().to_owned(),
            path: path.to_string_lossy().into_owned(),
            sha256: hex_digest(&Sha256::digest(bytes)),
            created_by: EventSource::Worker(worker_id.clone()),
        })
    }

    pub async fn workspace_snapshot(
        &self,
        worktree: &WorkerWorktree,
        cancellation: CancellationToken,
    ) -> Result<String, WorktreeError> {
        let root = worktree.path.canonicalize()?;
        if !root.starts_with(&self.orchestration_root) {
            return Err(WorktreeError::PathEscapedDataRoot(root));
        }
        let listed = self
            .git_bytes(
                &root,
                ["ls-files", "-z", "--cached"],
                None,
                cancellation,
                false,
            )
            .await?;
        let mut output = String::new();
        let mut included = 0_usize;
        for raw in listed
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
        {
            if included >= MAX_SNAPSHOT_FILES || output.len() >= MAX_SNAPSHOT_BYTES {
                break;
            }
            let relative = std::str::from_utf8(raw)
                .map_err(WorktreeError::StatusPathUtf8)?
                .replace('\\', "/");
            validate_relative_path(&relative)?;
            let path = root.join(&relative);
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() > MAX_SNAPSHOT_FILE_BYTES
            {
                continue;
            }
            let canonical = path.canonicalize()?;
            if !canonical.starts_with(&root) {
                return Err(WorktreeError::UnsafeChangedPath(relative));
            }
            let bytes = fs::read(&canonical)?;
            let Ok(text) = std::str::from_utf8(&bytes) else {
                continue;
            };
            let remaining = MAX_SNAPSHOT_BYTES.saturating_sub(output.len());
            let header = format!("\n--- {relative} ---\n");
            if header.len() >= remaining {
                break;
            }
            output.push_str(&header);
            let remaining = MAX_SNAPSHOT_BYTES.saturating_sub(output.len());
            let mut keep = remaining.min(text.len());
            while keep > 0 && !text.is_char_boundary(keep) {
                keep -= 1;
            }
            output.push_str(&text[..keep]);
            included += 1;
        }
        if listed
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
            .count()
            > included
        {
            output.push_str("\n--- snapshot truncated by file/byte bounds ---\n");
        }
        Ok(output)
    }

    pub async fn prepare(
        &self,
        worker_id: &WorkerId,
        attempt: u32,
        dependency_patches: &[PatchArtifact],
        cancellation: CancellationToken,
    ) -> Result<WorkerWorktree, WorktreeError> {
        self.prepare_with_resume(worker_id, attempt, dependency_patches, &[], cancellation)
            .await
    }

    pub async fn prepare_with_resume(
        &self,
        worker_id: &WorkerId,
        attempt: u32,
        dependency_patches: &[PatchArtifact],
        resume_patches: &[PatchArtifact],
        cancellation: CancellationToken,
    ) -> Result<WorkerWorktree, WorktreeError> {
        validate_component(worker_id.as_str())?;
        if attempt == 0 {
            return Err(WorktreeError::InvalidAttempt);
        }
        let name = format!("{}-attempt-{attempt}", worker_id.as_str());
        let path = self.orchestration_root.join(name);
        if path.exists() {
            return Err(WorktreeError::WorktreeAlreadyExists(path));
        }
        self.git_bytes(
            &self.repository_root,
            [
                OsString::from("worktree"),
                OsString::from("add"),
                OsString::from("--detach"),
                external_path(&path).into_os_string(),
                OsString::from(&self.base_commit),
            ],
            None,
            cancellation.clone(),
            false,
        )
        .await?;
        let path = path.canonicalize()?;
        if !path.starts_with(&self.orchestration_root) {
            return Err(WorktreeError::PathEscapedDataRoot(path));
        }
        for patch in dependency_patches {
            self.apply_patch(&path, &patch.bytes, cancellation.clone())
                .await?;
        }
        if !dependency_patches.is_empty() {
            self.git_bytes(
                &path,
                ["add", "-A", "--"],
                None,
                cancellation.clone(),
                false,
            )
            .await?;
            self.git_bytes(
                &path,
                [
                    "-c",
                    "user.name=LunaScope",
                    "-c",
                    "user.email=lunascope@local.invalid",
                    "commit",
                    "--no-gpg-sign",
                    "--no-verify",
                    "-m",
                    "LunaScope dependency handoff baseline",
                ],
                None,
                cancellation.clone(),
                false,
            )
            .await?;
        }
        for patch in resume_patches {
            self.apply_patch(&path, &patch.bytes, cancellation.clone())
                .await?;
        }
        if !resume_patches.is_empty() {
            // Resume patches are the same Worker's unfinished side effects. Keep them visible
            // as ordinary worktree changes so the next successful capture emits one complete
            // patch (checkpoint + continuation) for integration and downstream handoff.
            self.git_bytes(
                &path,
                ["reset", "--mixed", "HEAD", "--", "."],
                None,
                cancellation.clone(),
                false,
            )
            .await?;
        }
        Ok(WorkerWorktree {
            worker_id: worker_id.clone(),
            attempt,
            path,
            base_commit: self.base_commit.clone(),
        })
    }

    pub async fn capture_patch(
        &self,
        worktree: &WorkerWorktree,
        write_scopes: &[String],
        cancellation: CancellationToken,
    ) -> Result<Option<PatchArtifact>, WorktreeError> {
        let path = worktree.path.canonicalize()?;
        if !path.starts_with(&self.orchestration_root) {
            return Err(WorktreeError::PathEscapedDataRoot(path));
        }
        let status = self
            .git_bytes(
                &path,
                [
                    "-c",
                    "status.renames=false",
                    "status",
                    "--porcelain=v1",
                    "-z",
                    "--untracked-files=all",
                ],
                None,
                cancellation.clone(),
                false,
            )
            .await?;
        let changed_paths = parse_status_paths(&status)?
            .into_iter()
            .filter(|path| path != ".lunascope" && !path.starts_with(".lunascope/"))
            .collect::<Vec<_>>();
        if changed_paths.is_empty() {
            return Ok(None);
        }
        for changed in &changed_paths {
            if !write_scopes
                .iter()
                .any(|scope| path_is_in_scope(changed, scope))
            {
                return Err(WorktreeError::WriteOutsideOwnership {
                    worker_id: worktree.worker_id.clone(),
                    path: changed.clone(),
                });
            }
        }
        self.git_bytes(
            &path,
            ["add", "-N", "--", "."],
            None,
            cancellation.clone(),
            false,
        )
        .await?;
        let bytes = self
            .git_bytes(
                &path,
                [
                    "diff",
                    "--binary",
                    "--full-index",
                    "--no-ext-diff",
                    "--no-renames",
                    "--",
                    ".",
                ],
                None,
                cancellation,
                false,
            )
            .await?;
        if bytes.is_empty() {
            return Ok(None);
        }
        let sha256 = hex_digest(&Sha256::digest(&bytes));
        let artifact_id = ArtifactId::new(format!("artifact-{}", Uuid::new_v4()));
        let patch_path = self
            .orchestration_root
            .join("patches")
            .join(format!("{}.patch", artifact_id.as_str()));
        write_atomic(&patch_path, &bytes)?;
        Ok(Some(PatchArtifact {
            record: ArtifactRecord {
                artifact_id,
                name: format!("{} patch", worktree.worker_id),
                media_type: "text/x-diff".into(),
                path: patch_path.to_string_lossy().into_owned(),
                sha256,
                created_by: EventSource::Worker(worktree.worker_id.clone()),
            },
            worker_id: worktree.worker_id.clone(),
            bytes,
            changed_paths,
            base_commit: worktree.base_commit.clone(),
        }))
    }

    pub async fn integrate_patch(
        &self,
        patch: &PatchArtifact,
        cancellation: CancellationToken,
    ) -> Result<(), WorktreeError> {
        if patch.base_commit != self.base_commit {
            return Err(WorktreeError::InvalidCommit(patch.base_commit.clone()));
        }
        if self.shadow_repository {
            self.apply_patch(&self.repository_root, &patch.bytes, cancellation.clone())
                .await?;
            for changed in &patch.changed_paths {
                validate_relative_path(changed)?;
                let source = self.repository_root.join(changed);
                let target = self.workspace_root.join(changed);
                if source.is_file() {
                    let parent = target
                        .parent()
                        .ok_or_else(|| WorktreeError::UnsafeChangedPath(changed.clone()))?;
                    fs::create_dir_all(parent)?;
                    let parent = parent.canonicalize()?;
                    if !parent.starts_with(&self.workspace_root) {
                        return Err(WorktreeError::UnsafeChangedPath(changed.clone()));
                    }
                    fs::copy(source, target)?;
                } else if target.is_file() {
                    fs::remove_file(target)?;
                }
            }
        } else {
            self.git_bytes(
                &self.repository_root,
                ["apply", "--check", "-"],
                Some(&patch.bytes),
                cancellation.clone(),
                false,
            )
            .await?;
            self.git_bytes(
                &self.repository_root,
                ["apply", "-"],
                Some(&patch.bytes),
                cancellation,
                false,
            )
            .await?;
        }
        Ok(())
    }

    async fn apply_patch(
        &self,
        worktree: &Path,
        patch: &[u8],
        cancellation: CancellationToken,
    ) -> Result<(), WorktreeError> {
        self.git_bytes(
            worktree,
            ["apply", "--check", "--index", "-"],
            Some(patch),
            cancellation.clone(),
            false,
        )
        .await?;
        self.git_bytes(
            worktree,
            ["apply", "--index", "-"],
            Some(patch),
            cancellation,
            false,
        )
        .await?;
        Ok(())
    }

    async fn git_text<I, S>(
        &self,
        cwd: &Path,
        args: I,
        stdin: Option<&[u8]>,
        cancellation: CancellationToken,
    ) -> Result<String, WorktreeError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let bytes = self
            .git_bytes(cwd, args, stdin, cancellation, false)
            .await?;
        String::from_utf8(bytes).map_err(WorktreeError::Utf8)
    }

    async fn git_bytes<I, S>(
        &self,
        cwd: &Path,
        args: I,
        stdin: Option<&[u8]>,
        cancellation: CancellationToken,
        allow_exit_one: bool,
    ) -> Result<Vec<u8>, WorktreeError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let cwd = cwd.canonicalize()?;
        if cwd != self.repository_root && !cwd.starts_with(&self.orchestration_root) {
            return Err(WorktreeError::WorkingDirectoryNotAllowed(cwd));
        }
        let mut full_args = vec![
            OsString::from("-c"),
            OsString::from("core.hooksPath=NUL"),
            OsString::from("-c"),
            OsString::from("protocol.file.allow=never"),
            OsString::from("-c"),
            OsString::from(format!(
                "core.excludesFile={}",
                external_path(&self.git_excludes).display()
            )),
        ];
        full_args.extend(
            args.into_iter()
                .map(|argument| argument.as_ref().to_os_string()),
        );
        let environment = git_environment();
        let has_stdin = stdin.is_some();
        let mut command = CommandWrap::with_new(&self.git, |command| {
            command
                .args(&full_args)
                .current_dir(&cwd)
                .env_clear()
                .envs(environment)
                .stdin(if has_stdin {
                    Stdio::piped()
                } else {
                    Stdio::null()
                })
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
        });
        command.wrap(KillOnDrop);
        command.wrap(JobObject);
        let mut child = command.spawn()?;
        if let Some(input) = stdin {
            let mut child_stdin = child.stdin().take().ok_or(WorktreeError::MissingPipe)?;
            child_stdin.write_all(input).await?;
            child_stdin.shutdown().await?;
        }
        let stdout = child.stdout().take().ok_or(WorktreeError::MissingPipe)?;
        let stderr = child.stderr().take().ok_or(WorktreeError::MissingPipe)?;
        let stdout_task = tokio::spawn(read_limited(stdout));
        let stderr_task = tokio::spawn(read_limited(stderr));
        let deadline = tokio::time::sleep(GIT_TIMEOUT);
        tokio::pin!(deadline);
        let (status, cancelled, timed_out) = loop {
            if let Some(status) = child.try_wait()? {
                break (status, false, false);
            }
            tokio::select! {
                _ = cancellation.cancelled() => {
                    child.start_kill()?;
                    break (child.wait().await?, true, false);
                }
                _ = &mut deadline => {
                    child.start_kill()?;
                    break (child.wait().await?, false, true);
                }
                _ = tokio::time::sleep(Duration::from_millis(20)) => {}
            }
        };
        let (stdout, stdout_truncated) = stdout_task.await??;
        let (stderr, stderr_truncated) = stderr_task.await??;
        if cancelled {
            return Err(WorktreeError::Cancelled);
        }
        if timed_out {
            return Err(WorktreeError::TimedOut);
        }
        if stdout_truncated || stderr_truncated {
            return Err(WorktreeError::OutputTooLarge);
        }
        if !(status.success() || allow_exit_one && status.code() == Some(1)) {
            return Err(WorktreeError::GitFailed {
                exit_code: status.code(),
                stderr: String::from_utf8_lossy(&stderr).trim().to_owned(),
            });
        }
        Ok(stdout)
    }
}

fn copy_workspace_tree(
    source: &Path,
    destination: &Path,
    excluded_root: &Path,
) -> Result<(), WorktreeError> {
    const MAX_SHADOW_FILE_BYTES: u64 = 64 * 1024 * 1024;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        if source_path.starts_with(excluded_root) {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        let destination_path = destination.join(&name);
        if file_type.is_dir() {
            let excluded = matches!(
                name.to_string_lossy().to_ascii_lowercase().as_str(),
                ".git" | "target" | "node_modules" | ".venv" | "__pycache__"
            );
            if excluded {
                continue;
            }
            fs::create_dir_all(&destination_path)?;
            copy_workspace_tree(&source_path, &destination_path, excluded_root)?;
        } else if file_type.is_file() && entry.metadata()?.len() <= MAX_SHADOW_FILE_BYTES {
            fs::copy(source_path, destination_path)?;
        }
    }
    Ok(())
}

fn validate_component(value: &str) -> Result<(), WorktreeError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || value == "."
        || value == ".."
    {
        return Err(WorktreeError::UnsafeIdentifier(value.into()));
    }
    Ok(())
}

fn parse_status_paths(bytes: &[u8]) -> Result<Vec<String>, WorktreeError> {
    let mut paths = Vec::new();
    for entry in bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        if entry.len() < 4 || entry[2] != b' ' {
            return Err(WorktreeError::InvalidStatusRecord);
        }
        let path = std::str::from_utf8(&entry[3..])
            .map_err(WorktreeError::StatusPathUtf8)?
            .replace('\\', "/");
        validate_relative_path(&path)?;
        paths.push(path);
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn validate_relative_path(path: &str) -> Result<(), WorktreeError> {
    let candidate = Path::new(path);
    if candidate.is_absolute()
        || candidate.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(WorktreeError::UnsafeChangedPath(path.into()));
    }
    Ok(())
}

fn path_is_in_scope(path: &str, scope: &str) -> bool {
    let normalize = |value: &str| value.trim().trim_matches('/').replace('\\', "/");
    let path = normalize(path);
    let scope = normalize(scope);
    scope == "." || (!scope.is_empty() && (path == scope || path.starts_with(&(scope + "/"))))
}

fn resolve_git() -> Result<PathBuf, WorktreeError> {
    let system_root =
        PathBuf::from(std::env::var_os("SystemRoot").ok_or(WorktreeError::MissingSystemRoot)?);
    let output = Command::new(system_root.join("System32").join("where.exe"))
        .arg("$PATH:git.exe")
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(WorktreeError::GitNotFound);
    }
    let candidate = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(PathBuf::from)
        .ok_or(WorktreeError::GitNotFound)?
        .canonicalize()?;
    if !candidate.is_file() {
        return Err(WorktreeError::GitNotFound);
    }
    Ok(candidate)
}

fn external_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let rendered = path.to_string_lossy();
        if let Some(stripped) = rendered.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{stripped}"));
        }
        if let Some(stripped) = rendered.strip_prefix(r"\\?\") {
            return PathBuf::from(stripped);
        }
    }
    path.to_path_buf()
}

fn git_environment() -> BTreeMap<OsString, OsString> {
    const KEYS: &[&str] = &[
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "PATH",
        "PATHEXT",
        "COMSPEC",
    ];
    let mut environment = KEYS
        .iter()
        .filter_map(|key| std::env::var_os(key).map(|value| (OsString::from(key), value)))
        .collect::<BTreeMap<_, _>>();
    environment.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
    environment.insert("GIT_CONFIG_GLOBAL".into(), "NUL".into());
    environment.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
    environment.insert("GCM_INTERACTIVE".into(), "Never".into());
    environment
}

async fn read_limited(
    mut reader: impl tokio::io::AsyncRead + Unpin,
) -> Result<(Vec<u8>, bool), std::io::Error> {
    let mut retained = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let remaining = MAX_GIT_OUTPUT.saturating_sub(retained.len());
        let keep = remaining.min(read);
        retained.extend_from_slice(&buffer[..keep]);
        truncated |= keep != read;
    }
    Ok((retained, truncated))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), WorktreeError> {
    let parent = path
        .parent()
        .ok_or_else(|| WorktreeError::UnsafeChangedPath(path.display().to_string()))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".{}.tmp", Uuid::new_v4()));
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[derive(Debug, Error)]
pub enum WorktreeError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("workspace is not a directory: {0}")]
    WorkspaceNotDirectory(PathBuf),
    #[error("data root must be absolute: {0}")]
    DataRootNotAbsolute(PathBuf),
    #[error("path escaped the assigned data root: {0}")]
    PathEscapedDataRoot(PathBuf),
    #[error("Git working directory is outside the repository or orchestration root: {0}")]
    WorkingDirectoryNotAllowed(PathBuf),
    #[error("Git was not found")]
    GitNotFound,
    #[error("SystemRoot is unavailable")]
    MissingSystemRoot,
    #[error("unsafe orchestration or worker identifier: {0}")]
    UnsafeIdentifier(String),
    #[error("invalid base commit: {0}")]
    InvalidCommit(String),
    #[error("worker attempt must be positive")]
    InvalidAttempt,
    #[error("worker worktree already exists: {0}")]
    WorktreeAlreadyExists(PathBuf),
    #[error("Git process pipe was unavailable")]
    MissingPipe,
    #[error("Git command was cancelled")]
    Cancelled,
    #[error("Git command timed out")]
    TimedOut,
    #[error("Git output exceeded 16 MiB")]
    OutputTooLarge,
    #[error("worker artifact metadata is invalid")]
    InvalidArtifactMetadata,
    #[error("worker artifact exceeds 4 MiB: {0} bytes")]
    ArtifactTooLarge(usize),
    #[error("Git command failed with code {exit_code:?}: {stderr}")]
    GitFailed {
        exit_code: Option<i32>,
        stderr: String,
    },
    #[error("Git output reader failed: {0}")]
    ReaderTask(#[from] tokio::task::JoinError),
    #[error("invalid porcelain status record")]
    InvalidStatusRecord,
    #[error("Git status path is not UTF-8: {0}")]
    StatusPathUtf8(#[source] std::str::Utf8Error),
    #[error("unsafe changed path: {0}")]
    UnsafeChangedPath(String),
    #[error("worker {worker_id} changed path outside declared ownership: {path}")]
    WriteOutsideOwnership { worker_id: WorkerId, path: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_git(cwd: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .expect("git fixture");
        assert!(status.success(), "git fixture failed: {args:?}");
    }

    fn initialize_repository(root: &Path) {
        fs::create_dir_all(root.join("src")).expect("src");
        fs::write(root.join("README.md"), "base\n").expect("readme");
        run_git(root, &["init"]);
        run_git(root, &["config", "user.name", "LunaScope Test"]);
        run_git(root, &["config", "user.email", "lunascope@example.invalid"]);
        run_git(root, &["add", "."]);
        run_git(root, &["commit", "-m", "base"]);
    }

    #[tokio::test]
    async fn isolated_workers_handoff_disjoint_patches_to_verifier() {
        let temporary = tempfile::tempdir().expect("temp");
        let repository = temporary.path().join("repository");
        let data = temporary.path().join("data");
        fs::create_dir_all(&repository).expect("repo");
        initialize_repository(&repository);
        let orchestration_id = OrchestrationId::new(format!("orc-{}", Uuid::new_v4()));
        let cancellation = CancellationToken::new();
        let manager =
            WorktreeManager::open(&repository, &data, &orchestration_id, cancellation.clone())
                .await
                .expect("manager");
        let worker_a = WorkerId::from("worker-a");
        let worker_b = WorkerId::from("worker-b");
        let a = manager
            .prepare(&worker_a, 1, &[], cancellation.clone())
            .await
            .expect("a worktree");
        let b = manager
            .prepare(&worker_b, 1, &[], cancellation.clone())
            .await
            .expect("b worktree");
        assert_ne!(a.path, b.path);
        fs::create_dir_all(a.path.join("src")).expect("a src");
        fs::create_dir_all(b.path.join("src")).expect("b src");
        fs::write(a.path.join("src").join("a.txt"), "from a\n").expect("a edit");
        fs::write(b.path.join("src").join("b.txt"), "from b\n").expect("b edit");
        let patch_a = manager
            .capture_patch(&a, &["src/a.txt".into()], cancellation.clone())
            .await
            .expect("capture a")
            .expect("a patch");
        let patch_b = manager
            .capture_patch(&b, &["src/b.txt".into()], cancellation.clone())
            .await
            .expect("capture b")
            .expect("b patch");
        assert!(!repository.join("src").join("a.txt").exists());
        assert!(!repository.join("src").join("b.txt").exists());

        let verifier = manager
            .prepare(
                &WorkerId::from("verifier"),
                1,
                &[patch_a, patch_b],
                cancellation,
            )
            .await
            .expect("verifier worktree");
        assert_eq!(
            fs::read_to_string(verifier.path.join("src").join("a.txt")).expect("a result"),
            "from a\n"
        );
        assert_eq!(
            fs::read_to_string(verifier.path.join("src").join("b.txt")).expect("b result"),
            "from b\n"
        );
    }

    #[tokio::test]
    async fn worker_cannot_capture_a_change_outside_declared_scope() {
        let temporary = tempfile::tempdir().expect("temp");
        let repository = temporary.path().join("repository");
        let data = temporary.path().join("data");
        fs::create_dir_all(&repository).expect("repo");
        initialize_repository(&repository);
        let manager = WorktreeManager::open(
            &repository,
            &data,
            &OrchestrationId::new(format!("orc-{}", Uuid::new_v4())),
            CancellationToken::new(),
        )
        .await
        .expect("manager");
        let worktree = manager
            .prepare(
                &WorkerId::from("worker-owned"),
                1,
                &[],
                CancellationToken::new(),
            )
            .await
            .expect("worktree");
        fs::write(worktree.path.join("README.md"), "unauthorized\n").expect("edit");
        let result = manager
            .capture_patch(&worktree, &["src".into()], CancellationToken::new())
            .await;
        assert!(matches!(
            result,
            Err(WorktreeError::WriteOutsideOwnership { .. })
        ));
    }

    #[tokio::test]
    async fn internal_dependency_cache_is_not_captured_as_a_deliverable() {
        let temporary = tempfile::tempdir().expect("temp");
        let repository = temporary.path().join("repository");
        let data = temporary.path().join("data");
        fs::create_dir_all(&repository).expect("repo");
        initialize_repository(&repository);
        let manager = WorktreeManager::open(
            &repository,
            &data,
            &OrchestrationId::new(format!("orc-{}", Uuid::new_v4())),
            CancellationToken::new(),
        )
        .await
        .expect("manager");
        let worktree = manager
            .prepare(
                &WorkerId::from("worker-cache"),
                1,
                &[],
                CancellationToken::new(),
            )
            .await
            .expect("worktree");
        fs::create_dir_all(worktree.path.join(".lunascope/dependencies/npm/three"))
            .expect("cache directory");
        fs::write(
            worktree
                .path
                .join(".lunascope/dependencies/npm/three/receipt.json"),
            "{}",
        )
        .expect("cache receipt");
        let browser_cache = worktree
            .path
            .join("tests/out/edgprofile/Default/Service Worker/ScriptCache/index-dir");
        fs::create_dir_all(&browser_cache).expect("browser cache directory");
        fs::write(browser_cache.join("cache.bin"), b"volatile browser profile")
            .expect("browser cache file");
        fs::write(worktree.path.join("index.html"), "<!doctype html>").expect("deliverable");

        let patch = manager
            .capture_patch(&worktree, &[".".into()], CancellationToken::new())
            .await
            .expect("capture")
            .expect("deliverable patch");

        assert_eq!(patch.changed_paths, vec!["index.html"]);
        assert!(!String::from_utf8_lossy(&patch.bytes).contains(".lunascope"));
        assert!(!String::from_utf8_lossy(&patch.bytes).contains("edgprofile"));
    }

    #[tokio::test]
    async fn non_git_workspace_uses_shadow_repository_and_integrates_files() {
        let temporary = tempfile::tempdir().expect("temp");
        let workspace = temporary.path().join("plain-workspace");
        let data = temporary.path().join("data");
        fs::create_dir_all(&workspace).expect("workspace");
        fs::write(workspace.join("README.md"), "plain folder\n").expect("seed");
        let cancellation = CancellationToken::new();
        let manager = WorktreeManager::open(
            &workspace,
            &data,
            &OrchestrationId::new(format!("orc-{}", Uuid::new_v4())),
            cancellation.clone(),
        )
        .await
        .expect("shadow manager");
        assert!(manager.shadow_repository);
        assert!(!workspace.join(".git").exists());
        let worktree = manager
            .prepare(
                &WorkerId::from("worker-shadow"),
                1,
                &[],
                cancellation.clone(),
            )
            .await
            .expect("worker worktree");
        fs::write(worktree.path.join("created.txt"), "created by agent\n").expect("worker edit");
        let patch = manager
            .capture_patch(&worktree, &[".".into()], cancellation.clone())
            .await
            .expect("capture")
            .expect("patch");
        manager
            .integrate_patch(&patch, cancellation)
            .await
            .expect("integrate");
        assert_eq!(
            fs::read_to_string(workspace.join("created.txt")).expect("created file"),
            "created by agent\n"
        );
        assert!(!workspace.join(".git").exists());
    }
}
