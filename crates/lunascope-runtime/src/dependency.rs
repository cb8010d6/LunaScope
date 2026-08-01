use std::{
    io::{Cursor, Read, Write},
    path::{Component, Path, PathBuf},
    time::Duration,
};

use base64::Engine;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};
use tokio_util::sync::CancellationToken;

const MAX_PACKAGE_ARCHIVE_BYTES: usize = 32 * 1024 * 1024;
const MAX_PACKAGE_EXPANDED_BYTES: u64 = 128 * 1024 * 1024;
const MAX_PACKAGE_FILES: usize = 12_000;

#[derive(Clone, Debug)]
pub struct NpmPackageRequest {
    pub workspace_root: PathBuf,
    pub package: String,
    pub version: Option<String>,
    pub cancellation: CancellationToken,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NpmPackageReceipt {
    pub package: String,
    pub version: String,
    pub license: Option<String>,
    pub integrity: String,
    pub integrity_verified: bool,
    pub package_root: String,
    pub scripts_executed: bool,
    pub source: String,
}

pub async fn provision_npm_package(
    request: NpmPackageRequest,
) -> Result<NpmPackageReceipt, String> {
    validate_package_name(&request.package)?;
    let requested_version = request.version.as_deref().unwrap_or("latest");
    validate_package_version(requested_version)?;
    let workspace = request
        .workspace_root
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !workspace.is_dir() {
        return Err("dependency workspace is not a directory".into());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .user_agent("LunaScope/0.1 workspace dependency inspector")
        .build()
        .map_err(|error| error.to_string())?;
    let mut metadata_url =
        reqwest::Url::parse("https://registry.npmjs.org/").map_err(|error| error.to_string())?;
    metadata_url
        .path_segments_mut()
        .map_err(|_| "npm registry URL cannot be used as a base")?
        .push(&request.package);
    let response = tokio::select! {
        _ = request.cancellation.cancelled() => return Err("dependency provisioning cancelled".into()),
        response = client.get(metadata_url).send() => response.map_err(|error| error.to_string())?,
    };
    if !response.status().is_success() {
        return Err(format!(
            "npm registry metadata request failed with status {}",
            response.status()
        ));
    }
    let metadata: serde_json::Value = response.json().await.map_err(|error| error.to_string())?;
    let resolved_version = if requested_version == "latest" {
        metadata
            .pointer("/dist-tags/latest")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "npm package does not publish a latest dist-tag".to_owned())?
    } else {
        requested_version
    };
    let version_metadata = metadata
        .get("versions")
        .and_then(|versions| versions.get(resolved_version))
        .ok_or_else(|| {
            format!(
                "npm package {} does not publish version {}",
                request.package, resolved_version
            )
        })?;
    let tarball = version_metadata
        .pointer("/dist/tarball")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "npm package metadata is missing dist.tarball".to_owned())?;
    let integrity = version_metadata
        .pointer("/dist/integrity")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "npm package metadata is missing SHA-512 integrity".to_owned())?;
    if !integrity.starts_with("sha512-") {
        return Err("npm package integrity must use SHA-512".into());
    }
    let tarball_url = reqwest::Url::parse(tarball).map_err(|error| error.to_string())?;
    if tarball_url.scheme() != "https" || tarball_url.host_str() != Some("registry.npmjs.org") {
        return Err("npm tarball must remain on https://registry.npmjs.org".into());
    }

    let safe_package = request.package.replace(['@', '/'], "_");
    let dependency_root = workspace
        .join(".lunascope/dependencies/npm")
        .join(&safe_package)
        .join(resolved_version);
    let package_root = dependency_root.join("package");
    let receipt_path = dependency_root.join("receipt.json");
    if package_root.join("package.json").is_file() && receipt_path.is_file() {
        let receipt = std::fs::read_to_string(&receipt_path).map_err(|error| error.to_string())?;
        return serde_json::from_str(&receipt).map_err(|error| error.to_string());
    }

    let archive_response = tokio::select! {
        _ = request.cancellation.cancelled() => return Err("dependency provisioning cancelled".into()),
        response = client.get(tarball_url).send() => response.map_err(|error| error.to_string())?,
    };
    if !archive_response.status().is_success() {
        return Err(format!(
            "npm tarball request failed with status {}",
            archive_response.status()
        ));
    }
    if archive_response
        .content_length()
        .is_some_and(|length| length > MAX_PACKAGE_ARCHIVE_BYTES as u64)
    {
        return Err("npm package archive exceeds the 32 MiB limit".into());
    }
    let archive = archive_response
        .bytes()
        .await
        .map_err(|error| error.to_string())?;
    if archive.len() > MAX_PACKAGE_ARCHIVE_BYTES {
        return Err("npm package archive exceeds the 32 MiB limit".into());
    }
    let expected = base64::engine::general_purpose::STANDARD
        .decode(integrity.trim_start_matches("sha512-"))
        .map_err(|_| "npm package integrity is not valid base64")?;
    let actual = Sha512::digest(&archive);
    if actual.as_slice() != expected.as_slice() {
        return Err("npm package SHA-512 integrity verification failed".into());
    }

    let dependency_parent = dependency_root
        .parent()
        .ok_or_else(|| "dependency target has no parent".to_owned())?;
    std::fs::create_dir_all(dependency_parent).map_err(|error| error.to_string())?;
    let temporary = tempfile::Builder::new()
        .prefix("lunascope-npm-package-")
        .tempdir_in(dependency_parent)
        .map_err(|error| error.to_string())?;
    let temporary_package = temporary.path().join("package");
    unpack_npm_archive(&archive, temporary.path())?;
    if !temporary_package.join("package.json").is_file() {
        return Err("npm package archive does not contain package/package.json".into());
    }
    std::fs::create_dir_all(&dependency_root).map_err(|error| error.to_string())?;
    if package_root.exists() {
        std::fs::remove_dir_all(&package_root).map_err(|error| error.to_string())?;
    }
    std::fs::rename(&temporary_package, &package_root).map_err(|error| error.to_string())?;
    let license = version_metadata
        .get("license")
        .and_then(|value| match value {
            serde_json::Value::String(value) => Some(value.clone()),
            value if !value.is_null() => Some(value.to_string()),
            _ => None,
        });
    let package_root_relative = package_root
        .strip_prefix(&workspace)
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    let receipt = NpmPackageReceipt {
        package: request.package,
        version: resolved_version.into(),
        license,
        integrity: integrity.into(),
        integrity_verified: true,
        package_root: package_root_relative,
        scripts_executed: false,
        source: tarball.into(),
    };
    std::fs::write(
        &receipt_path,
        serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(receipt)
}

