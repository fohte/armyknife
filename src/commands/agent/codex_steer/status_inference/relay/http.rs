use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;

use super::RelayState;

const UPSTREAM_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_REQUEST_BYTES: usize = 64 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
const READ_CHUNK_BYTES: usize = 16 * 1024;
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(30);
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(25);
const HEADER_SEPARATOR: [u8; 4] = [0x0d, 0x0a, 0x0d, 0x0a];

struct IncomingRequest {
    headers: HeaderMap,
    body: Vec<u8>,
}

pub(super) fn handle(stream: &mut TcpStream, state: &RelayState) -> Result<()> {
    let request = read_request(stream, state)?;
    let body = rewrite_request_body(&request.body, &state.parent_session_id)?;
    let headers = upstream_headers(&request.headers, &state.parent_session_id)?;
    let client = state.client.clone();
    let cancelled = state.cancelled.clone();
    let result = state.runtime.block_on(async move {
        let response = tokio::select! {
            response = client.post(UPSTREAM_URL).headers(headers).body(body).send() => response,
            () = wait_until_cancelled(cancelled.clone()) => bail!("Codex relay was cancelled"),
        }
        .context("send Codex status request to the official endpoint")?;
        ensure!(
            !response.status().is_redirection(),
            "official Codex endpoint returned a redirect"
        );

        let status = response.status();
        let response_headers = response.headers().clone();
        let mut body: Vec<u8> = Vec::new();
        let mut response = response;
        loop {
            let chunk = tokio::select! {
                chunk = response.chunk() => chunk.context("read official Codex response")?,
                () = wait_until_cancelled(cancelled.clone()) => bail!("Codex relay was cancelled"),
            };
            let Some(chunk) = chunk else {
                break;
            };
            ensure!(
                body.len().saturating_add(chunk.len()) <= MAX_RESPONSE_BYTES,
                "official Codex response exceeds the relay size limit"
            );
            body.extend_from_slice(&chunk);
        }
        Ok::<_, anyhow::Error>((status, response_headers, body))
    });

    match result {
        Ok((status, headers, body)) => {
            if contains_tool_call(&body, headers.get(CONTENT_TYPE))? {
                write_rejection(stream, 502, "tool call rejected");
                bail!("Codex status response requested a tool call");
            }
            if let Some(usage) = super::usage::from_http_body(&body) {
                usage.log(&state.parent_session_id);
            }
            write_upstream_response(stream, status.as_u16(), &headers, &body)?;
        }
        Err(_) => write_rejection(stream, 502, "official Codex request failed"),
    }
    Ok(())
}

fn read_request(stream: &mut TcpStream, state: &RelayState) -> Result<IncomingRequest> {
    read_request_until(
        stream,
        &state.cancelled,
        &state.path,
        &state.authority,
        Instant::now() + REQUEST_READ_TIMEOUT,
    )
}

