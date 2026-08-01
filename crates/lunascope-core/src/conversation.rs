use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{ProjectId, RunId, ThreadId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ConversationRole {
    User,
    Assistant,
    System,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ConversationMessage {
    pub message_id: String,
    pub project_id: ProjectId,
    pub thread_id: ThreadId,
    pub run_id: Option<RunId>,
    #[ts(type = "number")]
    pub sequence: u64,
    pub role: ConversationRole,
    pub content: String,
    #[serde(default)]
    pub context_content: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ThreadContextSummary {
    pub thread_id: ThreadId,
    #[ts(type = "number")]
    pub revision: u64,
    #[ts(type = "number")]
    pub covered_through_sequence: u64,
    #[ts(type = "number")]
    pub source_tokens: u64,
    #[ts(type = "number")]
    pub summary_tokens: u64,
    pub summary: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ThreadContextWindow {
    pub thread_id: ThreadId,
    pub summary: Option<ThreadContextSummary>,
    pub messages: Vec<ConversationMessage>,
    #[ts(type = "number")]
    pub estimated_tokens: u64,
    #[ts(type = "number")]
    pub context_limit_tokens: u64,
    pub compacted: bool,
}
