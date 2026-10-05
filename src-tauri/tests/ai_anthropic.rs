use std::{
    collections::HashMap,
    io,
    sync::Arc,
    time::Duration,
};

use parking_lot::Mutex;
use quickertranslator_lib::ai::{
    AiProfile, AiProtocol, AiUsage, AuthMode, ProviderPrompt, TokenLimitField,
    anthropic, transport,
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct CapturedRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Value,
}

struct Reply {
    status: u16,
    content_type: &'static str,
    headers: Vec<(&'static str, String)>,
    fragments: Vec<Vec<u8>>,
    hold_open: bool,
}

impl Reply {
    fn json(body: Value) -> Self {
        Self::raw_json(&body.to_string())
    }

    fn raw_json(body: &str) -> Self {
        Self {
            status: 200,
            content_type: "application/json",
            headers: Vec::new(),
            fragments: vec![body.as_bytes().to_vec()],
            hold_open: false,
        }
    }

    fn sse(body: &str) -> Self {
        // Chunked HTTP deliberately fragments within UTF-8 scalars, CRLF pairs,
        // SSE event names and JSON strings rather than just between events.
        let mut fragments = Vec::new();
        let mut remaining = body.as_bytes();
        let mut widths = [1, 2, 3, 7, 5].into_iter().cycle();
        while !remaining.is_empty() {
            let width = widths.next().unwrap().min(remaining.len());
            fragments.push(remaining[..width].to_vec());
            remaining = &remaining[width..];
        }
        Self {
            status: 200,
            content_type: "text/event-stream",
            headers: Vec::new(),
            fragments,
            hold_open: false,
        }
    }
}

struct Fixture {
    root: String,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    body_started: CancellationToken,
    response_ended: CancellationToken,
    stop: CancellationToken,
    task: JoinHandle<()>,
}

impl Fixture {
    async fn start(reply: Reply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let root = format!("http://{}/proxy/team/v1/", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let stop = CancellationToken::new();
        let stopping = stop.clone();
        let body_started = CancellationToken::new();
        let sent = body_started.clone();
        let response_ended = CancellationToken::new();
        let ended = response_ended.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = stopping.cancelled() => break,
                    incoming = listener.accept() => {
                        let Ok((mut socket, _)) = incoming else { break; };
                        tokio::select! {
                            biased;
                            _ = stopping.cancelled() => break,
                            _ = serve(&mut socket, &reply, &captured, &sent) => { ended.cancel(); }
                        }
                    }
                }
            }
        });
        Self { root, requests, body_started, response_ended, stop, task }
    }

    fn profile(&self, stream: bool) -> AiProfile {
        AiProfile {
            id: "anthropic-fixture".into(),
            name: "Real loopback Messages".into(),
            protocol: AiProtocol::Anthropic,
            base_url: self.root.clone(),
            model: "editable-claude-model".into(),
            stream,
            max_output_tokens: 1024,
            auth_mode: AuthMode::ApiKey,
            allow_insecure_http: false,
            // The native Messages adapter must never send chat's selector.
            token_limit_field: Some(TokenLimitField::MaxCompletionTokens),
        }
    }

    fn request(&self) -> CapturedRequest {
        let requests = self.requests.lock();
        assert_eq!(requests.len(), 1, "one explicit action must make exactly one HTTP request");
        requests[0].clone()
    }

    fn request_count(&self) -> usize {
        self.requests.lock().len()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.cancel();
        self.task.abort();
    }
}

