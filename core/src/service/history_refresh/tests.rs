use super::*;
use crate::store::state::{AppSettings, WalletAddress, WalletState};

fn wallet(id: &str, chain: Chain, addresses: &[(Chain, &str)]) -> WalletState {
    WalletState {
        id: id.to_string(),
        name: format!("{id} wallet"),
        is_watch_only: false,
        chain_name: chain.chain_display_name().to_string(),
        include_in_portfolio_total: true,
        chain_id: chain.str_id().into(),
        xpub: None,
        derivation_preset: crate::store::wallet_domain::CoreSeedDerivationPreset::Standard,
        derivation_path: None,
        derivation_overrides: Default::default(),
        holdings: Vec::new(),
        addresses: addresses
            .iter()
            .map(|(chain, address)| WalletAddress {
                chain_name: chain.chain_display_name().to_string(),
                address: (*address).to_string(),
                kind: "receive".to_string(),
                derivation_path: None,
            })
            .collect(),
    }
}

/// Only the wallets on the chain, only the ones with an address, and the
/// address for the network each is on.
#[test]
fn targets_are_the_chains_wallets_that_have_an_address() {
    let state = CoreAppState {
        wallets: vec![
            wallet("w1", Chain::Solana, &[(Chain::Solana, "So1")]),
            wallet("w2", Chain::Solana, &[]),
            wallet("w3", Chain::Bitcoin, &[(Chain::Bitcoin, "bc1")]),
        ],
        ..Default::default()
    };

    let solana = targets(&state, Chain::Solana, &[]);
    assert_eq!(solana.len(), 1);
    assert_eq!(solana[0].wallet_id, "w1");
    assert_eq!(solana[0].address, "So1");
    assert_eq!(solana[0].wallet_name, "w1 wallet");

    // Scoped to one wallet, case-insensitively — ids cross the boundary in
    // whichever case the front end holds them.
    assert!(targets(&state, Chain::Solana, &["W1".to_string()]).len() == 1);
    assert!(targets(&state, Chain::Solana, &["w3".to_string()]).is_empty());
}

/// A wallet on a testnet fetches that network's history, and the record
/// still lands under the family.
///
/// Filing under the network would rename the asset and leave the mainnet
/// holding beside it; fetching from the family read the mainnet chain and
/// found nothing. The two are separate answers.
#[test]
fn a_testnet_wallet_fetches_and_persists_its_exact_network() {
    let mut state = CoreAppState {
        wallets: vec![wallet(
            "w1",
            Chain::Bitcoin,
            &[
                (Chain::Bitcoin, "bc1main"),
                (Chain::BitcoinTestnet4, "tb1test"),
            ],
        )],
        ..Default::default()
    };
    state.settings.selected_chain_by_family.insert(
        Chain::Bitcoin.str_id().to_string(),
        Chain::BitcoinTestnet4.str_id().to_string(),
    );

    state.wallets[0].chain_id = "bitcoin-testnet-4".into();
    let target = &targets(&state, Chain::Bitcoin, &[])[0];
    assert_eq!(target.network, Chain::BitcoinTestnet4, "fetched from");
    assert_eq!(target.address, "tb1test");

    let record = record_for(
        target,
        Chain::Bitcoin,
        crate::fetch::history_decode::NormalizedHistoryItem {
            deployment_id: None,
            kind: "receive".to_string(),
            status: "confirmed".to_string(),
            asset_display_name: "Bitcoin".to_string(),
            symbol: "tBTC".to_string(),
            chain_name: "Bitcoin Testnet4".to_string(),
            amount: 1.0,
            counterparty: "tb1other".to_string(),
            tx_hash: "abc".to_string(),
            block_height: None,
            timestamp: 1.0,
        },
    );
    assert_eq!(record.chain_name, "Bitcoin Testnet4", "filed under");
    assert_eq!(
        record.deployment_id.as_deref(),
        Some("bitcoin-testnet-4:native")
    );
}

