use reqwest::header::AUTHORIZATION;
use serde::Serialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::models::{AppError, AppResult};

use super::{
    AiProfile, AiUsage, AuthMode, DeltaSink, ProviderOutput, ProviderPrompt, TokenLimitField,
    sse::{SseControl, SseEvent, json_data},
    transport,
};

#[derive(Serialize)]
struct ResponsesRequest<'a> {
    model: &'a str,
    instructions: &'a str,
    input: &'a str,
    max_output_tokens: u32,
    stream: bool,
    store: bool,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: [ChatMessage<'a>; 2],
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
}

fn authorized_request(
    client: &reqwest::Client,
    profile: &AiProfile,
    credential: Option<&str>,
) -> AppResult<reqwest::RequestBuilder> {
    let request = client.post(transport::request_url(profile)?);
    match profile.auth_mode {
        AuthMode::None => Ok(request),
        AuthMode::ApiKey => Ok(request.header(AUTHORIZATION, transport::bearer_header(credential)?)),
    }
}

/// OpenAI Responses, with its native request and terminal-event semantics on any API root.
pub async fn translate_responses(
    client: &reqwest::Client,
    profile: &AiProfile,
    credential: Option<&str>,
    prompt: &ProviderPrompt,
    cancel: &CancellationToken,
    on_delta: &mut DeltaSink<'_>,
) -> AppResult<ProviderOutput> {
    let request = authorized_request(client, profile, credential)?.json(&ResponsesRequest {
        model: &profile.model,
        instructions: &prompt.system,
        input: &prompt.user,
        max_output_tokens: profile.max_output_tokens,
        stream: profile.stream,
        store: false,
    });
    let response = transport::send(request, cancel).await?;
    if !profile.stream {
        let value: Value = transport::read_json(response, cancel).await?;
        reject_error(&value)?;
        if value.get("status").and_then(Value::as_str) == Some("incomplete") {
            let error = incomplete_response(&value);
            if error.code == "ai_interrupted" && value.get("output").is_some() {
                let text = responses_text(&value, true)?;
                publish(&text, cancel, on_delta)?;
            }
            return Err(error);
        }
        let output = responses_output(&value)?;
        publish(&output.text, cancel, on_delta)?;
        return Ok(output);
    }

    let mut text = String::new();
    let mut usage = None;
    transport::stream_sse(response, cancel, |event| {
        if auxiliary_event(&event, &[
            "response.output_text.delta", "response.completed", "response.failed",
            "response.incomplete", "response.refusal.delta", "response.refusal.done", "error",
        ]) {
            return Ok(SseControl::Continue);
        }
        let value = json_data(&event)?;
        reject_error(&value)?;
        match event_type(&event, &value)? {
            "response.output_text.delta" => {
                let delta = required_str(&value, "delta")?;
                publish(delta, cancel, on_delta)?;
                text.push_str(delta);
                Ok(SseControl::Continue)
            }
            "response.completed" => {
                let output = responses_output(value.get("response")
                    .ok_or_else(|| protocol("The completed response is missing its result."))?)?;
                // The terminal response is authoritative. Never silently rewrite a streamed preview.
                let remainder = output.text.strip_prefix(text.as_str())
                    .ok_or_else(|| protocol("The completed response contradicts its streamed text."))?;
                publish(remainder, cancel, on_delta)?;
                text = output.text;
                usage = output.usage;
                Ok(SseControl::Done)
            }
            "response.failed" | "response.incomplete" => {
                let response = value.get("response")
                    .ok_or_else(|| protocol("The unsuccessful response is missing its result."))?;
                reject_error(response)?;
                Err(incomplete_response(response))
            }
            "response.refusal.delta" | "response.refusal.done" => Err(blocked()),
            "error" => Err(transport::provider_error(&value)),
            _ => Ok(SseControl::Continue),
        }
    }).await?;
    ensure_not_cancelled(cancel)?;
    Ok(ProviderOutput { text, usage })
}