fn read_request_until(
    stream: &mut TcpStream,
    cancelled: &std::sync::atomic::AtomicBool,
    expected_path: &str,
    expected_authority: &str,
    deadline: Instant,
) -> Result<IncomingRequest> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; READ_CHUNK_BYTES];
    let header_end = loop {
        if cancelled.load(Ordering::Relaxed) {
            bail!("Codex relay was cancelled");
        }
        ensure!(
            Instant::now() < deadline,
            "Codex relay request read timed out"
        );
        if let Some(header_end) = find_header_end(&bytes) {
            break header_end;
        }
        ensure!(
            bytes.len() < MAX_HEADER_BYTES,
            "Codex relay headers are too large"
        );
        match stream.read(&mut chunk) {
            Ok(0) => bail!("Codex relay request ended before its headers"),
            Ok(read) => bytes.extend_from_slice(&chunk[..read]),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => return Err(error).context("read Codex relay request headers"),
        }
    };

    let mut parsed_headers = [httparse::EMPTY_HEADER; 64];
    let mut parsed = httparse::Request::new(&mut parsed_headers);
    match parsed.parse(&bytes[..header_end + 4])? {
        httparse::Status::Complete(_) => {}
        httparse::Status::Partial => bail!("Codex relay request headers are incomplete"),
    }
    let method = parsed
        .method
        .context("Codex relay request has no method")?
        .to_string();
    let target = parsed
        .path
        .context("Codex relay request has no target")?
        .to_string();
    let mut headers = HeaderMap::new();
    let mut content_lengths = Vec::new();
    let mut has_transfer_encoding = false;
    for header in parsed.headers.iter() {
        let name = HeaderName::from_bytes(header.name.as_bytes())
            .context("Codex relay request has an invalid header name")?;
        let value = HeaderValue::from_bytes(header.value)
            .context("Codex relay request has an invalid header value")?;
        if name == "content-length" {
            content_lengths.push(value.clone());
        } else if name == "transfer-encoding" {
            has_transfer_encoding = true;
        }
        headers.append(name, value);
    }
    ensure!(
        !has_transfer_encoding,
        "Codex relay does not accept transfer encoding"
    );
    ensure!(
        content_lengths.len() == 1,
        "Codex relay request must have one content length"
    );
    let content_length = content_lengths[0]
        .to_str()
        .context("Codex relay content length is not ASCII")?
        .parse::<usize>()
        .context("Codex relay content length is invalid")?;
    ensure!(
        content_length <= MAX_REQUEST_BYTES,
        "Codex relay request exceeds the size limit"
    );
    validate_request_head(
        &method,
        &target,
        &headers,
        expected_path,
        expected_authority,
    )?;
    let body_start = header_end + 4;
    ensure!(
        body_start <= bytes.len(),
        "Codex relay headers are malformed"
    );
    let mut body = bytes[body_start..].to_vec();
    ensure!(
        body.len() <= content_length,
        "Codex relay does not support pipelined requests"
    );
    while body.len() < content_length {
        if cancelled.load(Ordering::Relaxed) {
            bail!("Codex relay was cancelled");
        }
        ensure!(
            Instant::now() < deadline,
            "Codex relay request read timed out"
        );
        let read = match stream.read(&mut chunk) {
            Ok(0) => bail!("Codex relay request body ended early"),
            Ok(read) => read,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error).context("read Codex relay request body"),
        };
        ensure!(
            body.len().saturating_add(read) <= content_length,
            "Codex relay does not support pipelined requests"
        );
        body.extend_from_slice(&chunk[..read]);
    }
    Ok(IncomingRequest { headers, body })
}

fn validate_request_head(
    method: &str,
    target: &str,
    headers: &HeaderMap,
    expected_path: &str,
    expected_authority: &str,
) -> Result<()> {
    ensure!(
        method == "POST",
        "Codex relay rejected an unsupported HTTP method"
    );
    validate_target(target, headers, expected_path, expected_authority)?;
    ensure!(
        headers.contains_key(AUTHORIZATION),
        "Codex relay request has no authorization header"
    );
    ensure!(
        headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().starts_with("application/json")),
        "Codex relay request is not JSON"
    );
    Ok(())
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == HEADER_SEPARATOR)
}

pub(super) fn validate_target(
    target: &str,
    headers: &HeaderMap,
    expected_path: &str,
    expected_authority: &str,
) -> Result<()> {
    ensure!(
        headers.get("host").and_then(|value| value.to_str().ok()) == Some(expected_authority),
        "Codex relay rejected an unknown host"
    );
    let path = if target.starts_with('/') {
        ensure!(
            !target.contains('?'),
            "Codex relay does not accept query strings"
        );
        target.to_string()
    } else {
        let url = reqwest::Url::parse(target).context("Codex relay request target is invalid")?;
        ensure!(
            url.scheme() == "http"
                && url.host_str() == Some("localhost")
                && url.port_or_known_default()
                    == expected_authority
                        .rsplit_once(':')
                        .and_then(|(_, port)| port.parse().ok())
                && url.query().is_none()
                && url.fragment().is_none(),
            "Codex relay rejected an absolute request target"
        );
        url.path().to_string()
    };
    ensure!(
        path == expected_path,
        "Codex relay rejected an unknown path"
    );
    Ok(())
}

