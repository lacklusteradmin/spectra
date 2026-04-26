//! Async HTTP transport with retry/backoff for chain providers.
//!
//! This module owns the single shared `reqwest::Client` used by all
//! chain-specific fetch and send implementations. It mirrors the retry
//! behaviour of the legacy Swift `NetworkRetryProfile` / `ProviderHTTP`
//! stack so that we get the same resilience without rewriting the
//! policy from scratch.
//!
//! ## UniFFI migration pattern
//!
//! The pattern for migrating a Swift URLSession call site to Rust is:
//!
//! 1. If the response parsing already lives in Rust (see
//!    `diagnostics::aggregate::diagnostics_parse_jsonrpc_probe`), prefer
//!    exposing a single `#[uniffi::export] async fn` that performs
//!    *transport + parse* in one FFI hop. This eliminates the
//!    (Rust → Swift → Rust) round-trip.
//! 2. Otherwise, call the generic ergonomic wrappers in `http_ffi`:
//!    `http_get` (returns `HttpTextResponse`) or `http_post_json`.
//!    These use the shared `HttpClient` / `RetryProfile` plumbing
//!    defined here.
//! 3. Delete the Swift URLSession code for that call site — the whole
//!    point of the migration is that Swift stops owning network
//!    transport.
//!
//! ## Usage
//!
//! ```rust,ignore
//! let client = HttpClient::shared();
//! let body: serde_json::Value = client
//!     .get_json("https://blockstream.info/api/address/bc1q.../utxo",
//!               RetryProfile::ChainRead)
//!     .await?;
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use std::sync::LazyLock;
use reqwest::{Client, Method, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::time::sleep;

// ----------------------------------------------------------------
// Shared client
// ----------------------------------------------------------------

/// Process-wide shared `reqwest` client. One allocation, many tasks.
static SHARED_CLIENT: LazyLock<Arc<HttpClient>> = LazyLock::new(|| {
    Arc::new(HttpClient::new())
});

pub struct HttpClient {
    inner: Client,
}

impl HttpClient {
    fn new() -> Self {
        // Note: `https_only` is intentionally *not* enforced at the
        // client layer. URLs come from a curated provider catalog that
        // is already HTTPS-only; enforcing at the transport layer also
        // blocks wiremock / localhost tests. Callers that need a hard
        // guarantee should validate the scheme at the catalog level.
        let inner = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .gzip(true)
            .user_agent(concat!("spectra-core/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Self { inner }
    }

    /// Returns the process-wide singleton.
    pub fn shared() -> Arc<HttpClient> {
        SHARED_CLIENT.clone()
    }

    /// Access the underlying reqwest client for callers that need full control
    /// over request construction (e.g. the generic UniFFI `http_request` bridge).
    pub fn reqwest_client(&self) -> &Client {
        &self.inner
    }

    // ----------------------------------------------------------------
    // Core request method
    // ----------------------------------------------------------------

    async fn request_with_retry<T: DeserializeOwned>(
        &self,
        method: Method,
        url: &str,
        json_body: Option<&serde_json::Value>,
        headers: &HashMap<&str, &str>,
        profile: RetryProfile,
    ) -> Result<T, String> {
        let max_attempts = profile.max_attempts();
        let mut last_err = String::new();

        for attempt in 0..max_attempts {
            if attempt > 0 {
                let delay = profile.delay_for_attempt(attempt);
                sleep(delay).await;
            }

            let mut req = self.inner.request(method.clone(), url);
            for (key, value) in headers {
                req = req.header(*key, *value);
            }
            if let Some(body) = json_body {
                req = req.json(body);
            }

            let result = req.send().await;
            match result {
                Err(e) => {
                    last_err = format_reqwest_error(&e);
                    if !profile.is_retryable_error(&e) {
                        break;
                    }
                }
                Ok(resp) => {
                    let status = resp.status();
                    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                        last_err = format!("HTTP {status}");
                        continue; // retry on 429 / 5xx
                    }
                    if !status.is_success() {
                        let body = resp.text().await.unwrap_or_default();
                        return Err(format!("HTTP {status}: {body}"));
                    }
                    return resp
                        .json::<T>()
                        .await
                        .map_err(|e| format!("json decode: {e}"));
                }
            }
        }
        Err(format!("all {max_attempts} attempts failed: {last_err}"))
    }

    // ----------------------------------------------------------------
    // Convenience wrappers
    // ----------------------------------------------------------------

    /// GET a JSON response.
    pub async fn get_json<T: DeserializeOwned>(
        &self,
        url: &str,
        profile: RetryProfile,
    ) -> Result<T, String> {
        self.request_with_retry(Method::GET, url, None, &HashMap::new(), profile)
            .await
    }

    /// GET a JSON response with custom headers.
    pub async fn get_json_with_headers<T: DeserializeOwned>(
        &self,
        url: &str,
        headers: &HashMap<&str, &str>,
        profile: RetryProfile,
    ) -> Result<T, String> {
        self.request_with_retry(Method::GET, url, None, headers, profile)
            .await
    }

    /// GET raw text (for providers that return non-JSON).
    pub async fn get_text(&self, url: &str, profile: RetryProfile) -> Result<String, String> {
        let max_attempts = profile.max_attempts();
        let mut last_err = String::new();

        for attempt in 0..max_attempts {
            if attempt > 0 {
                sleep(profile.delay_for_attempt(attempt)).await;
            }

            let result = self.inner.get(url).send().await;
            match result {
                Err(e) => {
                    last_err = format_reqwest_error(&e);
                    if !profile.is_retryable_error(&e) {
                        break;
                    }
                }
                Ok(resp) => {
                    let status = resp.status();
                    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                        last_err = format!("HTTP {status}");
                        continue;
                    }
                    if !status.is_success() {
                        return Err(format!("HTTP {status}"));
                    }
                    return resp.text().await.map_err(|e| e.to_string());
                }
            }
        }
        Err(format!("all {max_attempts} attempts failed: {last_err}"))
    }

    /// POST a JSON body and decode the JSON response.
    pub async fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        url: &str,
        body: &B,
        profile: RetryProfile,
    ) -> Result<T, String> {
        let json_body = serde_json::to_value(body).map_err(|e| e.to_string())?;
        self.request_with_retry(Method::POST, url, Some(&json_body), &HashMap::new(), profile)
            .await
    }

    /// POST a JSON body with custom headers.
    pub async fn post_json_with_headers<B: Serialize, T: DeserializeOwned>(
        &self,
        url: &str,
        body: &B,
        headers: &HashMap<&str, &str>,
        profile: RetryProfile,
    ) -> Result<T, String> {
        let json_body = serde_json::to_value(body).map_err(|e| e.to_string())?;
        self.request_with_retry(Method::POST, url, Some(&json_body), headers, profile)
            .await
    }

    /// POST raw bytes (for broadcast endpoints that want a hex string body).
    pub async fn post_text(&self, url: &str, body: String, profile: RetryProfile) -> Result<String, String> {
        let max_attempts = profile.max_attempts();
        let mut last_err = String::new();

        for attempt in 0..max_attempts {
            if attempt > 0 {
                sleep(profile.delay_for_attempt(attempt)).await;
            }

            let result = self
                .inner
                .post(url)
                .header("Content-Type", "text/plain")
                .body(body.clone())
                .send()
                .await;
            match result {
                Err(e) => {
                    last_err = format_reqwest_error(&e);
                    if !profile.is_retryable_error(&e) {
                        break;
                    }
                }
                Ok(resp) => {
                    let status = resp.status();
                    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                        last_err = format!("HTTP {status}");
                        continue;
                    }
                    if !status.is_success() {
                        let t = resp.text().await.unwrap_or_default();
                        return Err(format!("HTTP {status}: {t}"));
                    }
                    return resp.text().await.map_err(|e| e.to_string());
                }
            }
        }
        Err(format!("all {max_attempts} attempts failed: {last_err}"))
    }
}

