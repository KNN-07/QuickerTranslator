use std::time::Duration;

use quickertranslator_lib::{
    ai::{
        AiProfile, AiProtocol, AiUsage, AuthMode, ProviderOutput, ProviderPrompt, TokenLimitField,
        openai::{translate_chat, translate_responses}, transport::http_client,
    },
    models::AppResult,
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

const KEY: &str = "fixture-private-key";
const PRIVATE_BODY: &str = "private-request-and-provider-message";

struct Received {
    head: String,
    body: Value,
}

impl Received {
    fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().filter_map(|line| line.split_once(':'))
            .find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.trim())
    }
}

struct Reply {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
    fragmented: bool,
    keep_open: bool,
    abrupt_disconnect: bool,
    headers: String,
    // A deliberately strict compatible server rejects unsupported chat token-limit extensions.
    chat_limit: Option<TokenLimitField>,
}

impl Reply {
    fn json(body: Value) -> Self {
        Self { status: 200, content_type: "application/json", body: serde_json::to_vec(&body).unwrap(),
            fragmented: false, keep_open: false, abrupt_disconnect: false, headers: String::new(), chat_limit: None }
    }

    fn sse(body: String) -> Self {
        Self { status: 200, content_type: "text/event-stream", body: body.into_bytes(),
            fragmented: true, keep_open: false, abrupt_disconnect: false, headers: String::new(), chat_limit: None }
    }
}

struct Fixture {
    base: String,
    received: oneshot::Receiver<Received>,
    body_started: oneshot::Receiver<()>,
    disconnected: oneshot::Receiver<()>,
    task: JoinHandle<()>,
}

impl Fixture {
    async fn start(mut reply: Reply) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let base = format!("http://{}/proxy/team/v1/", listener.local_addr().unwrap());
        let (received_tx, received) = oneshot::channel();
        let (started_tx, body_started) = oneshot::channel();
        let (disconnected_tx, disconnected) = oneshot::channel();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let header_end = loop {
                let mut block = [0; 1024];
                let count = stream.read(&mut block).await.unwrap();
                assert!(count > 0, "request disconnected before its headers");
                bytes.extend_from_slice(&block[..count]);
                if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    break index + 4;
                }
            };
            let head = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
            let length: usize = head.lines().filter_map(|line| line.split_once(':'))
                .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                .unwrap().1.trim().parse().unwrap();
            while bytes.len() - header_end < length {
                let mut block = [0; 1024];
                let count = stream.read(&mut block).await.unwrap();
                assert!(count > 0, "request disconnected before its JSON body");
                bytes.extend_from_slice(&block[..count]);
            }
            let body: Value = serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
            if let Some(limit) = reply.chat_limit {
                let accepted = match limit {
                    TokenLimitField::MaxTokens => body.get("max_tokens").and_then(Value::as_u64) == Some(73)
                        && body.get("max_completion_tokens").is_none(),
                    TokenLimitField::MaxCompletionTokens => body.get("max_completion_tokens").and_then(Value::as_u64) == Some(73)
                        && body.get("max_tokens").is_none(),
                    TokenLimitField::Omit => body.get("max_tokens").is_none()
                        && body.get("max_completion_tokens").is_none(),
                } && body.get("stream_options").is_none();
                if !accepted {
                    reply = Reply::json(json!({"error":{"code":"unsupported_parameter"}}));
                    reply.status = 400;
                }
            }
            received_tx.send(Received { head, body }).ok();
            let headers = format!(
                "HTTP/1.1 {} Fixture\r\nContent-Type: {}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n{}\r\n",
                reply.status, reply.content_type, reply.headers,
            );
            if stream.write_all(headers.as_bytes()).await.is_err() { return; }
            let chunk_size = if reply.fragmented { 1 } else { reply.body.len().max(1) };
            let mut started_tx = Some(started_tx);
            for part in reply.body.chunks(chunk_size) {
                let chunk_head = format!("{:x}\r\n", part.len());
                if stream.write_all(chunk_head.as_bytes()).await.is_err()
                    || stream.write_all(part).await.is_err()
                    || stream.write_all(b"\r\n").await.is_err() { return; }
                if let Some(sender) = started_tx.take() { sender.send(()).ok(); }
                tokio::task::yield_now().await;
            }
            if reply.keep_open {
                let mut byte = [0; 1];
                if matches!(stream.read(&mut byte).await, Ok(0) | Err(_)) {
                    disconnected_tx.send(()).ok();
                }
            } else if !reply.abrupt_disconnect {
                stream.write_all(b"0\r\n\r\n").await.ok();
            }
        });
        Self { base, received, body_started, disconnected, task }
    }

    async fn request(self) -> Received {
        let received = tokio::time::timeout(Duration::from_secs(5), self.received).await.unwrap().unwrap();
        self.task.abort();
        received
    }
}

