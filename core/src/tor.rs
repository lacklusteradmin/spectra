//! Embedded Tor client (Arti) + local SOCKS5 proxy.
//!
//! ## Lifecycle
//!
//! WalletService reconciles committed settings after runtime registration.
//! Front ends provide a cache directory, render status and request reconnects.
//! Superseded bootstrap completions cannot reinstall a stopped proxy.
//!
//! ## Stream isolation
//!
//! Every SOCKS5 connection receives its own Tor circuit via Arti's default
//! isolation policy (one stream per TCP connection). This means balance
//! checks for different wallets cannot be correlated by a Tor exit node.

use arti_client::config::CfgPath;
use arti_client::{TorClient, TorClientConfig};
use parking_lot::Mutex;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, LazyLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tor_rtcompat::PreferredRuntime;

// ── Public FFI types ─────────────────────────────────────────────────────────

/// Tor lifecycle status surfaced to Swift via UniFFI.
#[derive(Debug, Clone, serde::Serialize, uniffi::Enum)]
pub enum TorStatus {
    /// Tor is not running; HTTP goes direct.
    Stopped,
    /// Arti is bootstrapping. `percent` is 0–100.
    Bootstrapping { percent: u8 },
    /// Tor is up; all HTTP routes through the SOCKS5 proxy.
    Ready,
    /// Bootstrap or proxy failed. `message` has the detail.
    Error { message: String },
}

enum TorInternalState {
    Stopped,
    Bootstrapping {
        percent: Arc<AtomicU8>,
        task: tokio::task::JoinHandle<()>,
    },
    Running {
        // Keep the client alive so the Tor circuits stay open.
        _client: Arc<TorClient<PreferredRuntime>>,
        proxy_task: tokio::task::JoinHandle<()>,
    },
    /// User supplied their own SOCKS5 proxy (e.g. Orbot). Arti is not running.
    CustomProxy,
    Error {
        message: String,
    },
}

static TOR_STATE: LazyLock<Mutex<TorInternalState>> =
    LazyLock::new(|| Mutex::new(TorInternalState::Stopped));

// ── Policy ───────────────────────────────────────────────────────────────────
//
// What the user asked for, as opposed to what Tor is currently doing. Both are
// core state (`AppSettings::tor_enabled` / `tor_kill_switch`); the service
// pushes them here whenever that state changes, so the HTTP layer can consult
// them without reading the store on every request.

static TOR_WANTED: AtomicBool = AtomicBool::new(false);
static KILL_SWITCH: AtomicBool = AtomicBool::new(false);

/// Adopt the stored Tor policy. Called by the service on load and on change.
pub(crate) fn apply_policy(tor_enabled: bool, kill_switch: bool) {
    TOR_WANTED.store(tor_enabled, Ordering::Relaxed);
    KILL_SWITCH.store(kill_switch, Ordering::Relaxed);
}

/// True when the user asked for Tor with the kill switch on and Tor is not
/// carrying traffic — the moment a request would otherwise go out in the
/// clear.
///
/// The switch had no reader at all: it was a toggle in one front end's
/// settings, written to `UserDefaults`, and the screen's own footer promised
/// that "network requests are paused if the Tor circuit drops instead of
/// falling back to a direct connection". Nothing paused anything.
pub(crate) fn kill_switch_engaged() -> bool {
    // The loads short-circuit before the status lock, so a request pays only
    // an atomic read when the switch is off — which is every request until a
    // user turns it on.
    KILL_SWITCH.load(Ordering::Relaxed)
        && kill_switch_verdict(true, TOR_WANTED.load(Ordering::Relaxed), &tor_status())
}

/// Keep routing policy and the HTTP client snapshot in the same critical
/// section as transport switches. Otherwise a request can observe Ready,
/// then clone the direct client installed by a concurrent stop.
pub(crate) fn with_routing_guard<T>(read: impl FnOnce(bool) -> T) -> T {
    let state = TOR_STATE.lock();
    let blocked = KILL_SWITCH.load(Ordering::Relaxed)
        && TOR_WANTED.load(Ordering::Relaxed)
        && !matches!(
            *state,
            TorInternalState::Running { .. } | TorInternalState::CustomProxy
        );
    read(blocked)
}

/// The rule itself, over values rather than globals, so it can be asserted
/// without engaging a process-wide switch other tests share.
pub(crate) fn kill_switch_verdict(kill_switch: bool, tor_wanted: bool, status: &TorStatus) -> bool {
    kill_switch && tor_wanted && !matches!(status, TorStatus::Ready)
}

// ── FFI surface ──────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq, Eq)]
enum RuntimeConfiguration {
    Off,
    Embedded(String),
    Proxy(String),
}
static CONFIGURATION: LazyLock<Mutex<Option<RuntimeConfiguration>>> =
    LazyLock::new(|| Mutex::new(None));

