use serde::{Deserialize, Serialize};
use crate::models::{AppResult, SourceLanguage, TextRange};

pub mod profiles;
pub mod prompt;
pub mod sse;
pub mod transport;
pub mod openai;
pub mod gemini;
pub mod anthropic;
pub mod runtime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiProtocol {
    #[serde(rename = "openai-responses")] OpenaiResponses,
    #[serde(rename = "openai-chat")] OpenaiChat,
    #[serde(rename = "gemini")] Gemini,
    #[serde(rename = "anthropic")] Anthropic,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthMode {
    #[serde(rename = "apiKey")] ApiKey,
    #[serde(rename = "none")] None,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenLimitField {
    #[serde(rename = "max_tokens")] MaxTokens,
    #[serde(rename = "max_completion_tokens")] MaxCompletionTokens,
    #[serde(rename = "omit")] Omit,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiProfile {
    pub id: String,
    pub name: String,
    pub protocol: AiProtocol,
    pub base_url: String,
    pub model: String,
    pub stream: bool,
    #[serde(default = "profiles::default_max_output_tokens")]
    pub max_output_tokens: u32,
    pub auth_mode: AuthMode,
    pub allow_insecure_http: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_limit_field: Option<TokenLimitField>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiMode { Translate, Improve }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiScope { Selection, Document }
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiTranslationRequest {
    pub job_id: String,
    pub document_id: String,
    pub source_revision: u64,
    pub target_revision: u64,
    pub profile_id: String,
    pub mode: AiMode,
    pub source_language: SourceLanguage,
    pub scope: AiScope,
    pub source_range: TextRange,
    pub source_text: String,
    pub target_text: Option<String>,
    pub instructions: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AiEvent {
    Started { job_id: String, chunk_count: usize },
    ChunkStarted { job_id: String, chunk_index: usize, source_range: TextRange },
    Delta { job_id: String, chunk_index: usize, text: String },
    ChunkCompleted { job_id: String, chunk_index: usize, text: String, usage: Option<AiUsage> },
    Completed { job_id: String, text: String, usage: Option<AiUsage> },
    Cancelled { job_id: String },
    Error { job_id: String, code: String, message: String, chunk_index: Option<usize> },
}

/// Prompts are immutable user-approved scope data; never Debug-log their contents.
pub struct ProviderPrompt {
    pub system: String,
    pub user: String,
}
#[derive(Clone)]
pub struct ProviderOutput {
    pub text: String,
    pub usage: Option<AiUsage>,
}
/// Real protocol adapters append output and publish deltas through this callback.
pub type DeltaSink<'a> = dyn FnMut(&str) -> AppResult<()> + Send + 'a;
