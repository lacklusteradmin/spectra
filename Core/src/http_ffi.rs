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
    LitecoinRead,
    LitecoinWrite,
    LitecoinDiagnostics,
}

impl From<HttpRetryProfile> for RetryProfile {
    fn from(value: HttpRetryProfile) -> Self {
        match value {
            HttpRetryProfile::ChainRead => RetryProfile::ChainRead,
            HttpRetryProfile::ChainWrite => RetryProfile::ChainWrite,
            HttpRetryProfile::Diagnostics => RetryProfile::Diagnostics,
            HttpRetryProfile::LitecoinRead => RetryProfile::LitecoinRead,
            HttpRetryProfile::LitecoinWrite => RetryProfile::LitecoinWrite,
            HttpRetryProfile::LitecoinDiagnostics => RetryProfile::LitecoinDiagnostics,
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
}

fn parse_method(method: &str) -> Result<Method, HttpError> {
    Method::from_bytes(method.to_uppercase().as_bytes())
        .map_err(|_| HttpError::InvalidMethod { method: method.to_string() })
}

#[uniffi::export]
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

// Lightweight single-shot probe (no retry). Used by endpoint health checks.
#[uniffi::export]
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
