use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

const CONFIG_VERSION: u32 = 1;
const CONFIG_FILE_NAME: &str = "data-root.json";
const CONFIG_BACKUP_FILE_NAME: &str = "data-root.json.backup";
const LEGACY_WINDOWS_DATA_ROOT: &str = r"D:\LunaScopeData";

static DATA_PATHS: OnceLock<DataPaths> = OnceLock::new();
#[cfg(test)]
static TEST_DATA_ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DataRootSource {
    Saved,
    Legacy,
    PerUserDefault,
    RecoveryFallback,
}

impl DataRootSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Saved => "saved",
            Self::Legacy => "legacy",
            Self::PerUserDefault => "per_user_default",
            Self::RecoveryFallback => "recovery_fallback",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DataRootStartupIssue {
    configured_root: Option<PathBuf>,
    technical_detail: String,
}

impl DataRootStartupIssue {
    pub(crate) fn configured_root(&self) -> Option<&Path> {
        self.configured_root.as_deref()
    }

    pub(crate) fn technical_detail(&self) -> &str {
        &self.technical_detail
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DataPaths {
    root: PathBuf,
    source: DataRootSource,
    startup_issue: Option<DataRootStartupIssue>,
}

impl DataPaths {
    fn new(root: PathBuf, source: DataRootSource) -> Self {
        Self {
            root,
            source,
            startup_issue: None,
        }
    }

    fn with_startup_issue(mut self, startup_issue: DataRootStartupIssue) -> Self {
        self.startup_issue = Some(startup_issue);
        self
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn source(&self) -> DataRootSource {
        self.source
    }

    pub(crate) fn startup_issue(&self) -> Option<&DataRootStartupIssue> {
        self.startup_issue.as_ref()
    }

    fn require_operational(&self) -> Result<(), DataPathError> {
        if self.startup_issue.is_some() {
            Err(DataPathError::RecoveryMode)
        } else {
            Ok(())
        }
    }

    pub(crate) fn state(&self) -> PathBuf {
        self.root.join("state")
    }

    pub(crate) fn extensions(&self) -> PathBuf {
        self.root.join("extensions")
    }

    pub(crate) fn companion(&self) -> PathBuf {
        self.root.join("companion")
    }

    pub(crate) fn companion_models(&self) -> PathBuf {
        self.companion().join("models")
    }

    pub(crate) fn companion_preload(&self) -> PathBuf {
        self.companion().join("preload")
    }

    pub(crate) fn companion_runtime(&self) -> PathBuf {
        self.companion().join("runtime")
    }

    pub(crate) fn worktrees(&self) -> PathBuf {
        self.root.join("worktrees")
    }

    pub(crate) fn companion_asset_directories(&self) -> [PathBuf; 3] {
        [
            self.companion_models(),
            self.companion_preload(),
            self.companion_runtime(),
        ]
    }

    #[cfg(test)]
    fn contains(&self, path: &Path) -> bool {
        normalize_absolute(path).is_ok_and(|candidate| candidate.starts_with(&self.root))
    }

    #[cfg(test)]
    fn allows_companion_asset(&self, path: &Path) -> bool {
        normalize_absolute(path).is_ok_and(|candidate| {
            self.companion_asset_directories()
                .iter()
                .any(|directory| candidate.starts_with(directory))
        })
    }

    fn ensure_directories(&self) -> Result<(), DataPathError> {
        for path in [
            self.state(),
            self.extensions(),
            self.companion_models(),
            self.companion_preload(),
            self.companion_runtime(),
            self.worktrees(),
        ] {
            fs::create_dir_all(&path).map_err(|source| DataPathError::NotWritable {
                path: path.clone(),
                source,
            })?;
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub(crate) enum DataPathError {
    #[error("DATA_ROOT_CONFIG_INVALID: {message}")]
    ConfigInvalid { message: String },
    #[error("DATA_ROOT_UNAVAILABLE: {path} is not a usable directory")]
    Unavailable { path: PathBuf },
    #[error("DATA_ROOT_NOT_WRITABLE: cannot write to {path}: {source}")]
    NotWritable {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("DATA_ROOT_CONFIG_WRITE_FAILED: cannot save {path}: {source}")]
    ConfigWriteFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("DATA_ROOT_ALREADY_INITIALIZED: data paths were initialized more than once")]
    AlreadyInitialized,
    #[error(
        "DATA_ROOT_UNAVAILABLE: LunaScope is in data-directory recovery mode; choose a valid data directory and restart before changing persistent state"
    )]
    RecoveryMode,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DataRootConfig {
    version: u32,
    data_root: PathBuf,
}

pub(crate) fn current() -> Result<&'static DataPaths, DataPathError> {
    #[cfg(test)]
    if DATA_PATHS.get().is_none() {
        let temporary = TEST_DATA_ROOT
            .get_or_init(|| tempfile::tempdir().expect("temporary desktop data root"));
        let root = temporary.path().join("data");
        let paths = DataPaths::new(root, DataRootSource::PerUserDefault);
        paths.ensure_directories()?;
        let _ = DATA_PATHS.set(paths);
    }
    DATA_PATHS
        .get()
        .ok_or_else(|| DataPathError::ConfigInvalid {
            message: "data paths are not initialized".into(),
        })
}

pub(crate) fn recovery_mode() -> bool {
    current().is_ok_and(|paths| paths.startup_issue().is_some())
}

pub(crate) fn legacy_root() -> PathBuf {
    PathBuf::from(LEGACY_WINDOWS_DATA_ROOT)
}

pub(crate) fn verify_current_writable() -> Result<(), DataPathError> {
    probe_writable(current()?.root()).map_err(|source| DataPathError::NotWritable {
        path: current()
            .map(|paths| paths.root().to_path_buf())
            .unwrap_or_default(),
        source,
    })
}

pub(crate) fn ensure_operational() -> Result<(), DataPathError> {
    current()?.require_operational()
}

pub(crate) fn initialize(
    config_dir: &Path,
    default_root: &Path,
    legacy_root: &Path,
) -> Result<&'static DataPaths, DataPathError> {
    let paths = resolve_with_probe(config_dir, default_root, legacy_root, probe_writable)?;
    paths.ensure_directories()?;
    DATA_PATHS
        .set(paths)
        .map_err(|_| DataPathError::AlreadyInitialized)?;
    current()
}

pub(crate) fn configure(config_dir: &Path, candidate: &Path) -> Result<PathBuf, DataPathError> {
    let root = prepare_root(candidate, &probe_writable)?;
    DataPaths::new(root.clone(), DataRootSource::Saved).ensure_directories()?;
    persist_config(config_dir, &root)?;
    Ok(root)
}

#[cfg(test)]
fn resolve(
    config_dir: &Path,
    default_root: &Path,
    legacy_root: &Path,
) -> Result<DataPaths, DataPathError> {
    resolve_with_probe(config_dir, default_root, legacy_root, probe_writable)
}

fn resolve_with_probe<F>(
    config_dir: &Path,
    default_root: &Path,
    legacy_root: &Path,
    probe: F,
) -> Result<DataPaths, DataPathError>
where
    F: Fn(&Path) -> io::Result<()>,
{
    let config_path = recover_interrupted_config_write(config_dir)?;
    if config_path.exists() {
        let mut configured_root = None;
        let saved_root = (|| {
            let bytes = fs::read(&config_path).map_err(|source| DataPathError::ConfigInvalid {
                message: format!("cannot read {}: {source}", config_path.display()),
            })?;
            let config: DataRootConfig =
                serde_json::from_slice(&bytes).map_err(|source| DataPathError::ConfigInvalid {
                    message: format!("cannot parse {}: {source}", config_path.display()),
                })?;
            configured_root = Some(config.data_root.clone());
            if config.version != CONFIG_VERSION {
                return Err(DataPathError::ConfigInvalid {
                    message: format!("unsupported data-root config version {}", config.version),
                });
            }
            open_existing_root(&config.data_root, &probe)
        })();
        match saved_root {
            Ok(root) => return Ok(DataPaths::new(root, DataRootSource::Saved)),
            Err(error) => {
                // Keep the saved configuration untouched: a removable or temporarily
                // unavailable drive may come back. The per-user fallback exists only so
                // the desktop can open a recovery UI instead of failing Tauri setup.
                let fallback = prepare_root(default_root, &probe)?;
                return Ok(DataPaths::new(fallback, DataRootSource::RecoveryFallback)
                    .with_startup_issue(DataRootStartupIssue {
                        configured_root,
                        technical_detail: error.to_string(),
                    }));
            }
        }
    }

    if legacy_root.is_dir() {
        match open_existing_root(legacy_root, &probe) {
            Ok(root) => {
                persist_config(config_dir, &root)?;
                return Ok(DataPaths::new(root, DataRootSource::Legacy));
            }
            Err(error) => {
                let fallback = prepare_root(default_root, &probe)?;
                return Ok(DataPaths::new(fallback, DataRootSource::RecoveryFallback)
                    .with_startup_issue(DataRootStartupIssue {
                        configured_root: Some(legacy_root.to_path_buf()),
                        technical_detail: error.to_string(),
                    }));
            }
        }
    }
    let root = prepare_root(default_root, &probe)?;
    persist_config(config_dir, &root)?;
    Ok(DataPaths::new(root, DataRootSource::PerUserDefault))
}

fn prepare_root<F>(candidate: &Path, probe: &F) -> Result<PathBuf, DataPathError>
where
    F: Fn(&Path) -> io::Result<()>,
{
    let candidate = normalize_absolute(candidate)?;
    if candidate.exists() && !candidate.is_dir() {
        return Err(DataPathError::Unavailable { path: candidate });
    }
    fs::create_dir_all(&candidate).map_err(|source| DataPathError::NotWritable {
        path: candidate.clone(),
        source,
    })?;
    probe(&candidate).map_err(|source| DataPathError::NotWritable {
        path: candidate.clone(),
        source,
    })?;
    candidate
        .canonicalize()
        .map_err(|_| DataPathError::Unavailable { path: candidate })
}

fn open_existing_root<F>(candidate: &Path, probe: &F) -> Result<PathBuf, DataPathError>
where
    F: Fn(&Path) -> io::Result<()>,
{
    let candidate = normalize_absolute(candidate)?;
    if !candidate.exists() || !candidate.is_dir() {
        return Err(DataPathError::Unavailable { path: candidate });
    }
    probe(&candidate).map_err(|source| DataPathError::NotWritable {
        path: candidate.clone(),
        source,
    })?;
    candidate
        .canonicalize()
        .map_err(|_| DataPathError::Unavailable { path: candidate })
}

fn normalize_absolute(path: &Path) -> Result<PathBuf, DataPathError> {
    if !path.is_absolute() {
        return Err(DataPathError::ConfigInvalid {
            message: format!("data root must be absolute: {}", path.display()),
        });
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(DataPathError::ConfigInvalid {
                    message: format!("data root must not contain '..': {}", path.display()),
                });
            }
        }
    }
    Ok(normalized)
}