/// Reconcile policy and transport atomically with respect to other reconciles.
/// The service calls this only with committed settings, under its state writer.
pub(crate) fn reconcile(
    settings: &crate::store::state::AppSettings,
    data_dir: &str,
    restart: bool,
) {
    let desired = if !settings.tor_enabled {
        RuntimeConfiguration::Off
    } else if settings.tor_use_custom_proxy {
        RuntimeConfiguration::Proxy(force_remote_dns(&settings.tor_custom_proxy_address))
    } else {
        RuntimeConfiguration::Embedded(data_dir.into())
    };
    let mut configuration = CONFIGURATION.lock();
    apply_policy(settings.tor_enabled, settings.tor_kill_switch);
    if !restart && configuration.as_ref() == Some(&desired) {
        return;
    }
    let mut state = TOR_STATE.lock();
    stop_runtime(&mut state);
    match &desired {
        RuntimeConfiguration::Off => {}
        RuntimeConfiguration::Proxy(url) => {
            crate::fetch::http::set_socks5_proxy(Some(url));
            *state = TorInternalState::CustomProxy;
        }
        RuntimeConfiguration::Embedded(dir) => {
            let percent = Arc::new(AtomicU8::new(0));
            let task = tokio::spawn(bootstrap_tor(dir.clone(), percent.clone()));
            *state = TorInternalState::Bootstrapping { percent, task };
        }
    }
    *configuration = Some(desired);
}

fn stop_runtime(state: &mut TorInternalState) {
    match std::mem::replace(state, TorInternalState::Stopped) {
        TorInternalState::Running { proxy_task, .. } => proxy_task.abort(),
        TorInternalState::Bootstrapping { task, .. } => task.abort(),
        _ => {}
    }
    crate::fetch::http::set_socks5_proxy(None);
}

/// Force proxy-side ("remote") DNS resolution by upgrading a plain `socks5://`
/// URL to `socks5h://`. With the leaky `socks5://` form, reqwest resolves the
/// target hostname *locally* before connecting to the proxy, so every chain
/// provider lookup leaks to the device resolver / ISP even while Tor is up —
/// defeating the whole point of routing through Tor. `socks5h://` makes the
/// proxy (Tor) perform the lookup. Any other scheme is passed through untouched.
fn force_remote_dns(socks5_url: &str) -> String {
    match socks5_url.strip_prefix("socks5://") {
        Some(rest) => format!("socks5h://{rest}"),
        None => socks5_url.to_string(),
    }
}

/// Poll the current Tor state. Cheap — just reads an atomic.
#[uniffi::export]
pub fn tor_status() -> TorStatus {
    match &*TOR_STATE.lock() {
        TorInternalState::Stopped => TorStatus::Stopped,
        TorInternalState::Bootstrapping { percent, .. } => TorStatus::Bootstrapping {
            percent: percent.load(Ordering::Relaxed),
        },
        TorInternalState::Running { .. } | TorInternalState::CustomProxy => TorStatus::Ready,
        TorInternalState::Error { message } => TorStatus::Error {
            message: message.clone(),
        },
    }
}

// ── Bootstrap task ───────────────────────────────────────────────────────────

fn is_current_bootstrap(state: &TorInternalState, percent: &Arc<AtomicU8>) -> bool {
    matches!(state, TorInternalState::Bootstrapping { percent: current, .. } if Arc::ptr_eq(current, percent))
}

async fn bootstrap_tor(data_dir: String, percent: Arc<AtomicU8>) {
    let result = async {
        let client = try_bootstrap(&data_dir, &percent).await?;
        // Bind before publishing Ready; use a free port to avoid collisions
        // with another Spectra process or a previous listener being aborted.
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| e.to_string())?;
        Ok::<_, String>((client, listener))
    }
    .await;
    let mut state = TOR_STATE.lock();
    if !is_current_bootstrap(&state, &percent) {
        return;
    }
    match result {
        Ok((client, listener)) => {
            let port = match listener.local_addr() {
                Ok(addr) => addr.port(),
                Err(error) => {
                    *state = TorInternalState::Error {
                        message: error.to_string(),
                    };
                    return;
                }
            };
            let client = Arc::new(client);
            let proxy_task = tokio::spawn(run_socks5_proxy(client.clone(), listener));
            crate::fetch::http::set_socks5_proxy(Some(&format!("socks5h://127.0.0.1:{port}")));
            *state = TorInternalState::Running {
                _client: client,
                proxy_task,
            };
        }
        Err(message) => *state = TorInternalState::Error { message },
    }
}

async fn try_bootstrap(
    data_dir: &str,
    percent: &Arc<AtomicU8>,
) -> Result<TorClient<PreferredRuntime>, String> {
    let mut builder = TorClientConfig::builder();
    builder
        .storage()
        .cache_dir(CfgPath::new(format!("{data_dir}/tor_cache")))
        .state_dir(CfgPath::new(format!("{data_dir}/tor_state")));
    let config = builder.build().map_err(|e| e.to_string())?;

    percent.store(5, Ordering::Relaxed);

    let client: TorClient<PreferredRuntime> = TorClient::builder()
        .config(config)
        .create_bootstrapped()
        .await
        .map_err(|e| e.to_string())?;

    percent.store(100, Ordering::Relaxed);
    Ok(client)
}

