//! A TCP connection, dialled under the same rules as every HTTP request.
//!
//! The Tor kill switch refuses before anything is dialled; a SOCKS5 proxy in
//! force (Tor's, or one the user set) carries the connection with the host
//! named, so it resolves at the far end; the loopback-only guard the
//! acceptance suite sets refuses and journals any other host. gRPC
//! (`api::grpc`) and Litecoin's peer-to-peer protocol (`api::litecoin_p2p`)
//! speak over it.

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::api::error::ApiError;

/// A connection to `host:port` for `endpoint`: refused under the kill
/// switch, and outside loopback while the loopback-only guard is set.
pub(crate) async fn connect(
    endpoint: &str,
    host: &str,
    port: u16,
) -> Result<tokio::net::TcpStream, ApiError> {
    if crate::tor::kill_switch_engaged() {
        return Err(ApiError::Transport(
            crate::api::http::KILL_SWITCH_MESSAGE.into(),
        ));
    }
    if !crate::api::http::may_contact(endpoint) {
        crate::api::http::refuse_non_loopback(endpoint);
        return Err(ApiError::Transport("non-loopback endpoint refused".into()));
    }
    dial(host, port).await
}

/// The peers to try for `endpoint`, in turn: each address `host` resolves
/// to when the connection goes direct — a DNS seed names several nodes, and
/// one may have no slot for another peer — or the name itself, which a proxy
/// in force resolves at the far end. Refused as `connect` refuses, before
/// anything is resolved.
pub(crate) async fn peers(endpoint: &str, host: &str, port: u16) -> Result<Vec<String>, ApiError> {
    if crate::tor::kill_switch_engaged() {
        return Err(ApiError::Transport(
            crate::api::http::KILL_SWITCH_MESSAGE.into(),
        ));
    }
    if !crate::api::http::may_contact(endpoint) {
        crate::api::http::refuse_non_loopback(endpoint);
        return Err(ApiError::Transport("non-loopback endpoint refused".into()));
    }
    let direct = crate::tor::with_routing_guard(|blocked| {
        !blocked && crate::api::http::current_proxy().is_none()
    });
    if !direct || host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(vec![host.to_string()]);
    }
    let mut peers: Vec<String> = Vec::new();
    for address in tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| ApiError::Transport(format!("resolve {host}: {error}")))?
    {
        let ip = address.ip().to_string();
        if !peers.contains(&ip) {
            peers.push(ip);
        }
    }
    Ok(peers)
}

/// Dial `host:port`, through the proxy in force if there is one.
pub(crate) async fn dial(host: &str, port: u16) -> Result<tokio::net::TcpStream, ApiError> {
    // The routing decision and the proxy it names are read together, so a
    // concurrent Tor stop cannot leave a direct dial behind a blocked one.
    let route =
        crate::tor::with_routing_guard(|blocked| (!blocked).then(crate::api::http::current_proxy));
    let Some(proxy) = route else {
        return Err(ApiError::Transport(
            crate::api::http::KILL_SWITCH_MESSAGE.into(),
        ));
    };
    match proxy {
        Some(proxy) => socks5(&proxy, host, port).await,
        None => tokio::net::TcpStream::connect((host, port))
            .await
            .map_err(|error| ApiError::Transport(format!("connect: {error}"))),
    }
}

/// A SOCKS5 CONNECT to `host:port` through `proxy` (`socks5h://` or
/// `socks5://`, optionally with a user name and password), naming the host
/// so the proxy resolves it.
async fn socks5(proxy: &str, host: &str, port: u16) -> Result<tokio::net::TcpStream, ApiError> {
    let proxy_error = |what: &str| ApiError::Transport(format!("SOCKS5 proxy: {what}"));
    let uri: http::Uri = proxy
        .parse()
        .map_err(|_| proxy_error("invalid proxy address"))?;
    if !matches!(uri.scheme_str(), Some("socks5h" | "socks5")) {
        return Err(proxy_error("only a SOCKS5 proxy carries this connection"));
    }
    let authority = uri
        .authority()
        .ok_or_else(|| proxy_error("no proxy address"))?;
    let credentials = authority
        .as_str()
        .rsplit_once('@')
        .and_then(|(user, _)| user.split_once(':'))
        .map(|(name, password)| (name.to_string(), password.to_string()));
    let mut stream = tokio::net::TcpStream::connect((
        authority
            .host()
            .trim_start_matches('[')
            .trim_end_matches(']'),
        authority.port_u16().unwrap_or(1080),
    ))
    .await
    .map_err(|error| ApiError::Transport(format!("SOCKS5 proxy: {error}")))?;
    let io = |error: std::io::Error| ApiError::Transport(format!("SOCKS5 proxy: {error}"));
    // Greeting: no authentication, or user name and password (RFC 1929).
    let method = if credentials.is_some() { 0x02 } else { 0x00 };
    stream.write_all(&[5, 1, method]).await.map_err(io)?;
    let mut reply = [0u8; 2];
    stream.read_exact(&mut reply).await.map_err(io)?;
    if reply != [5, method] {
        return Err(proxy_error("refused the authentication method"));
    }
    if let Some((name, password)) = credentials {
        if name.len() > 255 || password.len() > 255 {
            return Err(proxy_error("credentials too long"));
        }
        let mut auth = vec![1, name.len() as u8];
        auth.extend(name.as_bytes());
        auth.push(password.len() as u8);
        auth.extend(password.as_bytes());
        stream.write_all(&auth).await.map_err(io)?;
        stream.read_exact(&mut reply).await.map_err(io)?;
        if reply[1] != 0 {
            return Err(proxy_error("refused the credentials"));
        }
    }
    if host.len() > 255 {
        return Err(proxy_error("host name too long"));
    }
    let mut request = vec![5, 1, 0, 3, host.len() as u8];
    request.extend(host.as_bytes());
    request.extend(port.to_be_bytes());
    stream.write_all(&request).await.map_err(io)?;
    let mut head = [0u8; 4];
    stream.read_exact(&mut head).await.map_err(io)?;
    if head[1] != 0 {
        return Err(proxy_error(&format!("connect refused (code {})", head[1])));
    }
    // The bound address the proxy reports, which nothing here uses.
    let skip = match head[3] {
        1 => 4 + 2,
        4 => 16 + 2,
        3 => {
            let mut length = [0u8; 1];
            stream.read_exact(&mut length).await.map_err(io)?;
            usize::from(length[0]) + 2
        }
        _ => return Err(proxy_error("unknown address type")),
    };
    let mut bound = vec![0u8; skip];
    stream.read_exact(&mut bound).await.map_err(io)?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A SOCKS5 proxy is asked for the host by name, so it resolves it.
    #[tokio::test]
    async fn a_proxy_is_given_the_host_name() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let proxy = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut greeting = [0u8; 3];
            stream.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 1, 0]);
            stream.write_all(&[5, 0]).await.unwrap();
            let mut head = [0u8; 5];
            stream.read_exact(&mut head).await.unwrap();
            assert_eq!(head[..4], [5, 1, 0, 3]);
            let mut name = vec![0u8; usize::from(head[4]) + 2];
            stream.read_exact(&mut name).await.unwrap();
            stream
                .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
                .await
                .unwrap();
            name
        });
        socks5(&format!("socks5h://127.0.0.1:{port}"), "zec.rocks", 443)
            .await
            .unwrap();
        let name = proxy.await.unwrap();
        assert_eq!(&name[..9], b"zec.rocks");
        assert_eq!(name[9..], 443u16.to_be_bytes());
    }
}