fn unpack_npm_archive(bytes: &[u8], destination: &Path) -> Result<(), String> {
    let decoder = GzDecoder::new(Cursor::new(bytes));
    let mut archive = tar::Archive::new(decoder);
    let mut files = 0_usize;
    let mut expanded = 0_u64;
    for entry in archive.entries().map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path().map_err(|error| error.to_string())?;
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err("npm package archive contains an unsafe path".into());
        }
        let target = destination.join(&path);
        let entry_type = entry.header().entry_type();
        if entry_type.is_dir() {
            std::fs::create_dir_all(&target).map_err(|error| error.to_string())?;
            continue;
        }
        if !entry_type.is_file() {
            continue;
        }
        files += 1;
        expanded = expanded.saturating_add(entry.header().size().unwrap_or(0));
        if files > MAX_PACKAGE_FILES || expanded > MAX_PACKAGE_EXPANDED_BYTES {
            return Err("npm package expanded content exceeds the bounded extraction limit".into());
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut output = std::fs::File::create(&target).map_err(|error| error.to_string())?;
        let mut bounded = entry.take(MAX_PACKAGE_EXPANDED_BYTES.saturating_add(1));
        std::io::copy(&mut bounded, &mut output).map_err(|error| error.to_string())?;
        output.flush().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn validate_package_name(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 214 || value.contains("..") || value.contains('\\') {
        return Err("invalid npm package name".into());
    }
    let valid = value.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '@' | '/' | '-' | '_' | '.')
    });
    let scoped =
        !value.starts_with('@') || (value.matches('/').count() == 1 && !value.ends_with('/'));
    if !valid || !scoped || (!value.starts_with('@') && value.contains('/')) {
        return Err("invalid npm package name".into());
    }
    Ok(())
}

fn validate_package_version(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 80
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '+' | '_')
        })
    {
        return Err("invalid npm package version or dist-tag".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_npm_package_identifiers() {
        assert!(validate_package_name("three").is_ok());
        assert!(validate_package_name("@scope/package-name").is_ok());
        assert!(validate_package_name("../escape").is_err());
        assert!(validate_package_version("0.180.0").is_ok());
        assert!(validate_package_version("latest").is_ok());
    }

    #[tokio::test]
    #[ignore = "requires live access to the public npm registry"]
    async fn provisions_real_three_package_without_executing_scripts() {
        let workspace = tempfile::tempdir().expect("workspace");
        let receipt = provision_npm_package(NpmPackageRequest {
            workspace_root: workspace.path().to_path_buf(),
            package: "three".into(),
            version: Some("latest".into()),
            cancellation: CancellationToken::new(),
        })
        .await
        .expect("provision three");
        assert_eq!(receipt.package, "three");
        assert!(receipt.integrity_verified);
        assert!(!receipt.scripts_executed);
        let package_root = workspace.path().join(&receipt.package_root);
        assert!(package_root.join("package.json").is_file());
        assert!(package_root.join("LICENSE").is_file());
        assert!(package_root.join("build/three.module.js").is_file());
    }
}
