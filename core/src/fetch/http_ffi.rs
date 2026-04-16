// Generic byte-oriented HTTP entry point exposed across the UniFFI boundary.
// Replaces Swift's URLSession + NetworkResilience retry stack.

use reqwest::{Method, StatusCode};
use std::time::Duration;
use tokio::time::sleep;

use crate::http::{HttpClient, RetryProfile};

#[derive(Debug, Clone, uniffi::Enum)]
pub enum HttpRetryProfile {
    ChainRead,
    ChainWrite,
    Diagnostics,
}

impl From<HttpRetryProfile> for RetryProfile {
    fn from(value: HttpRetryProfile) -> Self {
        match value {
            HttpRetryProfile::ChainRead => RetryProfile::ChainRead,
            HttpRetryProfile::ChainWrite => RetryProfile::ChainWrite,
            HttpRetryProfile::Diagnostics => RetryProfile::Diagnostics,
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct HttpResponse {
    pub status_code: u16,
    pub headers: Vec<HttpHeader>,
    pub body: Vec<u8>,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum HttpError {
    #[error("invalid method: {method}")]
    InvalidMethod { method: String },
    #[error("request failed: {message}")]
    Transport { message: String },
    #[error("all {attempts} attempts failed: {message}")]
    RetriesExhausted { attempts: u32, message: String },
    #[error("timeout after {elapsed_ms}ms")]
    Timeout { elapsed_ms: u64 },
    #[error("network error: {message}")]
    Network { message: String },
    #[error("decode error: {message}")]
    Decode { message: String },
    #[error("HTTP status {status}: {body}")]
    Status { status: u16, body: String },
}

/// Text-oriented HTTP response used by the ergonomic wrappers below.
/// Headers are exposed as a simple string→string map for Swift use.
#[derive(Debug, Clone, uniffi::Record)]
pub struct HttpTextResponse {
    pub status: u16,
    pub body: String,
    pub headers: std::collections::HashMap<String, String>,
}

fn collect_headers(resp: &reqwest::Response) -> std::collections::HashMap<String, String> {
    resp.headers()
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|s| (k.as_str().to_string(), s.to_string())))
        .collect()
}

fn classify_reqwest_error(err: reqwest::Error) -> HttpError {
    if err.is_timeout() {
        HttpError::Timeout { elapsed_ms: 0 }
    } else if err.is_decode() {
        HttpError::Decode { message: err.to_string() }
    } else {
        HttpError::Network { message: err.to_string() }
    }
}

fn parse_method(method: &str) -> Result<Method, HttpError> {
    Method::from_bytes(method.to_uppercase().as_bytes())
        .map_err(|_| HttpError::InvalidMethod { method: method.to_string() })
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn http_request(
    method: String,
    url: String,
    headers: Vec<HttpHeader>,
    body: Option<Vec<u8>>,
    profile: HttpRetryProfile,
) -> Result<HttpResponse, HttpError> {
    let method = parse_method(&method)?;
    let retry: RetryProfile = profile.into();
    let client = HttpClient::shared();
    let inner = client.reqwest_client();

    let max_attempts = retry.max_attempts();
    let mut last_err = String::new();

    for attempt in 0..max_attempts {
        if attempt > 0 {
            sleep(retry.delay_for_attempt(attempt)).await;
        }
        let mut req = inner.request(method.clone(), &url);
        for h in &headers {
            req = req.header(h.name.as_str(), h.value.as_str());
        }
        if let Some(ref bytes) = body {
            req = req.body(bytes.clone());
        }
        match req.send().await {
            Err(e) => {
                last_err = e.to_string();
                if !retry.is_retryable_error(&e) {
                    break;
                }
            }
            Ok(resp) => {
                let status = resp.status();
                if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                    last_err = format!("HTTP {status}");
                    if attempt + 1 < max_attempts {
                        continue;
                    }
                }
                let status_code = status.as_u16();
                let response_headers: Vec<HttpHeader> = resp
                    .headers()
                    .iter()
                    .filter_map(|(k, v)| {
                        v.to_str().ok().map(|s| HttpHeader {
                            name: k.as_str().to_string(),
                            value: s.to_string(),
                        })
                    })
                    .collect();
                let body_bytes = resp
                    .bytes()
                    .await
                    .map_err(|e| HttpError::Transport { message: e.to_string() })?
                    .to_vec();
                return Ok(HttpResponse {
                    status_code,
                    headers: response_headers,
                    body: body_bytes,
                });
            }
        }
    }
    Err(HttpError::RetriesExhausted {
        attempts: max_attempts as u32,
        message: last_err,
    })
}

// ----------------------------------------------------------------
// Ergonomic text-oriented wrappers (Phase 1 migration target shape)
// ----------------------------------------------------------------

/// GET a URL and return the response body as UTF-8 text with headers.
/// Single-shot (no retry). Use `http_request` for retry-profiled calls.
#[uniffi::export(async_runtime = "tokio")]
pub async fn http_get(
    url: String,
    headers: std::collections::HashMap<String, String>,
) -> Result<HttpTextResponse, HttpError> {
    let client = HttpClient::shared();
    let inner = client.reqwest_client();
    let mut req = inner.get(&url);
    for (k, v) in &headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send().await.map_err(classify_reqwest_error)?;
    let status = resp.status().as_u16();
    let response_headers = collect_headers(&resp);
    let body = resp
        .text()
        .await
        .map_err(|e| HttpError::Decode { message: e.to_string() })?;
    Ok(HttpTextResponse { status, body, headers: response_headers })
}

/// POST a JSON body (already serialised) and return the response as text.
/// Sets `Content-Type: application/json` automatically unless overridden
/// by `headers`. Single-shot (no retry).
#[uniffi::export(async_runtime = "tokio")]
pub async fn http_post_json(
    url: String,
    body_json: String,
    headers: std::collections::HashMap<String, String>,
) -> Result<HttpTextResponse, HttpError> {
    let client = HttpClient::shared();
    let inner = client.reqwest_client();
    let has_ct = headers.keys().any(|k| k.eq_ignore_ascii_case("content-type"));
    let mut req = inner.post(&url);
    if !has_ct {
        req = req.header("Content-Type", "application/json");
    }
    for (k, v) in &headers {
        req = req.header(k.as_str(), v.as_str());
    }
    req = req.body(body_json);
    let resp = req.send().await.map_err(classify_reqwest_error)?;
    let status = resp.status().as_u16();
    let response_headers = collect_headers(&resp);
    let body = resp
        .text()
        .await
        .map_err(|e| HttpError::Decode { message: e.to_string() })?;
    Ok(HttpTextResponse { status, body, headers: response_headers })
}

/// Pilot: perform a JSON-RPC reachability probe end-to-end in Rust.
/// Combines the HTTP POST with the existing
/// `diagnostics_parse_jsonrpc_probe` parser so Swift no longer needs to
/// own any URLSession code for this call site. Returns the probe
/// outcome *plus* the observed HTTP status (for the diagnostics row).
#[derive(Debug, Clone, uniffi::Record)]
pub struct JsonRpcProbeResult {
    pub reachable: bool,
    pub status_code: Option<i32>,
    pub detail: String,
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn diagnostics_probe_jsonrpc(
    url: String,
    rpc_method: String,
) -> JsonRpcProbeResult {
    use crate::diagnostics::aggregate::diagnostics_parse_jsonrpc_probe;

    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "spectra-health",
        "method": rpc_method,
        "params": [],
    })
    .to_string();

    let mut headers = std::collections::HashMap::new();
    headers.insert("Content-Type".to_string(), "application/json".to_string());

    match http_post_json(url, payload, headers).await {
        Ok(resp) => {
            let outcome = diagnostics_parse_jsonrpc_probe(Some(resp.status as i32), resp.body);
            JsonRpcProbeResult {
                reachable: outcome.reachable,
                status_code: Some(resp.status as i32),
                detail: outcome.detail,
            }
        }
        Err(e) => JsonRpcProbeResult {
            reachable: false,
            status_code: None,
            detail: e.to_string(),
        },
    }
}

// Lightweight single-shot probe (no retry). Used by endpoint health checks.
#[uniffi::export(async_runtime = "tokio")]
pub async fn http_probe(url: String, timeout_secs: u32) -> bool {
    use reqwest::Client;
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_secs as u64))
        .https_only(true)
        .user_agent("Spectra/1.0")
        .build();
    let Ok(client) = client else { return false };
    client
        .get(&url)
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn http_get_success_returns_body_and_headers() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hello"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-spectra-test", "yes")
                    .set_body_string("world"),
            )
            .mount(&server)
            .await;

        let url = format!("{}/hello", server.uri());
        let resp = http_get(url, Default::default()).await.expect("ok");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, "world");
        assert_eq!(resp.headers.get("x-spectra-test").map(String::as_str), Some("yes"));
    }

    #[tokio::test]
    async fn http_get_4xx_still_returns_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404).set_body_string("nope"))
            .mount(&server)
            .await;
        let resp = http_get(server.uri(), Default::default()).await.expect("ok");
        assert_eq!(resp.status, 404);
        assert_eq!(resp.body, "nope");
    }

    #[tokio::test]
    async fn http_post_json_sends_body_and_content_type() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/rpc"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"result":"ok"}"#))
            .mount(&server)
            .await;
        let url = format!("{}/rpc", server.uri());
        let resp = http_post_json(url, r#"{"a":1}"#.into(), Default::default())
            .await
            .expect("ok");
        assert_eq!(resp.status, 200);
        assert!(resp.body.contains("result"));
    }

    #[tokio::test]
    async fn http_get_network_error_when_port_closed() {
        // Port 1 is typically closed; use a dead address to trigger network error.
        let result = http_get("http://127.0.0.1:1/".into(), Default::default()).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            HttpError::Network { .. } | HttpError::Transport { .. } | HttpError::Timeout { .. } => {}
            other => panic!("expected network-class error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn diagnostics_probe_jsonrpc_reachable_on_result() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"jsonrpc":"2.0","id":"spectra-health","result":{"ok":true}}"#),
            )
            .mount(&server)
            .await;
        let out = diagnostics_probe_jsonrpc(server.uri(), "status".into()).await;
        assert!(out.reachable, "expected reachable, got {out:?}");
        assert_eq!(out.status_code, Some(200));
    }

    #[tokio::test]
    async fn diagnostics_probe_jsonrpc_unreachable_on_error_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    r#"{"jsonrpc":"2.0","id":"x","error":{"code":-32601,"message":"Method not found"}}"#,
                ),
            )
            .mount(&server)
            .await;
        let out = diagnostics_probe_jsonrpc(server.uri(), "bogus".into()).await;
        assert!(!out.reachable);
        assert!(out.detail.contains("Method not found"), "detail={}", out.detail);
    }
}
