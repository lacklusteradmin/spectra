use super::*;
use crate::registry::Chain;
use crate::service::address_discovery::UtxoDerivation;
use crate::store::state::WalletState;
use std::sync::Arc;

const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

#[test]
fn public_children_match_full_derivation_for_every_discovery_network() {
    for chain in Chain::all().filter(|c| c.supports_deep_utxo_discovery()) {
        for purpose in [44, 49, 84, 86] {
            if match chain.mainnet_counterpart() {
                Chain::Bitcoin | Chain::Peercoin => false,
                Chain::Litecoin => purpose == 86,
                _ => purpose != 44,
            } {
                continue;
            }
            let default = crate::derivation::path::default_path_from_catalog(chain).unwrap();
            let coin = crate::derivation::bitcoin::parse_bip32_path(&default).unwrap()[1]
                - crate::derivation::primitives::HARDENED_OFFSET;
            let context =
                UtxoDerivation::new(chain, SEED, format!("m/{purpose}'/{coin}'/2'/1/9")).unwrap();
            for branch in [0, 1] {
                for index in [0, 1, 40] {
                    let (address, path) = context.derive_on_branch(branch, index).unwrap();
                    let expected = crate::derivation::dispatch::derive_for_chain(
                        chain, SEED, &path, None, None, None, true, false, false,
                    )
                    .unwrap()
                    .address
                    .unwrap();
                    assert_eq!(address, expected, "{chain:?} {path}");
                }
            }
            assert!(context.derive(0x80000000).is_err());
            assert!(context.derive_on_branch(2, 0).is_err());
        }
    }
    let btc = UtxoDerivation::new(Chain::Bitcoin, SEED, "m/84'/0'/0'/0/0".into()).unwrap();
    // BIP-84 published first receiving address.
    assert_eq!(
        btc.derive(0).unwrap().0,
        "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"
    );
}

#[test]
fn discovery_refuses_paths_whose_suffix_cannot_name_a_public_receive_or_change_child() {
    for path in [
        "m/84'/2'/0'/0'/0",
        "m/84'/2'/0'/0/0'",
        "m/84'/2'/0'/2/0",
        "m/0'/0",
        "m/0",
        "m/84'/0'/0'/0/0",
        "m/100'/2'/0'/0/0",
    ] {
        assert!(
            UtxoDerivation::new(Chain::Litecoin, SEED, path.into()).is_err(),
            "{path}"
        );
    }
}

#[tokio::test]
async fn known_utxo_addresses_deduplicate_witness_addresses_by_their_canonical_case() {
    for (chain, base) in [
        (Chain::Bitcoin, "m/84'/0'/0'/0/0"),
        (Chain::Litecoin, "m/84'/2'/0'/0/0"),
    ] {
        let context = UtxoDerivation::new(chain, SEED, base.into()).unwrap();
        let root = context.derive(0).unwrap().0;
        let service = WalletService::new(Vec::new()).unwrap();
        let database = std::env::temp_dir()
            .join(format!("utxo-case-{}.sqlite", crate::store::new_event_id()))
            .to_string_lossy()
            .into_owned();
        service.open_state(database).await.unwrap();
        service
            .apply_state_command(StateCommand::UpsertWallet {
                wallet: WalletState::single_address(
                    "watch",
                    "Watch",
                    chain,
                    root.to_ascii_uppercase(),
                    None,
                    true,
                ),
            })
            .await
            .unwrap();
        service
            .register_owned_address("watch".into(), chain, root.clone(), None, None, None)
            .await
            .unwrap();
        assert_eq!(
            service
                .known_utxo_addresses("watch".into(), chain)
                .await
                .unwrap(),
            vec![root]
        );
    }
}

