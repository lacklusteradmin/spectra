//! The transport under gRPC services: an HTTP/2 connection per endpoint,
//! dialled by `api::tcp` under the same rules as every HTTP request.
//!
//! The Tor kill switch refuses before anything is dialled, and the
//! loopback-only guard the acceptance suite sets refuses and journals any
//! other host. An `https` endpoint is TLS by rustls against the web PKI
//! roots, negotiating HTTP/2; an `http` one (a loopback fixture) is
//! plaintext HTTP/2. A Hedera consensus node's certificate is self-signed:
//! its channel is TLS pinned to the SHA-384 the network's address book gives
//! the certificate in PEM (`pinned_channel`).

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
    /// The SHA-384 of the server's certificate in PEM, which replaces the
    /// web PKI where it is set.
    pin: Option<[u8; 48]>,
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
    Ok(Target {
        tls,
        host,
        port,
        pin: None,
    })
}

/// A channel to `endpoint`, connected under the routing rules above.
pub(crate) async fn channel(endpoint: &str) -> Result<tonic::transport::Channel, ApiError> {
    open(endpoint, target(endpoint)?).await
}

/// A channel to `host:port` over TLS whose certificate must hash, in PEM
/// under SHA-384, to `cert_hash`: what a Hedera address book vouches for,
/// where no certificate authority does. A loopback host (a test fixture) is
/// plaintext.
pub(crate) async fn pinned_channel(
    host: &str,
    port: u16,
    cert_hash: [u8; 48],
) -> Result<tonic::transport::Channel, ApiError> {
    let loopback = host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    let shown = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let endpoint = format!(
        "{}://{shown}:{port}",
        if loopback { "http" } else { "https" }
    );
    open(
        &endpoint,
        Target {
            tls: !loopback,
            host: host.to_string(),
            port,
            pin: Some(cert_hash),
        },
    )
    .await
}

async fn open(endpoint: &str, target: Target) -> Result<tonic::transport::Channel, ApiError> {
    if crate::api::tor::kill_switch_engaged() {
        return Err(ApiError::Transport(
            crate::api::http::KILL_SWITCH_MESSAGE.into(),
        ));
    }
    if !crate::api::http::may_contact(endpoint) {
        crate::api::http::refuse_non_loopback(endpoint);
        return Err(ApiError::Transport("non-loopback endpoint refused".into()));
    }
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
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|error| ApiError::Transport(error.to_string()))?;
    let mut config = match target.pin {
        Some(hash) => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedCertificate { hash, provider }))
            .with_no_client_auth(),
        None => builder
            .with_root_certificates(rustls::RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            })
            .with_no_client_auth(),
    };
    config.alpn_protocols = vec![b"h2".to_vec()];
    let name = rustls::pki_types::ServerName::try_from(target.host.clone())
        .map_err(|_| ApiError::invalid(format!("Invalid TLS name {}", target.host)))?;
    let tls = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(name, stream)
        .await
        .map_err(|error| ApiError::Transport(format!("TLS: {error}")))?;
    Ok(Box::new(tls))
}

/// A certificate as PEM writes it: base64 in lines of 64 between the
/// `CERTIFICATE` markers, each line ended by a newline.
pub(crate) fn pem(der: &[u8]) -> String {
    use base64::Engine as _;
    let body = base64::engine::general_purpose::STANDARD.encode(der);
    let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
    for line in body.as_bytes().chunks(64) {
        pem.push_str(std::str::from_utf8(line).expect("base64 is ASCII"));
        pem.push('\n');
    }
    pem.push_str("-----END CERTIFICATE-----\n");
    pem
}

/// Accepts the one certificate whose PEM hashes to `hash` under SHA-384,
/// whoever issued it and whatever name it carries, and checks the handshake
/// is signed by that certificate's key.
#[derive(Debug)]
struct PinnedCertificate {
    hash: [u8; 48],
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl rustls::client::danger::ServerCertVerifier for PinnedCertificate {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        use sha2::Digest as _;
        if sha2::Sha384::digest(pem(end_entity).as_bytes())[..] == self.hash[..] {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
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
                port: 443,
                pin: None,
            }
        );
        assert_eq!(
            target("https://zec.rocks").unwrap(),
            Target {
                tls: true,
                host: "zec.rocks".into(),
                port: 443,
                pin: None,
            }
        );
        assert_eq!(
            target("http://127.0.0.1:9067").unwrap(),
            Target {
                tls: false,
                host: "127.0.0.1".into(),
                port: 9067,
                pin: None,
            }
        );
        assert_eq!(target("http://[::1]:9067").unwrap().host, "::1");
        for bad in ["zec.rocks:443", "ftp://zec.rocks", "https://"] {
            assert!(target(bad).is_err(), "{bad}");
        }
    }
}