pub(super) fn rewrite_request_body(body: &[u8], parent_session_id: &str) -> Result<Vec<u8>> {
    let mut value: Value = serde_json::from_slice(body).context("parse Codex Responses request")?;
    let object = value
        .as_object_mut()
        .context("Codex Responses request is not a JSON object")?;
    object.insert("tool_choice".to_string(), Value::String("none".to_string()));
    object.insert(
        "prompt_cache_key".to_string(),
        Value::String(parent_session_id.to_string()),
    );
    serde_json::to_vec(&value).context("serialize Codex Responses request")
}

pub(super) fn upstream_headers(incoming: &HeaderMap, parent_session_id: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    for (name, value) in incoming {
        if should_forward_header(name) {
            headers.append(name.clone(), value.clone());
        }
    }
    headers.insert(
        HeaderName::from_static("session-id"),
        HeaderValue::from_str(parent_session_id).context("parent Codex session ID is invalid")?,
    );
    // Cache affinity follows the parent; thread-id remains the fork's identity.
    Ok(headers)
}

fn should_forward_header(name: &HeaderName) -> bool {
    !matches!(
        name.as_str(),
        "connection"
            | "content-length"
            | "host"
            | "keep-alive"
            | "proxy-authorization"
            | "proxy-connection"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    ) && !name.as_str().starts_with("sec-websocket-")
}

