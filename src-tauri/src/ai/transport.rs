use std::{net::IpAddr, time::Duration};

use reqwest::{Client, RequestBuilder, Response, StatusCode, Url, header::HeaderValue};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::{AiProfile, AiProtocol, sse::{SseControl, SseDecoder, SseEvent}};
use crate::models::{AppError, AppResult};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
pub const IDLE_READ_TIMEOUT: Duration = Duration::from_secs(120);
const ERROR_BODY_LIMIT: usize = 64 * 1024;

/// One shared client: verified rustls TLS, no redirects, retries or total deadline.
pub fn http_client() -> AppResult<Client> {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(IDLE_READ_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .build()
        .map_err(|_| AppError::new("ai_connection", "The secure HTTP client could not be initialized."))
}

/// Canonical API root, never a final method URL. Credentials cannot be in the URL.
pub fn validate_endpoint(base_url: &str, allow_insecure_http: bool) -> AppResult<Url> {
    let invalid = || AppError::new(
        "ai_invalid_endpoint",
        "Use an HTTP(S) API root without credentials, a query string or a fragment.",
    );
    let mut url = Url::parse(base_url).map_err(|_| invalid())?;
    let authority_has_userinfo = base_url.split_once(':')
        .map(|(_, rest)| rest.trim_start_matches(['/', '\\'])
            .split(['/', '\\', '?', '#']).next().unwrap_or_default().contains('@'))
        .unwrap_or(false);
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none()
        || !url.username().is_empty() || url.password().is_some() || authority_has_userinfo
        || url.query().is_some() || url.fragment().is_some()
    {
        return Err(invalid());
    }
    if url.scheme() == "http" && !allow_insecure_http && !is_loopback(&url) {
        return Err(AppError::new(
            "ai_invalid_endpoint",
            "Plain HTTP outside loopback requires explicit insecure-transport permission for this profile.",
        ));
    }
    if url.path() != "/" && url.path().ends_with('/') {
        let path = url.path().trim_end_matches('/').to_owned();
        url.set_path(&path);
    }
    Ok(url)
}

fn is_loopback(url: &Url) -> bool {
    let Some(host) = url.host_str() else { return false };
    host.trim_end_matches('.').eq_ignore_ascii_case("localhost")
        || host.trim_matches(['[', ']']).parse::<IpAddr>().is_ok_and(|ip| {
            ip.is_loopback() || match ip {
                IpAddr::V6(ip) => ip.to_ipv4_mapped().is_some_and(|ip| ip.is_loopback()),
                IpAddr::V4(_) => false,
            }
        })
}

/// Append native routes as path segments so custom/version prefixes are retained.
pub fn request_url(profile: &AiProfile) -> AppResult<Url> {
    let mut url = validate_endpoint(&profile.base_url, profile.allow_insecure_http)?;
    {
        let mut path = url.path_segments_mut().map_err(|_| {
            AppError::new("ai_invalid_endpoint", "The API root cannot contain this URL form.")
        })?;
        path.pop_if_empty();
        match profile.protocol {
            AiProtocol::OpenaiResponses => { path.push("responses"); }
            AiProtocol::OpenaiChat => { path.push("chat").push("completions"); }
            AiProtocol::Anthropic => { path.push("messages"); }
            AiProtocol::Gemini => {
                let model = profile.model.strip_prefix("models/").unwrap_or(&profile.model);
                if model.is_empty() {
                    return Err(AppError::new("ai_protocol", "Enter a Gemini model ID before sending a request."));
                }
                let method = if profile.stream { "streamGenerateContent" } else { "generateContent" };
                path.push("models").push(&format!("{model}:{method}"));
            }
        }
    }
    if profile.protocol == AiProtocol::Gemini && profile.stream {
        url.query_pairs_mut().append_pair("alt", "sse");
    }
    Ok(url)
}

/// Literal API-key header. Saved credentials are never returned to the frontend.
pub fn credential_header(credential: Option<&str>) -> AppResult<HeaderValue> {
    let credential = credential.filter(|value| !value.trim().is_empty()).ok_or_else(|| {
        AppError::new("ai_missing_credentials", "Enter an API key for this profile and endpoint.")
    })?;
    let mut header = HeaderValue::from_str(credential).map_err(|_| {
        AppError::new("ai_missing_credentials", "The API key contains invalid header characters; re-enter it.")
    })?;
    header.set_sensitive(true);
    Ok(header)
}

pub fn bearer_header(credential: Option<&str>) -> AppResult<HeaderValue> {
    // Validate the key before adding the prefix so an absent key cannot become 'Bearer '.
    let key = credential_header(credential)?;
    let key = key.to_str().map_err(|_| {
        AppError::new("ai_missing_credentials", "The API key contains invalid header characters; re-enter it.")
    })?;
    credential_header(Some(&format!("Bearer {key}")))
}

pub fn cancelled_error() -> AppError {
    AppError::new("ai_cancelled", "The AI request was cancelled.")
}

/// Send exactly once. Dropping the selected-out send future/response cancels HTTP work.
pub async fn send(request: RequestBuilder, cancel: &CancellationToken) -> AppResult<Response> {
    let mut response = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(cancelled_error()),
        result = request.send() => result.map_err(connection_error)?,
    };
    if cancel.is_cancelled() {
        return Err(cancelled_error());
    }
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    // Read only a bounded error body for classification; never return its contents.
    let mut body = Vec::new();
    while body.len() < ERROR_BODY_LIMIT {
        match next_chunk(&mut response, cancel, IDLE_READ_TIMEOUT).await {
            Ok(Some(chunk)) => {
                let remaining = ERROR_BODY_LIMIT - body.len();
                body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
            }
            Ok(None) => break,
            Err(error) if error.code == "ai_cancelled" => return Err(error),
            Err(_) => break,
        }
    }
    let value = serde_json::from_slice::<Value>(&body).ok();
    Err(http_error(status, value.as_ref()))
}