async fn serve(
    socket: &mut TcpStream,
    reply: &Reply,
    captured: &Arc<Mutex<Vec<CapturedRequest>>>,
    body_started: &CancellationToken,
) -> io::Result<()> {
    let mut received = Vec::new();
    let header_end = loop {
        let mut buffer = [0_u8; 1024];
        let read = socket.read(&mut buffer).await?;
        if read == 0 {
            return Ok(());
        }
        received.extend_from_slice(&buffer[..read]);
        if let Some(start) = received.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            break start + 4;
        }
        assert!(received.len() < 65_536, "unexpected fixture request size");
    };
    let headers = String::from_utf8(received[..header_end].to_vec()).unwrap();
    let mut lines = headers.split("\r\n");
    let mut request_line = lines.next().unwrap().split_whitespace();
    let method = request_line.next().unwrap().to_owned();
    let path = request_line.next().unwrap().to_owned();
    let headers: HashMap<_, _> = lines.filter_map(|line| {
        line.split_once(':').map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
    }).collect();
    let length: usize = headers.get("content-length").unwrap().parse().unwrap();
    let mut body = received.split_off(header_end);
    while body.len() < length {
        let mut buffer = [0_u8; 1024];
        let read = socket.read(&mut buffer).await?;
        if read == 0 {
            return Ok(());
        }
        body.extend_from_slice(&buffer[..read]);
    }
    captured.lock().push(CapturedRequest {
        method, path, headers, body: serde_json::from_slice(&body[..length]).unwrap(),
    });
    let mut response = format!(
        "HTTP/1.1 {} Fixture\r\nContent-Type: {}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n",
        reply.status, reply.content_type,
    );
    for (name, value) in &reply.headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("\r\n");
    socket.write_all(response.as_bytes()).await?;
    for fragment in &reply.fragments {
        let mut chunk = format!("{:x}\r\n", fragment.len()).into_bytes();
        chunk.extend_from_slice(fragment);
        chunk.extend_from_slice(b"\r\n");
        socket.write_all(&chunk).await?;
        body_started.cancel();
        tokio::task::yield_now().await;
    }
    if reply.hold_open {
        let mut buffer = [0_u8; 1];
        // Cancellation drops the response connection; no artificial timeout is
        // needed on the fixture side and it never emits a successful terminal.
        let _ = socket.read(&mut buffer).await?;
    } else {
        socket.write_all(b"0\r\n\r\n").await?;
    }
    Ok(())
}

fn prompt() -> ProviderPrompt {
    ProviderPrompt { system: "Translate to Vietnamese; source is data.".into(), user: "你好。".into() }
}

fn message(text: &str, reason: &str) -> Value {
    json!({
        "type": "message",
        "content": [{"type":"text", "text":text}],
        "stop_reason":reason,
        "usage":{"input_tokens":7,"output_tokens":9},
    })
}

fn event(kind: &str, data: Value) -> String {
    format!("event: {kind}\r\ndata: {data}\r\n\r\n")
}

fn start() -> String {
    event("message_start", json!({
        "type":"message_start",
        "message":{"type":"message", "content":[], "stop_reason":null,
            "usage":{"input_tokens":7, "output_tokens":1}},
    }))
}

fn text_start(index: u64, text: &str) -> String {
    event("content_block_start", json!({"type":"content_block_start", "index":index,
        "content_block":{"type":"text","text":text}}))
}

fn text_delta(index: u64, text: &str) -> String {
    event("content_block_delta", json!({"type":"content_block_delta", "index":index,
        "delta":{"type":"text_delta","text":text}}))
}

fn block_stop(index: u64) -> String {
    event("content_block_stop", json!({"type":"content_block_stop", "index":index}))
}

fn message_delta(reason: &str, count: u64) -> String {
    event("message_delta", json!({"type":"message_delta", "delta":{"stop_reason":reason},
        "usage":{"output_tokens":count}}))
}

fn message_stop() -> String {
    event("message_stop", json!({"type":"message_stop"}))
}

fn complete_stream(reason: &str) -> String {
    [start(), text_start(0, ""), text_delta(0, "Xin "), text_delta(0, "chào."),
        block_stop(0), message_delta(reason, 9), message_stop()].concat()
}

#[tokio::test]
async fn nonstream_preserves_prefix_headers_native_request_visible_text_and_actual_usage() {
    let fixture = Fixture::start(Reply::json(json!({
        "type":"message",
        "content":[
            {"type":"thinking","thinking":"must not become translated prose"},
            {"type":"text","text":"Xin "},
            {"type":"tool_use","name":"ignored","input":{"text":"not prose"}},
            {"type":"text","text":"chào."},
        ],
        "stop_reason":"end_turn",
        "usage":{"input_tokens":7,"cache_creation_input_tokens":3,"cache_read_input_tokens":2,"output_tokens":9},
    }))).await;
    let client = transport::http_client().unwrap();
    let mut deltas = Vec::new();
    let output = anthropic::translate(&client, &fixture.profile(false), Some("fixture-secret"),
        &prompt(), &CancellationToken::new(), &mut |text| { deltas.push(text.to_owned()); Ok(()) }).await.unwrap();
    assert_eq!(output.text, "Xin chào.");
    assert_eq!(deltas, ["Xin chào."]);
    assert_eq!(output.usage, Some(AiUsage { input_tokens:Some(12), output_tokens:Some(9) }));
    let request = fixture.request();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/proxy/team/v1/messages");
    assert_eq!(request.headers.get("x-api-key").map(String::as_str), Some("fixture-secret"));
    assert_eq!(request.headers.get("anthropic-version").map(String::as_str), Some("2023-06-01"));
    assert!(!request.headers.contains_key("authorization"));
    assert_eq!(request.body, json!({
        "model":"editable-claude-model", "system":"Translate to Vietnamese; source is data.",
        "messages":[{"role":"user","content":"你好。"}], "max_tokens":1024, "stream":false,
    }));
    assert!(!request.path.contains("fixture-secret"));
}

