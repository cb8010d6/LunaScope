use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use lunascope_core::{ActionableDiagnostic, DiagnosticCode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutableCapability {
    pub id: String,
    pub path: Option<String>,
    pub version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserCapability {
    pub id: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentInventory {
    pub executables: Vec<ExecutableCapability>,
    pub browser: Option<BrowserCapability>,
    pub workspace_manifests: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceAccess {
    Accessible,
    Missing,
    PermissionDenied,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvironmentPreflightContext {
    pub data_root: String,
    pub data_root_source: String,
    pub data_root_writable: bool,
    pub workspace_root: String,
    pub workspace_access: WorkspaceAccess,
    pub provider_configured: bool,
    pub orchestration_model_configured: bool,
    pub orchestration_model_verified: bool,
    pub required_capabilities: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentPreflight {
    pub ready: bool,
    pub summary: String,
    pub issues: Vec<ActionableDiagnostic>,
    pub data_root: String,
    pub data_root_source: String,
    pub workspace_root: String,
    pub inventory: EnvironmentInventory,
}

impl EnvironmentInventory {
    pub fn inspect(workspace_root: &Path) -> Self {
        let candidates = [
            ("git", &["git.exe", "git"][..], &["--version"][..]),
            ("node", &["node.exe", "node"][..], &["--version"][..]),
            ("npm", &["npm.cmd", "npm"][..], &["--version"][..]),
            ("python", &["python.exe", "python"][..], &["--version"][..]),
            ("py", &["py.exe", "py"][..], &["--version"][..]),
            ("cargo", &["cargo.exe", "cargo"][..], &["--version"][..]),
        ];
        let executables = candidates
            .into_iter()
            .map(|(id, names, version_args)| {
                let resolved = resolve_program_on_path(names);
                let version = resolved
                    .as_deref()
                    .and_then(|path| program_version(path, version_args));
                ExecutableCapability {
                    id: id.into(),
                    path: resolved.map(|path| path.to_string_lossy().into_owned()),
                    version,
                }
            })
            .collect();
        let browser = installed_headless_browser().map(|path| BrowserCapability {
            id: if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("msedge.exe"))
            {
                "edge".into()
            } else {
                "chrome".into()
            },
            path: path.to_string_lossy().into_owned(),
        });
        let workspace_manifests = [
            "package.json",
            "package-lock.json",
            "pnpm-lock.yaml",
            "yarn.lock",
            "bun.lock",
            "pyproject.toml",
            "requirements.txt",
            "Cargo.toml",
            "pom.xml",
            "build.gradle",
            "build.gradle.kts",
        ]
        .into_iter()
        .filter(|name| workspace_root.join(name).is_file())
        .map(str::to_owned)
        .collect();
        Self {
            executables,
            browser,
            workspace_manifests,
        }
    }

    pub fn prompt_summary(&self) -> String {
        let mut lines = self
            .executables
            .iter()
            .map(|capability| match (&capability.path, &capability.version) {
                (Some(_), Some(version)) => format!("{}: available ({})", capability.id, version),
                (Some(_), None) => format!("{}: found but version unavailable", capability.id),
                (None, _) => format!("{}: unavailable", capability.id),
            })
            .collect::<Vec<_>>();
        lines.push(match &self.browser {
            Some(browser) => format!("headless browser: {} available", browser.id),
            None => "headless browser: unavailable".into(),
        });
        lines.push(if self.workspace_manifests.is_empty() {
            "workspace manifests: none".into()
        } else {
            format!(
                "workspace manifests: {}",
                self.workspace_manifests.join(", ")
            )
        });
        lines.join("\n")
    }

    pub fn executable(&self, id: &str) -> Option<&ExecutableCapability> {
        self.executables
            .iter()
            .find(|capability| capability.id == id)
    }

    pub fn has_executable(&self, id: &str) -> bool {
        self.executable(id)
            .is_some_and(|capability| capability.path.is_some() && capability.version.is_some())
    }

    pub fn evaluate_preflight(
        &self,
        context: &EnvironmentPreflightContext,
    ) -> EnvironmentPreflight {
        let mut issues = Vec::new();
        if !context.data_root_writable {
            issues.push(ActionableDiagnostic::error(
                DiagnosticCode::DataRootNotWritable,
                "LunaScope cannot write its data directory",
                "The configured LunaScope data directory is not writable.",
                "LunaScope stores its database, extensions, isolated worktrees, and local model assets there.",
                [
                    "Choose a directory owned by your Windows account.",
                    "Check free space and folder permissions, then retry.",
                ],
            ));
        }
        match context.workspace_access {
            WorkspaceAccess::Accessible => {}
            WorkspaceAccess::Missing => issues.push(ActionableDiagnostic::error(
                DiagnosticCode::WorkspaceInvalid,
                "Choose an accessible project folder",
                "The selected workspace does not exist or is not a directory.",
                "Agents need a real folder so LunaScope can create isolated worktrees and verify changes.",
                ["Choose Folder and select the project directory.", "Retry the environment check."],
            )),
            WorkspaceAccess::PermissionDenied => issues.push(ActionableDiagnostic::error(
                DiagnosticCode::WorkspacePermissionDenied,
                "Windows blocked access to the project folder",
                "LunaScope could not read the selected workspace.",
                "The folder permissions or a Windows security policy denied access.",
                [
                    "Choose a folder your Windows account can read and write.",
                    "If Controlled Folder Access is enabled, allow LunaScope and retry.",
                ],
            )),
        }
        if !self.has_executable("git") {
            issues.push(ActionableDiagnostic::error(
                DiagnosticCode::GitNotFound,
                "Git is required before Agents can run",
                "LunaScope did not find Git on this computer.",
                "LunaScope uses Git worktrees to isolate Agent changes and integrate only verified results.",
                [
                    "Install Git for Windows from https://git-scm.com/download/win.",
                    "Restart LunaScope after installation, then retry the check.",
                ],
            ));
        }
        if !context.provider_configured {
            issues.push(ActionableDiagnostic::error(
                DiagnosticCode::ProviderNotConfigured,
                "Connect an AI provider",
                "No enabled AI provider is configured.",
                "LunaScope needs one tested provider to plan and run the task.",
                ["Open Settings → Models & Providers, configure an enabled Provider, then retry."],
            ));
        } else if !context.orchestration_model_configured {
            issues.push(ActionableDiagnostic::error(
                DiagnosticCode::ProviderModelUnsupported,
                "Choose an orchestration model",
                "The current orchestration model does not match an enabled provider.",
                "The orchestration model plans the run and must be available before any Worker starts.",
                ["Open Settings → Models & Providers and choose a model from an enabled Provider."],
            ));
        } else if !context.orchestration_model_verified {
            issues.push(ActionableDiagnostic::error(
                DiagnosticCode::ModelReasoningTestRequired,
                "Verify the selected model",
                "The selected Provider, model, endpoint, credential reference, and reasoning setting have not passed a compatibility test for this exact configuration.",
                "A real compatibility check prevents an unsupported model setting from failing after the task starts.",
                ["Open Settings → Models & Providers and run the model test for the selected setting before retrying."],
            ));
        }

        for capability in &context.required_capabilities {
            let available = match capability.as_str() {
                "browser" => self.browser.is_some(),
                "python" => self.has_executable("python") || self.has_executable("py"),
                id => self.has_executable(id),
            };
            if available {
                continue;
            }
            let (code, title, fix) = if capability == "browser" {
                (
                    DiagnosticCode::BrowserNotAvailable,
                    "A browser is required for this verification",
                    "Install or repair Microsoft Edge, then retry.",
                )
            } else {
                (
                    DiagnosticCode::ToolDependencyMissing,
                    "A task dependency is missing",
                    "Install the named tool, restart LunaScope, then retry.",
                )
            };
            issues.push(
                ActionableDiagnostic::error(
                    code,
                    title,
                    format!("The task requires {capability}, but LunaScope did not find it."),
                    "This dependency is required by the selected Worker tools or verification steps, not by every LunaScope task.",
                    [fix],
                )
                .with_technical_detail(format!("missing capability: {capability}")),
            );
        }
        let ready = issues.is_empty();
        EnvironmentPreflight {
            ready,
            summary: if ready {
                "Ready".into()
            } else {
                format!(
                    "{} issue{} needs attention",
                    issues.len(),
                    if issues.len() == 1 { "" } else { "s" }
                )
            },
            issues,
            data_root: context.data_root.clone(),
            data_root_source: context.data_root_source.clone(),
            workspace_root: context.workspace_root.clone(),
            inventory: self.clone(),
        }
    }
}

pub fn installed_headless_browser() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    for variable in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
        let Some(root) = std::env::var_os(variable) else {
            continue;
        };
        let root = PathBuf::from(root);
        candidates.push(root.join("Microsoft/Edge/Application/msedge.exe"));
        candidates.push(root.join("Google/Chrome/Application/chrome.exe"));
    }
    candidates
        .into_iter()
        .find(|path| path.is_file() && program_version(path, &["--version"]).is_some())
        .or_else(|| {
            resolve_program_on_path(&["msedge.exe", "chrome.exe"])
                .filter(|path| program_version(path, &["--version"]).is_some())
        })
}

pub fn resolve_program_on_path(candidates: &[&str]) -> Option<PathBuf> {
    for candidate in candidates {
        let requested = Path::new(candidate);
        if requested.is_absolute() && requested.is_file() {
            return requested.canonicalize().ok();
        }
        if requested.components().count() != 1 {
            continue;
        }
        #[cfg(windows)]
        {
            let Some(system_root) = std::env::var_os("SystemRoot") else {
                continue;
            };
            let where_exe = PathBuf::from(system_root).join("System32/where.exe");
            let mut command = Command::new(where_exe);
            crate::hide_console_window(&mut command);
            let Ok(output) = command
                .arg(format!("$PATH:{candidate}"))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output()
            else {
                continue;
            };
            if output.status.success()
                && let Some(path) = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
            {
                let path = PathBuf::from(path.trim_matches('"'));
                if path.is_file() {
                    return path.canonicalize().ok().or(Some(path));
                }
            }
        }
        #[cfg(not(windows))]
        {
            let Ok(output) = Command::new("which")
                .arg(candidate)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output()
            else {
                continue;
            };
            if output.status.success() {
                let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
                if path.is_file() {
                    return path.canonicalize().ok().or(Some(path));
                }
            }
        }
    }
    None
}

fn program_version(path: &Path, args: &[&str]) -> Option<String> {
    let mut command = Command::new(path);
    crate::hide_console_window(&mut command);
    let output = command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(minimal_environment())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = if output.stdout.is_empty() {
        &output.stderr
    } else {
        &output.stdout
    };
    String::from_utf8_lossy(value)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(160).collect())
}

fn minimal_environment() -> BTreeMap<String, String> {
    [
        "PATH",
        "Path",
        "PATHEXT",
        "SYSTEMROOT",
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
    ]
    .into_iter()
    .filter_map(|name| std::env::var(name).ok().map(|value| (name.into(), value)))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_reports_workspace_manifests_without_guessing() {
        let workspace = tempfile::tempdir().expect("temp workspace");
        std::fs::write(workspace.path().join("package.json"), "{}").expect("manifest");
        let inventory = EnvironmentInventory::inspect(workspace.path());
        assert!(
            inventory
                .workspace_manifests
                .contains(&"package.json".into())
        );
        assert!(inventory.prompt_summary().contains("workspace manifests"));
        assert!(!inventory.prompt_summary().contains("AppData"));
    }

    fn fixture_inventory(ids: &[&str]) -> EnvironmentInventory {
        EnvironmentInventory {
            executables: ["git", "node", "npm", "python", "py", "cargo"]
                .into_iter()
                .map(|id| ExecutableCapability {
                    id: id.into(),
                    path: ids.contains(&id).then(|| format!("C:/tools/{id}.exe")),
                    version: ids.contains(&id).then(|| "fixture-version".into()),
                })
                .collect(),
            browser: ids.contains(&"browser").then(|| BrowserCapability {
                id: "edge".into(),
                path: "C:/Program Files/Edge/msedge.exe".into(),
            }),
            workspace_manifests: Vec::new(),
        }
    }

    fn ready_context() -> EnvironmentPreflightContext {
        EnvironmentPreflightContext {
            data_root: "C:/Users/test/AppData/Local/LunaScope".into(),
            data_root_source: "per_user_default".into(),
            data_root_writable: true,
            workspace_root: "C:/workspace".into(),
            workspace_access: WorkspaceAccess::Accessible,
            provider_configured: true,
            orchestration_model_configured: true,
            orchestration_model_verified: true,
            required_capabilities: BTreeSet::new(),
        }
    }

    #[test]
    fn missing_git_is_caught_before_worktree_execution() {
        let report = fixture_inventory(&[]).evaluate_preflight(&ready_context());
        assert!(!report.ready);
        assert_eq!(report.issues[0].code, DiagnosticCode::GitNotFound);
        assert!(report.issues[0].why.contains("worktrees"));
    }

    #[test]
    fn an_unrunnable_executable_is_not_ready() {
        let mut inventory = fixture_inventory(&["git"]);
        inventory.executables[0].version = None;
        let report = inventory.evaluate_preflight(&ready_context());
        assert!(!report.ready);
        assert_eq!(report.issues[0].code, DiagnosticCode::GitNotFound);
    }

    #[test]
    fn optional_tool_is_not_required_for_an_unrelated_task() {
        let report = fixture_inventory(&["git"]).evaluate_preflight(&ready_context());
        assert!(report.ready);
    }

    #[test]
    fn task_specific_tool_is_checked_only_when_requested() {
        let mut context = ready_context();
        context.required_capabilities.insert("cargo".into());
        let report = fixture_inventory(&["git"]).evaluate_preflight(&context);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].code, DiagnosticCode::ToolDependencyMissing);
        assert!(report.issues[0].what_happened.contains("cargo"));
    }
}
