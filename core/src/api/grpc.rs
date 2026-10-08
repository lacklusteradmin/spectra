//! The transport under gRPC services: an HTTP/2 connection per endpoint,
//! dialled by `api::tcp` under the same rules as every HTTP request.
//!
//! The Tor kill switch refuses before anything is dialled, and the
//! loopback-only guard the acceptance suite sets refuses and journals any
//! other host. An `https` endpoint is TLS by rustls against the web PKI
//! roots, negotiating HTTP/2; an `http` one (a loopback fixture) is
//! plaintext HTTP/2.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::api::error::ApiError;

/// A byte stream a channel speaks HTTP/2 over.
trait Io: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Io for T {}

/// Where an endpoint URL points.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    tls: bool,
    host: String,
    port: u16,
}

fn target(endpoint: &str) -> Result<Target, ApiError> {
    let uri: http::Uri = endpoint
        .parse()
        .map_err(|_| ApiError::invalid(format!("Invalid gRPC endpoint {endpoint}")))?;
    let tls = match uri.scheme_str() {
        Some("https") => true,
        Some("http") => false,
        _ => {
            return Err(ApiError::invalid(format!(
                "Invalid gRPC endpoint {endpoint}"
            )));
        }
    };
    let host = uri
        .host()
        .ok_or_else(|| ApiError::invalid(format!("Invalid gRPC endpoint {endpoint}")))?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let port = uri.port_u16().unwrap_or(if tls { 443 } else { 80 });
    Ok(Target { tls, host, port })
}

/// A channel to `endpoint`, connected under the routing rules above.
pub(crate) async fn channel(endpoint: &str) -> Result<tonic::transport::Channel, ApiError> {
    if crate::tor::kill_switch_engaged() {
        return Err(ApiError::Transport(
            crate::api::http::KILL_SWITCH_MESSAGE.into(),
        ));
    }
    if !crate::api::http::may_contact(endpoint) {
        crate::api::http::refuse_non_loopback(endpoint);
        return Err(ApiError::Transport("non-loopback endpoint refused".into()));
    }
    let target = target(endpoint)?;
    let connector = {
        let target = target.clone();
        tower::service_fn(move |_: http::Uri| {
            let target = target.clone();
            async move {
                connect(&target)
                    .await
                    .map(hyper_util::rt::TokioIo::new)
                    .map_err(|error| std::io::Error::other(error.to_string()))
            }
        })
    };
    // TLS, when the endpoint has it, is the connector's: the channel speaks
    // HTTP/2 over whatever stream it is handed.
    let host = if target.host.contains(':') {
        format!("[{}]", target.host)
    } else {
        target.host.clone()
    };
    tonic::transport::Endpoint::from_shared(format!("http://{host}:{}", target.port))
        .map_err(|error| ApiError::invalid(error.to_string()))?
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .connect_with_connector(connector)
        .await
        .map_err(|error| ApiError::Transport(format!("gRPC connect to {endpoint}: {error}")))
}

async fn connect(target: &Target) -> Result<Box<dyn Io>, ApiError> {
    let stream = crate::api::tcp::dial(&target.host, target.port).await?;
    if !target.tls {
        return Ok(Box::new(stream));
    }
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let mut config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|error| ApiError::Transport(error.to_string()))?
    .with_root_certificates(roots)
    .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec()];
    let name = rustls::pki_types::ServerName::try_from(target.host.clone())
        .map_err(|_| ApiError::invalid(format!("Invalid TLS name {}", target.host)))?;
    let tls = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(name, stream)
        .await
        .map_err(|error| ApiError::Transport(format!("TLS: {error}")))?;
    Ok(Box::new(tls))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_name_their_scheme_host_and_port() {
        assert_eq!(
            target("https://zec.rocks:443").unwrap(),
            Target {
                tls: true,
                host: "zec.rocks".into(),
                port: 443
            }
        );
        assert_eq!(
            target("https://zec.rocks").unwrap(),
            Target {
                tls: true,
                host: "zec.rocks".into(),
                port: 443
            }
        );
        assert_eq!(
            target("http://127.0.0.1:9067").unwrap(),
            Target {
                tls: false,
                host: "127.0.0.1".into(),
                port: 9067
            }
        );
        assert_eq!(target("http://[::1]:9067").unwrap().host, "::1");
        for bad in ["zec.rocks:443", "ftp://zec.rocks", "https://"] {
            assert!(target(bad).is_err(), "{bad}");
        }
    }
}
