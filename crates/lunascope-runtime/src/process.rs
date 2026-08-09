use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use process_wrap::tokio::{CommandWrap, JobObject, KillOnDrop};
use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

const MAX_TIMEOUT_MS: u64 = 10 * 60 * 1000;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub cancelled: bool,
    pub timed_out: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Clone, Debug)]
pub struct ProcessExecutor {
    workspace_root: PathBuf,
    allowed_programs: BTreeMap<String, PathBuf>,
}

impl ProcessExecutor {
    pub fn new(
        workspace_root: impl AsRef<Path>,
        allowed_programs: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, ProcessError> {
        let workspace_root = workspace_root.as_ref().canonicalize()?;
        if !workspace_root.is_dir() {
            return Err(ProcessError::WorkingDirectoryNotAllowed(workspace_root));
        }
        let mut resolved_programs = BTreeMap::new();
        for program in allowed_programs {
            let (name, path) = resolve_trusted_program(&program.into())?;
            if let Some(existing) = resolved_programs.insert(name.clone(), path.clone()) {
                if existing != path {
                    return Err(ProcessError::ConflictingProgramPaths {
                        name,
                        first: existing,
                        second: path,
                    });
                }
            }
        }
        Ok(Self {
            workspace_root,
            allowed_programs: resolved_programs,
        })
    }

    pub async fn execute(
        &self,
        spec: &ProcessSpec,
        cancellation: CancellationToken,
    ) -> Result<ProcessOutput, ProcessError> {
        if spec.timeout_ms == 0 || spec.timeout_ms > MAX_TIMEOUT_MS {
            return Err(ProcessError::InvalidTimeout(spec.timeout_ms));
        }
        let requested_program = Path::new(&spec.program);
        let program_name = requested_program
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ProcessError::ProgramNotAllowed(spec.program.clone()))?
            .to_lowercase();
        let trusted_program = self
            .allowed_programs
            .get(&program_name)
            .ok_or_else(|| ProcessError::ProgramNotAllowed(spec.program.clone()))?;
        if requested_program.is_absolute() {
            let canonical = requested_program.canonicalize()?;
            if &canonical != trusted_program {
                return Err(ProcessError::ProgramNotAllowed(spec.program.clone()));
            }
        } else if requested_program.components().count() != 1 {
            return Err(ProcessError::ProgramNotAllowed(spec.program.clone()));
        }
        let cwd = self.resolve_cwd(Path::new(&spec.cwd))?;
        validate_env(&spec.env)?;

        let inherited = inherited_environment();
        let mut command = CommandWrap::with_new(trusted_program, |command| {
            command
                .args(&spec.args)
                .current_dir(&cwd)
                .env_clear()
                .envs(inherited)
                .envs(&spec.env)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
        });
        command.wrap(KillOnDrop);
        command.wrap(JobObject);
        let mut child = command.spawn()?;
        let stdout = child.stdout().take().ok_or(ProcessError::MissingPipe)?;
        let stderr = child.stderr().take().ok_or(ProcessError::MissingPipe)?;
        let stdout_task = tokio::spawn(read_limited(stdout));
        let stderr_task = tokio::spawn(read_limited(stderr));
        let started = Instant::now();
        let deadline = tokio::time::sleep(Duration::from_millis(spec.timeout_ms));
        tokio::pin!(deadline);

        let (status, cancelled, timed_out) = loop {
            if let Some(status) = child.try_wait()? {
                break (Some(status), false, false);
            }
            tokio::select! {
                _ = cancellation.cancelled() => {
                    child.start_kill()?;
                    let status = child.wait().await?;
                    break (Some(status), true, false);
                }
                _ = &mut deadline => {
                    child.start_kill()?;
                    let status = child.wait().await?;
                    break (Some(status), false, true);
                }
                _ = tokio::time::sleep(Duration::from_millis(20)) => {}
            }
        };

        let (stdout, stdout_truncated) = stdout_task.await??;
        let (stderr, stderr_truncated) = stderr_task.await??;
        Ok(ProcessOutput {
            exit_code: status.and_then(|status| status.code()),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
            duration_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
            cancelled,
            timed_out,
            stdout_truncated,
            stderr_truncated,
        })
    }

    fn resolve_cwd(&self, cwd: &Path) -> Result<PathBuf, ProcessError> {
        let candidate = if cwd.is_absolute() {
            cwd.to_path_buf()
        } else {
            self.workspace_root.join(cwd)
        };
        let canonical = candidate.canonicalize()?;
        if !canonical.is_dir() || !canonical.starts_with(&self.workspace_root) {
            return Err(ProcessError::WorkingDirectoryNotAllowed(canonical));
        }
        Ok(canonical)
    }
}

fn resolve_trusted_program(program: &str) -> Result<(String, PathBuf), ProcessError> {
    let requested = Path::new(program);
    let name = requested
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ProcessError::ProgramNotAllowed(program.to_owned()))?
        .to_lowercase();
    let resolved = if requested.is_absolute() {
        requested.canonicalize()?
    } else {
        if requested.components().count() != 1 {
            return Err(ProcessError::ProgramNotAllowed(program.to_owned()));
        }
        let system_root = std::env::var_os("SystemRoot").ok_or(ProcessError::MissingSystemRoot)?;
        let where_exe = PathBuf::from(system_root)
            .join("System32")
            .join("where.exe");
        let mut command = Command::new(where_exe);
        crate::hide_console_window(&mut command);
        let output = command
            .arg(format!("$PATH:{program}"))
            .stdin(Stdio::null())
            .output()?;
        if !output.status.success() {
            return Err(ProcessError::ProgramResolutionFailed(program.to_owned()));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let first = stdout
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .ok_or_else(|| ProcessError::ProgramResolutionFailed(program.to_owned()))?;
        PathBuf::from(first.trim_matches('"')).canonicalize()?
    };
    if !resolved.is_file() {
        return Err(ProcessError::ProgramResolutionFailed(program.to_owned()));
    }
    Ok((name, resolved))
}