fn probe_writable(root: &Path) -> io::Result<()> {
    let probe = root.join(format!(".lunascope-write-probe-{}", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)?;
        file.write_all(b"LunaScope data-root write probe")?;
        file.sync_all()
    })();
    let cleanup = fs::remove_file(&probe);
    match (result, cleanup) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn persist_config(config_dir: &Path, root: &Path) -> Result<(), DataPathError> {
    fs::create_dir_all(config_dir).map_err(|source| DataPathError::ConfigWriteFailed {
        path: config_dir.to_path_buf(),
        source,
    })?;
    let destination = config_dir.join(CONFIG_FILE_NAME);
    let staging = config_dir.join(format!(".{CONFIG_FILE_NAME}.{}.tmp", Uuid::new_v4()));
    let backup = config_dir.join(CONFIG_BACKUP_FILE_NAME);
    let bytes = serde_json::to_vec_pretty(&DataRootConfig {
        version: CONFIG_VERSION,
        data_root: root.to_path_buf(),
    })
    .map_err(|source| DataPathError::ConfigInvalid {
        message: format!("cannot serialize data-root config: {source}"),
    })?;
    let result = (|| -> io::Result<()> {
        fs::write(&staging, bytes)?;
        if destination.exists() {
            if backup.exists() {
                fs::remove_file(&backup)?;
            }
            fs::rename(&destination, &backup)?;
        }
        if let Err(error) = fs::rename(&staging, &destination) {
            if backup.exists() {
                let _ = fs::rename(&backup, &destination);
            }
            return Err(error);
        }
        if backup.exists() {
            // The destination is already committed. A stale backup is safe and
            // will be removed on the next launch if cleanup is interrupted.
            let _ = fs::remove_file(&backup);
        }
        Ok(())
    })();
    if let Err(source) = result {
        let _ = fs::remove_file(staging);
        if backup.exists() && !destination.exists() {
            let _ = fs::rename(backup, &destination);
        }
        return Err(DataPathError::ConfigWriteFailed {
            path: destination,
            source,
        });
    }
    Ok(())
}