// ── SOCKS5 proxy server ──────────────────────────────────────────────────────
//
// A minimal SOCKS5 CONNECT-only server. reqwest sends CONNECT requests for
// HTTPS targets; we parse the target address and open a Tor circuit to it,
// then relay bytes bidirectionally.

async fn run_socks5_proxy(tor: Arc<TorClient<PreferredRuntime>>, listener: TcpListener) {
    while let Ok((stream, _)) = listener.accept().await {
        let tor = tor.clone();
        tokio::spawn(async move {
            let _ = handle_socks5(stream, tor).await;
        });
    }
}

async fn handle_socks5(
    mut tcp: TcpStream,
    tor: Arc<TorClient<PreferredRuntime>>,
) -> io::Result<()> {
    // ── 1. Greeting ──────────────────────────────────────────────────────────
    let mut hdr = [0u8; 2];
    tcp.read_exact(&mut hdr).await?;
    if hdr[0] != 5 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not SOCKS5"));
    }
    let nmethods = hdr[1] as usize;
    let mut methods = vec![0u8; nmethods];
    tcp.read_exact(&mut methods).await?;

    if !methods.contains(&0x00) {
        tcp.write_all(&[5, 0xFF]).await?;
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "no acceptable auth",
        ));
    }
    tcp.write_all(&[5, 0x00]).await?; // no-auth accepted

    // ── 2. CONNECT request ───────────────────────────────────────────────────
    let mut req = [0u8; 4];
    tcp.read_exact(&mut req).await?;
    // req: [VER=5, CMD, RSV, ATYP]
    if req[0] != 5 || req[1] != 0x01 {
        // Only CONNECT (0x01) is supported.
        tcp.write_all(&[5, 0x07, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "only CONNECT supported",
        ));
    }

    let host: String = match req[3] {
        0x01 => {
            // IPv4
            let mut a = [0u8; 4];
            tcp.read_exact(&mut a).await?;
            format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3])
        }
        0x03 => {
            // Domain name
            let mut len = [0u8; 1];
            tcp.read_exact(&mut len).await?;
            let mut domain = vec![0u8; len[0] as usize];
            tcp.read_exact(&mut domain).await?;
            String::from_utf8(domain)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad domain encoding"))?
        }
        0x04 => {
            // IPv6
            let mut a = [0u8; 16];
            tcp.read_exact(&mut a).await?;
            std::net::Ipv6Addr::from(a).to_string()
        }
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "unknown ATYP")),
    };

    let mut port_buf = [0u8; 2];
    tcp.read_exact(&mut port_buf).await?;
    let port = u16::from_be_bytes(port_buf);

    // ── 3. Open Tor circuit to target ────────────────────────────────────────
    let tor_stream = tor
        .connect((host.as_str(), port))
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, e.to_string()))?;

    // ── 4. Success reply ─────────────────────────────────────────────────────
    // VER REP RSV ATYP BND.ADDR(0.0.0.0) BND.PORT(0)
    tcp.write_all(&[5, 0x00, 0, 0x01, 0, 0, 0, 0, 0, 0]).await?;

    // ── 5. Relay ─────────────────────────────────────────────────────────────
    // DataStream implements futures::AsyncRead/Write, not tokio's. Wrap it
    // with tokio_util::compat so copy_bidirectional gets its Unpin + tokio
    // trait bounds satisfied.
    let mut tor_compat = Box::pin(tor_stream.compat());
    tokio::io::copy_bidirectional(&mut tcp, &mut tor_compat).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::force_remote_dns;

    #[tokio::test]
    async fn stopped_or_replaced_bootstrap_cannot_publish_a_late_completion() {
        use super::*;
        let old = Arc::new(AtomicU8::new(0));
        let task = tokio::spawn(std::future::pending());
        let abort = task.abort_handle();
        let mut state = TorInternalState::Bootstrapping {
            percent: old.clone(),
            task,
        };
        assert!(is_current_bootstrap(&state, &old));
        stop_runtime(&mut state);
        tokio::task::yield_now().await;
        assert!(abort.is_finished());
        assert!(!is_current_bootstrap(&state, &old));
        state = TorInternalState::CustomProxy;
        assert!(!is_current_bootstrap(&state, &old));
        let new = Arc::new(AtomicU8::new(0));
        state = TorInternalState::Bootstrapping {
            percent: new.clone(),
            task: tokio::spawn(std::future::pending()),
        };
        assert!(!is_current_bootstrap(&state, &old));
        assert!(is_current_bootstrap(&state, &new));
        stop_runtime(&mut state);
    }

    #[test]
    fn upgrades_leaky_socks5_scheme() {
        assert_eq!(
            force_remote_dns("socks5://127.0.0.1:9150"),
            "socks5h://127.0.0.1:9150"
        );
    }

    #[test]
    fn leaves_remote_dns_scheme_untouched() {
        assert_eq!(
            force_remote_dns("socks5h://127.0.0.1:9150"),
            "socks5h://127.0.0.1:9150"
        );
        // Non-socks schemes are passed through verbatim.
        assert_eq!(
            force_remote_dns("http://127.0.0.1:8080"),
            "http://127.0.0.1:8080"
        );
    }
}
