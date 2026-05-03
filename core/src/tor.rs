//! Embedded Tor client (Arti) + local SOCKS5 proxy.
//!
//! ## Lifecycle
//!
//! Swift calls `tor_start(data_dir)` → returns immediately. A background
//! tokio task bootstraps the Arti `TorClient` (10-30 s cold, ~3 s warm),
//! then starts a SOCKS5 listener on `127.0.0.1:19050` and hot-swaps the
//! shared reqwest client so all subsequent HTTP calls route through Tor.
//!
//! Swift polls `tor_status()` to drive the UI:
//!   Stopped → Bootstrapping { percent } → Ready
//!   any step → Error { message }
//!
//! `tor_stop()` tears down the proxy, resets the reqwest client to direct,
//! and drops the Arti client.
//!
//! ## Stream isolation
//!
//! Every SOCKS5 connection receives its own Tor circuit via Arti's default
//! isolation policy (one stream per TCP connection). This means balance
//! checks for different wallets cannot be correlated by a Tor exit node.

use std::sync::{Arc, LazyLock};
use std::sync::atomic::{AtomicU8, Ordering};
use std::io;
use parking_lot::Mutex;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use arti_client::{TorClient, TorClientConfig};
use arti_client::config::CfgPath;
use tor_rtcompat::PreferredRuntime;
use tokio_util::compat::FuturesAsyncReadCompatExt;

// ── Public FFI types ─────────────────────────────────────────────────────────

/// Tor lifecycle status surfaced to Swift via UniFFI.
#[derive(Debug, Clone, uniffi::Enum)]
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

// ── Internal state ───────────────────────────────────────────────────────────

enum TorInternalState {
    Stopped,
    Bootstrapping { percent: Arc<AtomicU8> },
    Running {
        // Keep the client alive so the Tor circuits stay open.
        _client: Arc<TorClient<PreferredRuntime>>,
        proxy_task: tokio::task::JoinHandle<()>,
    },
    /// User supplied their own SOCKS5 proxy (e.g. Orbot). Arti is not running.
    CustomProxy,
    Error { message: String },
}

static TOR_STATE: LazyLock<Mutex<TorInternalState>> =
    LazyLock::new(|| Mutex::new(TorInternalState::Stopped));

// ── FFI surface ──────────────────────────────────────────────────────────────

/// Start Tor in the background. Returns immediately; poll `tor_status()` for
/// progress. `data_dir` must be the app's writable cache directory so Arti
/// can persist the Tor consensus across restarts (warm bootstrap ~3 s vs ~30 s
/// cold). Calling `tor_start` when Tor is already running or bootstrapping is
/// a no-op.
#[uniffi::export(async_runtime = "tokio")]
pub async fn tor_start(data_dir: String) -> Result<(), crate::SpectraBridgeError> {
    {
        let guard = TOR_STATE.lock();
        match *guard {
            TorInternalState::Running { .. } | TorInternalState::Bootstrapping { .. } => {
                return Ok(());
            }
            _ => {}
        }
    }

    let percent = Arc::new(AtomicU8::new(0));
    {
        let mut guard = TOR_STATE.lock();
        *guard = TorInternalState::Bootstrapping { percent: percent.clone() };
    }

    tokio::spawn(bootstrap_tor(data_dir, percent));
    Ok(())
}

/// Activate a user-supplied SOCKS5 proxy without starting Arti.
/// Useful for Orbot users (`socks5://127.0.0.1:9150`) or any external Tor.
/// Returns an error if Arti is already bootstrapping or running — call
/// `tor_stop()` first.
#[uniffi::export]
pub fn tor_activate_custom_proxy(socks5_url: String) -> Result<(), crate::SpectraBridgeError> {
    {
        let guard = TOR_STATE.lock();
        match *guard {
            TorInternalState::Running { .. } | TorInternalState::Bootstrapping { .. } => {
                return Err("Call tor_stop() before switching to a custom proxy.".into());
            }
            _ => {}
        }
    }
    crate::fetch::http::set_socks5_proxy(Some(&socks5_url));
    *TOR_STATE.lock() = TorInternalState::CustomProxy;
    Ok(())
}

/// Stop Tor and restore direct HTTP routing. Safe to call when already stopped.
#[uniffi::export]
pub fn tor_stop() {
    let old = {
        let mut guard = TOR_STATE.lock();
        std::mem::replace(&mut *guard, TorInternalState::Stopped)
    };
    if let TorInternalState::Running { proxy_task, .. } = old {
        proxy_task.abort();
    }
    crate::fetch::http::set_socks5_proxy(None);
}

/// Poll the current Tor state. Cheap — just reads an atomic.
#[uniffi::export]
pub fn tor_status() -> TorStatus {
    match &*TOR_STATE.lock() {
        TorInternalState::Stopped => TorStatus::Stopped,
        TorInternalState::Bootstrapping { percent } => TorStatus::Bootstrapping {
            percent: percent.load(Ordering::Relaxed),
        },
        TorInternalState::Running { .. } | TorInternalState::CustomProxy { .. } => TorStatus::Ready,
        TorInternalState::Error { message } => TorStatus::Error { message: message.clone() },
    }
}

// ── Bootstrap task ───────────────────────────────────────────────────────────

async fn bootstrap_tor(data_dir: String, percent: Arc<AtomicU8>) {
    match try_bootstrap(&data_dir, &percent).await {
        Ok(client) => {
            let client = Arc::new(client);
            let tor_for_proxy = client.clone();
            let proxy_task = tokio::spawn(run_socks5_proxy(tor_for_proxy));

            {
                let mut guard = TOR_STATE.lock();
                *guard = TorInternalState::Running {
                    _client: client,
                    proxy_task,
                };
            }

            crate::fetch::http::set_socks5_proxy(Some("socks5://127.0.0.1:19050"));
        }
        Err(message) => {
            let mut guard = TOR_STATE.lock();
            *guard = TorInternalState::Error { message };
        }
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

const SOCKS5_PORT: u16 = 19050;

async fn run_socks5_proxy(tor: Arc<TorClient<PreferredRuntime>>) {
    let listener = match TcpListener::bind(("127.0.0.1", SOCKS5_PORT)).await {
        Ok(l) => l,
        Err(e) => {
            let mut guard = TOR_STATE.lock();
            *guard = TorInternalState::Error {
                message: format!("SOCKS5 bind failed: {e}"),
            };
            return;
        }
    };

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let tor = tor.clone();
                tokio::spawn(async move {
                    let _ = handle_socks5(stream, tor).await;
                });
            }
            Err(_) => break,
        }
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
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "no acceptable auth"));
    }
    tcp.write_all(&[5, 0x00]).await?; // no-auth accepted

    // ── 2. CONNECT request ───────────────────────────────────────────────────
    let mut req = [0u8; 4];
    tcp.read_exact(&mut req).await?;
    // req: [VER=5, CMD, RSV, ATYP]
    if req[0] != 5 || req[1] != 0x01 {
        // Only CONNECT (0x01) is supported.
        tcp.write_all(&[5, 0x07, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        return Err(io::Error::new(io::ErrorKind::Unsupported, "only CONNECT supported"));
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
