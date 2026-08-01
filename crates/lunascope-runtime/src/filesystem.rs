use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

const MAX_READ_BYTES: u64 = 1024 * 1024;
const MAX_CREATE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct FilesystemTools {
    root: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadRange {
    pub text: String,
    pub offset: u64,
    pub bytes_read: usize,
    pub file_size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextReplacement {
    pub old: String,
    pub new: String,
    pub expected_occurrences: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextPatch {
    pub path: String,
    pub expected_sha256: String,
    pub replacements: Vec<TextReplacement>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchResult {
    pub path: PathBuf,
    pub previous_sha256: String,
    pub sha256: String,
    pub replacements_applied: usize,
    pub bytes_written: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateResult {
    pub path: PathBuf,
    pub sha256: String,
    pub bytes_written: usize,
}

impl FilesystemTools {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, FilesystemError> {
        let root = root.as_ref().canonicalize()?;
        if !root.is_dir() {
            return Err(FilesystemError::RootNotDirectory(root));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn read_range(
        &self,
        path: impl AsRef<Path>,
        offset: u64,
        length: u64,
    ) -> Result<ReadRange, FilesystemError> {
        if length == 0 || length > MAX_READ_BYTES {
            return Err(FilesystemError::InvalidReadLength(length));
        }
        let path = self.resolve_existing(path.as_ref())?;
        reject_sensitive(&path)?;
        let mut file = fs::File::open(&path)?;
        let file_size = file.metadata()?.len();
        if offset > file_size {
            return Err(FilesystemError::OffsetPastEnd { offset, file_size });
        }
        let full_hash = sha256_reader(fs::File::open(&path)?)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::with_capacity(length as usize);
        file.take(length).read_to_end(&mut bytes)?;
        let text = String::from_utf8(bytes)?;
        Ok(ReadRange {
            bytes_read: text.len(),
            text,
            offset,
            file_size,
            sha256: full_hash,
        })
    }

    pub fn apply_text_patch(&self, patch: &TextPatch) -> Result<PatchResult, FilesystemError> {
        if patch.replacements.is_empty() {
            return Err(FilesystemError::EmptyPatch);
        }
        let requested_path = self.candidate(Path::new(&patch.path))?;
        let metadata = fs::symlink_metadata(&requested_path)?;
        if metadata.file_type().is_symlink() {
            return Err(FilesystemError::SymlinkNotAllowed(requested_path));
        }
        let path = self.resolve_existing(Path::new(&patch.path))?;
        reject_sensitive(&path)?;
        let original = fs::read(&path)?;
        let previous_sha256 = sha256_bytes(&original);
        if !previous_sha256.eq_ignore_ascii_case(&patch.expected_sha256) {
            return Err(FilesystemError::HashMismatch {
                expected: patch.expected_sha256.clone(),
                actual: previous_sha256,
            });
        }
        let mut text = String::from_utf8(original)?;
        let mut replacements_applied = 0usize;
        for replacement in &patch.replacements {
            if replacement.old.is_empty() {
                return Err(FilesystemError::EmptySearch);
            }
            let count = text.matches(&replacement.old).count();
            if count != replacement.expected_occurrences {
                return Err(FilesystemError::OccurrenceMismatch {
                    expected: replacement.expected_occurrences,
                    actual: count,
                });
            }
            text = text.replace(&replacement.old, &replacement.new);
            replacements_applied += count;
        }
        let parent = path
            .parent()
            .ok_or_else(|| FilesystemError::MissingParent(path.clone()))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(text.as_bytes())?;
        temporary.as_file_mut().sync_all()?;
        temporary
            .persist(&path)
            .map_err(|error| FilesystemError::Persist(error.error))?;
        Ok(PatchResult {
            path,
            previous_sha256: patch.expected_sha256.to_lowercase(),
            sha256: sha256_bytes(text.as_bytes()),
            replacements_applied,
            bytes_written: text.len(),
        })
    }

    pub fn create_text_file(
        &self,
        path: impl AsRef<Path>,
        content: &str,
    ) -> Result<CreateResult, FilesystemError> {
        if content.len() > MAX_CREATE_BYTES {
            return Err(FilesystemError::CreateLimit(content.len()));
        }
        let requested_path = self.candidate(path.as_ref())?;
        reject_sensitive(&requested_path)?;
        let parent = requested_path
            .parent()
            .ok_or_else(|| FilesystemError::MissingParent(requested_path.clone()))?
            .canonicalize()?;
        if !parent.starts_with(&self.root) {
            return Err(FilesystemError::OutsideRoot(parent));
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&requested_path)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        Ok(CreateResult {
            path: requested_path,
            sha256: sha256_bytes(content.as_bytes()),
            bytes_written: content.len(),
        })
    }

    fn resolve_existing(&self, path: &Path) -> Result<PathBuf, FilesystemError> {
        let candidate = self.candidate(path)?;
        let canonical = candidate.canonicalize()?;
        if !canonical.starts_with(&self.root) {
            return Err(FilesystemError::OutsideRoot(canonical));
        }
        Ok(canonical)
    }

    fn candidate(&self, path: &Path) -> Result<PathBuf, FilesystemError> {
        if path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(FilesystemError::AmbiguousPath(path.to_path_buf()));
        }
        Ok(if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        })
    }
}

fn reject_sensitive(path: &Path) -> Result<(), FilesystemError> {
    if path.components().any(|component| {
        let value = component.as_os_str().to_string_lossy().to_lowercase();
        matches!(
            value.as_str(),
            ".env"
                | ".ssh"
                | ".aws"
                | ".azure"
                | ".gnupg"
                | "credentials"
                | "id_rsa"
                | "id_ed25519"
        ) || value.starts_with(".env.")
    }) {
        return Err(FilesystemError::SensitivePath(path.to_path_buf()));
    }
    Ok(())
}

fn sha256_reader(mut reader: impl Read) -> Result<String, std::io::Error> {
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex_digest(digest.finalize().as_slice()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex_digest(Sha256::digest(bytes).as_slice())
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to a string cannot fail");
    }
    output
}

#[derive(Debug, Error)]
pub enum FilesystemError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("workspace root is not a directory: {0}")]
    RootNotDirectory(PathBuf),
    #[error("path is outside workspace root: {0}")]
    OutsideRoot(PathBuf),
    #[error("parent traversal is not accepted in tool paths: {0}")]
    AmbiguousPath(PathBuf),
    #[error("sensitive path is denied: {0}")]
    SensitivePath(PathBuf),
    #[error("symlink targets are not writable: {0}")]
    SymlinkNotAllowed(PathBuf),
    #[error("read length must be between 1 and {MAX_READ_BYTES}, got {0}")]
    InvalidReadLength(u64),
    #[error("created text must not exceed {MAX_CREATE_BYTES} bytes, got {0}")]
    CreateLimit(usize),
    #[error("offset {offset} is past file size {file_size}")]
    OffsetPastEnd { offset: u64, file_size: u64 },
    #[error("patch has no replacements")]
    EmptyPatch,
    #[error("patch search text cannot be empty")]
    EmptySearch,
    #[error("file hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
    #[error("replacement occurrence mismatch: expected {expected}, got {actual}")]
    OccurrenceMismatch { expected: usize, actual: usize },
    #[error("path has no parent: {0}")]
    MissingParent(PathBuf),
    #[error("failed to atomically persist patched file: {0}")]
    Persist(std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_hash_and_occurrence_guard_patch() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("sample.txt");
        fs::write(&path, "alpha\nbeta\n").expect("write fixture");
        let tools = FilesystemTools::new(directory.path()).expect("filesystem tools");
        let read = tools
            .read_range("sample.txt", 0, 128)
            .expect("read fixture");
        let result = tools
            .apply_text_patch(&TextPatch {
                path: "sample.txt".into(),
                expected_sha256: read.sha256,
                replacements: vec![TextReplacement {
                    old: "beta".into(),
                    new: "gamma".into(),
                    expected_occurrences: 1,
                }],
            })
            .expect("patch fixture");
        assert_eq!(result.replacements_applied, 1);
        assert_eq!(
            fs::read_to_string(path).expect("read patched"),
            "alpha\ngamma\n"
        );
    }

    #[test]
    fn rejects_hash_drift_and_sensitive_file() {
        let directory = tempfile::tempdir().expect("temp directory");
        fs::write(directory.path().join("sample.txt"), "current").expect("fixture");
        fs::write(directory.path().join(".env"), "SECRET=1").expect("fixture");
        let tools = FilesystemTools::new(directory.path()).expect("filesystem tools");
        assert!(matches!(
            tools.apply_text_patch(&TextPatch {
                path: "sample.txt".into(),
                expected_sha256: "deadbeef".into(),
                replacements: vec![TextReplacement {
                    old: "current".into(),
                    new: "changed".into(),
                    expected_occurrences: 1,
                }],
            }),
            Err(FilesystemError::HashMismatch { .. })
        ));
        assert!(matches!(
            tools.read_range(".env", 0, 100),
            Err(FilesystemError::SensitivePath(_))
        ));
    }

    #[test]
    fn creates_reads_and_modifies_a_real_text_file() {
        let directory = tempfile::tempdir().expect("temp directory");
        let tools = FilesystemTools::new(directory.path()).expect("filesystem tools");
        let created = tools
            .create_text_file("README.md", "# Tiny project\nstatus: draft\n")
            .expect("create file");
        assert_eq!(created.bytes_written, 29);
        let read = tools
            .read_range("README.md", 0, 128)
            .expect("read created file");
        let patched = tools
            .apply_text_patch(&TextPatch {
                path: "README.md".into(),
                expected_sha256: read.sha256,
                replacements: vec![TextReplacement {
                    old: "status: draft".into(),
                    new: "status: verified".into(),
                    expected_occurrences: 1,
                }],
            })
            .expect("modify file");
        assert_eq!(patched.replacements_applied, 1);
        assert_eq!(
            fs::read_to_string(directory.path().join("README.md")).unwrap(),
            "# Tiny project\nstatus: verified\n"
        );
    }
}