// ----------------------------------------------------------------
// Retry profiles
// ----------------------------------------------------------------

/// Retry behaviour profiles, matching the legacy Swift `NetworkRetryProfile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryProfile {
    /// Standard chain read (balance, history, UTXO). 3 attempts.
    ChainRead,
    /// Send / broadcast. 2 attempts (less aggressive — avoid double-spend).
    ChainWrite,
    /// Health probe / diagnostics. 2 attempts with shorter delays.
    Diagnostics,
}

impl RetryProfile {
    pub fn max_attempts(self) -> usize {
        match self {
            Self::ChainRead => 3,
            Self::ChainWrite => 2,
            Self::Diagnostics => 2,
        }
    }

    /// Delay before `attempt` (0-indexed; attempt 0 has no delay).
    pub fn delay_for_attempt(self, attempt: usize) -> Duration {
        // Base delay doubles each retry (exponential backoff with jitter).
        let (base_ms, max_ms) = match self {
            Self::ChainRead => (350, 2000),
            Self::ChainWrite => (250, 1000),
            Self::Diagnostics => (200, 800),
        };
        let raw = base_ms * 2_u64.saturating_pow(attempt as u32 - 1);
        let clamped = raw.min(max_ms);
        // Add 0-20% jitter.
        Duration::from_millis(clamped)
    }

