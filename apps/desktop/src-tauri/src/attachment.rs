use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use lunascope_core::{AttachmentKind, ImportedAttachment};
use serde::{Deserialize, Serialize};
use tauri::command;
use uuid::Uuid;
use zip::ZipArchive;

use crate::{data_root, display_error};

const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 160 * 1024 * 1024;
const MAX_FILES: usize = 16;
const MAX_EXTRACTED_CHARACTERS: usize = 600_000;
const MAX_EMBEDDED_IMAGES: usize = 24;
const MAX_EMBEDDED_IMAGE_BYTES: u64 = 12 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportAttachmentsRequest {
    pub(crate) paths: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredAttachment {
    metadata: ImportedAttachment,
    original_path: String,
    extracted_markdown_path: Option<String>,
    image_paths: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct AttachmentContext {
    pub(crate) markdown: String,
    pub(crate) visual_assets: Vec<AttachmentVisualAsset>,
}

#[derive(Clone, Debug)]
pub(crate) struct AttachmentVisualAsset {
    pub(crate) name: String,
    pub(crate) media_type: String,
    pub(crate) data_base64: String,
    pub(crate) is_document: bool,
}

#[command]
pub(crate) fn import_attachments(
    request: ImportAttachmentsRequest,
) -> Result<Vec<ImportedAttachment>, String> {
    if request.paths.is_empty() {
        return Err("select at least one file".into());
    }
    if request.paths.len() > MAX_FILES {
        return Err(format!("at most {MAX_FILES} files can be attached at once"));
    }
    let mut total = 0_u64;
    let mut imported = Vec::with_capacity(request.paths.len());
    for value in request.paths {
        let path = PathBuf::from(value);
        let canonical = path.canonicalize().map_err(display_error)?;
        if !canonical.is_file() {
            return Err(format!("attachment is not a file: {}", canonical.display()));
        }
        let size = canonical.metadata().map_err(display_error)?.len();
        if size == 0 {
            return Err(format!("attachment is empty: {}", canonical.display()));
        }
        if size > MAX_FILE_BYTES {
            return Err(format!(
                "{} exceeds the {} MiB per-file limit",
                canonical.display(),
                MAX_FILE_BYTES / 1024 / 1024
            ));
        }
        total = total.saturating_add(size);
        if total > MAX_TOTAL_BYTES {
            return Err(format!(
                "selected files exceed the {} MiB total limit",
                MAX_TOTAL_BYTES / 1024 / 1024
            ));
        }
        imported.push(import_one(&canonical, size)?);
    }
    Ok(imported)
}

#[command]
pub(crate) fn remove_imported_attachment(attachment_id: String) -> Result<(), String> {
    let directory = attachment_directory(&attachment_id)?;
    if directory.exists() {
        fs::remove_dir_all(&directory).map_err(display_error)?;
    }
    Ok(())
}

pub(crate) fn load_attachment_context(ids: &[String]) -> Result<AttachmentContext, String> {
    let mut markdown = String::new();
    let mut visual_assets = Vec::new();
    for id in ids.iter().take(MAX_FILES) {
        let stored = load_stored_attachment(id)?;
        markdown.push_str(&format!(
            "\n\n## Attachment: {}\n\n",
            stored.metadata.display_name
        ));
        if let Some(relative) = &stored.extracted_markdown_path {
            let path = attachment_directory(id)?.join(relative);
            let text = fs::read_to_string(path).map_err(display_error)?;
            markdown.push_str(&text);
        } else {
            markdown.push_str("_No machine-readable text was extracted. Use a vision-capable model for this asset._");
        }
        if stored.metadata.kind == AttachmentKind::Image
            || stored.metadata.kind == AttachmentKind::Pdf
        {
            visual_assets.push(load_visual(
                &stored.metadata.display_name,
                &stored.metadata.media_type,
                &PathBuf::from(&stored.original_path),
                stored.metadata.kind == AttachmentKind::Pdf,
            )?);
        }
        for relative in stored.image_paths.iter().take(MAX_EMBEDDED_IMAGES) {
            let path = attachment_directory(id)?.join(relative);
            let media_type = media_type_for_path(&path).unwrap_or("application/octet-stream");
            visual_assets.push(load_visual(
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("embedded-image"),
                media_type,
                &path,
                false,
            )?);
        }
    }
    if markdown.chars().count() > MAX_EXTRACTED_CHARACTERS {
        markdown = markdown
            .chars()
            .take(MAX_EXTRACTED_CHARACTERS)
            .collect::<String>();
        markdown.push_str("\n\n_[Attachment context truncated at the native safety limit.]_");
    }
    Ok(AttachmentContext {
        markdown,
        visual_assets,
    })
}

fn import_one(path: &Path, size_bytes: u64) -> Result<ImportedAttachment, String> {
    let display_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "attachment file name is not valid UTF-8".to_owned())?
        .to_owned();
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let (kind, media_type) = classify(&extension)?;
    let attachment_id = format!("attachment-{}", Uuid::new_v4());
    let directory = attachment_directory(&attachment_id)?;
    fs::create_dir_all(&directory).map_err(display_error)?;
    let original_name = format!(
        "original.{}",
        if extension.is_empty() {
            "bin"
        } else {
            &extension
        }
    );
    let original_path = directory.join(&original_name);
    fs::copy(path, &original_path).map_err(display_error)?;

    let (extracted, warning) = match kind {
        AttachmentKind::Text => (Some(read_text_bounded(&original_path)?), None),
        AttachmentKind::Pdf => match deformat::pdf::extract_file(&original_path) {
            Ok(value) => (Some(value.text), None),
            Err(error) => (
                None,
                Some(format!(
                    "PDF text extraction was incomplete; a vision-capable model can inspect the original pages: {error}"
                )),
            ),
        },
        AttachmentKind::Document | AttachmentKind::Spreadsheet | AttachmentKind::Presentation => {
            match office_oxide::Document::open(&original_path) {
                Ok(document) => (Some(document.to_markdown()), None),
                Err(error) => (None, Some(format!("Office extraction failed: {error}"))),
            }
        }
        AttachmentKind::Image => (None, None),
    };
    let extracted = extracted.map(|value| bounded_characters(value, MAX_EXTRACTED_CHARACTERS));
    let extracted_markdown_path = extracted.as_ref().map(|_| "extracted.md".to_owned());
    if let Some(text) = &extracted {
        fs::write(directory.join("extracted.md"), text).map_err(display_error)?;
    }
    let image_paths = if matches!(extension.as_str(), "docx" | "xlsx" | "pptx") {
        extract_ooxml_images(&original_path, &directory)?
    } else {
        Vec::new()
    };
    let metadata = ImportedAttachment {
        attachment_id: attachment_id.clone(),
        display_name,
        media_type: media_type.to_owned(),
        kind,
        size_bytes,
        extracted_characters: extracted
            .as_ref()
            .map(|value| value.chars().count() as u64)
            .unwrap_or(0),
        embedded_image_count: image_paths.len() as u32,
        preview_path: (kind == AttachmentKind::Image)
            .then(|| original_path.to_string_lossy().into()),
        extraction_warning: warning,
    };
    let stored = StoredAttachment {
        metadata: metadata.clone(),
        original_path: original_path.to_string_lossy().into_owned(),
        extracted_markdown_path,
        image_paths,
    };
    fs::write(
        directory.join("metadata.json"),
        serde_json::to_vec_pretty(&stored).map_err(display_error)?,
    )
    .map_err(display_error)?;
    Ok(metadata)
}

fn extract_ooxml_images(original: &Path, directory: &Path) -> Result<Vec<String>, String> {
    let file = File::open(original).map_err(display_error)?;
    let mut archive = ZipArchive::new(file).map_err(display_error)?;
    let images_dir = directory.join("images");
    let mut paths = Vec::new();
    for index in 0..archive.len() {
        if paths.len() >= MAX_EMBEDDED_IMAGES {
            break;
        }
        let mut entry = archive.by_index(index).map_err(display_error)?;
        let name = entry.name().replace('\\', "/");
        let accepted = name.starts_with("word/media/")
            || name.starts_with("ppt/media/")
            || name.starts_with("xl/media/");
        if !accepted || entry.is_dir() || entry.size() > MAX_EMBEDDED_IMAGE_BYTES {
            continue;
        }
        let Some(file_name) = Path::new(&name)
            .file_name()
            .and_then(|value| value.to_str())
        else {
            continue;
        };
        if media_type_for_path(Path::new(file_name)).is_none() {
            continue;
        }
        fs::create_dir_all(&images_dir).map_err(display_error)?;
        let safe_name = format!("{:02}-{}", paths.len() + 1, sanitize_file_name(file_name));
        let output = images_dir.join(&safe_name);
        let mut writer = File::create(&output).map_err(display_error)?;
        std::io::copy(&mut entry, &mut writer).map_err(display_error)?;
        writer.flush().map_err(display_error)?;
        paths.push(format!("images/{safe_name}"));
    }
    Ok(paths)
}

fn load_stored_attachment(id: &str) -> Result<StoredAttachment, String> {
    let path = attachment_directory(id)?.join("metadata.json");
    serde_json::from_slice(&fs::read(path).map_err(display_error)?).map_err(display_error)
}

fn load_visual(
    name: &str,
    media_type: &str,
    path: &Path,
    is_document: bool,
) -> Result<AttachmentVisualAsset, String> {
    let bytes = fs::read(path).map_err(display_error)?;
    Ok(AttachmentVisualAsset {
        name: name.to_owned(),
        media_type: media_type.to_owned(),
        data_base64: STANDARD.encode(bytes),
        is_document,
    })
}

fn attachment_directory(id: &str) -> Result<PathBuf, String> {
    if !id.starts_with("attachment-")
        || id.len() > 80
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err("invalid attachment id".into());
    }
    Ok(data_root()?.join("attachments").join(id))
}