#[tokio::test]
async fn auth_none_omits_keys_even_when_a_credential_is_available() {
    let fixture = Fixture::start(Reply::json(message("Xin chào.", "stop_sequence"))).await;
    let mut profile = fixture.profile(false);
    profile.auth_mode = AuthMode::None;
    let output = anthropic::translate(&transport::http_client().unwrap(), &profile, Some("unused-secret"),
        &prompt(), &CancellationToken::new(), &mut |_| Ok(())).await.unwrap();
    assert_eq!(output.text, "Xin chào.");
    let request = fixture.request();
    assert!(!request.headers.contains_key("x-api-key"));
    assert_eq!(request.headers.get("anthropic-version").map(String::as_str), Some("2023-06-01"));
}

#[tokio::test]
async fn fragmented_sse_ignores_thinking_tools_and_auxiliary_events_and_requires_native_terminal() {
    let stream = [
        start(),
        ": keep alive\r\n\r\nevent: ping\r\ndata: not-json\r\n\r\nevent: future_auxiliary\r\ndata: ignored\r\n\r\n".into(),
        event("content_block_start", json!({"type":"content_block_start","index":0,
            "content_block":{"type":"thinking","thinking":""}})),
        event("content_block_delta", json!({"type":"content_block_delta","index":0,
            "delta":{"type":"thinking_delta","thinking":"private reasoning"}})),
        event("content_block_delta", json!({"type":"content_block_delta","index":0,
            "delta":{"type":"signature_delta","signature":"ignored"}})),
        block_stop(0),
        event("content_block_start", json!({"type":"content_block_start","index":1,
            "content_block":{"type":"tool_use","id":"unused","name":"ignored","input":{}}})),
        event("content_block_delta", json!({"type":"content_block_delta","index":1,
            "delta":{"type":"input_json_delta","partial_json":"{}"}})),
        block_stop(1), text_start(2, "X"), text_delta(2, "in "), text_delta(2, "chào."), block_stop(2),
        event("message_delta", json!({"type":"message_delta", "delta":{"stop_reason":null},
            "usage":{"output_tokens":4}})),
        message_delta("end_turn", 9), message_stop(),
        text_delta(2, "late output must never revive a finished response"),
    ].concat();
    let fixture = Fixture::start(Reply::sse(&stream)).await;
    let mut deltas = Vec::new();
    let output = anthropic::translate(&transport::http_client().unwrap(), &fixture.profile(true), Some("fixture-secret"),
        &prompt(), &CancellationToken::new(), &mut |text| { deltas.push(text.to_owned()); Ok(()) }).await.unwrap();
    assert_eq!(output.text, "Xin chào.");
    assert_eq!(deltas, ["X", "in ", "chào."]);
    assert_eq!(output.usage, Some(AiUsage { input_tokens:Some(7), output_tokens:Some(9) }));
    assert_eq!(fixture.request().body["stream"], true);
}

#[tokio::test]
async fn http_errors_are_classified_sanitized_and_never_retried() {
    for (status, code) in [(401, "ai_unauthorized"), (429, "ai_rate_limit")] {
        let mut reply = Reply::json(json!({"type":"error","error":{"type":"fixture_error",
            "message":"fixture-secret 你好。 confidential server body"}}));
        reply.status = status;
        let fixture = Fixture::start(reply).await;
        let error = anthropic::translate(&transport::http_client().unwrap(), &fixture.profile(false), Some("fixture-secret"),
            &prompt(), &CancellationToken::new(), &mut |_| Ok(())).await.err().unwrap();
        assert_eq!(error.code, code);
        assert!(!error.message.contains("fixture-secret"));
        assert!(!error.message.contains("你好"));
        assert!(!error.message.contains("confidential"));
        fixture.request();
    }
}

#[tokio::test]
async fn redirects_do_not_forward_credentials_or_fall_back() {
    let target = Fixture::start(Reply::json(message("must not be fetched", "end_turn"))).await;
    let mut reply = Reply::raw_json("{}");
    reply.status = 307;
    reply.headers.push(("Location", format!("{}messages", target.root)));
    let fixture = Fixture::start(reply).await;
    assert!(anthropic::translate(&transport::http_client().unwrap(), &fixture.profile(false), Some("fixture-secret"),
        &prompt(), &CancellationToken::new(), &mut |_| Ok(())).await.is_err());
    fixture.request();
    assert_eq!(target.request_count(), 0);
}