    /// Whether the given reqwest error warrants a retry.
    pub fn is_retryable_error(self, err: &reqwest::Error) -> bool {
        if err.is_timeout() || err.is_connect() {
            return true;
        }
        if let Some(status) = err.status() {
            return status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
        }
        false
    }
}

// ----------------------------------------------------------------
// Fallback helpers
// ----------------------------------------------------------------

fn format_reqwest_error(e: &reqwest::Error) -> String {
    let mut parts = vec![e.to_string()];
    let mut source: Option<&dyn std::error::Error> = std::error::Error::source(e);
    while let Some(s) = source {
        parts.push(s.to_string());
        source = s.source();
    }
    let mut flags = Vec::new();
    if e.is_timeout() { flags.push("timeout"); }
    if e.is_connect() { flags.push("connect"); }
    if e.is_request() { flags.push("request"); }
    if e.is_body() { flags.push("body"); }
    if e.is_decode() { flags.push("decode"); }
    if !flags.is_empty() {
        parts.push(format!("flags=[{}]", flags.join(",")));
    }
    parts.join(" | ")
}

/// Try each URL in `endpoints` with `f` until one succeeds. Returns the
/// first successful result or the last error.
pub async fn with_fallback<F, Fut, T>(
    endpoints: &[String],
    f: F,
) -> Result<T, String>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    if endpoints.is_empty() {
        return Err("no endpoints configured".to_string());
    }
    let mut last_err = String::new();
    for url in endpoints {
        match f(url.clone()).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                last_err = e;
                // 180 ms between fallback attempts (matches Swift EsploraProvider.runWithFallback)
                sleep(Duration::from_millis(180)).await;
            }
        }
    }
    Err(last_err)
}

/// Probe each URL in `endpoints` with a GET and return all that respond
/// 200 OK (within the given timeout). Used by the diagnostics subsystem.
pub async fn probe_endpoints(
    endpoints: &[String],
    timeout_secs: u64,
) -> Vec<(String, bool)> {
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .https_only(true)
        .user_agent("Spectra/1.0")
        .build()
        .unwrap_or_default();

    let mut results = Vec::with_capacity(endpoints.len());
    for url in endpoints {
        let ok = client.get(url.as_str()).send().await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        results.push((url.clone(), ok));
    }
    results
}