/// Read an ordinary JSON response with cancellation and the same per-read idle limit.
pub async fn read_json<T: DeserializeOwned>(mut response: Response, cancel: &CancellationToken) -> AppResult<T> {
    let mut body = Vec::new();
    while let Some(chunk) = next_chunk(&mut response, cancel, IDLE_READ_TIMEOUT).await? {
        body.extend_from_slice(&chunk);
    }
    if cancel.is_cancelled() {
        return Err(cancelled_error());
    }
    serde_json::from_slice(&body).map_err(|_| {
        AppError::new("ai_protocol", "The provider returned malformed JSON.")
    })
}

/// Adapters ignore auxiliary events and return Done only at their native success terminal.
/// EOF is never success by itself, including after one or more complete delta frames.
pub async fn stream_sse<F>(mut response: Response, cancel: &CancellationToken, mut on_event: F) -> AppResult<()>
where
    F: FnMut(SseEvent) -> AppResult<SseControl> + Send,
{
    let mut decoder = SseDecoder::new();
    while let Some(chunk) = next_chunk(&mut response, cancel, IDLE_READ_TIMEOUT).await? {
        let control = decoder.feed(&chunk, &mut |event| {
            if cancel.is_cancelled() {
                return Err(cancelled_error());
            }
            on_event(event)
        })?;
        if cancel.is_cancelled() {
            return Err(cancelled_error());
        }
        if control == SseControl::Done {
            return Ok(());
        }
    }
    decoder.finish()?;
    Err(AppError::new("ai_interrupted", "The provider stream ended before its successful terminal event."))
}

async fn next_chunk(response: &mut Response, cancel: &CancellationToken, idle_timeout: Duration) -> AppResult<Option<bytes::Bytes>> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(cancelled_error()),
        result = tokio::time::timeout(idle_timeout, response.chunk()) => match result {
            Ok(Ok(chunk)) => Ok(chunk),
            Ok(Err(_)) => Err(AppError::new("ai_interrupted", "The provider response was interrupted or its connection timed out.")),
            Err(_) => Err(AppError::new("ai_interrupted", "The provider stopped sending data for too long.")),
        },
    }
}

