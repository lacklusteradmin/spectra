use super::*;
use crate::service::address_discovery::UtxoDerivation;
use crate::store::state::WalletState;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

async fn reopened_account_with_balances(
    chain: Chain,
    receive_units: u64,
    change_units: u64,
) -> (Arc<WalletService>, std::path::PathBuf, MockServer) {
    let base = crate::derivation::path::default_path_from_catalog(chain).unwrap();
    let context = UtxoDerivation::new(chain, SEED, base.clone()).unwrap();
    let root = context.derive(0).unwrap().0;
    let receive = context.derive(4).unwrap();
    let change = context.derive_on_branch(1, 3).unwrap();
    let values = HashMap::from([
        (root.clone(), "0".to_string()),
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
                let address = path.rsplit('/').next().unwrap();
                json!({"balance":values.get(address).expect("only wallet-owned addresses are read"),"unconfirmedBalance":"0"})
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
                Some(path),
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
        let (service, directory, _server) =
            reopened_account_with_balances(chain, u64::MAX, 1).await;
        let error = service
            .account_utxo_wallet_balance("w", chain)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("UTXO wallet balance overflow"));
        drop(service);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