fn recover_interrupted_config_write(config_dir: &Path) -> Result<PathBuf, DataPathError> {
    let destination = config_dir.join(CONFIG_FILE_NAME);
    let backup = config_dir.join(CONFIG_BACKUP_FILE_NAME);
    if destination.exists() {
        if backup.exists() {
            let _ = fs::remove_file(backup);
        }
        return Ok(destination);
    }
    if backup.exists() {
        fs::rename(&backup, &destination).map_err(|source| DataPathError::ConfigWriteFailed {
            path: destination.clone(),
            source,
        })?;
    }
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().expect("temporary data-path fixture");
        let config = temp.path().join("config");
        let default = temp.path().join("local-data");
        let legacy = temp.path().join("legacy");
        (temp, config, default, legacy)
    }

    #[test]
    fn existing_legacy_directory_is_reused_without_copying() {
        let (_temp, config, default, legacy) = roots();
        fs::create_dir_all(legacy.join("state")).unwrap();
        fs::write(legacy.join("state/lunascope.db"), b"legacy-db").unwrap();

        let paths = resolve(&config, &default, &legacy).unwrap();

        assert_eq!(paths.source(), DataRootSource::Legacy);
        assert_eq!(paths.root(), legacy.canonicalize().unwrap());
        assert_eq!(
            fs::read(legacy.join("state/lunascope.db")).unwrap(),
            b"legacy-db"
        );
        assert!(!default.exists());
    }

    #[test]
    fn missing_legacy_directory_uses_per_user_default() {
        let (_temp, config, default, legacy) = roots();
        let paths = resolve(&config, &default, &legacy).unwrap();
        assert_eq!(paths.source(), DataRootSource::PerUserDefault);
        assert_eq!(paths.root(), default.canonicalize().unwrap());
        assert!(paths.startup_issue().is_none());
    }

    #[test]
    fn unavailable_saved_root_uses_recovery_fallback_without_replacing_config() {
        let (_temp, config, default, legacy) = roots();
        let unavailable = config.parent().unwrap().join("disconnected-drive");
        fs::create_dir_all(&config).unwrap();
        let config_bytes = serde_json::to_vec_pretty(&DataRootConfig {
            version: CONFIG_VERSION,
            data_root: unavailable.clone(),
        })
        .unwrap();
        fs::write(config.join(CONFIG_FILE_NAME), &config_bytes).unwrap();

        let paths = resolve_with_probe(&config, &default, &legacy, |path| {
            if path == unavailable {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "fixture"))
            } else {
                probe_writable(path)
            }
        })
        .unwrap();

        assert_eq!(paths.source(), DataRootSource::RecoveryFallback);
        assert_eq!(paths.root(), default.canonicalize().unwrap());
        assert_eq!(
            paths.startup_issue().unwrap().configured_root(),
            Some(unavailable.as_path())
        );
        assert_eq!(
            fs::read(config.join(CONFIG_FILE_NAME)).unwrap(),
            config_bytes
        );
        assert!(matches!(
            paths.require_operational(),
            Err(DataPathError::RecoveryMode)
        ));
    }

    #[test]
    fn missing_saved_root_enters_recovery_without_recreating_the_directory() {
        let (_temp, config, default, legacy) = roots();
        let missing = config.parent().unwrap().join("moved-data-root");
        fs::create_dir_all(&config).unwrap();
        let config_bytes = serde_json::to_vec_pretty(&DataRootConfig {
            version: CONFIG_VERSION,
            data_root: missing.clone(),
        })
        .unwrap();
        fs::write(config.join(CONFIG_FILE_NAME), &config_bytes).unwrap();

        assert!(!missing.exists());
        let paths = resolve(&config, &default, &legacy).unwrap();

        assert_eq!(paths.source(), DataRootSource::RecoveryFallback);
        assert_eq!(paths.root(), default.canonicalize().unwrap());
        assert!(
            !missing.exists(),
            "a missing saved root must not be recreated"
        );
        assert_eq!(
            fs::read(config.join(CONFIG_FILE_NAME)).unwrap(),
            config_bytes,
            "recovery must preserve the saved configuration"
        );
        assert!(matches!(
            paths.require_operational(),
            Err(DataPathError::RecoveryMode)
        ));
    }

    #[test]
    fn unwritable_legacy_directory_starts_in_recovery_without_copying() {
        let (_temp, config, default, legacy) = roots();
        fs::create_dir_all(legacy.join("state")).unwrap();
        fs::write(legacy.join("state/lunascope.db"), b"legacy-db").unwrap();

        let paths = resolve_with_probe(&config, &default, &legacy, |path| {
            if path == legacy {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "fixture"))
            } else {
                probe_writable(path)
            }
        })
        .unwrap();

        assert_eq!(paths.source(), DataRootSource::RecoveryFallback);
        assert_eq!(paths.root(), default.canonicalize().unwrap());
        assert_eq!(
            fs::read(legacy.join("state/lunascope.db")).unwrap(),
            b"legacy-db"
        );
        assert!(!config.join(CONFIG_FILE_NAME).exists());
        assert!(matches!(
            paths.require_operational(),
            Err(DataPathError::RecoveryMode)
        ));
    }

    #[test]
    fn configuring_a_replacement_root_is_validated_and_persisted() {
        let (_temp, config, _default, _legacy) = roots();
        let selected = config.parent().unwrap().join("selected");

        let root = configure(&config, &selected).unwrap();
        let saved: DataRootConfig =
            serde_json::from_slice(&fs::read(config.join(CONFIG_FILE_NAME)).unwrap()).unwrap();

        assert_eq!(root, selected.canonicalize().unwrap());
        assert_eq!(saved.data_root, root);
        assert!(root.join("state").is_dir());
        assert!(root.join("companion/models").is_dir());
        assert!(root.join("worktrees").is_dir());
    }

    #[test]
    fn interrupted_config_replace_restores_the_saved_root_from_backup() {
        let (_temp, config, default, legacy) = roots();
        let saved_root = config.parent().unwrap().join("saved-root");
        fs::create_dir_all(&saved_root).unwrap();
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join(CONFIG_BACKUP_FILE_NAME),
            serde_json::to_vec(&DataRootConfig {
                version: CONFIG_VERSION,
                data_root: saved_root.clone(),
            })
            .unwrap(),
        )
        .unwrap();

        let paths = resolve(&config, &default, &legacy).unwrap();

        assert_eq!(paths.source(), DataRootSource::Saved);
        assert_eq!(paths.root(), saved_root.canonicalize().unwrap());
        assert!(config.join(CONFIG_FILE_NAME).is_file());
        assert!(!config.join(CONFIG_BACKUP_FILE_NAME).exists());
    }

    #[test]
    fn saved_custom_root_has_highest_priority() {
        let (_temp, config, default, legacy) = roots();
        let custom = config.parent().unwrap().join("custom");
        fs::create_dir_all(&custom).unwrap();
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            config.join(CONFIG_FILE_NAME),
            serde_json::to_vec(&DataRootConfig {
                version: CONFIG_VERSION,
                data_root: custom.clone(),
            })
            .unwrap(),
        )
        .unwrap();

        let paths = resolve(&config, &default, &legacy).unwrap();
        assert_eq!(paths.source(), DataRootSource::Saved);
        assert_eq!(paths.root(), custom.canonicalize().unwrap());
    }

    #[test]
    fn unwritable_directory_is_reported_deterministically() {
        let (_temp, config, default, legacy) = roots();
        let error = resolve_with_probe(&config, &default, &legacy, |_| {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "fixture"))
        })
        .unwrap_err();
        assert!(matches!(error, DataPathError::NotWritable { .. }));
        assert!(error.to_string().starts_with("DATA_ROOT_NOT_WRITABLE:"));
    }

    #[test]
    fn missing_directory_is_created_when_parent_is_writable() {
        let (_temp, config, default, legacy) = roots();
        assert!(!default.exists());
        let paths = resolve(&config, &default, &legacy).unwrap();
        assert!(paths.root().is_dir());
    }

    #[test]
    fn legacy_choice_is_persisted_for_future_launches() {
        let (_temp, config, default, legacy) = roots();
        fs::create_dir_all(&legacy).unwrap();
        let first = resolve(&config, &default, &legacy).unwrap();
        assert_eq!(first.source(), DataRootSource::Legacy);

        let second = resolve(&config, &default, &config.parent().unwrap().join("missing")).unwrap();
        assert_eq!(second.source(), DataRootSource::Saved);
        assert_eq!(second.root(), first.root());
    }

    #[test]
    fn every_derived_directory_stays_inside_the_root() {
        let (_temp, config, default, legacy) = roots();
        let paths = resolve(&config, &default, &legacy).unwrap();
        for path in [
            paths.state(),
            paths.extensions(),
            paths.companion(),
            paths.companion_models(),
            paths.companion_preload(),
            paths.companion_runtime(),
            paths.worktrees(),
        ] {
            assert!(
                paths.contains(&path),
                "{} escaped the data root",
                path.display()
            );
        }
    }

    #[test]
    fn companion_asset_paths_cannot_escape_the_three_allowed_directories() {
        let (_temp, config, default, legacy) = roots();
        let paths = resolve(&config, &default, &legacy).unwrap();
        assert!(paths.allows_companion_asset(&paths.companion_models().join("model/texture.png")));
        assert!(!paths.allows_companion_asset(&paths.companion().join("settings.json")));
        assert!(!paths.allows_companion_asset(&paths.root().join("state/lunascope.db")));
        assert!(!paths.allows_companion_asset(&paths.companion_models().join("../settings.json")));
    }
}