fn profile(base: &str, protocol: AiProtocol, stream: bool) -> AiProfile {
    AiProfile {
        id: "loopback-openai".into(), name: "Local protocol fixture".into(), protocol,
        base_url: base.into(), model: "chosen-model".into(), stream, max_output_tokens: 73,
        auth_mode: AuthMode::ApiKey, allow_insecure_http: false, token_limit_field: None,
    }
}

async fn invoke(
    profile: &AiProfile,
    key: Option<&str>,
    cancel: &CancellationToken,
) -> (AppResult<ProviderOutput>, String) {
    let client = http_client().unwrap();
    let prompt = ProviderPrompt { system: "Return Vietnamese prose only.".into(), user: "你好。".into() };
    let mut preview = String::new();
    let mut delta = |text: &str| { preview.push_str(text); Ok(()) };
    let operation = async {
        match profile.protocol {
            AiProtocol::OpenaiResponses => translate_responses(&client, profile, key, &prompt, cancel, &mut delta).await,
            AiProtocol::OpenaiChat => translate_chat(&client, profile, key, &prompt, cancel, &mut delta).await,
            _ => unreachable!(),
        }
    };
    let result = tokio::time::timeout(Duration::from_secs(5), operation).await.expect("adapter hung");
    (result, preview)
}

fn responses(text: &str) -> Value {
    json!({"status":"completed", "output":[
        {"type":"reasoning", "summary":[]},
        {"type":"message", "status":"completed", "content":[{"type":"output_text", "text":text}]}
    ]})
}

fn chat(text: &str) -> Value {
    json!({"choices":[{"message":{"content":text,"refusal":null},"finish_reason":"stop"}]})
}

fn frame(kind: Option<&str>, value: Value) -> String {
    let name = kind.map(|kind| format!("event: {kind}\r\n")).unwrap_or_default();
    format!("{name}data: {value}\r\n\r\n")
}

fn response_delta(text: &str) -> String {
    frame(Some("response.output_text.delta"), json!({"type":"response.output_text.delta","delta":text}))
}

fn response_completed(value: Value) -> String {
    frame(Some("response.completed"), json!({"type":"response.completed","response":value}))
}

fn chat_delta(text: &str) -> String {
    frame(None, json!({"choices":[{"delta":{"content":text},"finish_reason":null}]}))
}

fn chat_stop() -> String {
    frame(None, json!({"choices":[{"delta":{},"finish_reason":"stop"}]}))
}

fn chat_stream() -> String {
    chat_delta("Xin ") + &chat_delta("chào.") + &chat_stop() + "data: [DONE]\r\n\r\n"
}

#[tokio::test]
async fn responses_fragmented_unicode_multiline_data_and_terminal_usage() {
    let mut completed = responses("Xin chào.");
    completed["usage"] = json!({"input_tokens":12,"output_tokens":4});
    let sse = ": keepalive\r\n\r\nevent: custom.ping\r\ndata: not JSON\r\n\r\n".to_owned()
        + "event: response.output_text.delta\r\ndata: {\"type\":\"response.output_text.delta\",\r\ndata: \"delta\":\"Xin \"}\r\n\r\n"
        + &response_delta("chào.") + &response_completed(completed);
    let mut reply = Reply::sse(sse);
    reply.keep_open = true;
    let fixture = Fixture::start(reply).await;
    let configured = profile(&fixture.base, AiProtocol::OpenaiResponses, true);
    let (result, preview) = invoke(&configured, Some(KEY), &CancellationToken::new()).await;
    let output = result.unwrap();
    assert_eq!(preview, "Xin chào.");
    assert_eq!(output.text, preview);
    assert_eq!(output.usage, Some(AiUsage { input_tokens: Some(12), output_tokens: Some(4) }));
    let request = fixture.request().await;
    assert!(request.head.starts_with("POST /proxy/team/v1/responses HTTP/1.1\r\n"));
    assert_eq!(request.header("authorization"), Some("Bearer fixture-private-key"));
    assert!(!request.head.lines().next().unwrap().contains(KEY));
}