async fn read_limited(
    mut reader: impl tokio::io::AsyncRead + Unpin,
) -> Result<(Vec<u8>, bool), std::io::Error> {
    let mut retained = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let remaining = MAX_OUTPUT_BYTES.saturating_sub(retained.len());
        let keep = read.min(remaining);
        retained.extend_from_slice(&buffer[..keep]);
        truncated |= keep < read;
    }
    Ok((retained, truncated))
}

fn inherited_environment() -> BTreeMap<OsString, OsString> {
    const KEYS: &[&str] = &[
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "PATH",
        "PATHEXT",
        "COMSPEC",
    ];
    KEYS.iter()
        .filter_map(|key| std::env::var_os(key).map(|value| (OsString::from(key), value)))
        .collect()
}

fn validate_env(env: &BTreeMap<String, String>) -> Result<(), ProcessError> {
    const DENIED: &[&str] = &[
        "HOME",
        "USERPROFILE",
        "CODEX_HOME",
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
    ];
    if let Some(key) = env
        .keys()
        .find(|key| DENIED.iter().any(|denied| key.eq_ignore_ascii_case(denied)))
    {
        return Err(ProcessError::EnvironmentKeyDenied(key.clone()));
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("program is not in the task allowlist: {0}")]
    ProgramNotAllowed(String),
    #[error("SystemRoot is required to resolve trusted Windows executables")]
    MissingSystemRoot,
    #[error("program could not be resolved from the trusted PATH: {0}")]
    ProgramResolutionFailed(String),
    #[error("allowlist contains conflicting paths for {name}: {first} and {second}")]
    ConflictingProgramPaths {
        name: String,
        first: PathBuf,
        second: PathBuf,
    },
    #[error("working directory is outside the workspace: {0}")]
    WorkingDirectoryNotAllowed(PathBuf),
    #[error("timeout must be between 1 and {MAX_TIMEOUT_MS} ms, got {0}")]
    InvalidTimeout(u64),
    #[error("environment key is denied: {0}")]
    EnvironmentKeyDenied(String),
    #[error("process pipe was not available")]
    MissingPipe,
    #[error("output reader task failed: {0}")]
    ReaderTask(#[from] tokio::task::JoinError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn captures_literal_process_output() {
        let directory = tempfile::tempdir().expect("temp directory");
        std::fs::write(
            directory.path().join("powershell.exe"),
            "not a real executable",
        )
        .expect("write same-name workspace trap");
        let executor =
            ProcessExecutor::new(directory.path(), ["powershell.exe"]).expect("executor");
        let output = executor
            .execute(
                &ProcessSpec {
                    program: "powershell.exe".into(),
                    args: vec![
                        "-NoProfile".into(),
                        "-NonInteractive".into(),
                        "-Command".into(),
                        "Write-Output 'hello'; [Console]::Error.WriteLine('warning')".into(),
                    ],
                    cwd: directory.path().display().to_string(),
                    env: BTreeMap::new(),
                    timeout_ms: 10_000,
                },
                CancellationToken::new(),
            )
            .await
            .expect("execute");
        assert_eq!(output.exit_code, Some(0));
        assert_eq!(output.stdout.trim(), "hello");
        assert_eq!(output.stderr.trim(), "warning");
        assert!(!output.cancelled);
    }

    #[tokio::test]
    async fn cancellation_terminates_descendant_process_tree() {
        let directory = tempfile::tempdir().expect("temp directory");
        let marker = directory.path().join("descendant-survived.txt");
        let escaped = marker.display().to_string().replace('\'', "''");
        let script = format!(
            "$child = Start-Process powershell.exe -PassThru -WindowStyle Hidden \
             -ArgumentList @('-NoProfile','-NonInteractive','-Command',\
             \"Start-Sleep -Seconds 2; Set-Content -LiteralPath '{escaped}' -Value survived\"); \
             Wait-Process -Id $child.Id"
        );
        let executor =
            ProcessExecutor::new(directory.path(), ["powershell.exe"]).expect("executor");
        let cancellation = CancellationToken::new();
        let cancel = cancellation.clone();
        let task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(250)).await;
            cancel.cancel();
        });
        let output = executor
            .execute(
                &ProcessSpec {
                    program: "powershell.exe".into(),
                    args: vec![
                        "-NoProfile".into(),
                        "-NonInteractive".into(),
                        "-Command".into(),
                        script,
                    ],
                    cwd: directory.path().display().to_string(),
                    env: BTreeMap::new(),
                    timeout_ms: 10_000,
                },
                cancellation,
            )
            .await
            .expect("execute");
        task.await.expect("canceller");
        assert!(output.cancelled);
        tokio::time::sleep(Duration::from_millis(2_300)).await;
        assert!(
            !marker.exists(),
            "a descendant survived cancellation and wrote the marker"
        );
    }
}
