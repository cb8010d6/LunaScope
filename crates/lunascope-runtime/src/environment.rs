use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

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
                (Some(path), Some(version)) => {
                    format!("{}: available at {} ({})", capability.id, path, version)
                }
                (Some(path), None) => format!("{}: available at {}", capability.id, path),
                (None, _) => format!("{}: unavailable", capability.id),
            })
            .collect::<Vec<_>>();
        lines.push(match &self.browser {
            Some(browser) => format!("headless browser: {} at {}", browser.id, browser.path),
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
        .find(|path| path.is_file())
        .or_else(|| resolve_program_on_path(&["msedge.exe", "chrome.exe"]))
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
            let Ok(output) = Command::new(where_exe)
                .arg(format!("$PATH:{candidate}"))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output()
            else {
                continue;
            };
            if output.status.success() {
                if let Some(path) = String::from_utf8_lossy(&output.stdout)
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
    let output = Command::new(path)
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
    }
}