/// A wallet on a testnet fetches the address for that network.
#[test]
fn a_target_follows_the_network_the_wallet_is_on() {
    let mut state = CoreAppState {
        wallets: vec![wallet(
            "w1",
            Chain::Bitcoin,
            &[
                (Chain::Bitcoin, "bc1main"),
                (Chain::BitcoinTestnet4, "tb1test"),
            ],
        )],
        ..Default::default()
    };
    assert_eq!(targets(&state, Chain::Bitcoin, &[])[0].address, "bc1main");

    state.settings = AppSettings {
        selected_chain_by_family: [(
            Chain::Bitcoin.str_id().to_string(),
            Chain::BitcoinTestnet4.str_id().to_string(),
        )]
        .into_iter()
        .collect(),
        ..AppSettings::default()
    };
    state.wallets[0].chain_id = "bitcoin-testnet-4".into();
    assert_eq!(targets(&state, Chain::Bitcoin, &[])[0].address, "tb1test");
}

/// The record carries the wallet the entry belongs to, and an id a front
/// end can parse back.
#[test]
fn a_record_names_its_wallet_and_carries_a_uuid() {
    let target = Target {
        wallet_id: "w1".to_string(),
        wallet_name: "Main".to_string(),
        address: "So1".to_string(),
        network: Chain::Solana,
    };
    let record = record_for(
        &target,
        Chain::Solana,
        crate::fetch::history_decode::NormalizedHistoryItem {
            deployment_id: None,
            kind: "receive".to_string(),
            status: "confirmed".to_string(),
            asset_display_name: "Solana".to_string(),
            symbol: "SOL".to_string(),
            chain_name: "Solana".to_string(),
            amount: 1.5,
            counterparty: "So2".to_string(),
            tx_hash: "sig".to_string(),
            block_height: Some(7),
            timestamp: 1_700_000_000.0,
        },
    );
    assert_eq!(record.wallet_id.as_deref(), Some("w1"));
    assert_eq!(record.wallet_name, "Main");
    assert_eq!(record.transaction_history_source.as_deref(), Some("rust"));
    assert_eq!(record.created_at_unix, 1_700_000_000.0);
    assert_eq!(record.receipt_block_number, Some(7));
    // Parseable as a UUID: a front end drops a row whose id is not.
    assert_eq!(record.id.len(), 36);
    assert_eq!(
        record.id.chars().filter(|c| *c == '-').count(),
        4,
        "{}",
        record.id
    );
    assert_eq!(&record.id[14..15], "4", "version nibble: {}", record.id);

    // An empty hash is no hash, not an empty one.
    let mut entry = crate::fetch::history_decode::NormalizedHistoryItem {
        deployment_id: None,
        kind: "send".to_string(),
        status: "confirmed".to_string(),
        asset_display_name: "Solana".to_string(),
        symbol: "SOL".to_string(),
        chain_name: "Solana".to_string(),
        amount: 0.0,
        counterparty: String::new(),
        tx_hash: String::new(),
        block_height: None,
        timestamp: 0.0,
    };
    assert_eq!(
        record_for(&target, Chain::Solana, entry.clone()).transaction_hash,
        None
    );
    entry.tx_hash = "abc".to_string();
    assert_eq!(
        record_for(&target, Chain::Solana, entry)
            .transaction_hash
            .as_deref(),
        Some("abc")
    );
}

