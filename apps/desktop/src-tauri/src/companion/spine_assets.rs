use std::fs;
use std::path::{Component, Path, PathBuf};

const TEXTURE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpineAssetSet {
    pub asset_dir: PathBuf,
    pub skel: String,
    pub atlas_files: Vec<String>,
    pub texture_files: Vec<String>,
    pub atlas_texture_refs: Vec<String>,
}

/// Returns texture page names from the non-indented lines of a Spine atlas.
pub fn atlas_texture_refs(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let is_top_level = !line.chars().next().is_some_and(char::is_whitespace);
            let extension = Path::new(trimmed)
                .extension()
                .and_then(|extension| extension.to_str())?;
            (is_top_level
                && !trimmed.is_empty()
                && TEXTURE_EXTENSIONS
                    .iter()
                    .any(|allowed| extension.eq_ignore_ascii_case(allowed)))
            .then(|| trimmed.to_string())
        })
        .collect()
}

/// Reject absolute paths and traversal before resolving a path inside an asset directory.
pub fn safe_relative_path(value: &str) -> Result<PathBuf, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("Path must not be empty.".to_string());
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "Path must stay inside the asset directory: {value}"
        ));
    }
    if path
        .components()
        .all(|component| matches!(component, Component::CurDir))
    {
        return Err("Path must name a file.".to_string());
    }
    Ok(path.to_path_buf())
}

pub fn list_skeletons(asset_dir: &Path) -> Result<Vec<String>, String> {
    let root = asset_dir
        .canonicalize()
        .map_err(|error| format!("Cannot read Spine asset directory: {error}"))?;
    list_files(&root).map(|files| {
        let mut skeletons = files
            .into_iter()
            .filter(|path| has_extension(path, "skel"))
            .map(|path| display_relative(&root, &path))
            .collect::<Vec<_>>();
        sort_paths(&mut skeletons);
        skeletons
    })
}

/// Selects a single skeleton predictably, requiring an explicit selector for ambiguity.
pub fn select_skeleton(asset_dir: &Path, requested: Option<&str>) -> Result<String, String> {
    if let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) {
        let relative = safe_relative_path(requested)?;
        if !has_extension(&relative, "skel") {
            return Err("Choose a Spine .skel file.".to_string());
        }
        ensure_contained_file(asset_dir, &relative)?;
        return Ok(normalize_relative(&relative));
    }

    let skeletons = list_skeletons(asset_dir)?;
    match skeletons.as_slice() {
        [] => Err("No Spine .skel file was found.".to_string()),
        [skel] => Ok(skel.clone()),
        _ => Err(format!(
            "Multiple Spine .skel files were found. Select one explicitly: {}",
            skeletons.join(", ")
        )),
    }
}

#[cfg(test)]
pub fn validate_spine_asset_selection(skel_path: &Path) -> Result<SpineAssetSet, String> {
    let asset_dir = skel_path
        .parent()
        .ok_or_else(|| "Choose a Spine .skel file.".to_string())?;
    let skel = skel_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Choose a Spine .skel file.".to_string())?;
    validate_spine_asset_dir(asset_dir, skel)
}