#[tokio::test]
async fn responses_terminal_suffix_is_published_but_contradiction_is_rejected() {
    let fixture = Fixture::start(Reply::sse(response_delta("Xin ") + &response_completed(responses("Xin chào.")))).await;
    let (result, preview) = invoke(&profile(&fixture.base, AiProtocol::OpenaiResponses, true), Some(KEY), &CancellationToken::new()).await;
    assert_eq!(result.unwrap().text, "Xin chào.");
    assert_eq!(preview, "Xin chào.");
    fixture.request().await;

    let fixture = Fixture::start(Reply::sse(response_delta("Xin ") + &response_completed(responses("Khác.")))).await;
    let (result, preview) = invoke(&profile(&fixture.base, AiProtocol::OpenaiResponses, true), Some(KEY), &CancellationToken::new()).await;
    assert_eq!(result.err().unwrap().code, "ai_protocol");
    assert_eq!(preview, "Xin ");
    fixture.request().await;
}

#[tokio::test]
async fn chat_fragmented_stream_uses_first_choice_and_optional_usage() {
    let sse = ": ping\n\n".to_owned()
        + &frame(None, json!({"choices":[{"delta":{"role":"assistant","content":""},"finish_reason":null}]}))
        + &frame(None, json!({"choices":[
            {"delta":{"content":"Xin "},"finish_reason":null},
            {"delta":{"content":"not the selected choice"},"finish_reason":null}
        ]}))
        + &chat_delta("chào.") + &chat_stop()
        + &frame(None, json!({"choices":[],"usage":{"prompt_tokens":9,"completion_tokens":null}}))
        + "data: [DONE]\n\n";
    let mut reply = Reply::sse(sse);
    reply.keep_open = true;
    let fixture = Fixture::start(reply).await;
    let (result, preview) = invoke(&profile(&fixture.base, AiProtocol::OpenaiChat, true), Some(KEY), &CancellationToken::new()).await;
    let output = result.unwrap();
    assert_eq!(preview, "Xin chào.");
    assert_eq!(output.text, preview);
    assert_eq!(output.usage, Some(AiUsage { input_tokens: Some(9), output_tokens: None }));
    let request = fixture.request().await;
    assert!(request.head.starts_with("POST /proxy/team/v1/chat/completions HTTP/1.1\r\n"));
    assert_eq!(request.header("authorization"), Some("Bearer fixture-private-key"));
}

#[tokio::test]
async fn nonstream_outputs_are_validated_before_preview_and_auth_none_sends_no_key() {
    let mut response = responses("unused");
    response["output"][1]["content"] = json!([
        {"type":"output_text","text":"Xin "}, {"type":"output_text","text":"chào."}
    ]);
    response["usage"] = json!({"input_tokens":3,"output_tokens":4});
    let mut completion = chat("Xin chào.");
    completion["usage"] = json!({"prompt_tokens":3,"completion_tokens":4});
    for (protocol, body) in [(AiProtocol::OpenaiResponses, response), (AiProtocol::OpenaiChat, completion)] {
        let fixture = Fixture::start(Reply::json(body)).await;
        let mut configured = profile(&fixture.base, protocol, false);
        configured.auth_mode = AuthMode::None;
        let (result, preview) = invoke(&configured, Some(KEY), &CancellationToken::new()).await;
        let output = result.unwrap();
        assert_eq!(output.text, "Xin chào.");
        assert_eq!(preview, output.text);
        assert_eq!(output.usage, Some(AiUsage { input_tokens: Some(3), output_tokens: Some(4) }));
        let request = fixture.request().await;
        assert!(request.header("authorization").is_none());
        assert_eq!(request.body["stream"], false);
    }
}