/// The tokens a page decodes with are the user's enabled ones for that
/// chain, contracts in their canonical form.
#[test]
fn descriptors_are_the_enabled_tokens_for_the_chain() {
    use crate::store::wallet_domain::{CoreTokenPreferenceCategory, CoreTokenPreferenceEntry};
    fn entry(chain: &str, contract: &str, enabled: bool) -> CoreTokenPreferenceEntry {
        CoreTokenPreferenceEntry {
            token: crate::tokens::TokenDeploymentEntry {
                deployment_id: "fixture:token".into(),
                token_id: "fixture:token".into(),
                kind: crate::tokens::TokenKind::Protocol {
                    standard: "fixture".into(),
                    identifier: "fixture".into(),
                },
                chain_id: chain.to_string(),
                name: "Token".to_string(),
                symbol: "TKN".to_string(),
                token_standard: "erc20".to_string(),
                contract: contract.to_string(),
                coingecko_id: String::new(),
                decimals: 6,
                tags: Vec::new(),
                color: None,
                artwork_name: String::new(),
                enabled: true,
            },
            category: CoreTokenPreferenceCategory::Stablecoin,
            is_built_in: true,
            is_enabled: enabled,
        }
    }
    let state = CoreAppState {
        token_preferences: vec![
            entry(
                "ethereum",
                "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                true,
            ),
            entry(
                "ethereum",
                "0xBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB",
                false,
            ),
            entry(
                "solana",
                "So11111111111111111111111111111111111111112",
                true,
            ),
        ],
        ..Default::default()
    };

    let descriptors = token_descriptors(&state, Chain::Ethereum);
    assert_eq!(descriptors.len(), 1);
    assert_eq!(
        descriptors[0].contract, "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "the contract crosses in its canonical form"
    );
    assert_eq!(descriptors[0].decimals, 6);
    // A chain that hosts no known tokens decodes none.
    assert!(token_descriptors(&state, Chain::EthereumClassic).is_empty());
}

/// A chain no explorer serves fails per wallet and says so, rather than
/// raising and losing the wallets that did answer.
///
/// Offline: `explorer_query_url` refuses before any request is made.
#[tokio::test]
async fn a_chain_no_explorer_serves_counts_a_failure_and_reports_it() {
    let service = WalletService::new(Vec::new()).expect("service");
    // The merge writes, so the store has to be open — a failed page still
    // ends in a merge of nothing.
    let db = std::env::temp_dir()
        .join(format!(
            "spectra-evm-history-{}.sqlite",
            crate::store::new_event_id()
        ))
        .to_string_lossy()
        .into_owned();
    service.open_state(db).await.expect("open");
    service
        .apply_state_command(crate::store::state::StateCommand::UpsertWallet {
            wallet: wallet("w1", Chain::Cronos, &[(Chain::Ethereum, "0xabc")]),
        })
        .await
        .expect("wallet");

    let outcome = service
        .refresh_evm_chain_history(Chain::Cronos.str_id().to_string(), Vec::new(), false, None)
        .await
        .expect("refresh");
    assert_eq!(outcome.wallets_refreshed, 0);
    assert_eq!(outcome.wallets_failed, 1);
    assert_eq!(outcome.added, 0);
    assert!(!outcome.exhausted, "a failed page is not the last page");
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].wallet_id, "w1");
    assert_eq!(outcome.diagnostics[0].source_used, "none");
    assert!(outcome.diagnostics[0].error.is_some());
}

/// A UTXO wallet with no known addresses is not refreshed, and asking for
/// an unknown chain is refused.
///
/// Offline: the keypool is empty, so no provider is reached.
#[tokio::test]
async fn a_utxo_wallet_with_no_known_addresses_is_skipped() {
    let service = WalletService::new(Vec::new()).expect("service");
    let db = std::env::temp_dir()
        .join(format!(
            "spectra-utxo-history-{}.sqlite",
            crate::store::new_event_id()
        ))
        .to_string_lossy()
        .into_owned();
    service.open_state(db).await.expect("open");
    service
        .apply_state_command(crate::store::state::StateCommand::UpsertWallet {
            wallet: wallet("w1", Chain::Litecoin, &[(Chain::Litecoin, "ltc1")]),
        })
        .await
        .expect("wallet");

    let outcome = service
        .refresh_utxo_chain_history(Chain::Litecoin.str_id().to_string(), Vec::new(), false)
        .await
        .expect("refresh");
    assert_eq!(outcome.wallets_refreshed, 0);
    assert_eq!(outcome.wallets_failed, 0);
    assert_eq!(outcome.added, 0);

    assert!(service
        .refresh_utxo_chain_history("not-a-chain".to_string(), Vec::new(), false)
        .await
        .is_err());
}

