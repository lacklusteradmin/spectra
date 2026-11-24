use super::*;
use crate::api::endpoint::EndpointApi;
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
        EndpointApi::Esplora => json!({
            "address": "",
            "chain_stats": {"funded_txo_sum": units, "spent_txo_sum": 0, "tx_count": 1},
            "mempool_stats": {"funded_txo_sum": 0, "spent_txo_sum": 0, "tx_count": 0},
        }),
        EndpointApi::Blockcypher => json!({"balance": units, "unconfirmed_balance": 0}),
        EndpointApi::Whatsonchain => json!({"confirmed": units, "unconfirmed": 0}),
        EndpointApi::Insight => json!({"balanceSat": units, "balance": 0}),
        EndpointApi::KaspaRest => json!({"address": "", "balance": units}),
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
        std::env::temp_dir().join(format!("account-balance-{}", crate::store::new_id()));
    std::fs::create_dir_all(&directory).unwrap();
    let database = directory.join("state.db").to_string_lossy().into_owned();
    let endpoints = vec![ChainEndpoints {
        capabilities: vec![EndpointCapability::Balance],
        chain_id: chain,
        endpoints: vec![server.uri()],
    }];
    let service = WalletService::new(endpoints.clone()).unwrap();
    service.opened(database.clone()).await.unwrap();
    service
        .upsert_wallet(WalletState::single_address(
            "w",
            "Account",
            chain,
            root,
            Some(base),
            true,
        ))
        .await
        .unwrap();
    for (address, path, branch, index) in [
        (
            receive.0,
            receive.1,
            crate::wallet_db::AddressBranch::Receive,
            4,
        ),
        (
            change.0,
            change.1,
            crate::wallet_db::AddressBranch::Change,
            3,
        ),
    ] {
        service
            .register_owned_address("w".into(), chain, address, path, Some(branch), Some(index))
            .await
            .unwrap();
    }
    drop(service);
    let reopened = WalletService::new(endpoints).unwrap();
    reopened.opened(database).await.unwrap();
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
            crate::amount::decimal::from_units(14_691_356, u32::from(chain.native_decimals()))
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
        let (root, receive, change) = if chain.default_api() == Some(EndpointApi::Whatsonchain) {
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

mod balance_summaries {
    //! Exercise app-facing adapters through protocol fixtures, not their decoders.
    use super::*;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn service(chain: crate::registry::Chain, server: &MockServer) -> Arc<WalletService> {
        WalletService::new(vec![ChainEndpoints {
            capabilities: EndpointCapability::ALL.to_vec(),
            chain_id: chain,
            endpoints: vec![server.uri()],
        }])
        .unwrap()
    }

    #[tokio::test]
    async fn one_blockbook_adapter_reads_each_network_and_keeps_bch_address_rules() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/address/holder"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"balance":"123456789"})))
            .expect(5)
            .mount(&server)
            .await;
        for chain in [
            Chain::Litecoin,
            Chain::BitcoinCashTestnet,
            Chain::BitcoinGold,
            Chain::Dash,
            Chain::Zcash,
        ] {
            let address = if chain.mainnet_counterpart() == Chain::BitcoinCash {
                "bitcoincash:holder"
            } else {
                "holder"
            };
            let balance = service(chain, &server)
                .fetch_native_balance_summary(chain, address.into())
                .await
                .unwrap();
            assert_eq!(balance.smallest_unit, "123456789");
            assert_eq!(balance.amount_display, "1.23456789");
        }
    }

    #[tokio::test]
    async fn balance_summary_keeps_sub_micro_native_amounts() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"method":"eth_getBalance"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result":"0x1"})))
            .mount(&server)
            .await;
        let balance = service(crate::registry::Chain::Ethereum, &server)
            .fetch_native_balance_summary(crate::registry::Chain::Ethereum, "holder".into())
            .await
            .unwrap();
        assert_eq!(balance.amount_display, "0.000000000000000001");
    }

    /// Storage is decoded only after Asset Hub identity and its layout are verified.
    #[tokio::test]
    async fn a_polkadot_balance_is_read_from_system_account_storage() {
        let server = MockServer::start().await;
        for (request, result) in [
            (
                json!({"method":"chain_getBlockHash","params":[0]}),
                json!(
                    crate::registry::Chain::Polkadot
                        .substrate_genesis_hash()
                        .unwrap()
                ),
            ),
            (
                json!({"method":"chain_getBlockHash","params":[]}),
                json!(format!("0x{}", "11".repeat(32))),
            ),
            (
                json!({"method":"chain_getFinalizedHead"}),
                json!(format!("0x{}", "11".repeat(32))),
            ),
            (
                json!({"method":"chain_getHeader"}),
                json!({"number":"0x64"}),
            ),
            (
                json!({"method":"state_getRuntimeVersion"}),
                json!({"specVersion":2_005_000,"transactionVersion":15}),
            ),
            (
                json!({"method":"state_getMetadata"}),
                json!(format!(
                    "0x{}",
                    hex::encode(crate::api::substrate_json_rpc::tests::fixture(
                        crate::registry::Chain::Polkadot
                    ))
                )),
            ),
        ] {
            Mock::given(method("POST"))
                .and(body_partial_json(request))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"jsonrpc":"2.0","id":1,"result":result})),
                )
                .mount(&server)
                .await;
        }
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"method": "state_getStorage"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0", "id": 1,
                "result": "0x000000000000000001000000000000001a8ea401a31900000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000080"
            })))
            .mount(&server)
            .await;
        let summary = service(crate::registry::Chain::Polkadot, &server)
            .fetch_native_balance_summary(
                crate::registry::Chain::Polkadot,
                "13UVJyLnbVp9RBZYFwFGyDvVd1y27Tt8tkntv6Q7JVPhFsTB".into(),
            )
            .await
            .unwrap();
        assert_eq!(summary.smallest_unit, "28187897925146");
        assert_eq!(summary.amount_display, "2818.7897925146");
    }
}
