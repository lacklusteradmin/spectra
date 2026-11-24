use super::*;
use crate::service::app_refresh::AppRefreshIntent;
use crate::service::refresh_engine::refresh_entries_for;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn balance_commit_replaces_protocol_aliases_without_duplicating_funds() {
    let chain = Chain::BnbChain;
    let token = crate::catalog::tokens::catalog()
        .iter()
        .find(|token| token.chain_id == chain && !token.is_native())
        .unwrap();
    let mut current = token.holding_template();
    current.amount = "4".into();
    let mut old = current.clone();
    old.token_standard = Some(crate::registry::TokenStandard::Erc20);
    old.name = "Old custom token".into();
    old.coingecko_id.clear();
    old.amount = "9".into();
    let mut wallet = WalletState::single_address(
        "aliases",
        "Aliases",
        chain,
        "0x1111111111111111111111111111111111111111",
        None,
        true,
    );
    wallet.holdings = vec![old.clone(), old, current.clone()];
    let service = WalletService::new(Vec::new()).unwrap();
    let db = std::env::temp_dir().join(format!(
        "spectra-balance-alias-{}.sqlite",
        crate::store::new_id()
    ));
    service.opened(db.to_string_lossy().into()).await.unwrap();
    service.upsert_wallet(wallet).await.unwrap();
    let entry = refresh_entries_for(&service.app_state().await).remove(0);
    current.amount = "2.5".into();
    current = current.identified();
    let updated = service
        .commit_balance_result(entry.clone(), vec![current.clone()])
        .await
        .unwrap();
    assert_eq!(updated.holdings, [current.clone()]);
    let reopened = WalletService::new(Vec::new()).unwrap();
    let state = reopened.opened(db.to_string_lossy().into()).await.unwrap();
    assert_eq!(state.wallets[0].holdings.len(), 1);
    assert_eq!(state.wallets[0].holdings[0].clone().identified(), current);
    current.amount = "0".into();
    let updated = service
        .commit_balance_result(entry, vec![current.clone()])
        .await
        .unwrap();
    assert_eq!(updated.holdings, [current]);
}

/// A token whose read failed is left out of what the refresh brings back,
/// and keeps the balance it had; the balances that were read replace
/// theirs, a zero included.
#[test]
fn a_token_the_refresh_could_not_read_keeps_its_stored_balance() {
    let chain = Chain::Ethereum;
    let token = crate::catalog::tokens::catalog()
        .iter()
        .find(|token| token.chain_id == chain && token.symbol == "USDC")
        .unwrap();
    let mut native = chain.native_holding_template();
    native.amount = "1".into();
    let mut held = token.holding_template();
    held.amount = "2".into();
    let mut stored = vec![native.clone(), held.clone()];
    native.amount = "0".into();
    merge_balances(&mut stored, vec![native.clone()]);
    assert_eq!(stored, [native, held]);
}