async fn scanning_service(endpoint: String) -> (Arc<WalletService>, String) {
    use crate::store::secret_backends::InMemorySecretStore;
    let service = WalletService::new(vec![crate::service::ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: crate::registry::Chain::Bitcoin,
        endpoints: vec![endpoint],
    }])
    .unwrap();
    let dir = std::env::temp_dir()
        .join(format!("discovery-{}.sqlite", crate::store::new_event_id()))
        .to_string_lossy()
        .into_owned();
    service.open_state(dir.clone()).await.unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    crate::store::wallet_secrets::store_seed_phrase(&*secrets, "scan", SEED, None).unwrap();
    service.set_secret_store(secrets);
    service
        .apply_state_command(StateCommand::UpsertWallet {
            wallet: WalletState::single_address(
                "scan",
                "Scan",
                crate::registry::Chain::Bitcoin,
                "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu",
                Some("m/84'/0'/0'/0/0".into()),
                false,
            ),
        })
        .await
        .unwrap();
    (service, dir)
}

#[tokio::test]
async fn discovery_has_four_in_flight_probes_and_returns_index_order() {
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::{Semaphore, mpsc};
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
    let (service, _dir) = scanning_service(endpoint).await;
    let scan_service = service.clone();
    let scan = tokio::spawn(async move {
        scan_service
            .discover_utxo_addresses("scan".into(), crate::registry::Chain::Bitcoin)
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
    assert!(
        tokio::time::timeout(Duration::from_millis(50), rx.recv())
            .await
            .is_err()
    );
    permits.add_permits(100);
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let addresses = scan.await.unwrap().unwrap();
    let context = UtxoDerivation::new(Chain::Bitcoin, SEED, "m/84'/0'/0'/0/0".into()).unwrap();
    assert_eq!(addresses, vec![context.derive(0).unwrap().0]);
    server.abort();
}

#[tokio::test]
async fn activity_probes_include_pending_and_spent_addresses_without_transaction_bodies() {
    use serde_json::json;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
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
        let service = WalletService::new(vec![crate::service::ChainEndpoints {
            capabilities: EndpointCapability::ALL.to_vec(),
            chain_id: chain,
            endpoints: vec![server.uri()],
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
    let service = WalletService::new(vec![crate::service::ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: Chain::BitcoinSV,
        endpoints: vec![server.uri()],
    }])
    .unwrap();
    assert!(
        service
            .utxo_address_has_activity(Chain::BitcoinSV, "a")
            .await
            .unwrap()
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn malformed_activity_is_an_error_and_does_not_advance_or_register() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    let (service, _dir) = scanning_service(server.uri()).await;
    let before = service
        .reserve_receive_index("scan".into(), crate::registry::Chain::Bitcoin, 1)
        .await
        .unwrap();
    assert!(
        service
            .discover_utxo_addresses("scan".into(), crate::registry::Chain::Bitcoin)
            .await
            .is_err()
    );
    assert!(
        service
            .advance_used_utxo_reservations(crate::registry::Chain::Bitcoin)
            .await
            .is_err()
    );
    assert_eq!(
        service
            .keypool_state("scan".into(), crate::registry::Chain::Bitcoin)
            .await
            .unwrap()
            .reserved_receive_index,
        Some(before)
    );
    assert!(
        service
            .keypool
            .read()
            .await
            .owned_everywhere()
            .next()
            .is_none()
    );
}

#[tokio::test]
async fn litecoin_segwit_recovers_receive_and_change_past_the_old_scan_bound() {
    use crate::store::secret_backends::InMemorySecretStore;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{any, path},
    };

    for chain in [Chain::Litecoin, Chain::LitecoinTestnet] {
        for purpose in [49, 84] {
            let coin = if chain.is_testnet() { 1 } else { 2 };
            let base = format!("m/{purpose}'/{coin}'/2'/0/0");
            let context = UtxoDerivation::new(chain, SEED, base.clone()).unwrap();
            let root = context.derive(0).unwrap().0;
            let receive = context.derive(7).unwrap();
            let change = context.derive_on_branch(1, 4).unwrap();
            let server = MockServer::start().await;
            Mock::given(any())
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"txs":0,"unconfirmedTxs":0})),
                )
                .with_priority(10)
                .mount(&server)
                .await;
            for address in [&receive.0, &change.0] {
                Mock::given(path(format!("/api/v2/address/{address}")))
                    .respond_with(
                        ResponseTemplate::new(200)
                            .set_body_json(json!({"txs":1,"unconfirmedTxs":0})),
                    )
                    .expect(1)
                    .mount(&server)
                    .await;
            }
            let service = WalletService::new(vec![crate::service::ChainEndpoints {
                capabilities: EndpointCapability::ALL.to_vec(),
                chain_id: chain,
                endpoints: vec![server.uri()],
            }])
            .unwrap();
            let database = std::env::temp_dir()
                .join(format!(
                    "ltc-discovery-{}.sqlite",
                    crate::store::new_event_id()
                ))
                .to_string_lossy()
                .into_owned();
            service.open_state(database.clone()).await.unwrap();
            let secrets = Arc::new(InMemorySecretStore::new());
            crate::store::wallet_secrets::store_seed_phrase(&*secrets, "scan", SEED, None).unwrap();
            service.set_secret_store(secrets);
            let mut wallet = WalletState::single_address(
                "scan",
                "Scan",
                chain,
                &root,
                Some(base.clone()),
                false,
            );
            wallet.xpub = Some(
                UtxoDerivation::account_xpub(chain, SEED, &base, &Default::default()).unwrap(),
            );
            service
                .apply_state_command(StateCommand::UpsertWallet { wallet })
                .await
                .unwrap();

            let reserved = service
                .receive_address("scan".into(), chain, true)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(reserved, context.derive(1).unwrap().0);
            let found = service
                .discover_utxo_addresses("scan".into(), chain)
                .await
                .unwrap();
            assert_eq!(
                found,
                vec![root, reserved, receive.0.clone(), change.0.clone()]
            );
            let requests = server.received_requests().await.unwrap();
            for (branch, index) in [(0, 27), (1, 24)] {
                let address = context.derive_on_branch(branch, index).unwrap().0;
                assert!(
                    requests
                        .iter()
                        .any(|request| request.url.path() == format!("/api/v2/address/{address}"))
                );
            }
            let state = service.keypool_state("scan".into(), chain).await.unwrap();
            assert_eq!(state.next_external_index, 8);
            assert_eq!(state.next_change_index, 5);
            assert_eq!(state.reserved_receive_index, Some(1));
            let tables = service.keypool.read().await;
            for (address, path, branch, index) in [
                (&receive.0, &receive.1, "external", 7),
                (&change.0, &change.1, "change", 4),
            ] {
                let row = tables
                    .owned_on(chain)
                    .iter()
                    .find(|row| &row.address == address)
                    .unwrap();
                assert_eq!(row.derivation_path.as_deref(), Some(path.as_str()));
                assert_eq!(row.branch.as_deref(), Some(branch));
                assert_eq!(row.branch_index, Some(index));
            }
            drop(tables);
            let restored = WalletService::new(Vec::new()).unwrap();
            restored.open_state(database).await.unwrap();
            assert_eq!(
                restored.keypool_state("scan".into(), chain).await.unwrap(),
                state
            );
            let addresses = restored
                .known_utxo_addresses("scan".into(), chain)
                .await
                .unwrap();
            assert!(addresses.contains(&receive.0));
            assert!(addresses.contains(&change.0));
        }
    }
}

