use super::*;
use crate::registry::Chain;
use crate::service::address_discovery::UtxoDerivation;
use crate::store::state::WalletSummary;
use std::sync::Arc;

const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

#[test]
fn public_children_match_full_derivation_for_every_discovery_network() {
    for chain in Chain::all().filter(|c| c.supports_deep_utxo_discovery()) {
        for purpose in [44, 49, 84, 86] {
            if chain.mainnet_counterpart() != Chain::Bitcoin && purpose != 44 {
                continue;
            }
            let context =
                UtxoDerivation::new(chain, SEED, format!("m/{purpose}'/0'/2'/1/9")).unwrap();
            for index in [0, 1, 40] {
                let (address, path) = context.derive(index).unwrap();
                let expected = crate::derivation::dispatch::derive_for_chain_name(
                    chain.chain_display_name(),
                    SEED,
                    &path,
                    None,
                    None,
                    None,
                    true,
                    false,
                    false,
                )
                .unwrap()
                .address
                .unwrap();
                assert_eq!(address, expected, "{chain:?} {path}");
            }
            assert!(context.derive(0x80000000).is_none());
        }
    }
    let btc = UtxoDerivation::new(Chain::Bitcoin, SEED, "m/84'/0'/0'/0/0".into()).unwrap();
    // BIP-84 published first receiving address.
    assert_eq!(
        btc.derive(0).unwrap().0,
        "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"
    );
}

async fn scanning_service(endpoint: String) -> Arc<WalletService> {
    use crate::store::secret_backends::InMemorySecretStore;
    let service = WalletService::new_typed(vec![crate::service::ChainEndpoints {
        chain_id: "bitcoin".into(),
        endpoints: vec![endpoint],
        api_key: None,
    }])
    .unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    crate::store::wallet_secrets::store_seed_phrase(&*secrets, "scan", SEED, None).unwrap();
    service.set_secret_store(secrets);
    service
        .apply_state_command(StateCommand::UpsertWallet {
            wallet: WalletSummary::single_address(
                "scan",
                "Scan",
                "Bitcoin",
                "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu",
                Some("m/84'/0'/0'/0/0".into()),
                false,
            ),
        })
        .await
        .unwrap();
    service
}

#[tokio::test]
async fn discovery_has_four_in_flight_probes_and_returns_index_order() {
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::{mpsc, Semaphore};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let permits = Arc::new(Semaphore::new(0));
    let release = permits.clone();
    let server = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let tx = tx.clone();
            let release = release.clone();
            connections.spawn(async move {
                let mut request = Vec::new();
                let mut buf = [0u8; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = socket.read(&mut buf).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                }
                tx.send(String::from_utf8(request).unwrap()).unwrap();
                release.acquire().await.unwrap().forget();
                let body = r#"{"address":"unused","chain_stats":{"funded_txo_sum":0,"spent_txo_sum":0,"tx_count":0},"mempool_stats":{"funded_txo_sum":0,"spent_txo_sum":0,"tx_count":0}}"#;
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).as_bytes()).await.unwrap();
            });
        }
    });
    let service = scanning_service(endpoint).await;
    let scan_service = service.clone();
    let scan = tokio::spawn(async move {
        scan_service
            .discover_utxo_addresses("scan".into(), "bitcoin".into())
            .await
    });
    for _ in 0..4 {
        let request = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(request.starts_with("GET /address/"));
        assert!(!request.contains("/txs"));
    }
    assert!(tokio::time::timeout(Duration::from_millis(50), rx.recv())
        .await
        .is_err());
    permits.add_permits(4);
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    permits.add_permits(1);
    let addresses = scan.await.unwrap().unwrap();
    let context = UtxoDerivation::new(Chain::Bitcoin, SEED, "m/84'/0'/0'/0/0".into()).unwrap();
    assert_eq!(
        addresses,
        (0..5)
            .map(|i| context.derive(i).unwrap().0)
            .collect::<Vec<_>>()
    );
    server.abort();
}

#[tokio::test]
async fn activity_probes_include_pending_and_spent_addresses_without_transaction_bodies() {
    use serde_json::json;
    use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};
    for (chain, url, body) in [
        (
            Chain::Bitcoin,
            "/address/a",
            json!({"address":"a", "chain_stats":{"funded_txo_sum":0,"spent_txo_sum":0,"tx_count":0}, "mempool_stats":{"funded_txo_sum":0,"spent_txo_sum":0,"tx_count":1}}),
        ),
        (
            Chain::BitcoinCash,
            "/api/v2/address/a",
            json!({"txs":1,"unconfirmedTxs":0}),
        ),
        (
            Chain::Litecoin,
            "/api/v2/address/a",
            json!({"txs":0,"unconfirmedTxs":1}),
        ),
        (
            Chain::Dogecoin,
            "/addrs/a/balance",
            json!({"n_tx":1,"unconfirmed_n_tx":0}),
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(path(url))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
        let service = WalletService::new_typed(vec![crate::service::ChainEndpoints {
            chain_id: chain.str_id().into(),
            endpoints: vec![server.uri()],
            api_key: None,
        }])
        .unwrap();
        assert!(service.utxo_address_has_activity(chain, "a").await.unwrap());
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
    let server = MockServer::start().await;
    Mock::given(path("/address/a/balance"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"confirmed":0,"unconfirmed":0})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/address/a/history"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!([{"tx_hash":"spent","height":123}])),
        )
        .expect(1)
        .mount(&server)
        .await;
    let service = WalletService::new_typed(vec![crate::service::ChainEndpoints {
        chain_id: Chain::BitcoinSV.str_id().into(),
        endpoints: vec![server.uri()],
        api_key: None,
    }])
    .unwrap();
    assert!(service
        .utxo_address_has_activity(Chain::BitcoinSV, "a")
        .await
        .unwrap());
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn malformed_activity_is_an_error_and_does_not_advance_or_register() {
    use wiremock::{matchers::any, Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    let service = scanning_service(server.uri()).await;
    let before = service
        .reserve_receive_index("scan".into(), "Bitcoin".into(), 1)
        .await
        .unwrap();
    assert!(service
        .discover_utxo_addresses("scan".into(), "bitcoin".into())
        .await
        .is_err());
    assert!(service
        .advance_used_utxo_reservations("bitcoin".into())
        .await
        .is_err());
    assert_eq!(
        service
            .keypool_state("scan".into(), "Bitcoin".into())
            .await
            .unwrap()
            .reserved_receive_index,
        Some(before)
    );
    assert!(service
        .owned_addresses
        .read()
        .await
        .values()
        .all(Vec::is_empty));
}
