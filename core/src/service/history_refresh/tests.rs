use super::*;
use crate::store::state::{AppSettings, WalletAddress, WalletSummary};

fn wallet(id: &str, chain: Chain, addresses: &[(Chain, &str)]) -> WalletSummary {
    WalletSummary {
        id: id.to_string(),
        name: format!("{id} wallet"),
        is_watch_only: false,
        chain_name: chain.chain_display_name().to_string(),
        include_in_portfolio_total: true,
        network_mode: None,
        xpub: None,
        derivation_preset: "standard".to_string(),
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
    let mut state = CoreAppState::default();
    state.wallets = vec![
        wallet("w1", Chain::Solana, &[(Chain::Solana, "So1")]),
        wallet("w2", Chain::Solana, &[]),
        wallet("w3", Chain::Bitcoin, &[(Chain::Bitcoin, "bc1")]),
    ];

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
fn a_testnet_wallet_fetches_its_network_and_files_under_its_family() {
    let mut state = CoreAppState::default();
    state.wallets = vec![wallet(
        "w1",
        Chain::Bitcoin,
        &[
            (Chain::Bitcoin, "bc1main"),
            (Chain::BitcoinTestnet4, "tb1test"),
        ],
    )];
    state.settings.network_chain_by_family.insert(
        Chain::Bitcoin.str_id().to_string(),
        Chain::BitcoinTestnet4.str_id().to_string(),
    );

    let target = &targets(&state, Chain::Bitcoin, &[])[0];
    assert_eq!(target.network, Chain::BitcoinTestnet4, "fetched from");
    assert_eq!(target.address, "tb1test");

    let record = record_for(
        target,
        Chain::Bitcoin,
        crate::fetch::history_decode::NormalizedHistoryItem {
            kind: "receive".to_string(),
            status: "confirmed".to_string(),
            asset_name: "Bitcoin Testnet4".to_string(),
            symbol: "BTC".to_string(),
            chain_name: "Bitcoin Testnet4".to_string(),
            amount: 1.0,
            counterparty: "tb1other".to_string(),
            tx_hash: "abc".to_string(),
            block_height: None,
            timestamp: 1.0,
        },
    );
    assert_eq!(record.chain_name, "Bitcoin", "filed under");
}

/// A wallet on a testnet fetches the address for that network.
#[test]
fn a_target_follows_the_network_the_wallet_is_on() {
    let mut state = CoreAppState::default();
    state.wallets = vec![wallet(
        "w1",
        Chain::Bitcoin,
        &[
            (Chain::Bitcoin, "bc1main"),
            (Chain::BitcoinTestnet4, "tb1test"),
        ],
    )];
    assert_eq!(targets(&state, Chain::Bitcoin, &[])[0].address, "bc1main");

    state.settings = AppSettings {
        network_chain_by_family: [(
            Chain::Bitcoin.str_id().to_string(),
            Chain::BitcoinTestnet4.str_id().to_string(),
        )]
        .into_iter()
        .collect(),
        ..AppSettings::default()
    };
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
            kind: "receive".to_string(),
            status: "confirmed".to_string(),
            asset_name: "Solana".to_string(),
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
        kind: "send".to_string(),
        status: "confirmed".to_string(),
        asset_name: "Solana".to_string(),
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
            token: crate::tokens::TokenEntry {
                chain: chain.to_string(),
                name: "Token".to_string(),
                symbol: "TKN".to_string(),
                token_standard: "erc20".to_string(),
                contract: contract.to_string(),
                coingecko_id: String::new(),
                decimals: 6,
                tags: Vec::new(),
                color: String::new(),
                asset_name: String::new(),
                enabled: true,
            },
            category: CoreTokenPreferenceCategory::Stablecoin,
            is_built_in: true,
            is_enabled: enabled,
        }
    }
    let mut state = CoreAppState::default();
    state.token_preferences = vec![
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
    ];

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
    let service = WalletService::new_typed(Vec::new()).expect("service");
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
    let service = WalletService::new_typed(Vec::new()).expect("service");
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

    let service = WalletService::new_typed(vec![crate::service::ChainEndpoints {
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
    let service = WalletService::new_typed(Vec::new()).expect("service");
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
    let empty = WalletService::new_typed(Vec::new()).expect("service");
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
    let service = WalletService::new_typed(Vec::new()).expect("service");
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
    let service = WalletService::new_typed(Vec::new()).expect("service");
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