fn connection_error(error: reqwest::Error) -> AppError {
    if error.is_builder() {
        AppError::new("ai_protocol", "The provider request could not be constructed.")
    } else {
        // reqwest Display includes URLs and may include secret-bearing data; never use it.
        AppError::new("ai_connection", "Could not connect securely to the provider, or the connection timed out. Check the endpoint, network and TLS certificate.")
    }
}

fn http_error(status: StatusCode, value: Option<&Value>) -> AppError {
    match status.as_u16() {
        401 | 403 => AppError::new("ai_unauthorized", "The provider rejected authentication or access to this model."),
        429 => AppError::new("ai_rate_limit", "The provider rate limit or quota was reached. Retry only when you choose to."),
        413 => AppError::new("ai_context_too_long", "The selected input exceeds the provider's context limit."),
        _ => {
            if let Some(value) = value {
                let error = provider_error(value);
                if error.code != "ai_protocol" {
                    return error;
                }
            }
            if status.is_redirection() {
                AppError::new("ai_protocol", "The provider attempted a redirect. Use the intended API root directly; redirects are disabled to protect credentials.")
            } else if status.is_server_error() {
                AppError::new("ai_connection", format!("The provider is unavailable (HTTP {}). No automatic retry was sent.", status.as_u16()))
            } else {
                AppError::new("ai_protocol", format!("The provider rejected the request (HTTP {}). Check the model, protocol and endpoint settings.", status.as_u16()))
            }
        }
    }
}

