//! The one shared `reqwest::Client` and the retry policy every provider call
//! goes through.
//!
//! When moving a call site off Swift's URLSession: if the parsing already
//! lives here, export *transport + parse* as one async fn rather than handing
//! Swift a body to send back.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use reqwest::{Client, Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::sync::LazyLock;
use tokio::time::sleep;

use crate::api::error::ApiError;

// ── Shared client

/// Process-wide shared `reqwest` client. Interior `RwLock` lets `tor.rs`
/// swap in a SOCKS5-proxied client at runtime without touching call sites.
static SHARED_CLIENT: LazyLock<Arc<HttpClient>> = LazyLock::new(|| Arc::new(HttpClient::new(None)));

pub struct HttpClient {
    inner: RwLock<Client>,
}

fn build_reqwest_client(proxy_url: Option<&str>) -> Client {
    // Note: `https_only` is intentionally *not* enforced at the client layer.
    // URLs come from a curated provider catalog that is already HTTPS-only;
    // enforcing at the transport layer also blocks wiremock / localhost tests.
    let mut builder = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .gzip(true)
        .user_agent(concat!("spectra-core/", env!("CARGO_PKG_VERSION")));
    // Under test, keep no idle connections.
    //
    // This client is a process-wide singleton, and hyper binds each pooled
    // connection's dispatch task to whichever tokio runtime first drove it.
    // The app has one runtime for its lifetime, so pooling is free there. A
    // test binary has one runtime *per test*, each dropped when its test ends,
    // so a later test reusing a pooled connection gets "runtime dropped the
    // dispatch task" — a failure with nothing to do with what it was testing.
    #[cfg(test)]
    {
        builder = builder.pool_max_idle_per_host(0);
    }
    // Listed first, so it sees every request before a user's proxy does.
    if LOOPBACK_JOURNAL.is_some() {
        builder = builder.proxy(reqwest::Proxy::custom(|url| {
            (!is_loopback(url) && refuse_non_loopback(url.as_str())).then_some(DEAD_PROXY)
        }));
    }
    if let Some(url) = proxy_url {
        match reqwest::Proxy::all(url) {
            Ok(proxy) => builder = builder.proxy(proxy),
            // Fail closed: a proxy (e.g. Tor) was requested but is unusable.
            // Returning a direct client here would silently de-anonymize the
            // user over clearnet, so block all traffic instead.
            Err(_) => return build_blocked_client(),
        }
    }
    match builder.build() {
        Ok(client) => client,
        // Same fail-closed posture if the proxied client fails to build.
        Err(_) if proxy_url.is_some() || LOOPBACK_JOURNAL.is_some() => build_blocked_client(),
        Err(_) => Client::new(),
    }
}

/// The blocked client, built once — the kill switch reaches for it per request.
static BLOCKED_CLIENT: LazyLock<Client> = LazyLock::new(build_blocked_client);

/// The message a caller sees when the Tor kill switch stopped the request.
pub(crate) const KILL_SWITCH_MESSAGE: &str =
    "Tor kill switch: the circuit is not ready, so the request was not sent.";

/// A proxy nothing listens on, so whatever is routed to it cannot connect.
const DEAD_PROXY: &str = "socks5h://127.0.0.1:1";

/// A client that cannot reach the network: every request is routed to a dead
/// loopback port. Used when a proxy was requested but could not be applied, so
/// connectivity breaks loudly instead of leaking over a direct connection.
fn build_blocked_client() -> Client {
    Client::builder()
        .proxy(reqwest::Proxy::all(DEAD_PROXY).expect("static proxy url is valid"))
        .build()
        .unwrap_or_else(|_| Client::new())
}

// ── Loopback-only mode

/// Names a journal file and confines the process to loopback hosts.
///
/// Set by `scripts/cli-acceptance.sh`, which promises no external network: a
/// check that reached a live provider would pass or fail on the day's chain
/// state rather than on core's rules. Endpoint selection leaves remote
/// endpoints out ([`may_contact`]), so a chain is served by its loopback
/// fixtures alone. Any other destination is refused, never sent, and
/// journaled with the command that tried it, so the suite fails naming it
/// even when the refusal resembled the offline error the check expected.
const LOOPBACK_ONLY_ENV: &str = "SPECTRA_LOOPBACK_ONLY";

