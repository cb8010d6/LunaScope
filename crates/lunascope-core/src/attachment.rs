use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, rename_all = "snake_case")]
pub enum AttachmentKind {
    Text,
    Pdf,
    Document,
    Spreadsheet,
    Presentation,
    Image,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, rename_all = "camelCase")]
pub struct ImportedAttachment {
    pub attachment_id: String,
    pub display_name: String,
    pub media_type: String,
    pub kind: AttachmentKind,
    #[ts(type = "number")]
    pub size_bytes: u64,
    #[ts(type = "number")]
    pub extracted_characters: u64,
    pub embedded_image_count: u32,
    pub preview_path: Option<String>,
    pub extraction_warning: Option<String>,
}