#[tokio::test]
async fn discovery_scans_past_known_floor_and_refuses_to_truncate_at_the_ceiling() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "address":"unused", "chain_stats":{"funded_txo_sum":0,"spent_txo_sum":0,"tx_count":0}, "mempool_stats":{"funded_txo_sum":0,"spent_txo_sum":0,"tx_count":0}
        })))
        .mount(&server)
        .await;
    let (service, _) = scanning_service(server.uri()).await;
    service
        .reserve_receive_index("scan".into(), Chain::Bitcoin, 31)
        .await
        .unwrap();
    service
        .discover_utxo_addresses("scan".into(), Chain::Bitcoin)
        .await
        .unwrap();
    let context = UtxoDerivation::new(Chain::Bitcoin, SEED, "m/84'/0'/0'/0/0".into()).unwrap();
    let tail = context.derive(51).unwrap().0;
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|request| request.url.path() == format!("/address/{tail}"))
    );
    {
        let mut tables = service.keypool.write().await;
        tables.set_state(
            super::keypool_key("scan", Chain::Bitcoin),
            crate::wallet_db::KeypoolState {
                next_external_index: 990,
                next_change_index: 0,
                reserved_receive_index: None,
            },
        );
    }
    let error = service
        .discover_utxo_addresses("scan".into(), Chain::Bitcoin)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("ceiling 999"));
}