#[tokio::test]
async fn strict_chat_servers_support_each_explicit_token_limit_mode_without_usage_extensions() {
    for (field, configured_field) in [
        (TokenLimitField::MaxTokens, None),
        (TokenLimitField::MaxCompletionTokens, Some(TokenLimitField::MaxCompletionTokens)),
        (TokenLimitField::Omit, Some(TokenLimitField::Omit)),
    ] {
        for stream in [false, true] {
            let mut reply = if stream { Reply::sse(chat_stream()) } else { Reply::json(chat("Xin chào.")) };
            reply.chat_limit = Some(field);
            let fixture = Fixture::start(reply).await;
            let mut configured = profile(&fixture.base, AiProtocol::OpenaiChat, stream);
            configured.token_limit_field = configured_field;
            let (result, _) = invoke(&configured, Some(KEY), &CancellationToken::new()).await;
            assert_eq!(result.unwrap().text, "Xin chào.");
            fixture.request().await;
        }
    }
}

#[tokio::test]
async fn nonstream_truncation_keeps_valid_visible_prose_without_success() {
    let mut response = responses("Xin chào.🙂");
    response["status"] = json!("incomplete");
    response["incomplete_details"] = json!({"reason":"max_output_tokens"});
    response["output"][1]["status"] = json!("incomplete");
    let mut completion = chat("Xin chào.🙂");
    completion["choices"][0]["finish_reason"] = json!("length");
    for (protocol, body) in [(AiProtocol::OpenaiResponses, response), (AiProtocol::OpenaiChat, completion)] {
        let fixture = Fixture::start(Reply::json(body)).await;
        let (result, preview) = invoke(&profile(&fixture.base, protocol, false), Some(KEY), &CancellationToken::new()).await;
        assert_eq!(result.err().unwrap().code, "ai_interrupted");
        assert_eq!(preview, "Xin chào.🙂");
        fixture.request().await;
    }
}

#[tokio::test]
async fn empty_sse_event_names_keep_default_protocol_messages() {
    let chat_frames = frame(Some(""), json!({"choices":[{"delta":{"content":"Xin chào."},"finish_reason":null}]}))
        + &frame(Some(""), json!({"choices":[{"delta":{},"finish_reason":"stop"}]}))
        + "event:\ndata: [DONE]\n\n";
    let response_frames = frame(Some(""), json!({"type":"response.output_text.delta","delta":"Xin chào."}))
        + &frame(Some(""), json!({"type":"response.completed","response":responses("Xin chào.")}));
    for (protocol, frames) in [(AiProtocol::OpenaiChat, chat_frames), (AiProtocol::OpenaiResponses, response_frames)] {
        let fixture = Fixture::start(Reply::sse(frames)).await;
        let (result, preview) = invoke(&profile(&fixture.base, protocol, true), Some(KEY), &CancellationToken::new()).await;
        assert_eq!(result.unwrap().text, "Xin chào.");
        assert_eq!(preview, "Xin chào.");
        fixture.request().await;
    }
}