/// A UTXO wallet one of whose addresses did not answer stores nothing.
///
/// Netting is over the whole address set, so an address that did not
/// answer is a wrong amount rather than a missing row: a transaction whose
/// change went there nets to the legs that did answer. The refresh used to
/// aggregate and merge whatever came back, so a figure no address agreed
/// with was stored — here the send leg alone, unnetted by the change leg
/// the failing address holds. The wallet is counted failed, nothing is
/// merged for it, and its cursor is left loadable so a later refresh nets
/// the whole set again.
///
/// One address answers with a transaction and the other refuses, which is
/// the case the offline gate cannot reach — hence the mock backend.
#[tokio::test]
async fn a_utxo_wallet_whose_address_did_not_answer_stores_nothing() {
    use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};
    const ANSWERS: &str = "ltc1qw508d6qejxtdg4y5r3zarvary0c5xw7kgmn4n9";
    const REFUSES: &str = "LhK2kQwiaAvhjWY799cZvMyYwnQAcxkarr";

    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(|request: &Request| {
            if !request.url.path().contains(ANSWERS) {
                return ResponseTemplate::new(500);
            }
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "transactions": [{
                    "txid": "aa11",
                    "blockTime": 1_700_000_000u64,
                    "blockHeight": 900_000u64,
                    "value": "100000",
                    "fees": "500",
                    "vin": [{ "addresses": [ANSWERS] }],
                }]
            }))
        })
        .mount(&server)
        .await;

    let service = WalletService::new(vec![crate::service::ChainEndpoints {
        chain_id: Chain::Litecoin.str_id().into(),
        endpoints: vec![server.uri()],
        api_key: None,
    }])
    .expect("service");
    let db = std::env::temp_dir()
        .join(format!(
            "spectra-utxo-fail-{}.sqlite",
            crate::store::new_event_id()
        ))
        .to_string_lossy()
        .into_owned();
    service.open_state(db).await.expect("open");
    service
        .apply_state_command(crate::store::state::StateCommand::UpsertWallet {
            wallet: wallet("w1", Chain::Litecoin, &[(Chain::Litecoin, ANSWERS)]),
        })
        .await
        .expect("wallet");
    // The second address is the wallet's keypool, which is where the
    // refresh reads the rest of the set from.
    service
        .register_owned_address(
            "w1".to_string(),
            Chain::Litecoin.chain_display_name().to_string(),
            REFUSES.to_string(),
            None,
            None,
            None,
        )
        .await
        .expect("owned address");

    let outcome = service
        .refresh_utxo_chain_history(Chain::Litecoin.str_id().to_string(), Vec::new(), false)
        .await
        .expect("refresh");
    assert_eq!(outcome.wallets_refreshed, 0);
    assert_eq!(outcome.wallets_failed, 1);
    assert_eq!(
        outcome.added, 0,
        "a half-fetched transaction nets to a figure no address agrees with"
    );
    assert_eq!(outcome.updated, 0);
    assert!(
        !service
            .history_cursor(Chain::Litecoin.str_id().to_string(), "w1".to_string())
            .is_exhausted,
        "a wallet that failed must stay loadable"
    );
}

/// A Bitcoin wallet with neither an address nor an xpub is a failure with
/// a reason, not a silent skip.
///
/// Offline: the three sources are tried in order and none of them has an
/// identifier to fetch for, so no provider is reached.
#[tokio::test]
async fn a_bitcoin_wallet_with_nothing_to_fetch_for_says_so() {
    let service = WalletService::new(Vec::new()).expect("service");
    let db = std::env::temp_dir()
        .join(format!(
            "spectra-btc-history-{}.sqlite",
            crate::store::new_event_id()
        ))
        .to_string_lossy()
        .into_owned();
    service.open_state(db).await.expect("open");
    service
        .apply_state_command(crate::store::state::StateCommand::UpsertWallet {
            wallet: wallet("w1", Chain::Bitcoin, &[]),
        })
        .await
        .expect("wallet");

    let outcome = service
        .refresh_bitcoin_history(Vec::new(), false, None)
        .await
        .expect("refresh");
    assert_eq!(outcome.wallets_refreshed, 0);
    assert_eq!(outcome.wallets_failed, 1);
    assert_eq!(outcome.added, 0);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].source_used, "none");
    assert!(outcome.diagnostics[0]
        .error
        .as_deref()
        .is_some_and(|error| error.contains("no Bitcoin address")));
    // The row names the wallet when it has nothing else to be named by.
    assert_eq!(outcome.diagnostics[0].identifier, "w1 wallet");
    // A failure leaves the cursor where it was. Writing `None` there says
    // "the chain confirms there is no more", which a fetch that failed did
    // not say: it marked the wallet exhausted, so this outcome reported
    // more to load while the wallet's own cursor refused to load it.
    assert!(!outcome.exhausted, "a failed page is not the last page");
    assert!(
        !service
            .history_cursor(Chain::Bitcoin.str_id().to_string(), "w1".to_string())
            .is_exhausted,
        "a failure must not mark the wallet exhausted"
    );

    // No Bitcoin wallets at all is not a failure.
    let empty = WalletService::new(Vec::new()).expect("service");
    let outcome = empty
        .refresh_bitcoin_history(Vec::new(), false, None)
        .await
        .expect("refresh");
    assert_eq!(outcome.wallets_failed, 0);
    assert!(outcome.exhausted);
}