static LOOPBACK_JOURNAL: LazyLock<Option<std::path::PathBuf>> = LazyLock::new(|| {
    std::env::var_os(LOOPBACK_ONLY_ENV)
        .filter(|path| !path.is_empty())
        .map(Into::into)
});

fn is_loopback(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Whether this process may contact `url`: always, unless loopback-only
/// mode confines it to loopback hosts.
pub(crate) fn may_contact(url: &str) -> bool {
    LOOPBACK_JOURNAL.is_none() || reqwest::Url::parse(url).is_ok_and(|url| is_loopback(&url))
}

/// In loopback-only mode, journal `target` and return true: the caller must
/// not reach it. Outside that mode, return false.
pub(crate) fn refuse_non_loopback(target: &str) -> bool {
    use std::io::Write;
    let Some(journal) = LOOPBACK_JOURNAL.as_ref() else {
        return false;
    };
    let command = std::env::args().collect::<Vec<_>>().join(" ");
    // One write per line: raced requests journal concurrently, and an
    // appended write is only whole when it is a single one.
    let line = format!("{target}\t{command}\n");
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(journal)
        .and_then(|mut file| file.write_all(line.as_bytes()));
    if written.is_err() {
        // A refusal nobody hears about is the silent pass this mode exists
        // to prevent.
        eprintln!("{LOOPBACK_ONLY_ENV}: refused {target} and could not journal it");
        std::process::exit(70);
    }
    true
}

impl HttpClient {
    /// Read a REST resource from whichever base URL answers first.
    pub(crate) async fn get_path<T: serde::de::DeserializeOwned>(
        &self,
        endpoints: &[String],
        path: &str,
    ) -> Result<T, ApiError> {
        race(endpoints, |base| async move {
            self.get_json(
                &format!("{}{}", base.trim_end_matches('/'), path),
                RetryProfile::ChainRead,
            )
            .await
        })
        .await
    }

    fn new(proxy_url: Option<&str>) -> Self {
        Self {
            inner: RwLock::new(build_reqwest_client(proxy_url)),
        }
    }

    /// Returns the process-wide singleton.
    pub fn shared() -> Arc<HttpClient> {
        SHARED_CLIENT.clone()
    }

    /// Rebuild the inner reqwest client with a new proxy setting.
    /// Called by `tor.rs` when Tor comes up or is stopped.
    pub(crate) fn set_proxy(&self, proxy_url: Option<&str>) {
        *self.inner.write() = build_reqwest_client(proxy_url);
    }

    /// Clone the inner reqwest client (cheap — it is an Arc internally).
    /// Callers must NOT hold this across a lock boundary.
    fn get_client(&self) -> Client {
        // The one choke point every request passes through, so a caller that
        // reaches for a client without checking the guards below still cannot
        // send in the clear.
        crate::tor::with_routing_guard(|blocked| {
            if blocked {
                BLOCKED_CLIENT.clone()
            } else {
                self.inner.read().clone()
            }
        })
    }

    /// Access the underlying reqwest client for callers that need full control
    /// over request construction (e.g. the generic UniFFI `http_request` bridge).
    pub fn reqwest_client(&self) -> Client {
        self.get_client()
    }

    // ── Core request method

    /// Send one request, retrying transport failures, 429 and 5xx as `profile`
    /// allows, and return the first success. Any other status is the
    /// service's answer and is not retried.
    async fn send_with_retry(
        &self,
        build: impl Fn(&Client) -> reqwest::RequestBuilder,
        profile: RetryProfile,
    ) -> Result<reqwest::Response, ApiError> {
        if crate::tor::kill_switch_engaged() {
            return Err(ApiError::Transport(KILL_SWITCH_MESSAGE.to_string()));
        }
        let max_attempts = profile.max_attempts();
        let mut last_err = String::new();

        for attempt in 0..max_attempts {
            if attempt > 0 {
                sleep(profile.delay_for_attempt(attempt)).await;
            }
            match build(&self.get_client()).send().await {
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
                        return Err(ApiError::Status {
                            status: status.as_u16(),
                            body: resp.text().await.unwrap_or_default(),
                        });
                    }
                    return Ok(resp);
                }
            }
        }
        Err(ApiError::Transport(format!(
            "all {max_attempts} attempts failed: {last_err}"
        )))
    }

    async fn request_with_retry<T: DeserializeOwned>(
        &self,
        method: Method,
        url: &str,
        json_body: Option<&serde_json::Value>,
        headers: &HashMap<&str, &str>,
        profile: RetryProfile,
    ) -> Result<T, ApiError> {
        let resp = self
            .send_with_retry(
                |client| {
                    let mut req = client.request(method.clone(), url);
                    for (key, value) in headers {
                        req = req.header(*key, *value);
                    }
                    if let Some(body) = json_body {
                        req = req.json(body);
                    }
                    req
                },
                profile,
            )
            .await?;
        resp.json::<T>()
            .await
            .map_err(|e| ApiError::Decode(format!("json decode: {e}")))
    }

    // ── Convenience wrappers

    /// GET a JSON response.
    pub async fn get_json<T: DeserializeOwned>(
        &self,
        url: &str,
        profile: RetryProfile,
    ) -> Result<T, ApiError> {
        self.request_with_retry(Method::GET, url, None, &HashMap::new(), profile)
            .await
    }

    /// GET a JSON response with custom headers.
    pub async fn get_json_with_headers<T: DeserializeOwned>(
        &self,
        url: &str,
        headers: &HashMap<&str, &str>,
        profile: RetryProfile,
    ) -> Result<T, ApiError> {
        self.request_with_retry(Method::GET, url, None, headers, profile)
            .await
    }

    /// POST a JSON body and decode the JSON response.
    pub async fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        url: &str,
        body: &B,
        profile: RetryProfile,
    ) -> Result<T, ApiError> {
        let json_body = serde_json::to_value(body).map_err(ApiError::invalid)?;
        self.request_with_retry(
            Method::POST,
            url,
            Some(&json_body),
            &HashMap::new(),
            profile,
        )
        .await
    }

    /// POST a raw body and return the successful answer's status and bytes.
    pub(crate) async fn post_bytes(
        &self,
        url: &str,
        content_type: &str,
        body: Vec<u8>,
        profile: RetryProfile,
    ) -> Result<(u16, Vec<u8>), ApiError> {
        let resp = self
            .send_with_retry(
                |client| {
                    client
                        .post(url)
                        .header("Content-Type", content_type)
                        .body(body.clone())
                },
                profile,
            )
            .await?;
        let status = resp.status().as_u16();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ApiError::Transport(format_reqwest_error(&e)))?;
        Ok((status, bytes.to_vec()))
    }

    /// POST a text body (for broadcast endpoints that want a hex string).
    pub async fn post_text(
        &self,
        url: &str,
        body: String,
        profile: RetryProfile,
    ) -> Result<String, ApiError> {
        let (_, bytes) = self
            .post_bytes(url, "text/plain", body.into_bytes(), profile)
            .await?;
        String::from_utf8(bytes).map_err(ApiError::decode)
    }
}

