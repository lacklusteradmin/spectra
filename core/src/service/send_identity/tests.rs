use super::*;
use crate::store::secret_backends::InMemorySecretStore;
use crate::store::state::WalletSummary;
use crate::store::wallet_secrets::{store_private_key, store_seed_phrase};

const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const ETH: &str = "0x9858effd232b4033e47d90003d41ec34ecaeda94";
const KEY_ADDRESS: &str = "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf";

async fn wallet(
    address: &str,
    password: Option<&str>,
) -> (Arc<WalletService>, Arc<InMemorySecretStore>) {
    let service = WalletService::new_typed(vec![]).unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    service.set_secret_store(secrets.clone());
    service
        .apply_state_command(StateCommand::UpsertWallet {
            wallet: WalletSummary::single_address(
                "w",
                "Wallet",
                "Ethereum",
                address,
                Some("m/44'/60'/0'/0/0".into()),
                false,
            ),
        })
        .await
        .unwrap();
    store_seed_phrase(&*secrets, "w", SEED, password).unwrap();
    (service, secrets)
}

#[tokio::test]
async fn stored_mnemonic_resolves_the_same_identity_on_evm_chains() {
    let (service, _) = wallet(ETH, None).await;
    for chain in [Chain::Ethereum, Chain::Arbitrum, Chain::Polygon] {
        assert_eq!(
            service
                .send_identity_address("w".into(), chain.str_id().into(), None)
                .await
                .unwrap(),
            ETH
        );
    }
}

#[tokio::test]
async fn mismatched_missing_and_watch_only_wallets_are_refused() {
    let (service, _) = wallet(KEY_ADDRESS, None).await;
    assert!(service
        .send_identity_address("w".into(), "ethereum".into(), None)
        .await
        .unwrap_err()
        .to_string()
        .contains("does not match"));
    assert!(service
        .send_identity_address("w".into(), "solana".into(), None)
        .await
        .unwrap_err()
        .to_string()
        .contains("no address"));
    assert!(service
        .send_identity_address("missing".into(), "ethereum".into(), None)
        .await
        .is_err());
    let mut stored = service.app_state().await.wallets[0].clone();
    stored.is_watch_only = true;
    service
        .apply_state_command(StateCommand::UpsertWallet { wallet: stored })
        .await
        .unwrap();
    assert!(service
        .send_identity_address("w".into(), "ethereum".into(), None)
        .await
        .unwrap_err()
        .to_string()
        .contains("watch-only"));
}

#[tokio::test]
async fn ambiguous_stored_material_is_refused_instead_of_preferring_a_key() {
    let (service, secrets) = wallet(ETH, None).await;
    store_private_key(&*secrets, "w", &format!("{:064x}", 1), None).unwrap();
    assert!(service
        .send_identity_address("w".into(), "ethereum".into(), None)
        .await
        .unwrap_err()
        .to_string()
        .contains("both mnemonic and private key"));
}

#[tokio::test]
async fn private_key_wallet_needs_no_caller_or_stored_derivation_path() {
    let (service, secrets) = wallet(KEY_ADDRESS, None).await;
    crate::store::wallet_secrets::delete(&*secrets, "w").unwrap();
    store_private_key(&*secrets, "w", &format!("0x{:064x}", 1), None).unwrap();
    let mut stored = service.app_state().await.wallets[0].clone();
    stored.derivation_path = None;
    service
        .apply_state_command(StateCommand::UpsertWallet { wallet: stored })
        .await
        .unwrap();
    assert_eq!(
        service
            .send_identity_address("w".into(), "ethereum".into(), None)
            .await
            .unwrap(),
        KEY_ADDRESS
    );
}