/// Validates a complete, self-contained Spine runtime directory.
pub fn validate_spine_asset_dir(asset_dir: &Path, skel: &str) -> Result<SpineAssetSet, String> {
    let selected_skel = select_skeleton(asset_dir, Some(skel))?;
    let root = asset_dir
        .canonicalize()
        .map_err(|error| format!("Cannot read Spine asset directory: {error}"))?;
    if !root.is_dir() {
        return Err("Spine asset path must be a directory.".to_string());
    }

    let files = list_files(&root)?;
    let mut atlas_paths = files
        .iter()
        .filter(|path| has_extension(path, "atlas"))
        .cloned()
        .collect::<Vec<_>>();
    atlas_paths.sort();
    let mut texture_paths = files
        .iter()
        .filter(|path| is_texture(path))
        .cloned()
        .collect::<Vec<_>>();
    texture_paths.sort();
    if atlas_paths.is_empty() || texture_paths.is_empty() {
        return Err("The selected .skel folder must also contain at least one .atlas file and one texture image.".to_string());
    }

    let mut references = Vec::new();
    let mut missing = Vec::new();
    for atlas in &atlas_paths {
        let text = fs::read_to_string(atlas).map_err(|error| {
            format!(
                "Unable to read Spine atlas file {}: {error}",
                display_relative(&root, atlas)
            )
        })?;
        let atlas_name = display_relative(&root, atlas);
        for texture in atlas_texture_refs(&text) {
            let relative = safe_relative_path(&texture).map_err(|error| {
                format!("Invalid atlas texture reference in {atlas_name}: {error}")
            })?;
            let texture_path = atlas.parent().unwrap_or(&root).join(&relative);
            match ensure_contained_file(
                &root,
                texture_path.strip_prefix(&root).unwrap_or(&texture_path),
            ) {
                Ok(_) => references.push(display_relative(&root, &texture_path)),
                Err(_) => missing.push(format!("{atlas_name} -> {texture}")),
            }
        }
    }
    if !missing.is_empty() {
        missing.sort();
        missing.dedup();
        return Err(format!(
            "Missing atlas texture file(s): {}",
            missing.join(", ")
        ));
    }

    references.sort();
    references.dedup();
    let mut atlas_files = atlas_paths
        .iter()
        .map(|path| display_relative(&root, path))
        .collect::<Vec<_>>();
    let mut texture_files = texture_paths
        .iter()
        .map(|path| display_relative(&root, path))
        .collect::<Vec<_>>();
    sort_paths(&mut atlas_files);
    sort_paths(&mut texture_files);
    Ok(SpineAssetSet {
        asset_dir: root,
        skel: selected_skel,
        atlas_files,
        texture_files,
        atlas_texture_refs: references,
    })
}

fn list_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("Cannot read Spine asset directory: {error}"))?;
    if !root.is_dir() {
        return Err("Spine asset path must be a directory.".to_string());
    }
    let mut files = Vec::new();
    collect_files(&root, &root, &mut files)?;
    Ok(files)
}

fn collect_files(root: &Path, directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_files(root, &path, files)?;
        } else if file_type.is_file() {
            let canonical = path.canonicalize().map_err(|error| error.to_string())?;
            if !canonical.starts_with(root) {
                return Err(format!(
                    "Asset file escapes its directory: {}",
                    path.to_string_lossy()
                ));
            }
            files.push(canonical);
        }
    }
    Ok(())
}

fn ensure_contained_file(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("Cannot read Spine asset directory: {error}"))?;
    let candidate = root.join(relative);
    let canonical = candidate
        .canonicalize()
        .map_err(|_| format!("File does not exist: {}", candidate.to_string_lossy()))?;
    if !canonical.starts_with(&root) || !canonical.is_file() {
        return Err(format!(
            "File must stay inside the asset directory: {}",
            candidate.to_string_lossy()
        ));
    }
    Ok(canonical)
}

fn has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
}

fn is_texture(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            TEXTURE_EXTENSIONS
                .iter()
                .any(|allowed| extension.eq_ignore_ascii_case(allowed))
        })
}

fn display_relative(root: &Path, path: &Path) -> String {
    normalize_relative(path.strip_prefix(root).unwrap_or(path))
}

fn normalize_relative(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn sort_paths(paths: &mut [String]) {
    paths.sort_by(|left, right| {
        left.to_ascii_lowercase()
            .cmp(&right.to_ascii_lowercase())
            .then(left.cmp(right))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "spine-assets-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn extracts_only_top_level_texture_references() {
        assert_eq!(
            atlas_texture_refs("page.png\nsize: 1,1\nslot\n  rotate: true\npage.webp\n"),
            vec!["page.png", "page.webp"]
        );
    }

    #[test]
    fn requires_explicit_selection_for_multiple_skeletons() {
        let root = temp_dir("multiple");
        fs::write(root.join("b.skel"), []).unwrap();
        fs::write(root.join("a.skel"), []).unwrap();
        let error = select_skeleton(&root, None).unwrap_err();
        assert!(error.contains("a.skel, b.skel"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_atlas_path_traversal() {
        let root = temp_dir("traversal");
        fs::write(root.join("model.skel"), []).unwrap();
        fs::write(root.join("model.png"), []).unwrap();
        fs::write(root.join("model.atlas"), "../outside.png\nsize: 1,1\n").unwrap();
        let error = validate_spine_asset_dir(&root, "model.skel").unwrap_err();
        assert!(error.contains("Invalid atlas texture reference"));
        let _ = fs::remove_dir_all(root);
    }
}