// ── Retry profiles

/// Retry behaviour profiles, selected per call site by how costly a retry is.
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

// ── Tor proxy switch (called by tor.rs)

/// Replace the shared reqwest client with one that routes all traffic
/// through the given SOCKS5 URL, or remove the proxy when `None`.
/// Called by `crate::tor` when Arti finishes bootstrapping / is stopped.
pub(crate) fn set_socks5_proxy(proxy_url: Option<&str>) {
    SHARED_CLIENT.set_proxy(proxy_url);
}

// ── Fallback helpers

fn format_reqwest_error(e: &reqwest::Error) -> String {
    let mut parts = vec![e.to_string()];
    let mut source: Option<&dyn std::error::Error> = std::error::Error::source(e);
    while let Some(s) = source {
        parts.push(s.to_string());
        source = s.source();
    }
    let mut flags = Vec::new();
    if e.is_timeout() {
        flags.push("timeout");
    }
    if e.is_connect() {
        flags.push("connect");
    }
    if e.is_request() {
        flags.push("request");
    }
    if e.is_body() {
        flags.push("body");
    }
    if e.is_decode() {
        flags.push("decode");
    }
    if !flags.is_empty() {
        parts.push(format!("flags=[{}]", flags.join(",")));
    }
    parts.join(" | ")
}