/// OpenAI-compatible Chat Completions. No usage extensions or implicit protocol fallbacks.
pub async fn translate_chat(
    client: &reqwest::Client,
    profile: &AiProfile,
    credential: Option<&str>,
    prompt: &ProviderPrompt,
    cancel: &CancellationToken,
    on_delta: &mut DeltaSink<'_>,
) -> AppResult<ProviderOutput> {
    let limit_field = profile.token_limit_field.unwrap_or(TokenLimitField::MaxTokens);
    let request = authorized_request(client, profile, credential)?.json(&ChatRequest {
        model: &profile.model,
        messages: [
            ChatMessage { role: "system", content: &prompt.system },
            ChatMessage { role: "user", content: &prompt.user },
        ],
        stream: profile.stream,
        max_tokens: (limit_field == TokenLimitField::MaxTokens).then_some(profile.max_output_tokens),
        max_completion_tokens: (limit_field == TokenLimitField::MaxCompletionTokens)
            .then_some(profile.max_output_tokens),
    });
    let response = transport::send(request, cancel).await?;
    if !profile.stream {
        let value: Value = transport::read_json(response, cancel).await?;
        reject_error(&value)?;
        let choice = first_choice(&value)?.ok_or_else(|| protocol("The provider returned no completion choice."))?;
        let finish = match require_stop(choice.get("finish_reason")) {
            Err(error) if error.code != "ai_interrupted" => return Err(error),
            finish => finish,
        };
        let message = choice.get("message").and_then(Value::as_object)
            .ok_or_else(|| protocol("The completion is missing its message."))?;
        reject_refusal(message.get("refusal"))?;
        let text = message.get("content").and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| protocol("The provider returned no translated text."))?;
        let usage = parse_usage(value.get("usage"), "prompt_tokens", "completion_tokens")?;
        publish(text, cancel, on_delta)?;
        finish?;
        return Ok(ProviderOutput { text: text.to_owned(), usage });
    }

    let mut text = String::new();
    let mut usage = None;
    let mut stopped = false;
    transport::stream_sse(response, cancel, |event| {
        if auxiliary_event(&event, &["error"]) {
            return Ok(SseControl::Continue);
        }
        if event.data.trim() == "[DONE]" {
            if !stopped {
                return Err(interrupted("The completion ended without a successful stop reason."));
            }
            if text.is_empty() {
                return Err(protocol("The provider returned no translated text."));
            }
            return Ok(SseControl::Done);
        }
        let value = json_data(&event)?;
        reject_error(&value)?;
        if event.event.as_deref() == Some("error") {
            return Err(transport::provider_error(&value));
        }
        if let Some(reported) = parse_usage(value.get("usage"), "prompt_tokens", "completion_tokens")? {
            usage = Some(reported);
        }
        let Some(choice) = first_choice(&value)? else {
            // Some servers independently provide a final usage-only chunk. Never request that extension.
            return Ok(SseControl::Continue);
        };
        let delta = choice.get("delta").and_then(Value::as_object)
            .ok_or_else(|| protocol("The streamed completion is missing its delta."))?;
        reject_refusal(delta.get("refusal"))?;
        match delta.get("content") {
            None | Some(Value::Null) => {}
            Some(Value::String(content)) => {
                if stopped && !content.is_empty() {
                    return Err(protocol("The provider sent text after the completion stopped."));
                }
                publish(content, cancel, on_delta)?;
                text.push_str(content);
            }
            Some(_) => return Err(protocol("The completion text has an invalid format.")),
        }
        if choice.get("finish_reason").is_some_and(|reason| !reason.is_null()) {
            require_stop(choice.get("finish_reason"))?;
            stopped = true;
        }
        Ok(SseControl::Continue)
    }).await?;
    ensure_not_cancelled(cancel)?;
    Ok(ProviderOutput { text, usage })
}

fn responses_output(value: &Value) -> AppResult<ProviderOutput> {
    reject_error(value)?;
    match required_str(value, "status")? {
        "completed" => {}
        "failed" | "incomplete" | "cancelled" => return Err(incomplete_response(value)),
        _ => return Err(protocol("The provider did not return a completed response.")),
    }
    let text = responses_text(value, false)?;
    Ok(ProviderOutput { text, usage: parse_usage(value.get("usage"), "input_tokens", "output_tokens")? })
}

fn responses_text(value: &Value, allow_incomplete: bool) -> AppResult<String> {
    let output = value.get("output").and_then(Value::as_array)
        .ok_or_else(|| protocol("The response is missing its output blocks."))?;
    let mut text = String::new();
    for item in output {
        if required_str(item, "type")? != "message" {
            continue;
        }
        if item.get("status").is_some_and(|status| status.as_str() != Some("completed")
            && !(allow_incomplete && status.as_str() == Some("incomplete"))) {
            return Err(interrupted("A response message was not completed."));
        }
        let content = item.get("content").and_then(Value::as_array)
            .ok_or_else(|| protocol("The response message is missing its content blocks."))?;
        for part in content {
            match required_str(part, "type")? {
                "output_text" => text.push_str(required_str(part, "text")?),
                "refusal" => return Err(blocked()),
                _ => {}
            }
        }
    }
    if text.is_empty() {
        return Err(protocol("The provider returned no translated text."));
    }
    Ok(text)
}

