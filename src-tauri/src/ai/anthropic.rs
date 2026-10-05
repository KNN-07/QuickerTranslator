use reqwest::Client;
use serde::Serialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::{
    AiProfile, AiUsage, AuthMode, DeltaSink, ProviderOutput, ProviderPrompt,
    sse::{self, SseControl, SseEvent},
    transport,
};
use crate::models::{AppError, AppResult};

#[derive(Serialize)]
struct MessageRequest<'a> {
    model: &'a str,
    system: &'a str,
    messages: [UserMessage<'a>; 1],
    max_tokens: u32,
    stream: bool,
}

#[derive(Serialize)]
struct UserMessage<'a> {
    role: &'static str,
    content: &'a str,
}

/// Speak the native Messages protocol even when the API root is a custom endpoint.
pub async fn translate(
    client: &Client,
    profile: &AiProfile,
    credential: Option<&str>,
    prompt: &ProviderPrompt,
    cancel: &CancellationToken,
    on_delta: &mut DeltaSink<'_>,
) -> AppResult<ProviderOutput> {
    if cancel.is_cancelled() {
        return Err(AppError::new("ai_cancelled", "The AI request was cancelled."));
    }
    let body = MessageRequest {
        model: &profile.model,
        system: &prompt.system,
        messages: [UserMessage { role: "user", content: &prompt.user }],
        max_tokens: profile.max_output_tokens,
        stream: profile.stream,
    };
    let mut request = client
        .post(transport::request_url(profile)?)
        .header("anthropic-version", "2023-06-01")
        .json(&body);
    if profile.auth_mode == AuthMode::ApiKey {
        request = request.header("x-api-key", transport::credential_header(credential)?);
    }
    let response = transport::send(request, cancel).await?;
    if profile.stream {
        let mut state = MessageStream::default();
        transport::stream_sse(response, cancel, |event| state.accept(event, on_delta)).await?;
        state.output()
    } else {
        let message: Value = transport::read_json(response, cancel).await?;
        parse_message(&message, on_delta)
    }
}

fn protocol_error() -> AppError {
    AppError::new("ai_protocol", "The Anthropic endpoint returned an invalid Messages response.")
}

fn string_field<'a>(value: &'a Value, key: &str) -> AppResult<&'a str> {
    value.get(key).and_then(Value::as_str).ok_or_else(protocol_error)
}

fn success_reason(reason: &str) -> AppResult<()> {
    match reason {
        "end_turn" | "stop_sequence" => Ok(()),
        "max_tokens" => Err(AppError::new(
            "ai_interrupted",
            "The Anthropic response reached its output token limit and is incomplete.",
        )),
        "refusal" => Err(AppError::new(
            "ai_content_blocked",
            "Anthropic declined to provide this translation.",
        )),
        "model_context_window_exceeded" => Err(AppError::new(
            "ai_context_too_long",
            "The request exceeded the model's context window.",
        )),
        "tool_use" | "pause_turn" => Err(AppError::new(
            "ai_protocol",
            "The model requested tools or another turn instead of completing the translation.",
        )),
        _ => Err(protocol_error()),
    }
}

/// Streaming usage is cumulative, not a per-event increment. Cached input is billed
/// input too; retain those counters when a later message_delta omits them.
#[derive(Default)]
struct Usage {
    input: Option<u64>,
    cache_creation: Option<u64>,
    cache_read: Option<u64>,
    output: Option<u64>,
}

impl Usage {
    fn update(&mut self, value: Option<&Value>) -> AppResult<()> {
        let Some(value) = value.filter(|value| !value.is_null()) else { return Ok(()); };
        if !value.is_object() {
            return Err(protocol_error());
        }
        for (name, slot) in [
            ("input_tokens", &mut self.input),
            ("cache_creation_input_tokens", &mut self.cache_creation),
            ("cache_read_input_tokens", &mut self.cache_read),
            ("output_tokens", &mut self.output),
        ] {
            if let Some(count) = value.get(name).filter(|count| !count.is_null()) {
                *slot = Some(count.as_u64().ok_or_else(protocol_error)?);
            }
        }
        Ok(())
    }

    fn reported(&self) -> AppResult<Option<AiUsage>> {
        let mut input = None;
        for count in [self.input, self.cache_creation, self.cache_read].into_iter().flatten() {
            input = Some(input.unwrap_or(0_u64).checked_add(count).ok_or_else(protocol_error)?);
        }
        if input.is_none() && self.output.is_none() {
            return Ok(None);
        }
        Ok(Some(AiUsage { input_tokens: input, output_tokens: self.output }))
    }
}

fn parse_message(message: &Value, on_delta: &mut DeltaSink<'_>) -> AppResult<ProviderOutput> {
    if message.get("type").and_then(Value::as_str) == Some("error") || message.get("error").is_some() {
        return Err(transport::provider_error(message));
    }
    if string_field(message, "type")? != "message" {
        return Err(protocol_error());
    }
    let blocks = message.get("content").and_then(Value::as_array).ok_or_else(protocol_error)?;
    let mut text = String::new();
    for block in blocks {
        match string_field(block, "type")? {
            "text" => text.push_str(string_field(block, "text")?),
            "refusal" => return Err(AppError::new("ai_content_blocked", "Anthropic declined to provide this translation.")),
            _ => {} // Thinking, tools, and future non-text blocks are never prose.
        }
    }
    // Publish once even on an output-limit failure: partial text belongs only in
    // the preview, and the adapter still returns an error rather than success.
    if !text.is_empty() {
        on_delta(&text)?;
    }
    success_reason(string_field(message, "stop_reason")?)?;
    if text.trim().is_empty() {
        return Err(AppError::new("ai_protocol", "Anthropic returned no translated text."));
    }
    let mut usage = Usage::default();
    usage.update(message.get("usage"))?;
    Ok(ProviderOutput { text, usage: usage.reported()? })
}