#[tokio::test]
async fn imported_litecoin_root_paths_raise_receive_and_change_floors() {
    use crate::store::secret_backends::InMemorySecretStore;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};
    for root_branch in [0, 1] {
        let chain = Chain::Litecoin;
        let base = format!("m/84'/2'/2'/{root_branch}/80");
        let context = UtxoDerivation::new(chain, SEED, base.clone()).unwrap();
        let root = context.derive_on_branch(root_branch, 80).unwrap().0;
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"txs":0,"unconfirmedTxs":0})),
            )
            .mount(&server)
            .await;
        let service = WalletService::new(vec![crate::service::ChainEndpoints {
            capabilities: EndpointCapability::ALL.to_vec(),
            chain_id: chain,
            endpoints: vec![server.uri()],
        }])
        .unwrap();
        let database = std::env::temp_dir()
            .join(format!(
                "ltc-root-floor-{}.sqlite",
                crate::store::new_event_id()
            ))
            .to_string_lossy()
            .into_owned();
        service.open_state(database).await.unwrap();
        let secrets = Arc::new(InMemorySecretStore::new());
        crate::store::wallet_secrets::store_seed_phrase(&*secrets, "scan", SEED, None).unwrap();
        service.set_secret_store(secrets);
        let mut wallet =
            WalletState::single_address("scan", "Scan", chain, &root, Some(base.clone()), false);
        wallet.xpub =
            Some(UtxoDerivation::account_xpub(chain, SEED, &base, &Default::default()).unwrap());
        service
            .apply_state_command(StateCommand::UpsertWallet { wallet })
            .await
            .unwrap();
        let state = service.keypool_state("scan".into(), chain).await.unwrap();
        assert_eq!(
            state.next_external_index,
            if root_branch == 0 { 81 } else { 1 }
        );
        assert_eq!(
            state.next_change_index,
            if root_branch == 1 { 81 } else { 0 }
        );
        let reserved = service
            .receive_address("scan".into(), chain, true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            reserved,
            context
                .derive(if root_branch == 0 { 81 } else { 1 })
                .unwrap()
                .0
        );
        let addresses = service
            .discover_utxo_addresses("scan".into(), chain)
            .await
            .unwrap();
        assert!(addresses.contains(&root));
        let tail = context.derive_on_branch(root_branch, 100).unwrap().0;
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|request| request.url.path() == format!("/api/v2/address/{tail}"))
        );
    }
}

#[test]
fn public_utxo_accounts_match_seed_derivation_and_refuse_mismatched_identity() {
    use crate::derivation::bitcoin::{
        ExtendedPublicKey, XPUB_VERSION_MAINNET, XPUB_VERSION_TESTNET,
    };
    let overrides = crate::store::wallet_domain::WalletDerivationOverrides {
        passphrase: Some("public account fixture".into()),
        ..Default::default()
    };
    for chain in Chain::all().filter(|chain| chain.uses_account_utxo()) {
        let default = crate::derivation::path::default_path_from_catalog(chain).unwrap();
        let coin = crate::derivation::bitcoin::parse_bip32_path(&default).unwrap()[1]
            - crate::derivation::primitives::HARDENED_OFFSET;
        let version = if chain.is_testnet() {
            XPUB_VERSION_TESTNET
        } else {
            XPUB_VERSION_MAINNET
        };
        for purpose in [44, 49, 84, 86] {
            if purpose == 86 && chain.mainnet_counterpart() != Chain::Peercoin {
                continue;
            }
            let base = format!("m/{purpose}'/{coin}'/2'/1/9");
            let seed_context =
                UtxoDerivation::with_overrides(chain, SEED, base.clone(), &overrides).unwrap();
            let root = seed_context.derive_on_branch(1, 9).unwrap().0;
            let xpub = UtxoDerivation::account_xpub(chain, SEED, &base, &overrides).unwrap();
            let public_context =
                UtxoDerivation::from_account_xpub(chain, &xpub, base.clone(), &root).unwrap();
            for branch in [0, 1] {
                for index in [0, 7, 40] {
                    assert_eq!(
                        public_context.derive_on_branch(branch, index).unwrap(),
                        seed_context.derive_on_branch(branch, index).unwrap()
                    );
                }
            }
            let (account, _) = ExtendedPublicKey::from_xpub_string(&xpub).unwrap();
            let wrong_network = account.to_xpub_string(if chain.is_testnet() {
                XPUB_VERSION_MAINNET
            } else {
                XPUB_VERSION_TESTNET
            });
            assert!(
                UtxoDerivation::from_account_xpub(chain, &wrong_network, base.clone(), &root)
                    .is_err()
            );
            for (depth, child_number) in [(2, account.child_number), (3, account.child_number + 1)]
            {
                let mut altered = account.clone();
                altered.depth = depth;
                altered.child_number = child_number;
                assert!(
                    UtxoDerivation::from_account_xpub(
                        chain,
                        &altered.to_xpub_string(version),
                        base.clone(),
                        &root
                    )
                    .is_err()
                );
            }
            let different =
                UtxoDerivation::with_overrides(chain, SEED, base.clone(), &Default::default())
                    .unwrap()
                    .derive_on_branch(1, 9)
                    .unwrap()
                    .0;
            assert!(UtxoDerivation::from_account_xpub(chain, &xpub, base, &different).is_err());
        }
    }
}