/// Ask every URL in `endpoints` at once and answer with the first success.
///
/// No endpoint is preferred and none waits for another to fail: a dead one
/// costs nothing while any other answers. The result is an error only when
/// every endpoint failed, and then it is the last failure.
pub async fn race<F, Fut, T, E>(endpoints: &[String], f: F) -> Result<T, E>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: From<ApiError>,
{
    first_success(endpoints.iter().cloned().map(f)).await
}

/// Run every request at once; the first `Ok` wins and the rest are dropped.
pub(crate) async fn first_success<Fut, T, E>(
    requests: impl IntoIterator<Item = Fut>,
) -> Result<T, E>
where
    Fut: std::future::Future<Output = Result<T, E>>,
    E: From<ApiError>,
{
    use futures::StreamExt;
    let mut pending: futures::stream::FuturesUnordered<Fut> = requests.into_iter().collect();
    let mut last_err = None;
    while let Some(result) = pending.next().await {
        match result {
            Ok(value) => return Ok(value),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| ApiError::NoEndpoint.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn loopback_hosts_are_only_the_local_machine() {
        let loopback = |url: &str| is_loopback(&reqwest::Url::parse(url).unwrap());
        for url in [
            "http://127.0.0.1:8545",
            "http://127.9.9.9/",
            "http://localhost:3000/rpc",
            "http://LOCALHOST",
            "http://[::1]:9000",
        ] {
            assert!(loopback(url), "{url}");
        }
        for url in [
            "https://rpc.polkadot.io",
            "http://10.0.0.2:9050",
            "http://[::2]/",
            "http://localhost.example",
            "http://127.0.0.1.nip.io",
        ] {
            assert!(!loopback(url), "{url}");
        }
    }

    #[tokio::test]
    async fn post_bytes_sends_body_and_content_type() {
        use wiremock::matchers::header;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/rpc"))
            .and(header("Content-Type", "application/cbor"))
            .respond_with(ResponseTemplate::new(202).set_body_string("ok"))
            .mount(&server)
            .await;
        let (status, body) = HttpClient::shared()
            .post_bytes(
                &format!("{}/rpc", server.uri()),
                "application/cbor",
                vec![1, 2],
                RetryProfile::Diagnostics,
            )
            .await
            .expect("ok");
        assert_eq!((status, body.as_slice()), (202, b"ok".as_slice()));
    }

    /// A refusal is the service's answer and keeps its status; only transport
    /// failures and 429/5xx are worth another attempt.
    #[tokio::test]
    async fn a_client_error_status_is_returned_not_retried() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404).set_body_string("gone"))
            .expect(1)
            .mount(&server)
            .await;
        let err = HttpClient::shared()
            .get_json::<serde_json::Value>(&server.uri(), RetryProfile::ChainRead)
            .await
            .unwrap_err();
        assert_eq!(
            err,
            ApiError::Status {
                status: 404,
                body: "gone".into()
            }
        );
        assert!(!err.is_transient());
    }

    /// Every endpoint is asked at once: a slow or dead one listed first does
    /// not hold up one that answers, and only when all fail is it an error.
    #[tokio::test]
    async fn race_answers_with_whichever_endpoint_succeeds_first() {
        let slow = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("slow")
                    .set_delay(Duration::from_secs(5)),
            )
            .mount(&slow)
            .await;
        let fast = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("fast"))
            .mount(&fast)
            .await;
        let get = |url: String| async move {
            HttpClient::shared()
                .reqwest_client()
                .get(url)
                .send()
                .await
                .map_err(|e| ApiError::Transport(e.to_string()))?
                .text()
                .await
                .map_err(|e| ApiError::Transport(e.to_string()))
        };
        let started = std::time::Instant::now();
        let answer = race(&[slow.uri(), fast.uri()], get).await.unwrap();
        assert_eq!(answer, "fast");
        assert!(started.elapsed() < Duration::from_secs(2));

        let failing = |url: String| async move { Err::<(), _>(ApiError::Transport(url)) };
        assert!(race(&["a".into(), "b".into()], failing).await.is_err());
        assert_eq!(race(&[], failing).await, Err(ApiError::NoEndpoint));
    }
}
