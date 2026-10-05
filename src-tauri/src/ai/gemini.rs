use reqwest::{Client, header::ACCEPT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::models::{AppError, AppResult};
use super::{AiProfile, AiUsage, AuthMode, DeltaSink, ProviderOutput, ProviderPrompt, sse::{SseControl, SseEvent}, transport};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerateRequest<'a> {
    system_instruction: RequestContent<'a>,
    contents: [RequestContent<'a>; 1],
    generation_config: GenerationConfig,
}

#[derive(Serialize)]
struct RequestContent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'static str>,
    parts: [RequestPart<'a>; 1],
}

#[derive(Serialize)]
struct RequestPart<'a> {
    text: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationConfig {
    max_output_tokens: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
    prompt_feedback: Option<PromptFeedback>,
    usage_metadata: Option<UsageMetadata>,
    error: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Candidate {
    content: Option<ResponseContent>,
    finish_reason: Option<String>,
    #[serde(default)]
    safety_ratings: Vec<SafetyRating>,
}

#[derive(Deserialize)]
struct ResponseContent {
    #[serde(default)]
    parts: Vec<ResponsePart>,
}

#[derive(Deserialize)]
struct ResponsePart {
    text: Option<String>,
    #[serde(default)]
    thought: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PromptFeedback {
    block_reason: Option<String>,
    #[serde(default)]
    safety_ratings: Vec<SafetyRating>,
}

#[derive(Deserialize)]
struct SafetyRating {
    #[serde(default)]
    blocked: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageMetadata {
    prompt_token_count: Option<u64>,
    candidates_token_count: Option<u64>,
}

#[derive(Default)]
struct ResponseState {
    text: String,
    usage: Option<AiUsage>,
}

fn protocol_error(message: &'static str) -> AppError {
    AppError::new("ai_protocol", message)
}

fn blocked_error() -> AppError {
    AppError::new("ai_content_blocked", "Gemini blocked or refused the translation. Any partial output is incomplete.")
}

fn inspect_finish(reason: Option<&str>) -> AppResult<bool> {
    match reason {
        None | Some("") | Some("FINISH_REASON_UNSPECIFIED") => Ok(false),
        Some("STOP") => Ok(true),
        Some("MAX_TOKENS") => Err(AppError::new("ai_interrupted", "Gemini reached the output token limit. Increase the limit before retrying; any partial output is incomplete.")),
        Some("SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" | "IMAGE_SAFETY" | "IMAGE_PROHIBITED_CONTENT" | "IMAGE_RECITATION" | "ESCALATION" | "PUP_LIMITED_DISABLED") => Err(blocked_error()),
        Some(_) => Err(protocol_error("Gemini stopped without a successful text completion. Check the chosen model and protocol before retrying.")),
    }
}

impl ResponseState {
    fn accept(&mut self, response: GenerateResponse, mut on_delta: Option<&mut DeltaSink<'_>>) -> AppResult<bool> {
        if let Some(error) = response.error {
            return Err(transport::provider_error(&error));
        }
        if let Some(feedback) = &response.prompt_feedback {
            if feedback.block_reason.as_deref().is_some_and(|reason| !reason.is_empty() && reason != "BLOCK_REASON_UNSPECIFIED")
                || feedback.safety_ratings.iter().any(|rating| rating.blocked)
            {
                return Err(blocked_error());
            }
        }
        let metadata_present = response.usage_metadata.is_some();
        if let Some(metadata) = response.usage_metadata {
            // Streaming usage is a cumulative snapshot, not a per-delta count.
            // Keep previously reported counts when a subsequent snapshot omits one.
            if metadata.prompt_token_count.is_some() || metadata.candidates_token_count.is_some() {
                let usage = self.usage.get_or_insert_with(AiUsage::default);
                if metadata.prompt_token_count.is_some() {
                    usage.input_tokens = metadata.prompt_token_count;
                }
                if metadata.candidates_token_count.is_some() {
                    usage.output_tokens = metadata.candidates_token_count;
                }
            }
        }
        let Some(candidate) = response.candidates.into_iter().next() else {
            if metadata_present || response.prompt_feedback.is_some() {
                return Ok(false);
            }
            return Err(protocol_error("Gemini returned no translation candidate."));
        };
        if candidate.safety_ratings.iter().any(|rating| rating.blocked) {
            return Err(blocked_error());
        }
        if let Some(content) = candidate.content {
            for part in content.parts {
                if part.thought {
                    continue;
                }
                if let Some(text) = part.text {
                    if !text.is_empty() {
                        self.text.push_str(&text);
                        if let Some(sink) = on_delta.as_deref_mut() {
                            sink(&text)?;
                        }
                    }
                }
            }
        }
        let completed = inspect_finish(candidate.finish_reason.as_deref())?;
        if completed && self.text.trim().is_empty() {
            return Err(protocol_error("Gemini completed without visible translated text."));
        }
        Ok(completed)
    }

    fn finish(self) -> ProviderOutput {
        ProviderOutput { text: self.text, usage: self.usage }
    }
}

fn stream_response(event: SseEvent) -> AppResult<Option<GenerateResponse>> {
    match event.event.as_deref() {
        None | Some("" | "message") => {},
        Some("error") => {
            return Err(transport::provider_error(&super::sse::json_data(&event)?));
        },
        Some(_) => return Ok(None),
    }
    serde_json::from_str(&event.data)
        .map(Some)
        .map_err(|_| protocol_error("Gemini sent malformed generation data."))
}

/// Uses Gemini's native wire format on official and explicitly configured roots.
/// Only a first-candidate STOP is successful; partial output is never a completion.
pub async fn translate(
    client: &Client,
    profile: &AiProfile,
    credential: Option<&str>,
    prompt: &ProviderPrompt,
    cancel: &CancellationToken,
    on_delta: &mut DeltaSink<'_>,
) -> AppResult<ProviderOutput> {
    if cancel.is_cancelled() {
        return Err(transport::cancelled_error());
    }
    let payload = GenerateRequest {
        system_instruction: RequestContent { role: None, parts: [RequestPart { text: &prompt.system }] },
        contents: [RequestContent { role: Some("user"), parts: [RequestPart { text: &prompt.user }] }],
        generation_config: GenerationConfig { max_output_tokens: profile.max_output_tokens },
    };
    let mut request = client.post(transport::request_url(profile)?).json(&payload);
    if profile.auth_mode == AuthMode::ApiKey {
        request = request.header("x-goog-api-key", transport::credential_header(credential)?);
    }
    if profile.stream {
        request = request.header(ACCEPT, "text/event-stream");
    }
    let response = transport::send(request, cancel).await?;
    let mut state = ResponseState::default();
    if profile.stream {
        transport::stream_sse(response, cancel, |event| {
            let Some(response) = stream_response(event)? else {
                return Ok(SseControl::Continue);
            };
            if state.accept(response, Some(&mut *on_delta))? {
                Ok(SseControl::Done)
            } else {
                Ok(SseControl::Continue)
            }
        }).await?;
    } else {
        let response = transport::read_json::<GenerateResponse>(response, cancel).await?;
        if cancel.is_cancelled() {
            return Err(transport::cancelled_error());
        }
        if !state.accept(response, Some(&mut *on_delta))? {
            return Err(protocol_error("Gemini returned a response without a successful finish reason."));
        }
        if cancel.is_cancelled() {
            return Err(transport::cancelled_error());
        }
    }
    if cancel.is_cancelled() {
        return Err(transport::cancelled_error());
    }
    Ok(state.finish())
}