/// Only an EVM chain has an explorer page to fetch.
#[tokio::test]
async fn a_non_evm_chain_is_refused() {
    let service = WalletService::new(Vec::new()).expect("service");
    assert!(service
        .refresh_evm_chain_history(Chain::Solana.str_id().to_string(), Vec::new(), false, None)
        .await
        .is_err());
    // With no wallets there is nothing to fetch, no error and no store to
    // write to.
    let outcome = service
        .refresh_evm_chain_history(
            Chain::Ethereum.str_id().to_string(),
            Vec::new(),
            false,
            None,
        )
        .await
        .expect("refresh");
    assert_eq!(outcome.wallets_refreshed, 0);
    assert!(outcome.exhausted);
}

/// A chain with no wallets is not an error and not a network call.
#[tokio::test]
async fn a_chain_with_no_wallets_refreshes_nothing() {
    let service = WalletService::new(Vec::new()).expect("service");
    let outcome = service
        .refresh_chain_history(Chain::Solana.str_id().to_string(), Vec::new())
        .await
        .expect("refresh");
    assert_eq!(outcome.wallets_refreshed, 0);
    assert_eq!(outcome.added, 0);
    assert!(service
        .refresh_chain_history("not-a-chain".to_string(), Vec::new())
        .await
        .is_err());
}

#[tokio::test]
async fn owned_history_scope_and_failed_clock_are_core_decisions() {
    use crate::service::HistoryRefreshScope;
    let service = WalletService::new(vec![]).unwrap();
    assert!(service
        .refresh_history(HistoryRefreshScope::All, false, None, 0.0)
        .await
        .is_err());
    let path = std::env::temp_dir().join(format!(
        "history-owned-{}.sqlite",
        crate::store::new_event_id()
    ));
    service
        .open_state(path.to_string_lossy().into())
        .await
        .unwrap();
    service
        .apply_state_command(crate::store::state::StateCommand::UpsertWallet {
            wallet: wallet("w1", Chain::Cronos, &[(Chain::Ethereum, "0xabc")]),
        })
        .await
        .unwrap();
    assert!(service
        .refresh_history(
            HistoryRefreshScope::Wallets { wallet_ids: vec![] },
            false,
            None,
            0.0
        )
        .await
        .unwrap()
        .is_empty());
    assert!(service
        .refresh_history(
            HistoryRefreshScope::Chains {
                chain_ids: vec!["invalid".into()]
            },
            false,
            None,
            0.0
        )
        .await
        .is_err());
    for _ in 0..2 {
        let result = service
            .refresh_history(HistoryRefreshScope::All, false, None, 3600.0)
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].outcome.as_ref().unwrap().wallets_failed, 1);
    }
    assert!(
        !service
            .history_cursor("cronos".into(), "w1".into())
            .is_exhausted
    );
    // The run records its own outcome where the diagnostics screen reads it:
    // a failed read marks the chain degraded, in the English template the
    // screen localizes.
    assert_eq!(
        service
            .diagnostic_state()
            .await
            .degraded
            .get("Cronos")
            .map(String::as_str),
        Some("Cronos history refresh failed. Using cached history.")
    );
}

