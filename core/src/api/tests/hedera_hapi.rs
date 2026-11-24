use super::*;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn id(text: &str) -> HederaAccountId {
    HederaAccountId::parse(Chain::Hedera, text).unwrap()
}

#[test]
fn prechecks_accept_pass_on_or_refuse() {
    for (code, expected) in [
        (0, Precheck::Accepted),
        (11, Precheck::Accepted),
        (12, Precheck::Busy),
        (67, Precheck::Busy),
        (69, Precheck::Busy),
        (3, Precheck::Busy),
        (7, Precheck::Refused),
        (9, Precheck::Refused),
        (10, Precheck::Refused),
        (4, Precheck::Refused),
        (5, Precheck::Refused),
        (9999, Precheck::Refused),
    ] {
        assert_eq!(precheck(code).0, expected, "{code}");
    }
}

#[test]
fn a_node_certificate_hashes_to_its_address_book_entry() {
    // Node 0.0.3's certificate as its port 50212 served it on 2026-11-24,
    // and the hash mainnet's address book gave it.
    use base64::Engine as _;
    use sha2::Digest as _;
    let served = include_str!("../../../tests/fixtures/hedera-node-0.0.3.pem");
    let body: String = served
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(body)
        .unwrap();
    let pem = crate::api::grpc::pem(&der);
    assert_eq!(pem, served);
    assert_eq!(
        hex::encode(sha2::Sha384::digest(pem.as_bytes())),
        "34244d50a8ed4d4cbadf25620d1ab88a32081977d2f8d70ba82ba52b3305a4560824cf6b0106d635ed5c39272fffabe8"
    );
}

/// An address book on a mock mirror node naming `nodes` at loopback ports.
async fn book(nodes: &[(&str, u16)]) -> MockServer {
    let server = MockServer::start().await;
    let listed: Vec<_> = nodes
        .iter()
        .map(|(account, port)| {
            json!({
                "node_account_id": account,
                "node_cert_hash": format!("0x{}", "00".repeat(48)),
                "service_endpoints": [
                    {"domain_name": "", "ip_address_v4": "127.0.0.1", "port": 50211},
                    {"domain_name": "", "ip_address_v4": "127.0.0.1", "port": port}
                ]
            })
        })
        .collect();
    Mock::given(method("GET"))
        .and(path("/api/v1/network/nodes"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"nodes": listed, "links": {"next": null}})),
        )
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn a_busy_node_passes_the_transaction_to_the_next() {
    let busy = test_node::start(|_| 12).await;
    let taker = test_node::start(|_| 0).await;
    let server = book(&[("0.0.3", busy.port), ("0.0.4", taker.port)]).await;
    let client = HederaHapiClient::new(Arc::new(vec![format!(
        "{}/api/v1/network/nodes",
        server.uri()
    )]));
    // 0.0.9 is not in the book; 0.0.3 is busy; 0.0.4 takes it.
    let transactions = [
        (id("0.0.9"), vec![9]),
        (id("0.0.3"), vec![3]),
        (id("0.0.4"), vec![4]),
    ];
    let accepted = client
        .submit(&transactions, "0.0.1234-1795522566-435000006")
        .await
        .unwrap();
    assert_eq!(accepted, "0.0.1234-1795522566-435000006");
    assert_eq!(*busy.received.lock().unwrap(), [vec![3u8]]);
    assert_eq!(*taker.received.lock().unwrap(), [vec![4u8]]);
}

#[tokio::test]
async fn a_refusal_ends_the_submission() {
    let refuser = test_node::start(|_| 10).await;
    let spare = test_node::start(|_| 0).await;
    let server = book(&[("0.0.3", refuser.port), ("0.0.4", spare.port)]).await;
    let client = HederaHapiClient::new(Arc::new(vec![format!(
        "{}/api/v1/network/nodes",
        server.uri()
    )]));
    let error = client
        .submit(
            &[(id("0.0.3"), vec![3]), (id("0.0.4"), vec![4])],
            "0.0.1234-1795522566-435000006",
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&error, ApiError::Rejected(message) if message.contains("INSUFFICIENT_PAYER_BALANCE")),
        "{error}"
    );
    assert!(spare.received.lock().unwrap().is_empty());
    // A book that names none of the nodes takes nothing.
    let error = client
        .submit(&[(id("0.0.9"), vec![9])], "0.0.1234-1795522566-435000006")
        .await
        .unwrap_err();
    assert!(matches!(error, ApiError::Transport(_)), "{error}");
}