#[derive(Clone, Copy)]
enum BlockKind { Text, Other }

#[derive(Default)]
struct MessageStream {
    started: bool,
    next_index: u64,
    active: Option<(u64, BlockKind)>,
    stopped: bool,
    has_stop_reason: bool,
    text: String,
    usage: Usage,
}

impl MessageStream {
    fn emit(&mut self, text: &str, on_delta: &mut DeltaSink<'_>) -> AppResult<()> {
        if !text.is_empty() {
            on_delta(text)?;
            self.text.push_str(text);
        }
        Ok(())
    }

    fn accept(&mut self, event: SseEvent, on_delta: &mut DeltaSink<'_>) -> AppResult<SseControl> {
        // Unknown auxiliary events and pings need not contain JSON.
        if let Some(kind) = event.event.as_deref() {
            if !matches!(kind, "message_start" | "content_block_start" | "content_block_delta" | "content_block_stop" | "message_delta" | "message_stop" | "error") {
                return Ok(SseControl::Continue);
            }
        }
        let data = sse::json_data(&event)?;
        let kind = string_field(&data, "type")?;
        if event.event.as_deref().is_some_and(|name| name != kind) {
            return Err(protocol_error());
        }
        match kind {
            "error" => return Err(transport::provider_error(&data)),
            "message_start" => {
                if self.started {
                    return Err(protocol_error());
                }
                let message = data.get("message").ok_or_else(protocol_error)?;
                if string_field(message, "type")? != "message"
                    || !message.get("content").and_then(Value::as_array).is_some_and(Vec::is_empty)
                    || message.get("stop_reason").is_some_and(|reason| !reason.is_null())
                {
                    return Err(protocol_error());
                }
                self.usage.update(message.get("usage"))?;
                self.started = true;
            }
            "content_block_start" => {
                self.require_content_phase()?;
                if self.active.is_some() || self.index(&data)? != self.next_index {
                    return Err(protocol_error());
                }
                let block = data.get("content_block").ok_or_else(protocol_error)?;
                let block_kind = match string_field(block, "type")? {
                    "text" => {
                        self.emit(string_field(block, "text")?, on_delta)?;
                        BlockKind::Text
                    }
                    "refusal" => return Err(AppError::new("ai_content_blocked", "Anthropic declined to provide this translation.")),
                    _ => BlockKind::Other,
                };
                self.active = Some((self.next_index, block_kind));
            }
            "content_block_delta" => {
                self.require_content_phase()?;
                let (index, block_kind) = self.active.ok_or_else(protocol_error)?;
                if self.index(&data)? != index {
                    return Err(protocol_error());
                }
                let delta = data.get("delta").ok_or_else(protocol_error)?;
                let delta_kind = string_field(delta, "type")?;
                if let BlockKind::Text = block_kind {
                    if delta_kind != "text_delta" {
                        return Err(protocol_error());
                    }
                    self.emit(string_field(delta, "text")?, on_delta)?;
                }
            }
            "content_block_stop" => {
                self.require_content_phase()?;
                if self.active.map(|(index, _)| index) != Some(self.index(&data)?) {
                    return Err(protocol_error());
                }
                self.active = None;
                self.next_index = self.next_index.checked_add(1).ok_or_else(protocol_error)?;
            }
            "message_delta" => {
                if !self.started || self.active.is_some() {
                    return Err(protocol_error());
                }
                let delta = data.get("delta").filter(|delta| delta.is_object()).ok_or_else(protocol_error)?;
                if let Some(reason) = delta.get("stop_reason").filter(|reason| !reason.is_null()) {
                    let reason = reason.as_str().ok_or_else(protocol_error)?;
                    success_reason(reason)?;
                    if self.has_stop_reason {
                        return Err(protocol_error());
                    }
                    self.has_stop_reason = true;
                }
                self.usage.update(data.get("usage"))?;
            }
            "message_stop" => {
                if !self.started || self.active.is_some() || !self.has_stop_reason {
                    return Err(protocol_error());
                }
                if self.text.trim().is_empty() {
                    return Err(AppError::new("ai_protocol", "Anthropic returned no translated text."));
                }
                self.stopped = true;
                return Ok(SseControl::Done);
            }
            _ => {} // An unnamed future auxiliary event is harmless too.
        }
        Ok(SseControl::Continue)
    }

    fn require_content_phase(&self) -> AppResult<()> {
        if !self.started || self.has_stop_reason {
            Err(protocol_error())
        } else {
            Ok(())
        }
    }

    fn index(&self, value: &Value) -> AppResult<u64> {
        value.get("index").and_then(Value::as_u64).ok_or_else(protocol_error)
    }

    fn output(self) -> AppResult<ProviderOutput> {
        if !self.stopped {
            return Err(AppError::new("ai_interrupted", "The Anthropic stream ended before message_stop."));
        }
        Ok(ProviderOutput { text: self.text, usage: self.usage.reported()? })
    }
}
