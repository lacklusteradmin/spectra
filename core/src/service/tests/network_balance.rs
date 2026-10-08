use super::*;
use crate::service::address_discovery::UtxoDerivation;
use crate::store::state::WalletState;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

/// `value` as an address's confirmed balance, in the shape the chain's
/// default indexer answers.
fn indexer_balance(chain: Chain, value: &str) -> serde_json::Value {
    let units: u64 = value.parse().unwrap();
    match chain.default_api().unwrap() {
        crate::EndpointApi::Esplora => json!({
            "address": "",
            "chain_stats": {"funded_txo_sum": units, "spent_txo_sum": 0, "tx_count": 1},
            "mempool_stats": {"funded_txo_sum": 0, "spent_txo_sum": 0, "tx_count": 0},
        }),
        crate::EndpointApi::Blockcypher => json!({"balance": units, "unconfirmed_balance": 0}),
        crate::EndpointApi::Whatsonchain => json!({"confirmed": units, "unconfirmed": 0}),
        crate::EndpointApi::Insight => json!({"balanceSat": units, "balance": 0}),
        crate::EndpointApi::KaspaRest => json!({"address": "", "balance": units}),
        _ => json!({"balance": value, "unconfirmedBalance": "0"}),
    }
}

async fn reopened_account_with_balances(
    chain: Chain,
    receive_units: u64,
    change_units: u64,
) -> (Arc<WalletService>, std::path::PathBuf, MockServer) {
    reopened_account_holding(chain, 0, receive_units, change_units).await
}

async fn reopened_account_holding(
    chain: Chain,
    root_units: u64,
    receive_units: u64,
    change_units: u64,
) -> (Arc<WalletService>, std::path::PathBuf, MockServer) {
    let base = crate::derivation::path::default_path_from_catalog(chain).unwrap();
    let context = UtxoDerivation::new(chain, SEED, base.clone()).unwrap();
    let root = context.derive(0).unwrap().0;
    let receive = context.derive(4).unwrap();
    let change = context.derive_on_branch(1, 3).unwrap();
    let values = HashMap::from([
        (root.clone(), root_units.to_string()),
        (receive.0.clone(), receive_units.to_string()),
        (change.0.clone(), change_units.to_string()),
    ]);
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let path = request.url.path();
            let response = if path == "/api/v2" {
                let coin = if chain.is_testnet() { "Peercoin Testnet" } else { "Peercoin" };
                let network = if chain.is_testnet() { "testnet" } else { "livenet" };
                json!({"blockbook":{"coin":coin,"decimals":chain.native_decimals()},"backend":{"chain":network,"blocks":900000}})
            } else {
                let segments: Vec<&str> = path.split('/').collect();
                let address = segments
                    .iter()
                    .position(|segment| matches!(*segment, "address" | "addrs" | "addr" | "addresses"))
                    .and_then(|at| segments.get(at + 1))
                    .expect("an address read");
                let value = values.get(*address).expect("only wallet-owned addresses are read");
                indexer_balance(chain, value)
            };
            ResponseTemplate::new(200).set_body_json(response)
        })
        .mount(&server)
        .await;
    let directory =
        std::env::temp_dir().join(format!("account-balance-{}", crate::store::new_event_id()));
    std::fs::create_dir_all(&directory).unwrap();
    let database = directory.join("state.db").to_string_lossy().into_owned();
    let endpoints = vec![ChainEndpoints {
        capabilities: vec![EndpointCapability::Balance],
        chain_id: chain,
        endpoints: vec![server.uri()],
    }];
    let service = WalletService::new(endpoints.clone()).unwrap();
    service.open_state(database.clone()).await.unwrap();
    service
        .apply_state_command(StateCommand::UpsertWallet {
            wallet: WalletState::single_address("w", "Account", chain, root, Some(base), true),
        })
        .await
        .unwrap();
    for (address, path, branch, index) in [
        (receive.0, receive.1, "external", 4),
        (change.0, change.1, "change", 3),
    ] {
        service
            .register_owned_address(
                "w".into(),
                chain,
                address,
                path,
                Some(branch.into()),
                Some(index),
            )
            .await
            .unwrap();
    }
    drop(service);
    let reopened = WalletService::new(endpoints).unwrap();
    reopened.open_state(database).await.unwrap();
    (reopened, directory, server)
}

#[tokio::test]
async fn account_utxo_balances_include_rotated_receive_and_change_with_native_precision() {
    for chain in Chain::all().filter(|chain| chain.uses_account_utxo()) {
        let (reopened, directory, _server) =
            reopened_account_with_balances(chain, 12_345_678, 2_345_678).await;
        let wallet = reopened.refresh_wallet_balances("w".into()).await.unwrap();
        let native = wallet
            .holdings
            .iter()
            .find(|holding| holding.is_native())
            .unwrap();
        assert_eq!(
            native.amount,
            crate::decimal::from_units(14_691_356, u32::from(chain.native_decimals()))
        );
        assert_eq!(native.chain_id, chain);
        assert_eq!(
            reopened
                .account_utxo_wallet_balance("w", chain)
                .await
                .unwrap()
                .smallest_unit,
            "14691356"
        );
        drop(reopened);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[tokio::test]
async fn peercoin_wallet_balance_can_exceed_the_per_transaction_money_limit() {
    for chain in [Chain::Peercoin, Chain::PeercoinTestnet] {
        let (service, directory, _server) =
            reopened_account_with_balances(chain, 20_000_000_000_000, 2_000_000_000_000).await;
        let balance = service
            .account_utxo_wallet_balance("w", chain)
            .await
            .unwrap();
        assert_eq!(balance.smallest_unit, "22000000000000");
        assert_eq!(balance.amount_display, "22000000");
        drop(service);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[tokio::test]
async fn account_utxo_wallet_balance_refuses_integer_overflow() {
    for chain in Chain::all().filter(|chain| chain.uses_account_utxo()) {
        // Whatsonchain reports a balance as a signed integer, so its total
        // overflows across three addresses.
        let (root, receive, change) =
            if chain.default_api() == Some(crate::EndpointApi::Whatsonchain) {
                let half = u64::try_from(i64::MAX).unwrap();
                (2, half, half)
            } else {
                (0, u64::MAX, 1)
            };
        let (service, directory, _server) =
            reopened_account_holding(chain, root, receive, change).await;
        let error = service
            .account_utxo_wallet_balance("w", chain)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("UTXO wallet balance overflow"),
            "{chain}: {error}"
        );
        drop(service);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