#[tokio::test]
async fn passwords_unlock_stored_material_and_wrong_passwords_fail() {
    let (service, _) = wallet(ETH, Some("secret")).await;
    for password in [None, Some("wrong".into())] {
        assert!(service
            .send_identity_address("w".into(), "ethereum".into(), password)
            .await
            .is_err());
    }
    assert_eq!(
        service
            .send_identity_address("w".into(), "ethereum".into(), Some("secret".into()))
            .await
            .unwrap(),
        ETH
    );
}

#[tokio::test]
async fn every_mainnet_mnemonic_identity_resolves_using_stored_derivation_data() {
    let service = WalletService::new_typed(vec![]).unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    service.set_secret_store(secrets.clone());
    let defaults = crate::app_core_derivation_paths_for_preset(0).unwrap();
    for chain in Chain::mainnets() {
        let name = chain.chain_display_name();
        let path = defaults.path_for(chain).unwrap_or_default();
        let derived = crate::derivation::dispatch::derive_for_chain_name(
            name, SEED, path, None, None, None, true, false, false,
        )
        .unwrap();
        let address = derived.address.unwrap();
        service
            .apply_state_command(StateCommand::UpsertWallet {
                wallet: WalletSummary::single_address(
                    "w",
                    "Wallet",
                    name,
                    &address,
                    Some(path.into()),
                    false,
                ),
            })
            .await
            .unwrap();
        store_seed_phrase(&*secrets, "w", SEED, None).unwrap();
        let resolved = service
            .send_identity_address("w".into(), chain.str_id().into(), None)
            .await;
        assert_eq!(
            resolved.unwrap_or_else(|e| panic!("{chain:?}: {e}")),
            crate::send::flow::normalize_address(name, &address)
        );
    }
}

#[tokio::test]
async fn monero_rpc_is_bound_to_the_checked_sender_and_endpoint() {
    use crate::service::send_params::{ExecuteSendParams, MoneroSendParams, SendParams};
    use wiremock::{matchers::body_partial_json, Mock, MockServer, ResponseTemplate};
    for matches_wallet in [false, true] {
        let rpc = MockServer::start().await;
        let backup = MockServer::start().await;
        Mock::given(body_partial_json(serde_json::json!({"method": "get_address"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jsonrpc": "2.0", "result": {"address": if matches_wallet { "selected" } else { "other" }}
            }))).expect(1).mount(&rpc).await;
        Mock::given(body_partial_json(serde_json::json!({"method": "transfer"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jsonrpc": "2.0", "result": {"tx_hash": "mock-tx"}
            })))
            .expect(if matches_wallet { 1 } else { 0 })
            .mount(&rpc)
            .await;
        let service = WalletService::new_typed(vec![crate::service::ChainEndpoints {
            chain_id: "monero".into(),
            endpoints: vec![rpc.uri(), backup.uri()],
            api_key: None,
        }])
        .unwrap();
        let result = service
            .sign_and_broadcast_send(
                Chain::Monero,
                ExecuteSendParams::Native(SendParams::Monero(MoneroSendParams {
                    from: "selected".into(),
                    to: "recipient".into(),
                    piconeros: 1,
                    priority: None,
                })),
            )
            .await;
        if matches_wallet {
            assert!(result.unwrap().contains("mock-tx"));
        } else {
            assert!(result.unwrap_err().to_string().contains("does not match"));
        }
        assert!(backup.received_requests().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn near_named_accounts_are_resolved_but_implicit_accounts_must_match_the_key() {
    let service = WalletService::new_typed(vec![]).unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    service.set_secret_store(secrets.clone());
    store_seed_phrase(&*secrets, "w", SEED, None).unwrap();
    for (address, valid) in [("alice.near".to_string(), true), ("11".repeat(32), false)] {
        service
            .apply_state_command(StateCommand::UpsertWallet {
                wallet: WalletSummary::single_address("w", "Wallet", "NEAR", &address, None, false),
            })
            .await
            .unwrap();
        assert_eq!(
            service
                .send_identity_address("w".into(), "near".into(), None)
                .await
                .is_ok(),
            valid
        );
    }
}