/// Inspect only known native error fields and emit fixed, sanitized explanations.
/// Accepts either a full {error: ...} envelope or its inner error object.
pub fn provider_error(value: &Value) -> AppError {
    let value = value.get("error").unwrap_or(value);
    let numeric_code = value.get("code").and_then(Value::as_u64);
    if matches!(numeric_code, Some(401 | 403)) {
        return AppError::new("ai_unauthorized", "The provider rejected authentication or model access.");
    }
    if numeric_code == Some(429) {
        return AppError::new("ai_rate_limit", "The provider rate limit or quota was reached.");
    }
    if numeric_code == Some(413) {
        return AppError::new("ai_context_too_long", "The selected input exceeds the provider's context limit.");
    }
    let fields = ["code", "type", "status", "reason", "message"];
    let texts = value.as_str().into_iter()
        .chain(fields.into_iter().filter_map(|field| value.get(field).and_then(Value::as_str)));
    for text in texts {
        let text = text.to_ascii_lowercase();
        if ["context_length", "context length", "context window", "context_limit", "context too long", "too many tokens", "input token count", "input too long", "input is too long", "prompt is too long", "request_too_large", "token limit exceeded"].iter().any(|needle| text.contains(needle)) {
            return AppError::new("ai_context_too_long", "The selected input exceeds the provider's context limit.");
        }
        if ["content_filter", "content filter", "content_block", "content blocked", "safety", "refusal", "blocked", "prohibited_content"].iter().any(|needle| text.contains(needle)) {
            return AppError::new("ai_content_blocked", "The provider blocked or refused this content.");
        }
        if ["invalid_api_key", "authentication", "unauthenticated", "unauthorized", "permission_denied", "permission_error"].iter().any(|needle| text.contains(needle)) {
            return AppError::new("ai_unauthorized", "The provider rejected authentication or model access.");
        }
        if ["rate_limit", "rate limit", "quota", "resource_exhausted"].iter().any(|needle| text.contains(needle)) {
            return AppError::new("ai_rate_limit", "The provider rate limit or quota was reached.");
        }
        if ["overloaded_error", "server_error", "service_unavailable", "internal_error", "unavailable"].iter().any(|needle| text.contains(needle)) {
            return AppError::new("ai_connection", "The provider is unavailable. No automatic retry was sent.");
        }
    }
    AppError::new("ai_protocol", "The provider reported an error. Check the model, protocol and request settings.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{AuthMode, TokenLimitField, sse::json_data};
    use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::{TcpListener, TcpStream}, task::JoinHandle};

    fn profile(base_url: &str, protocol: AiProtocol, stream: bool) -> AiProfile {
        AiProfile {
            id: "transport-fixture".into(),
            name: "Local transport fixture".into(),
            protocol,
            base_url: base_url.into(),
            model: "fixture-model".into(),
            stream,
            max_output_tokens: 4096,
            auth_mode: AuthMode::ApiKey,
            allow_insecure_http: false,
            token_limit_field: (protocol == AiProtocol::OpenaiChat).then_some(TokenLimitField::MaxTokens),
        }
    }

    async fn read_request(socket: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        loop {
            let mut byte = [0];
            socket.read_exact(&mut byte).await.unwrap();
            bytes.push(byte[0]);
            assert!(bytes.len() < 64 * 1024);
            if bytes.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8(bytes).unwrap()
    }

    /// A real local TCP HTTP response, not an adapter mock or synthetic Response.
    async fn reply(status: &str, headers: &str, body: &[u8], fragment_size: usize) -> (String, JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let status = status.to_owned();
        let headers = headers.to_owned();
        let body = body.to_vec();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            socket.write_all(format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
                body.len(),
            ).as_bytes()).await.unwrap();
            for fragment in body.chunks(fragment_size.max(1)) {
                // The client may drop the response immediately at a native terminal.
                if socket.write_all(fragment).await.is_err() {
                    break;
                }
                tokio::task::yield_now().await;
            }
            request
        });
        (base, handle)
    }

    #[test]
    fn endpoints_enforce_transport_and_strip_only_trailing_path_slashes() {
        for endpoint in [
            "https://user:secret@example.com/v1",
            "https://@example.com/v1",
            "https://example.com/v1?key=secret",
            "https://example.com/v1?",
            "https://example.com/v1#fragment",
            "https://example.com/v1#",
            "ftp://example.com/v1",
            "file:///tmp/endpoint",
            "not a URL",
            "http://example.com/v1",
        ] {
            let error = validate_endpoint(endpoint, false).unwrap_err();
            assert_eq!(error.code, "ai_invalid_endpoint");
            assert!(!error.message.contains("secret"));
        }
        for endpoint in ["http://localhost/v1", "http://127.0.0.1/v1", "http://127.2.3.4/v1", "http://[::1]/v1"] {
            validate_endpoint(endpoint, false).unwrap();
        }
        assert_eq!(
            validate_endpoint("HTTPS://EXAMPLE.COM:443/proxy/team/v1///", false).unwrap().as_str(),
            "https://example.com/proxy/team/v1",
        );
        assert_eq!(
            validate_endpoint("https://example.com/proxy%2Fteam/v1/", false).unwrap().as_str(),
            "https://example.com/proxy%2Fteam/v1",
        );
        validate_endpoint("http://example.com/v1", true).unwrap();
        assert!(validate_endpoint("https://key@example.com/v1", true).is_err());
    }

    #[test]
    fn gemini_model_is_one_encoded_segment_and_query_is_native_only() {
        let mut settings = profile("https://example.com/proxy/team/v1/", AiProtocol::Gemini, true);
        settings.model = "models/model/with space?private#fragment%".into();
        let url = request_url(&settings).unwrap();
        assert_eq!(
            url.as_str(),
            "https://example.com/proxy/team/v1/models/model%2Fwith%20space%3Fprivate%23fragment%25:streamGenerateContent?alt=sse",
        );
        settings.stream = false;
        assert_eq!(
            request_url(&settings).unwrap().as_str(),
            "https://example.com/proxy/team/v1/models/model%2Fwith%20space%3Fprivate%23fragment%25:generateContent",
        );
        settings.model = "models/".into();
        assert_eq!(request_url(&settings).unwrap_err().code, "ai_protocol");
    }

    #[test]
    fn credential_values_are_sensitive_and_never_echoed() {
        assert!(credential_header(Some("fixture-key")).unwrap().is_sensitive());
        let bearer = bearer_header(Some("fixture-key")).unwrap();
        assert!(bearer.is_sensitive());
        assert_eq!(bearer.to_str().unwrap(), "Bearer fixture-key");
        for key in [None, Some(""), Some("  "), Some("private-key\n")] {
            let error = credential_header(key).unwrap_err();
            assert_eq!(error.code, "ai_missing_credentials");
            assert!(!error.message.contains("private-key"));
        }
    }

    #[tokio::test]
    async fn actual_client_preserves_all_routes_and_sensitive_headers() {
        let client = http_client().unwrap();
        for (protocol, route, header_name) in [
            (AiProtocol::OpenaiResponses, "/responses", "authorization"),
            (AiProtocol::OpenaiChat, "/chat/completions", "authorization"),
            (AiProtocol::Gemini, "/models/fixture-model:generateContent", "x-goog-api-key"),
            (AiProtocol::Anthropic, "/messages", "x-api-key"),
        ] {
            let (base, fixture) = reply("200 OK", "Content-Type: application/json\r\n", b"{\"ok\":true}", 1).await;
            let settings = profile(&format!("{base}/proxy/team/v1/"), protocol, false);
            let header = if header_name == "authorization" {
                bearer_header(Some("transport-secret")).unwrap()
            } else {
                credential_header(Some("transport-secret")).unwrap()
            };
            let cancel = CancellationToken::new();
            let response = send(client.post(request_url(&settings).unwrap()).header(header_name, header), &cancel).await.unwrap();
            let body: Value = read_json(response, &cancel).await.unwrap();
            assert_eq!(body["ok"], true);
            let request = fixture.await.unwrap();
            assert!(request.starts_with(&format!("POST /proxy/team/v1{route} HTTP/1.1\r\n")));
            let key_value = if header_name == "authorization" { "Bearer transport-secret" } else { "transport-secret" };
            assert!(request.contains(&format!("{header_name}: {key_value}\r\n")));
            assert!(!request.lines().next().unwrap().contains("transport-secret"));
        }
    }

    #[tokio::test]
    async fn redirects_never_reach_a_target_or_forward_credentials() {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_url = format!("http://{}/do-not-follow", target.local_addr().unwrap());
        let (base, fixture) = reply("307 Temporary Redirect", &format!("Location: {target_url}\r\n"), b"", 1).await;
        let error = send(
            http_client().unwrap().post(format!("{base}/proxy/v1/responses"))
                .header("authorization", bearer_header(Some("redirect-secret")).unwrap()),
            &CancellationToken::new(),
        ).await.unwrap_err();
        assert_eq!(error.code, "ai_protocol");
        assert!(!error.message.contains("redirect-secret"));
        assert!(fixture.await.unwrap().contains("authorization: Bearer redirect-secret"));
        assert!(tokio::time::timeout(Duration::from_millis(40), target.accept()).await.is_err());
    }

    #[tokio::test]
    async fn http_error_categories_never_return_body_or_key_material() {
        let client = http_client().unwrap();
        for (status, body, code) in [
            ("401 Unauthorized", "{\"error\":{\"message\":\"private source and secret-key\"}}", "ai_unauthorized"),
            ("429 Too Many Requests", "{\"error\":{\"message\":\"private source and secret-key\"}}", "ai_rate_limit"),
            ("400 Bad Request", "{\"error\":{\"code\":\"context_length_exceeded\",\"message\":\"secret-key\"}}", "ai_context_too_long"),
            ("400 Bad Request", "{\"error\":{\"status\":\"SAFETY\",\"message\":\"secret-key\"}}", "ai_content_blocked"),
            ("400 Bad Request", "private malformed secret-key", "ai_protocol"),
            ("503 Service Unavailable", "private secret-key", "ai_connection"),
        ] {
            let (base, fixture) = reply(status, "Content-Type: application/json\r\n", body.as_bytes(), 3).await;
            let error = send(client.post(format!("{base}/endpoint")), &CancellationToken::new()).await.unwrap_err();
            assert_eq!(error.code, code);
            assert!(!error.message.contains("private"));
            assert!(!error.message.contains("secret-key"));
            fixture.await.unwrap();
        }
    }

    #[tokio::test]
    async fn actual_fragmented_sse_ignores_auxiliary_events_and_requires_terminal() {
        let body = concat!(
            ": ping\r\n\r\n",
            "event: ping\r\ndata: not required JSON\r\n\r\n",
            "event: delta\r\ndata: {\"text\":\"Xin chào. 🙂\"}\r\n\r\n",
            "event: complete\r\ndata: {}\r\n\r\n",
        );
        let (base, fixture) = reply("200 OK", "Content-Type: text/event-stream\r\n", body.as_bytes(), 1).await;
        let cancel = CancellationToken::new();
        let response = send(http_client().unwrap().post(base), &cancel).await.unwrap();
        let mut text = String::new();
        stream_sse(response, &cancel, |event| {
            match event.event.as_deref() {
                Some("delta") => text.push_str(json_data(&event)?["text"].as_str().unwrap()),
                Some("complete") => {
                    json_data(&event)?;
                    return Ok(SseControl::Done);
                }
                _ => {}
            }
            Ok(SseControl::Continue)
        }).await.unwrap();
        assert_eq!(text, "Xin chào. 🙂");
        fixture.await.unwrap();
    }

    #[tokio::test]
    async fn framed_deltas_without_terminal_and_unframed_eof_are_interrupted() {
        let client = http_client().unwrap();
        for body in [
            "data: {\"text\":\"partial\"}\n\n",
            "data: {\"text\":\"partial\"}\n\ndata: unfinished",
            ": only a ping\n\n",
        ] {
            let (base, fixture) = reply("200 OK", "Content-Type: text/event-stream\r\n", body.as_bytes(), 2).await;
            let cancel = CancellationToken::new();
            let response = send(client.post(base), &cancel).await.unwrap();
            let mut partial = String::new();
            let error = stream_sse(response, &cancel, |event| {
                partial.push_str(json_data(&event)?["text"].as_str().unwrap());
                Ok(SseControl::Continue)
            }).await.unwrap_err();
            assert_eq!(error.code, "ai_interrupted");
            if body.starts_with("data:") {
                assert_eq!(partial, "partial");
            }
            fixture.await.unwrap();
        }
    }

    #[tokio::test]
    async fn required_sse_json_and_nonstream_json_errors_are_sanitized() {
        let client = http_client().unwrap();
        let (base, fixture) = reply("200 OK", "Content-Type: text/event-stream\r\n", b"data: private secret invalid JSON\n\n", 1).await;
        let cancel = CancellationToken::new();
        let response = send(client.post(base), &cancel).await.unwrap();
        let error = stream_sse(response, &cancel, |event| {
            json_data(&event)?;
            Ok(SseControl::Continue)
        }).await.unwrap_err();
        assert_eq!(error.code, "ai_protocol");
        assert!(!error.message.contains("private"));
        fixture.await.unwrap();
        let (base, fixture) = reply("200 OK", "Content-Type: application/json\r\n", b"private secret invalid JSON", 2).await;
        let response = send(client.post(base), &cancel).await.unwrap();
        let error = read_json::<Value>(response, &cancel).await.unwrap_err();
        assert_eq!(error.code, "ai_protocol");
        assert!(!error.message.contains("private"));
        fixture.await.unwrap();
    }

    #[tokio::test]
    async fn cancellation_before_send_never_opens_http() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let error = send(
            http_client().unwrap().post(format!("http://{}/endpoint", listener.local_addr().unwrap())),
            &cancel,
        ).await.unwrap_err();
        assert_eq!(error.code, "ai_cancelled");
        assert!(tokio::time::timeout(Duration::from_millis(40), listener.accept()).await.is_err());
    }

    async fn stalled_response(prefix: &'static [u8]) -> (Response, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let fixture = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_request(&mut socket).await;
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 100000\r\n\r\n").await.unwrap();
            socket.write_all(prefix).await.unwrap();
            // Hold the actual response open until the test observes cancellation/idle error.
            std::future::pending::<()>().await;
        });
        let response = send(http_client().unwrap().post(base), &CancellationToken::new()).await.unwrap();
        (response, fixture)
    }

    #[tokio::test]
    async fn cancelling_after_delta_preserves_partial_and_drops_stalled_response() {
        let (response, fixture) = stalled_response(b"data: {\"text\":\"partial\"}\n\n").await;
        let cancel = CancellationToken::new();
        let mut partial = String::new();
        let result = tokio::time::timeout(Duration::from_secs(1), stream_sse(response, &cancel, |event| {
            partial.push_str(json_data(&event)?["text"].as_str().unwrap());
            cancel.cancel();
            Ok(SseControl::Continue)
        })).await.unwrap();
        assert_eq!(result.unwrap_err().code, "ai_cancelled");
        assert_eq!(partial, "partial");
        fixture.abort();
    }

    #[tokio::test]
    async fn ordinary_response_reads_are_cancellable_while_waiting() {
        let (response, fixture) = stalled_response(b"{").await;
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let task = tokio::spawn(async move { read_json::<Value>(response, &task_cancel).await });
        tokio::task::yield_now().await;
        cancel.cancel();
        let error = tokio::time::timeout(Duration::from_secs(1), task).await.unwrap().unwrap().unwrap_err();
        assert_eq!(error.code, "ai_cancelled");
        fixture.abort();
    }

    #[tokio::test]
    async fn idle_timeout_uses_actual_response_and_does_not_echo_data() {
        let (mut response, fixture) = stalled_response(b"").await;
        let error = next_chunk(&mut response, &CancellationToken::new(), Duration::from_millis(20)).await.unwrap_err();
        assert_eq!(error.code, "ai_interrupted");
        assert_eq!(IDLE_READ_TIMEOUT, Duration::from_secs(120));
        assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(15));
        fixture.abort();
    }

    #[tokio::test]
    async fn premature_http_disconnect_is_not_malformed_json_success() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let fixture = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_request(&mut socket).await;
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n{\"partial\":").await.unwrap();
        });
        let cancel = CancellationToken::new();
        let response = send(http_client().unwrap().post(base), &cancel).await.unwrap();
        let error = read_json::<Value>(response, &cancel).await.unwrap_err();
        assert_eq!(error.code, "ai_interrupted");
        fixture.await.unwrap();
    }

    #[tokio::test]
    async fn auth_none_sends_no_credential_header() {
        let (base, fixture) = reply("200 OK", "Content-Type: application/json\r\n", b"{}", 1).await;
        let mut settings = profile(&format!("{base}/proxy/v1"), AiProtocol::OpenaiChat, false);
        settings.auth_mode = AuthMode::None;
        let cancel = CancellationToken::new();
        let response = send(http_client().unwrap().post(request_url(&settings).unwrap()), &cancel).await.unwrap();
        read_json::<Value>(response, &cancel).await.unwrap();
        let request = fixture.await.unwrap();
        assert!(request.starts_with("POST /proxy/v1/chat/completions HTTP/1.1\r\n"));
        assert!(!request.contains("authorization:"));
        assert!(!request.contains("x-api-key:"));
        assert!(!request.contains("x-goog-api-key:"));
    }

    #[test]
    fn native_error_envelopes_and_inner_objects_have_fixed_classification() {
        for (field, value, code) in [
            ("type", "authentication_error", "ai_unauthorized"),
            ("type", "permission_error", "ai_unauthorized"),
            ("type", "rate_limit_error", "ai_rate_limit"),
            ("type", "refusal_error", "ai_content_blocked"),
            ("type", "overloaded_error", "ai_connection"),
            ("status", "RESOURCE_EXHAUSTED", "ai_rate_limit"),
            ("code", "context_length_exceeded", "ai_context_too_long"),
            ("type", "unrecognized_error", "ai_protocol"),
        ] {
            let mut inner = serde_json::Map::new();
            inner.insert(field.into(), Value::String(value.into()));
            inner.insert("message".into(), Value::String("private source and secret-key".into()));
            let inner = Value::Object(inner);
            for error in [provider_error(&inner), provider_error(&serde_json::json!({"error": inner}))] {
                assert_eq!(error.code, code);
                assert!(!error.message.contains("private"));
                assert!(!error.message.contains("secret-key"));
            }
        }
    }
}