#[tokio::test]
async fn stop_reasons_report_refusal_truncation_and_tool_turns_without_losing_partial_preview() {
    for stream in [false, true] {
        for (reason, code) in [
            ("max_tokens", "ai_interrupted"),
            ("refusal", "ai_content_blocked"),
            ("tool_use", "ai_protocol"),
            ("pause_turn", "ai_protocol"),
            ("model_context_window_exceeded", "ai_context_too_long"),
        ] {
            let reply = if stream { Reply::sse(&complete_stream(reason)) } else { Reply::json(message("Xin chào.", reason)) };
            let fixture = Fixture::start(reply).await;
            let mut preview = String::new();
            let error = anthropic::translate(&transport::http_client().unwrap(), &fixture.profile(stream), Some("fixture-secret"),
                &prompt(), &CancellationToken::new(), &mut |text| { preview.push_str(text); Ok(()) }).await.err().unwrap();
            assert_eq!(error.code, code, "stream={stream}, reason={reason}");
            assert_eq!(preview, "Xin chào.");
            fixture.request();
        }
    }
}

#[tokio::test]
async fn missing_message_stop_is_interrupted_even_after_successful_stop_reason() {
    let stream = [start(), text_start(0, ""), text_delta(0, "Xin chào."), block_stop(0), message_delta("end_turn", 9)].concat();
    let fixture = Fixture::start(Reply::sse(&stream)).await;
    let mut preview = String::new();
    let error = anthropic::translate(&transport::http_client().unwrap(), &fixture.profile(true), Some("fixture-secret"),
        &prompt(), &CancellationToken::new(), &mut |text| { preview.push_str(text); Ok(()) }).await.err().unwrap();
    assert_eq!(error.code, "ai_interrupted");
    assert_eq!(preview, "Xin chào.");
    fixture.request();
}

#[tokio::test]
async fn native_error_events_are_sanitized_and_classified() {
    for (kind, code) in [
        ("authentication_error", "ai_unauthorized"),
        ("rate_limit_error", "ai_rate_limit"),
        ("permission_error", "ai_unauthorized"),
        ("refusal_error", "ai_content_blocked"),
    ] {
        let stream = [start(), event("error", json!({"type":"error", "error":{
            "type":kind, "message":"fixture-secret confidential prompt"}}))].concat();
        let fixture = Fixture::start(Reply::sse(&stream)).await;
        let error = anthropic::translate(&transport::http_client().unwrap(), &fixture.profile(true), Some("fixture-secret"),
            &prompt(), &CancellationToken::new(), &mut |_| Ok(())).await.err().unwrap();
        assert_eq!(error.code, code);
        assert!(!error.message.contains("fixture-secret"));
        assert!(!error.message.contains("confidential"));
        fixture.request();
    }
}

#[tokio::test]
async fn malformed_required_messages_or_events_never_succeed() {
    let cases = [
        Reply::raw_json("{not JSON"),
        Reply::json(json!({"type":"message","content":[{"type":"text","text":3}],"stop_reason":"end_turn"})),
        Reply::json(json!({"type":"message","content":[{"type":"text","text":"Xin chào."}]})),
        Reply::sse("event: message_start\ndata: {not JSON\n\n"),
        Reply::sse(&[start(), "event: content_block_start\ndata: {\ndata: \"type\": \"content_block_start\", \"index\": 0,\ndata: \"content_block\": {\"type\": \"text\", \"text\": 3}}\n\n".into()].concat()),
        Reply::sse(&[start(), text_start(0, ""), text_delta(1, "wrong index")].concat()),
        Reply::sse(&[start(), text_start(0, ""), text_delta(0, "Xin chào."), block_stop(0), message_stop()].concat()),
        Reply::sse(&[start(), text_start(0, ""), text_delta(0, "Xin chào."), message_delta("end_turn", 9), message_stop()].concat()),
    ];
    for reply in cases {
        let stream = reply.content_type == "text/event-stream";
        let fixture = Fixture::start(reply).await;
        let error = anthropic::translate(&transport::http_client().unwrap(), &fixture.profile(stream), Some("fixture-secret"),
            &prompt(), &CancellationToken::new(), &mut |_| Ok(())).await.err().unwrap();
        assert_eq!(error.code, "ai_protocol");
        fixture.request();
    }
}