pub(super) fn contains_tool_call(body: &[u8], content_type: Option<&HeaderValue>) -> Result<bool> {
    let content_type = content_type
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if content_type.starts_with("text/event-stream") {
        for line in body.split(|byte| *byte == b'\n') {
            let line = super::trim_ascii(line);
            let Some(data) = line.strip_prefix(b"data:") else {
                continue;
            };
            let data = super::trim_ascii(data);
            if data == b"[DONE]" || data.is_empty() {
                continue;
            }
            let event: Value = serde_json::from_slice(data)
                .context("parse official Codex event before forwarding")?;
            if value_contains_tool_call(&event) {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    ensure!(
        content_type.starts_with("application/json"),
        "official Codex response has an unsupported content type"
    );
    let response: Value =
        serde_json::from_slice(body).context("parse official Codex response before forwarding")?;
    Ok(value_contains_tool_call(&response))
}

pub(super) fn value_contains_tool_call(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(value_contains_tool_call),
        Value::Object(object) => {
            let typed_tool_call = object
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(is_tool_call_type);
            typed_tool_call || object.values().any(value_contains_tool_call)
        }
        _ => false,
    }
}

fn is_tool_call_type(value: &str) -> bool {
    value.ends_with("_call")
        || value.starts_with("response.function_call_")
        || value.starts_with("response.mcp_call_")
}

async fn wait_until_cancelled(cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    while !cancelled.load(Ordering::Relaxed) {
        tokio::time::sleep(CANCEL_POLL_INTERVAL).await;
    }
}

fn write_upstream_response(
    stream: &mut TcpStream,
    status: u16,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<()> {
    let reason = reqwest::StatusCode::from_u16(status)
        .ok()
        .and_then(|status| status.canonical_reason())
        .context("official Codex response has an invalid status")?;
    let response_headers = indoc::formatdoc! {"
        HTTP/1.1 {status} {reason}
        Content-Length: {}
        Connection: close

    ", body.len()}
    .replace('\n', "\r\n");
    stream
        .write_all(response_headers.as_bytes())
        .context("write Codex relay response headers")?;
    for name in [CONTENT_TYPE, HeaderName::from_static("x-request-id")]
        .into_iter()
        .chain([
            HeaderName::from_static("openai-processing-ms"),
            HeaderName::from_static("openai-version"),
            HeaderName::from_static("retry-after"),
        ])
    {
        if let Some(value) = headers.get(&name)
            && let Ok(value) = value.to_str()
        {
            write!(stream, "{}: {value}\r\n", name.as_str())
                .context("write Codex relay response header")?;
        }
    }
    stream
        .write_all(b"\r\n")
        .and_then(|()| stream.write_all(body))
        .context("write Codex relay response body")
}

pub(super) fn write_rejection(stream: &mut TcpStream, status: u16, reason: &str) {
    let body = reason.as_bytes();
    let response = indoc::formatdoc! {"
        HTTP/1.1 {status} {reason}
        Content-Type: text/plain
        Content-Length: {}
        Connection: close

        {reason}
    ", body.len()}
    .replace('\n', "\r\n");
    let _ = stream.write_all(response.as_bytes());
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::AtomicBool;
    use std::time::{Duration, Instant};

    const SSE_TOOL_CALL_RESPONSE: &[u8] = indoc::indoc! {r#"
        event: response.output_item.added
        data: {"type":"response.output_item.added","item":{"type":"function_call"}}

    "#}
    .as_bytes();
    const PARTIAL_BODY_REQUEST: &str = indoc::indoc! {"
        POST /fixture-token/backend-api/codex/responses HTTP/1.1
        Host: localhost:43817
        Authorization: Bearer fixture-token
        Content-Type: application/json
        Content-Length: 8

        {}
    "};
    const MISSING_AUTH_REQUEST: &str = indoc::indoc! {"
        POST /fixture-token/backend-api/codex/responses HTTP/1.1
        Host: localhost:43817
        Content-Type: application/json
        Content-Length: 8

    "};
    const WRONG_PATH_REQUEST: &str = indoc::indoc! {"
        POST /other HTTP/1.1
        Host: localhost:43817
        Authorization: Bearer fixture-token
        Content-Type: application/json
        Content-Length: 8

    "};

    use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
    use rstest::rstest;
    use serde_json::{Value, json};

    use super::{
        contains_tool_call, read_request_until, rewrite_request_body, upstream_headers,
        value_contains_tool_call,
    };

    #[rstest]
    #[case::partial_headers("POST")]
    #[case::partial_body(PARTIAL_BODY_REQUEST)]
    fn partial_request_expires_at_the_absolute_deadline(#[case] request: &'static str) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("fixture listener should bind");
        let mut client = TcpStream::connect(listener.local_addr().expect("listener address"))
            .expect("fixture client should connect");
        let (mut server, _) = listener.accept().expect("fixture connection should arrive");
        server
            .set_read_timeout(Some(Duration::from_millis(1)))
            .expect("fixture socket should have a short read timeout");
        let request = request.replace('\n', "\r\n");
        client
            .write_all(request.as_bytes())
            .expect("fixture client should send a partial request");

        let result = read_request_until(
            &mut server,
            &AtomicBool::new(false),
            "/fixture-token/backend-api/codex/responses",
            "localhost:43817",
            Instant::now() + Duration::from_millis(10),
        );
        let actual = result
            .map(|_| "ok".to_string())
            .unwrap_or_else(|error| format!("{error:#}"));
        assert_eq!(actual, "Codex relay request read timed out");
    }

    #[rstest]
    #[case::missing_authorization(
        MISSING_AUTH_REQUEST,
        "Codex relay request has no authorization header"
    )]
    #[case::wrong_path(WRONG_PATH_REQUEST, "Codex relay rejected an unknown path")]
    fn rejects_invalid_request_heads_before_reading_the_body(
        #[case] request: &'static str,
        #[case] expected_error: &str,
    ) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("fixture listener should bind");
        let mut client = TcpStream::connect(listener.local_addr().expect("listener address"))
            .expect("fixture client should connect");
        let (mut server, _) = listener.accept().expect("fixture connection should arrive");
        server
            .set_read_timeout(Some(Duration::from_millis(1)))
            .expect("fixture socket should have a short read timeout");
        let request = request.replace('\n', "\r\n");
        client
            .write_all(request.as_bytes())
            .expect("fixture client should send request headers");

        let result = read_request_until(
            &mut server,
            &AtomicBool::new(false),
            "/fixture-token/backend-api/codex/responses",
            "localhost:43817",
            Instant::now() + Duration::from_secs(1),
        );
        let actual = result
            .map(|_| "ok".to_string())
            .unwrap_or_else(|error| format!("{error:#}"));
        assert_eq!(actual, expected_error);
    }

    #[test]
    fn request_rewrite_disables_tools_and_uses_parent_cache_affinity() {
        let rewritten = rewrite_request_body(
            br#"{"tool_choice":"auto","prompt_cache_key":"fixture-child","input":[]}"#,
            "fixture-parent",
        )
        .expect("fixture request should be valid");
        let actual = serde_json::from_slice::<Value>(&rewritten)
            .expect("rewritten request should remain valid JSON");
        assert_eq!(
            actual,
            json!({
                "tool_choice": "none",
                "prompt_cache_key": "fixture-parent",
                "input": [],
            }),
        );
    }

    #[test]
    fn upstream_headers_keep_authorization_and_child_identity_but_replace_cache_affinity() {
        let incoming = HeaderMap::from_iter([
            (
                AUTHORIZATION,
                HeaderValue::from_static("Bearer fixture-token"),
            ),
            (
                HeaderName::from_static("session-id"),
                HeaderValue::from_static("fixture-child-session"),
            ),
            (
                HeaderName::from_static("thread-id"),
                HeaderValue::from_static("fixture-child-thread"),
            ),
            (
                HeaderName::from_static("connection"),
                HeaderValue::from_static("keep-alive"),
            ),
        ]);
        let headers = upstream_headers(&incoming, "fixture-parent-session")
            .expect("fixture headers should be valid");
        let actual = headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_string(),
                    value
                        .to_str()
                        .expect("fixture header should be ASCII")
                        .to_string(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            actual,
            std::collections::BTreeMap::from([
                (
                    "authorization".to_string(),
                    "Bearer fixture-token".to_string()
                ),
                (
                    "session-id".to_string(),
                    "fixture-parent-session".to_string()
                ),
                ("thread-id".to_string(), "fixture-child-thread".to_string()),
            ]),
        );
    }

    #[rstest]
    #[case::function_call(json!({"type":"response.output_item.added","item":{"type":"function_call"}}), true)]
    #[case::mcp_call(json!({"type":"response.output_item.done","item":{"type":"mcp_call"}}), true)]
    #[case::ordinary_message(json!({"type":"response.output_item.done","item":{"type":"message"}}), false)]
    fn response_tool_call_detection_is_recursive(#[case] value: Value, #[case] expected: bool) {
        assert_eq!(value_contains_tool_call(&value), expected);
    }

    #[rstest]
    #[case::sse_tool_call(SSE_TOOL_CALL_RESPONSE, "text/event-stream", true)]
    #[case::json_function_call(
        br#"{"output":[{"type":"function_call","name":"fixture_tool","arguments":"{}"}]}"#,
        "application/json",
        true
    )]
    #[case::json_message(
        br#"{"output":[{"type":"message","content":[]}]}"#,
        "application/json",
        false
    )]
    fn response_body_is_checked_before_forwarding(
        #[case] body: &'static [u8],
        #[case] content_type: &'static str,
        #[case] expected: bool,
    ) {
        let actual = contains_tool_call(body, Some(&HeaderValue::from_static(content_type)))
            .expect("fixture response should be checked");
        assert_eq!(actual, expected);
    }
}