fn classify(extension: &str) -> Result<(AttachmentKind, &'static str), String> {
    let value = match extension {
        "txt" | "md" | "markdown" | "csv" | "tsv" | "json" | "jsonl" | "xml" | "html" | "htm"
        | "tex" | "bib" | "yaml" | "yml" | "rtf" => {
            (AttachmentKind::Text, media_type_for_extension(extension))
        }
        "pdf" => (AttachmentKind::Pdf, "application/pdf"),
        "doc" | "docx" => (
            AttachmentKind::Document,
            media_type_for_extension(extension),
        ),
        "xls" | "xlsx" => (
            AttachmentKind::Spreadsheet,
            media_type_for_extension(extension),
        ),
        "ppt" | "pptx" => (
            AttachmentKind::Presentation,
            media_type_for_extension(extension),
        ),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" => {
            (AttachmentKind::Image, media_type_for_extension(extension))
        }
        _ => {
            return Err(format!(
                "unsupported attachment type '.{extension}'; use PDF, Office, image, Markdown, text, CSV, JSON, XML, HTML, TeX, or BibTeX"
            ));
        }
    };
    Ok(value)
}

fn media_type_for_extension(extension: &str) -> &'static str {
    match extension {
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "json" | "jsonl" => "application/json",
        "xml" => "application/xml",
        "html" | "htm" => "text/html",
        "tex" => "application/x-tex",
        "bib" => "application/x-bibtex",
        "yaml" | "yml" => "application/yaml",
        "rtf" => "application/rtf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        _ => "text/plain",
    }
}