#[tokio::test]
async fn protected_account_utxo_public_context_receives_after_restart_without_opening_secrets() {
    for chain in Chain::all().filter(|chain| chain.uses_account_utxo()) {
        let default = crate::derivation::path::default_path_from_catalog(chain).unwrap();
        let coin = crate::derivation::bitcoin::parse_bip32_path(&default).unwrap()[1]
            - crate::derivation::primitives::HARDENED_OFFSET;
        let base = format!("m/84'/{coin}'/2'/0/0");
        let context = UtxoDerivation::new(chain, SEED, base.clone()).unwrap();
        let root = context.derive(0).unwrap().0;
        let mut wallet =
            WalletState::single_address("sealed", "Sealed", chain, root, Some(base.clone()), false);
        wallet.signing = crate::store::state::WalletSigning::SeedPhrase {
            password_protected: true,
        };
        wallet.xpub =
            Some(UtxoDerivation::account_xpub(chain, SEED, &base, &Default::default()).unwrap());
        let database = std::env::temp_dir()
            .join(format!(
                "account-utxo-public-receive-{}.sqlite",
                crate::store::new_event_id()
            ))
            .to_string_lossy()
            .into_owned();
        let service = WalletService::new(Vec::new()).unwrap();
        service.open_state(database.clone()).await.unwrap();
        service
            .apply_state_command(StateCommand::UpsertWallet { wallet })
            .await
            .unwrap();
        let receiver = service
            .receive_address("sealed".into(), chain, true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receiver, context.derive(1).unwrap().0);
        let reopened = WalletService::new(Vec::new()).unwrap();
        reopened.open_state(database).await.unwrap();
        assert_eq!(
            reopened
                .receive_address("sealed".into(), chain, false)
                .await
                .unwrap()
                .as_deref(),
            Some(receiver.as_str())
        );
    }
}

#[test]
fn owned_receive_derivation_uses_the_wallet_passphrase() {
    let path = "m/84'/0'/2'/0/0".to_string();
    let overrides = crate::store::wallet_domain::WalletDerivationOverrides {
        passphrase: Some("different wallet".into()),
        ..Default::default()
    };
    let context =
        UtxoDerivation::with_overrides(Chain::Bitcoin, SEED, path.clone(), &overrides).unwrap();
    let (address, derived_path) = context.derive(3).unwrap();
    let expected = crate::derivation::dispatch::derive_for_chain(
        crate::registry::Chain::Bitcoin,
        SEED,
        &derived_path,
        Some("different wallet"),
        None,
        None,
        true,
        false,
        false,
    )
    .unwrap()
    .address
    .unwrap();
    assert_eq!(address, expected);
    assert_ne!(
        address,
        UtxoDerivation::new(Chain::Bitcoin, SEED, path)
            .unwrap()
            .derive(3)
            .unwrap()
            .0
    );
}