fn first_choice(value: &Value) -> AppResult<Option<&Value>> {
    value.get("choices").and_then(Value::as_array).map(|choices| choices.first())
        .ok_or_else(|| protocol("The completion is missing its choices."))
}

fn require_stop(reason: Option<&Value>) -> AppResult<()> {
    match reason.and_then(Value::as_str) {
        Some("stop") => Ok(()),
        Some("length") => Err(interrupted("The provider reached its output token limit. Increase the limit before retrying.")),
        Some("content_filter") => Err(blocked()),
        Some("tool_calls" | "function_call") => Err(protocol("The provider returned a tool call instead of translated prose.")),
        None => Err(interrupted("The completion has no successful stop reason.")),
        Some(_) => Err(protocol("The provider returned an unsupported completion stop reason.")),
    }
}

fn incomplete_response(value: &Value) -> AppError {
    match value.pointer("/incomplete_details/reason").and_then(Value::as_str) {
        Some("content_filter") => blocked(),
        Some("max_output_tokens") => interrupted("The provider reached its output token limit. Increase the limit before retrying."),
        _ => interrupted("The provider did not complete the response. Partial text is preview-only."),
    }
}

fn reject_error(value: &Value) -> AppResult<()> {
    if value.get("error").is_some_and(|error| !error.is_null()) {
        Err(transport::provider_error(value))
    } else {
        Ok(())
    }
}

fn reject_refusal(value: Option<&Value>) -> AppResult<()> {
    match value {
        None | Some(Value::Null) => Ok(()),
        Some(Value::String(refusal)) if refusal.is_empty() => Ok(()),
        Some(Value::String(_)) => Err(blocked()),
        Some(_) => Err(protocol("The provider returned an invalid refusal field.")),
    }
}

fn parse_usage(value: Option<&Value>, input: &str, output: &str) -> AppResult<Option<AiUsage>> {
    let Some(value) = value.filter(|value| !value.is_null()) else { return Ok(None); };
    let object = value.as_object().ok_or_else(|| protocol("The provider returned invalid token usage."))?;
    let count = |field: &str| match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some)
            .ok_or_else(|| protocol("The provider returned invalid token usage.")),
    };
    Ok(Some(AiUsage { input_tokens: count(input)?, output_tokens: count(output)? }))
}

fn required_str<'a>(value: &'a Value, field: &str) -> AppResult<&'a str> {
    value.get(field).and_then(Value::as_str)
        .ok_or_else(|| protocol("The provider response is missing a required text field."))
}

fn event_type<'a>(event: &'a SseEvent, value: &'a Value) -> AppResult<&'a str> {
    let body_type = value.get("type").and_then(Value::as_str);
    let event_name = event.event.as_deref().filter(|name| *name != "message");
    if matches!((event_name, body_type), (Some(name), Some(kind)) if name != kind) {
        return Err(protocol("The provider event name contradicts its type."));
    }
    body_type.or(event_name).ok_or_else(|| protocol("The provider event is missing its type."))
}

fn auxiliary_event(event: &SseEvent, handled: &[&str]) -> bool {
    event.event.as_deref().is_some_and(|kind| kind != "message" && !handled.contains(&kind))
}

fn publish(text: &str, cancel: &CancellationToken, on_delta: &mut DeltaSink<'_>) -> AppResult<()> {
    ensure_not_cancelled(cancel)?;
    if !text.is_empty() { on_delta(text)?; }
    ensure_not_cancelled(cancel)
}

fn ensure_not_cancelled(cancel: &CancellationToken) -> AppResult<()> {
    if cancel.is_cancelled() {
        Err(transport::cancelled_error())
    } else {
        Ok(())
    }
}

fn protocol(message: &str) -> AppError { AppError::new("ai_protocol", message) }
fn interrupted(message: &str) -> AppError { AppError::new("ai_interrupted", message) }
fn blocked() -> AppError { AppError::new("ai_content_blocked", "The provider refused or blocked this content.") }