fn media_type_for_path(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
    )
    .then(|| media_type_for_extension(&extension))
}

fn read_text_bounded(path: &Path) -> Result<String, String> {
    let file = File::open(path).map_err(display_error)?;
    let mut bytes = Vec::new();
    file.take((MAX_EXTRACTED_CHARACTERS * 4) as u64)
        .read_to_end(&mut bytes)
        .map_err(display_error)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn bounded_characters(value: String, maximum: usize) -> String {
    if value.chars().count() <= maximum {
        value
    } else {
        let mut bounded = value.chars().take(maximum).collect::<String>();
        bounded.push_str("\n\n_[Document text truncated at the native safety limit.]_");
        bounded
    }
}

fn sanitize_file_name(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .take(120)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_ids_cannot_escape_the_data_root() {
        assert!(attachment_directory("../outside").is_err());
        assert!(attachment_directory("attachment-good-123").is_ok());
    }

    #[test]
    fn supported_types_are_explicit() {
        assert_eq!(classify("pptx").unwrap().0, AttachmentKind::Presentation);
        assert_eq!(classify("xlsx").unwrap().0, AttachmentKind::Spreadsheet);
        assert_eq!(classify("pdf").unwrap().0, AttachmentKind::Pdf);
        assert!(classify("exe").is_err());
    }

    #[test]
    #[ignore = "run explicitly with LUNASCOPE_ATTACHMENT_FIXTURE_ROOT"]
    fn common_document_fixture_canary() {
        let root = std::env::var("LUNASCOPE_ATTACHMENT_FIXTURE_ROOT")
            .expect("LUNASCOPE_ATTACHMENT_FIXTURE_ROOT");
        for (name, expect_text, expect_images) in [
            ("lecture.pdf", true, false),
            ("lecture.docx", true, true),
            ("lecture.xlsx", true, true),
            ("lecture.pptx", true, true),
            ("formula.png", false, false),
        ] {
            let path = Path::new(&root).join(name);
            let metadata = path.metadata().expect("fixture metadata");
            let imported = import_one(&path, metadata.len()).expect("fixture import");
            assert_eq!(imported.display_name, name);
            if expect_text {
                assert!(
                    imported.extracted_characters > 20,
                    "{name} should yield meaningful extracted text"
                );
            }
            if expect_images {
                assert!(
                    imported.embedded_image_count > 0,
                    "{name} should yield embedded images"
                );
            }
            remove_imported_attachment(imported.attachment_id).expect("fixture cleanup");
        }
    }
}