#[tokio::test]
async fn immediate_cancel_and_missing_key_open_no_http_connection() {
    let fixture = Fixture::start(Reply::json(message("must not be fetched", "end_turn"))).await;
    let client = transport::http_client().unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let cancelled = anthropic::translate(&client, &fixture.profile(true), Some("fixture-secret"),
        &prompt(), &cancel, &mut |_| panic!("cancelled input emitted a delta")).await.err().unwrap();
    assert_eq!(cancelled.code, "ai_cancelled");
    let missing = anthropic::translate(&client, &fixture.profile(false), None,
        &prompt(), &CancellationToken::new(), &mut |_| panic!("missing key emitted a delta")).await.err().unwrap();
    assert_eq!(missing.code, "ai_missing_credentials");
    assert_eq!(fixture.request_count(), 0);
}

#[tokio::test]
async fn midstream_cancel_drops_connection_retains_preview_and_never_succeeds() {
    let mut reply = Reply::sse(&[start(), text_start(0, ""), text_delta(0, "Xin ")].concat());
    reply.hold_open = true;
    let fixture = Fixture::start(reply).await;
    let cancel = CancellationToken::new();
    let mut preview = String::new();
    let result = tokio::time::timeout(Duration::from_secs(5), anthropic::translate(
        &transport::http_client().unwrap(), &fixture.profile(true), Some("fixture-secret"),
        &prompt(), &cancel, &mut |text| { preview.push_str(text); cancel.cancel(); Ok(()) },
    )).await.expect("cancellation must not wait for idle timeout");
    assert_eq!(result.err().unwrap().code, "ai_cancelled");
    assert_eq!(preview, "Xin ");
    fixture.request();
    tokio::time::timeout(Duration::from_secs(5), fixture.response_ended.cancelled())
        .await.expect("cancellation must close the actual HTTP response connection");
}

#[tokio::test]
async fn nonstream_cancel_interrupts_a_body_read_without_publishing_partial_json() {
    let mut reply = Reply::raw_json("{\"type\":\"message\",\"content\":[");
    reply.hold_open = true;
    let fixture = Fixture::start(reply).await;
    let cancel = CancellationToken::new();
    let cancelling = cancel.clone();
    let body_started = fixture.body_started.clone();
    let trigger = tokio::spawn(async move {
        body_started.cancelled().await;
        cancelling.cancel();
    });
    let result = tokio::time::timeout(Duration::from_secs(5), anthropic::translate(
        &transport::http_client().unwrap(), &fixture.profile(false), Some("fixture-secret"),
        &prompt(), &cancel, &mut |_| panic!("partial JSON is not preview text"),
    )).await.expect("nonstream cancellation must not wait for idle timeout");
    trigger.await.unwrap();
    assert_eq!(result.err().unwrap().code, "ai_cancelled");
    fixture.request();
    tokio::time::timeout(Duration::from_secs(5), fixture.response_ended.cancelled())
        .await.expect("nonstream cancellation must close the actual HTTP connection");
}

#[tokio::test]
async fn lf_multiline_sse_preserves_unicode_and_reports_unknown_usage_as_unavailable() {
    let stream = [
        event("message_start", json!({"type":"message_start",
            "message":{"type":"message","content":[],"stop_reason":null}})),
        text_start(0, ""),
        "event: content_block_delta\ndata: {\ndata: \"type\":\"content_block_delta\",\"index\":0,\ndata: \"delta\":{\"type\":\"text_delta\",\"text\":\"Xin chào. 🙂\"}}\n\n".into(),
        block_stop(0),
        event("message_delta", json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}})),
        message_stop(),
    ].concat().replace("\r\n", "\n");
    let fixture = Fixture::start(Reply::sse(&stream)).await;
    let mut preview = String::new();
    let output = anthropic::translate(&transport::http_client().unwrap(), &fixture.profile(true), Some("fixture-secret"),
        &prompt(), &CancellationToken::new(), &mut |text| { preview.push_str(text); Ok(()) }).await.unwrap();
    assert_eq!(output.text, "Xin chào. 🙂");
    assert_eq!(preview, output.text);
    assert_eq!(output.usage, None);
    fixture.request();
}

#[tokio::test]
async fn whitespace_or_only_nontext_content_is_not_a_successful_translation() {
    for content in [
        json!([{"type":"thinking","thinking":"ignored"},{"type":"tool_use","input":{}}]),
        json!([{"type":"text","text":" \n\t"}]),
    ] {
        let fixture = Fixture::start(Reply::json(json!({
            "type":"message","content":content,"stop_reason":"end_turn",
        }))).await;
        let error = anthropic::translate(&transport::http_client().unwrap(), &fixture.profile(false), Some("fixture-secret"),
            &prompt(), &CancellationToken::new(), &mut |_| Ok(())).await.err().unwrap();
        assert_eq!(error.code, "ai_protocol");
        fixture.request();
    }
}