#[tokio::test]
async fn trc10_and_trc20_refresh_as_distinct_assets_and_survive_reopen() {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    let owner = "TJRabPrwbZy45sbavfcjinPJC18kjpRTv8";
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/wallet/getaccount"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"address":owner,"balance":1000000,"assetV2":[{"key":"1002000","value":2500000}]})))
        .mount(&server)
        .await;
    for (route, value) in [
        (
            "/wallet/getblockbynum",
            json!({"blockID":Chain::Tron.tron_genesis_block_id().unwrap()}),
        ),
        (
            "/wallet/getassetissuebyid",
            json!({"id":"1002000","name":hex::encode("T10"),"precision":6}),
        ),
    ] {
        Mock::given(method("POST"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_json(value))
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path(format!("/v1/accounts/{owner}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"trc20": []}]})))
        .expect(1)
        .mount(&server)
        .await;
    let service = WalletService::new(vec![ChainEndpoints {
        chain_id: Chain::Tron,
        endpoints: vec![server.uri()],
        capabilities: EndpointCapability::ALL.to_vec(),
    }])
    .unwrap();
    let db = std::env::temp_dir().join(format!(
        "mixed-protocol-balances-{}.sqlite",
        crate::store::new_id()
    ));
    service.opened(db.to_string_lossy().into()).await.unwrap();
    service
        .apply_state_command(StateCommand::AddCustomToken {
            chain_id: Chain::Tron,
            standard: Some(crate::registry::TokenStandard::Trc10),
            symbol: "T10".into(),
            name: "T10".into(),
            contract: "1002000".into(),
            decimals: Some(6),
            coingecko_id: String::new(),
            coinpaprika_id: String::new(),
        })
        .await
        .unwrap();
    let mut wallet = WalletState::single_address("mixed", "Mixed", Chain::Tron, owner, None, true);
    let state = service.app_state().await;
    let mut token20 = state
        .token_preferences
        .iter()
        .find(|p| {
            p.token.chain_id == Chain::Tron
                && p.token.token_standard == Some(crate::registry::TokenStandard::Trc20)
        })
        .unwrap()
        .token
        .holding_template();
    token20.amount = "9".into();
    let token20_id = token20.deployment_id();
    let mut token10 = state
        .token_preferences
        .iter()
        .find(|p| p.token.token_standard == Some(crate::registry::TokenStandard::Trc10))
        .unwrap()
        .token
        .holding_template();
    token10.amount = "9".into();
    wallet.holdings = vec![token20, token10];
    service.upsert_wallet(wallet).await.unwrap();
    let updated = service
        .refresh_wallet_balances("mixed".into())
        .await
        .unwrap();
    assert_eq!(
        updated
            .holdings
            .iter()
            .find(|h| h.deployment_id() == token20_id)
            .unwrap()
            .amount,
        "0"
    );
    assert_eq!(
        updated
            .holdings
            .iter()
            .find(|h| h.token_standard == Some(crate::registry::TokenStandard::Trc10))
            .unwrap()
            .amount,
        "2.5"
    );
    let reopened = WalletService::new(Vec::new()).unwrap();
    let state = reopened.opened(db.to_string_lossy().into()).await.unwrap();
    assert_eq!(
        state.wallets[0]
            .holdings
            .iter()
            .find(|h| h.token_standard == Some(crate::registry::TokenStandard::Trc10))
            .unwrap()
            .amount,
        "2.5"
    );
    assert!(
        state
            .token_preferences
            .iter()
            .any(|p| p.token.token_standard == Some(crate::registry::TokenStandard::Trc10))
    );
}

/// The import's zero is a placeholder; a first read of the same zero is a
/// balance, and is stored as read even though no amount changed.
#[tokio::test]
async fn a_first_read_of_unchanged_zeros_still_marks_the_balances_read() {
    let service = WalletService::new(vec![]).unwrap();
    let mut w = WalletState::single_address(
        "w",
        "W",
        crate::registry::Chain::Ethereum,
        "0x1111111111111111111111111111111111111111",
        None,
        true,
    );
    let coin = crate::registry::Chain::Ethereum.native_holding_template();
    w.holdings = vec![coin.clone()];
    service.upsert_wallet(w).await.unwrap();
    assert!(
        service.app_state().await.wallets[0]
            .balances_read_at
            .is_none()
    );
    let entry = refresh_entries_for(&service.app_state().await).remove(0);
    let read = service
        .commit_balance_result(entry.clone(), vec![coin.clone()])
        .await
        .unwrap();
    let first = read.balances_read_at.expect("read");
    assert_eq!(
        service.app_state().await.wallets[0].balances_read_at,
        Some(first)
    );
    let again = service
        .commit_balance_result(entry, vec![coin])
        .await
        .unwrap();
    assert_eq!(
        again.balances_read_at,
        Some(first),
        "only the first read is recorded"
    );
}

#[tokio::test]
async fn balance_commit_preserves_metadata_and_refuses_stale_network() {
    let service = WalletService::new(vec![]).unwrap();
    let path =
        std::env::temp_dir().join(format!("balance-owned-{}.sqlite", crate::store::new_id()));
    service.opened(path.to_string_lossy().into()).await.unwrap();
    let mut w = WalletState::single_address(
        "w",
        "Original",
        crate::registry::Chain::Ethereum,
        "0x1111111111111111111111111111111111111111",
        None,
        false,
    );
    let mut coin = crate::registry::Chain::Ethereum.native_holding_template();
    coin.amount = "4".into();
    w.holdings = vec![coin.clone()];
    service.upsert_wallet(w).await.unwrap();
    let entry = refresh_entries_for(&service.app_state().await).remove(0);
    coin.amount = "0".into();
    let updated = service
        .commit_balance_result(entry.clone(), vec![coin.clone()])
        .await
        .unwrap();
    assert_eq!(updated.name, "Original");
    assert_eq!(updated.holdings[0].amount, "0");
    assert!(
        updated.balances_read_at.is_some(),
        "the first read marks the balances read"
    );
    let database = rusqlite::Connection::open(&path).unwrap();
    database.execute_batch("CREATE TRIGGER reject_balance BEFORE UPDATE ON wallets BEGIN SELECT RAISE(FAIL, 'balance write refused'); END;").unwrap();
    coin.amount = "7".into();
    assert!(
        service
            .commit_balance_result(entry.clone(), vec![coin.clone()])
            .await
            .is_err()
    );
    assert_eq!(service.app_state().await.wallets[0].holdings[0].amount, "0");
    assert_eq!(
        crate::wallet_db::wallet_load_all(&crate::wallet_db::WalletDatabase::new(
            path.to_str().unwrap()
        ))
        .unwrap()[0]
            .holdings[0]
            .amount,
        "0"
    );
    database
        .execute_batch("DROP TRIGGER reject_balance")
        .unwrap();

    // A result fetched for the wallet's previous network does not land.
    let mut moved = service.app_state().await.wallets[0].clone();
    moved.chain_id = crate::registry::Chain::EthereumSepolia;
    service.upsert_wallet(moved).await.unwrap();
    coin.amount = "99".into();
    service
        .commit_balance_result(entry, vec![coin])
        .await
        .unwrap();
    let reopened = WalletService::new(vec![]).unwrap();
    assert_eq!(
        reopened
            .opened(path.to_string_lossy().into())
            .await
            .unwrap()
            .wallets[0]
            .holdings[0]
            .amount,
        "0"
    );
    for raw in ["bad", "NaN", "inf", "-1"] {
        assert!(balance_amount(raw).is_err());
    }
}

#[tokio::test]
async fn requested_refresh_is_bounded_and_shares_overlapping_wallet_reads() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (sent, mut requests) = tokio::sync::mpsc::unbounded_channel();
    let permits = Arc::new(tokio::sync::Semaphore::new(0));
    let release = permits.clone();
    let server = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let sent = sent.clone();
            let release = release.clone();
            connections.spawn(async move {
                let mut request = Vec::new();
                let mut buf = [0; 2048];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = socket.read(&mut buf).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    request.extend_from_slice(&buf[..n]);
                }
                let request = String::from_utf8(request).unwrap();
                let body = if request.lines().next().unwrap().contains("/payments") {
                    r#"{"_embedded":{"records":[]}}"#
                } else {
                    sent.send(()).unwrap();
                    release.acquire().await.unwrap().forget();
                    r#"{"sequence":"1","balances":[{"asset_type":"native","balance":"2.0000000"}]}"#
                };
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            });
        }
    });
    let service = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: crate::registry::Chain::Stellar,
        endpoints: vec![endpoint],
    }])
    .unwrap();
    let path = std::env::temp_dir().join(format!(
        "balance-concurrency-{}.sqlite",
        crate::store::new_id()
    ));
    service.opened(path.to_string_lossy().into()).await.unwrap();
    for i in 0..10 {
        service
            .upsert_wallet(WalletState::single_address(
                format!("w{i}"),
                "Concurrent",
                crate::registry::Chain::Stellar,
                "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
                None,
                true,
            ))
            .await
            .unwrap();
    }
    let requested = service.clone();
    let refresh = tokio::spawn(async move {
        requested
            .refresh_app(
                AppRefreshIntent::AfterSend {
                    chain_id: crate::registry::Chain::Stellar,
                },
                crate::service::refresh_policy::DeviceConditions {
                    app_is_active: true,
                    is_network_reachable: true,
                    is_constrained_network: false,
                    is_expensive_network: false,
                    is_low_power_mode: false,
                    battery_level: 1.0,
                    wants_price_refresh: false,
                },
            )
            .await
    });
    for _ in 0..8 {
        tokio::time::timeout(Duration::from_secs(10), requests.recv())
            .await
            .unwrap()
            .unwrap();
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(50), requests.recv())
            .await
            .is_err()
    );
    // Poll an overlapping reader into the in-flight work before releasing the node.
    let overlapping = service.refresh_wallet_balances("w0".into());
    tokio::pin!(overlapping);
    assert!(futures::poll!(&mut overlapping).is_pending());
    permits.add_permits(10);
    let result = tokio::time::timeout(Duration::from_secs(10), refresh)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(result.failures.is_empty(), "{:?}", result.failures);
    assert_eq!(
        requests.len(),
        2,
        "one native request per wallet, including the overlapping read"
    );
    // Even an unpolled waiter must not turn a completed read into a cache.
    permits.add_permits(1);
    assert_eq!(
        service
            .refresh_wallet_balances("w0".into())
            .await
            .unwrap()
            .holdings[0]
            .amount,
        "2"
    );
    assert_eq!(requests.len(), 3);
    assert_eq!(overlapping.await.unwrap().holdings[0].amount, "2");
    let reopened = WalletService::new(vec![]).unwrap();
    let state = reopened
        .opened(path.to_string_lossy().into())
        .await
        .unwrap();
    assert!(state.wallets.iter().all(|w| w.holdings[0].amount == "2"));
    server.abort();
}