#[test]
fn evm_groups_share_only_the_same_network_and_address() {
    let target = |id: &str, network, address: &str| Target {
        wallet_id: id.into(),
        wallet_name: id.into(),
        address: address.into(),
        network,
    };
    let targets = vec![
        target("main-a", Chain::Ethereum, "0xAbC"),
        target("test", Chain::EthereumSepolia, "0xabc"),
        target("main-b", Chain::Ethereum, "0xabc"),
    ];
    let groups = evm_history_groups(&targets, false);
    assert_eq!(groups.len(), 2);
    assert!(groups
        .iter()
        .any(|(ids, _)| ids == &vec!["main-a", "main-b"]));
    assert!(groups.iter().any(|(ids, _)| ids == &vec!["test"]));
    assert_eq!(evm_history_groups(&targets, true).len(), 3);
}

#[tokio::test]
async fn wallet_history_scope_does_not_consume_another_wallet_cooldown() {
    use crate::fetch::refresh::policy::HistoryRefreshKey;
    use crate::service::HistoryRefreshScope;
    let service = WalletService::new(vec![]).unwrap();
    let path = std::env::temp_dir().join(format!(
        "history-clock-{}.sqlite",
        crate::store::new_event_id()
    ));
    service
        .open_state(path.to_string_lossy().into())
        .await
        .unwrap();
    // Cronos has no keyless history provider: refusal is offline and remains retryable.
    for id in ["a", "b"] {
        service
            .apply_state_command(crate::store::state::StateCommand::UpsertWallet {
                wallet: wallet(id, Chain::Cronos, &[(Chain::Ethereum, "0xabc")]),
            })
            .await
            .unwrap();
    }
    service
        .record_history_refresh(HistoryRefreshKey::new("a", "cronos"))
        .await;
    let scope = |id: &str| HistoryRefreshScope::Wallets {
        wallet_ids: vec![id.into()],
    };
    assert!(service
        .refresh_history(scope("A"), false, None, 3600.0)
        .await
        .unwrap()
        .is_empty());
    for _ in 0..2 {
        let results = service
            .refresh_history(scope("B"), false, None, 3600.0)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].outcome.as_ref().unwrap().wallets_failed, 1);
    }
}