#[tokio::test]
async fn responses_nonstream_refusal_failed_incomplete_and_malformed_results_are_not_appliable() {
    let cases = [
        (json!({"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}), "ai_interrupted"),
        (json!({"status":"incomplete","incomplete_details":{"reason":"content_filter"}}), "ai_content_blocked"),
        (json!({"status":"failed","error":{"code":"context_length_exceeded","message":PRIVATE_BODY}}), "ai_context_too_long"),
        (json!({"status":"completed","output":[{"type":"message","content":[
            {"type":"output_text","text":"partial"},{"type":"refusal","refusal":PRIVATE_BODY}
        ]}]}), "ai_content_blocked"),
        (json!({"status":"in_progress","output":[]}), "ai_protocol"),
        (json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":123}]}]}), "ai_protocol"),
    ];
    for (body, code) in cases {
        let fixture = Fixture::start(Reply::json(body)).await;
        let (result, preview) = invoke(&profile(&fixture.base, AiProtocol::OpenaiResponses, false), Some(KEY), &CancellationToken::new()).await;
        let error = result.err().unwrap();
        assert_eq!(error.code, code);
        assert!(!error.message.contains(PRIVATE_BODY));
        assert!(preview.is_empty());
        fixture.request().await;
    }
}

#[tokio::test]
async fn responses_stream_refusal_failure_malformed_data_and_eof_preserve_only_partial_preview() {
    let cases = [
        (String::new(), "ai_interrupted"),
        (frame(Some("response.refusal.delta"), json!({"type":"response.refusal.delta","delta":PRIVATE_BODY})), "ai_content_blocked"),
        (frame(Some("response.failed"), json!({"type":"response.failed","response":{"status":"failed","error":{"code":"context_length_exceeded","message":PRIVATE_BODY}}})), "ai_context_too_long"),
        (frame(Some("response.incomplete"), json!({"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}})), "ai_interrupted"),
        (frame(Some("response.output_text.delta"), json!({"type":"response.output_text.delta","delta":123})), "ai_protocol"),
        ("event: response.output_text.delta\ndata: invalid JSON\n\n".to_owned(), "ai_protocol"),
        (response_completed(json!({"status":"completed"})), "ai_protocol"),
        (frame(Some("response.output_text.delta"), json!({"type":"response.completed","response":responses("Xin chào.")})), "ai_protocol"),
    ];
    for (terminal, code) in cases {
        let fixture = Fixture::start(Reply::sse(response_delta("Xin ") + &terminal)).await;
        let (result, preview) = invoke(&profile(&fixture.base, AiProtocol::OpenaiResponses, true), Some(KEY), &CancellationToken::new()).await;
        let error = result.err().unwrap();
        assert_eq!(error.code, code);
        assert_eq!(preview, "Xin ");
        assert!(!error.message.contains(PRIVATE_BODY));
        fixture.request().await;
    }
}

#[tokio::test]
async fn chat_nonstream_errors_preserve_only_valid_visible_preview() {
    let cases = [
        (json!({"choices":[{"message":{"content":"partial"},"finish_reason":"length"}]}), "ai_interrupted", "partial"),
        (json!({"choices":[{"message":{"content":"partial"},"finish_reason":"content_filter"}]}), "ai_content_blocked", ""),
        (json!({"choices":[{"message":{"content":null,"tool_calls":[{}]},"finish_reason":"tool_calls"}]}), "ai_protocol", ""),
        (json!({"choices":[{"message":{"content":"partial"},"finish_reason":null}]}), "ai_interrupted", "partial"),
        (json!({"choices":[{"message":{"content":"partial","refusal":PRIVATE_BODY},"finish_reason":"stop"}]}), "ai_content_blocked", ""),
        (json!({"choices":[{"message":{"content":123},"finish_reason":"stop"}]}), "ai_protocol", ""),
    ];
    for (body, code, expected_preview) in cases {
        let fixture = Fixture::start(Reply::json(body)).await;
        let (result, preview) = invoke(&profile(&fixture.base, AiProtocol::OpenaiChat, false), Some(KEY), &CancellationToken::new()).await;
        let error = result.err().unwrap();
        assert_eq!(error.code, code);
        assert!(!error.message.contains(PRIVATE_BODY));
        assert_eq!(preview, expected_preview);
        fixture.request().await;
    }
}

#[tokio::test]
async fn chat_requires_successful_finish_and_done_and_keeps_failures_preview_only() {
    let cases = [
        (chat_stop(), "ai_interrupted"),
        ("data: [DONE]\n\n".to_owned(), "ai_interrupted"),
        (frame(None, json!({"choices":[{"delta":{},"finish_reason":"length"}]})), "ai_interrupted"),
        (frame(None, json!({"choices":[{"delta":{},"finish_reason":"content_filter"}]})), "ai_content_blocked"),
        (frame(None, json!({"choices":[{"delta":{"refusal":PRIVATE_BODY},"finish_reason":null}]})), "ai_content_blocked"),
        (frame(None, json!({"error":{"code":"context_length_exceeded","message":PRIVATE_BODY}})), "ai_context_too_long"),
        ("data: invalid JSON\n\n".to_owned(), "ai_protocol"),
        (chat_stop() + &chat_delta("late"), "ai_protocol"),
    ];
    for (terminal, code) in cases {
        let fixture = Fixture::start(Reply::sse(chat_delta("Xin ") + &terminal)).await;
        let (result, preview) = invoke(&profile(&fixture.base, AiProtocol::OpenaiChat, true), Some(KEY), &CancellationToken::new()).await;
        let error = result.err().unwrap();
        assert_eq!(error.code, code);
        assert_eq!(preview, "Xin ");
        assert!(!error.message.contains(PRIVATE_BODY));
        fixture.request().await;
    }
}

#[tokio::test]
async fn http_unauthorized_and_rate_limit_errors_never_echo_credentials_or_provider_bodies() {
    for protocol in [AiProtocol::OpenaiResponses, AiProtocol::OpenaiChat] {
        for (status, code) in [(401, "ai_unauthorized"), (429, "ai_rate_limit")] {
            let mut reply = Reply::json(json!({"error":{"message":format!("{PRIVATE_BODY} {KEY}")}}));
            reply.status = status;
            let fixture = Fixture::start(reply).await;
            let (result, preview) = invoke(&profile(&fixture.base, protocol, true), Some(KEY), &CancellationToken::new()).await;
            let error = result.err().unwrap();
            assert_eq!(error.code, code);
            assert!(!error.message.contains(PRIVATE_BODY));
            assert!(!error.message.contains(KEY));
            assert!(preview.is_empty());
            fixture.request().await;
        }
    }
}

#[tokio::test]
async fn missing_credentials_and_pre_cancelled_requests_do_not_open_http() {
    for protocol in [AiProtocol::OpenaiResponses, AiProtocol::OpenaiChat] {
        for cancelled in [false, true] {
            let fixture = Fixture::start(Reply::json(chat("unused"))).await;
            let mut configured = profile(&fixture.base, protocol, false);
            let cancel = CancellationToken::new();
            if cancelled { configured.auth_mode = AuthMode::None; cancel.cancel(); }
            let (result, preview) = invoke(&configured, None, &cancel).await;
            assert_eq!(result.err().unwrap().code, if cancelled { "ai_cancelled" } else { "ai_missing_credentials" });
            assert!(preview.is_empty());
            assert!(tokio::time::timeout(Duration::from_millis(100), fixture.received).await.is_err());
            fixture.task.abort();
        }
    }
}

#[tokio::test]
async fn midstream_cancellation_preserves_preview_and_drops_the_response() {
    for protocol in [AiProtocol::OpenaiResponses, AiProtocol::OpenaiChat] {
        let mut reply = Reply::sse(match protocol {
            AiProtocol::OpenaiResponses => response_delta("Xin "),
            AiProtocol::OpenaiChat => chat_delta("Xin "),
            _ => unreachable!(),
        });
        reply.keep_open = true;
        let fixture = Fixture::start(reply).await;
        let configured = profile(&fixture.base, protocol, true);
        let cancel = CancellationToken::new();
        let client = http_client().unwrap();
        let prompt = ProviderPrompt { system: "Translate to Vietnamese.".into(), user: "你好".into() };
        let mut preview = String::new();
        let (seen_tx, seen_rx) = oneshot::channel();
        let mut seen_tx = Some(seen_tx);
        let cancel_read = cancel.clone();
        let cancel_task = tokio::spawn(async move { seen_rx.await.unwrap(); cancel_read.cancel(); });
        let mut delta = |text: &str| {
            preview.push_str(text);
            if let Some(sender) = seen_tx.take() { sender.send(()).ok(); }
            Ok(())
        };
        let operation = async {
            match protocol {
                AiProtocol::OpenaiResponses => translate_responses(&client, &configured, Some(KEY), &prompt, &cancel, &mut delta).await,
                AiProtocol::OpenaiChat => translate_chat(&client, &configured, Some(KEY), &prompt, &cancel, &mut delta).await,
                _ => unreachable!(),
            }
        };
        let result = tokio::time::timeout(Duration::from_secs(5), operation).await.unwrap();
        assert_eq!(result.err().unwrap().code, "ai_cancelled");
        assert_eq!(preview, "Xin ");
        cancel_task.await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), fixture.disconnected).await.unwrap().unwrap();
        fixture.received.await.unwrap();
        fixture.task.abort();
    }
}

#[tokio::test]
async fn nonstream_body_read_is_cancellable_without_a_total_request_timeout() {
    for protocol in [AiProtocol::OpenaiResponses, AiProtocol::OpenaiChat] {
        let mut reply = Reply::json(json!({"partial":"body"}));
        reply.body = b"{\"partial\":".to_vec();
        reply.keep_open = true;
        let fixture = Fixture::start(reply).await;
        let configured = profile(&fixture.base, protocol, false);
        let cancel = CancellationToken::new();
        let cancel_read = cancel.clone();
        let task = tokio::spawn(async move { fixture.body_started.await.unwrap(); cancel_read.cancel(); });
        let (result, preview) = invoke(&configured, Some(KEY), &cancel).await;
        assert_eq!(result.err().unwrap().code, "ai_cancelled");
        assert!(preview.is_empty());
        task.await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), fixture.disconnected).await.unwrap().unwrap();
        fixture.received.await.unwrap();
        fixture.task.abort();
    }
}

#[tokio::test]
async fn redirects_are_rejected_before_a_second_host_receives_credentials() {
    for protocol in [AiProtocol::OpenaiResponses, AiProtocol::OpenaiChat] {
        let target = Fixture::start(Reply::json(match protocol {
            AiProtocol::OpenaiResponses => responses("Xin chào."),
            AiProtocol::OpenaiChat => chat("Xin chào."),
            _ => unreachable!(),
        })).await;
        let mut redirect = Reply::json(json!({}));
        redirect.status = 307;
        redirect.headers = format!("Location: {}secret-target\r\n", target.base);
        let origin = Fixture::start(redirect).await;
        let (result, preview) = invoke(&profile(&origin.base, protocol, false), Some(KEY), &CancellationToken::new()).await;
        assert!(result.is_err());
        assert!(preview.is_empty());
        origin.request().await;
        assert!(tokio::time::timeout(Duration::from_millis(100), target.received).await.is_err());
        target.task.abort();
    }
}

#[tokio::test]
async fn streaming_auth_none_never_sends_a_supplied_credential() {
    for protocol in [AiProtocol::OpenaiResponses, AiProtocol::OpenaiChat] {
        let stream = match protocol {
            AiProtocol::OpenaiResponses => response_delta("Xin chào.") + &response_completed(responses("Xin chào.")),
            AiProtocol::OpenaiChat => chat_stream(),
            _ => unreachable!(),
        };
        let fixture = Fixture::start(Reply::sse(stream)).await;
        let mut configured = profile(&fixture.base, protocol, true);
        configured.auth_mode = AuthMode::None;
        let (result, preview) = invoke(&configured, Some(KEY), &CancellationToken::new()).await;
        assert_eq!(result.unwrap().text, "Xin chào.");
        assert_eq!(preview, "Xin chào.");
        assert!(fixture.request().await.header("authorization").is_none());
    }
}

#[tokio::test]
async fn nonstream_invalid_json_does_not_publish_any_preview() {
    for protocol in [AiProtocol::OpenaiResponses, AiProtocol::OpenaiChat] {
        let mut reply = Reply::json(json!({}));
        reply.body = b"{invalid JSON".to_vec();
        let fixture = Fixture::start(reply).await;
        let (result, preview) = invoke(&profile(&fixture.base, protocol, false), Some(KEY), &CancellationToken::new()).await;
        assert_eq!(result.err().unwrap().code, "ai_protocol");
        assert!(preview.is_empty());
        fixture.request().await;
    }
}

#[tokio::test]
async fn abrupt_http_disconnect_after_a_complete_delta_is_never_successful() {
    for protocol in [AiProtocol::OpenaiResponses, AiProtocol::OpenaiChat] {
        let data = match protocol {
            AiProtocol::OpenaiResponses => response_delta("Xin "),
            AiProtocol::OpenaiChat => chat_delta("Xin "),
            _ => unreachable!(),
        };
        let mut reply = Reply::sse(data);
        reply.abrupt_disconnect = true;
        let fixture = Fixture::start(reply).await;
        let (result, preview) = invoke(&profile(&fixture.base, protocol, true), Some(KEY), &CancellationToken::new()).await;
        assert_eq!(result.err().unwrap().code, "ai_interrupted");
        assert_eq!(preview, "Xin ");
        fixture.request().await;
    }
}