#[tokio::test]
async fn history_identity_merges_sends_on_the_exact_network_and_rejects_deleted_wallets() {
    use crate::fetch::history_decode::*;
    use crate::service::TransactionCommand;
    let service = WalletService::new(vec![]).unwrap();
    let path = std::env::temp_dir().join(format!(
        "history-identity-{}.sqlite",
        crate::store::new_event_id()
    ));
    let db = path.to_string_lossy().to_string();
    service.open_state(db.clone()).await.unwrap();
    let mut w = wallet(
        "w",
        Chain::Ethereum,
        &[(
            Chain::Ethereum,
            "0x1111111111111111111111111111111111111111",
        )],
    );
    w.chain_id = Chain::EthereumSepolia.str_id().into();
    service
        .apply_state_command(crate::store::state::StateCommand::UpsertWallet { wallet: w })
        .await
        .unwrap();
    let page = EvmHistoryPageDecoded {
        tokens: vec![],
        native: vec![EvmNativeTransferItem {
            status: "failed".into(),
            from_address: "0x1111111111111111111111111111111111111111".into(),
            to_address: "0x2222222222222222222222222222222222222222".into(),
            amount_decimal: "1".into(),
            transaction_hash: "same-hash".into(),
            block_number: 123,
            timestamp: 1700000000.0,
        }],
    };
    let fetched = evm_record(
        plan_evm_transaction_records(EvmTransactionRecordRequest {
            decoded_page: page,
            normalized_address: "0x1111111111111111111111111111111111111111".into(),
            chain_name: Chain::EthereumSepolia.chain_display_name().into(),
            token_source_used: None,
            native_asset_display_name: "Ether".into(),
            native_asset_symbol: "ETH".into(),
            wallets: vec![EvmTransactionRecordWalletInput {
                wallet_id: "w".into(),
                wallet_name: "W".into(),
            }],
            unknown_timestamp_sentinel_unix: 0.0,
        })
        .remove(0),
    );
    assert_eq!(fetched.status, "failed");
    assert_eq!(
        fetched.deployment_id.as_deref(),
        Some("ethereum-sepolia:native")
    );
    let mut local = fetched.clone();
    local.id = "local-send".into();
    local.status = "pending".into();
    service
        .apply_transaction_command(TransactionCommand::Upsert {
            records: vec![local.into()],
        })
        .await
        .unwrap();
    let change = service
        .merge_fetched_history(vec![fetched.clone()])
        .await
        .unwrap();
    assert!(change.added.is_empty());
    assert_eq!(change.updated, vec!["local-send"]);
    let stored = service.transactions().await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(
        stored[0].status,
        crate::store::wallet_domain::CoreTransactionStatus::Failed
    );
    assert_eq!(
        stored[0].chain_name,
        Chain::EthereumSepolia.chain_display_name()
    );
    let mut wrong_network = fetched.clone();
    wrong_network.chain_name = "Ethereum".into();
    wrong_network.id = "wrong-network".into();
    assert!(service
        .merge_fetched_history(vec![wrong_network])
        .await
        .unwrap()
        .is_empty());
    // The response was fetched before deletion and is submitted after the wallet transaction commits.
    crate::wallet_db::delete_wallet_data(&crate::wallet_db::WalletDatabase::new(&db), "w").unwrap();
    assert!(service
        .merge_fetched_history(vec![fetched])
        .await
        .unwrap()
        .is_empty());
    let reopened = WalletService::new(vec![]).unwrap();
    reopened.open_state(db).await.unwrap();
    assert!(reopened.transactions().await.unwrap().is_empty());
}

#[test]
fn history_tokens_with_the_same_symbol_keep_distinct_contract_identities() {
    assert_eq!(
        crate::tokens::history_deployment(
            Chain::EthereumSepolia,
            Some("0x1111111111111111111111111111111111111111")
        )
        .as_deref(),
        Some("ethereum-sepolia:erc-20:0x1111111111111111111111111111111111111111")
    );

    use crate::fetch::history_decode::*;
    let tokens = [
        "0x1111111111111111111111111111111111111111",
        "0x2222222222222222222222222222222222222222",
    ]
    .into_iter()
    .map(|contract| EvmTokenTransferItem {
        contract_address: contract.into(),
        token_name: "Same".into(),
        symbol: "SAME".into(),
        decimals: 6,
        from_address: "from".into(),
        to_address: "to".into(),
        amount_decimal: "1".into(),
        transaction_hash: "tx".into(),
        block_number: 1,
        log_index: 0,
        timestamp: 1.0,
    })
    .collect();
    let rows = plan_evm_transaction_records(EvmTransactionRecordRequest {
        decoded_page: EvmHistoryPageDecoded {
            tokens,
            native: vec![],
        },
        normalized_address: "from".into(),
        chain_name: "Ethereum".into(),
        token_source_used: None,
        native_asset_display_name: "Ether".into(),
        native_asset_symbol: "ETH".into(),
        wallets: vec![EvmTransactionRecordWalletInput {
            wallet_id: "w".into(),
            wallet_name: "W".into(),
        }],
        unknown_timestamp_sentinel_unix: 0.0,
    })
    .into_iter()
    .map(evm_record)
    .collect();
    let merged = crate::fetch::transactions::merge_transactions(
        crate::fetch::transactions::TransactionMergeRequest {
            existing_transactions: vec![],
            incoming_transactions: rows,
            strategy: crate::fetch::transactions::TransactionMergeStrategy::Evm,
            chain_name: "Ethereum".into(),
            include_symbol_in_identity: true,
            preserve_created_at_sentinel_unix: None,
        },
    );
    assert_eq!(merged.len(), 2);
    assert_ne!(merged[0].deployment_id, merged[1].deployment_id);
    assert!(merged.iter().all(|r| r.deployment_id.is_some()));
}
