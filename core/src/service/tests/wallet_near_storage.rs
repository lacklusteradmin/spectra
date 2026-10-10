//! Which contracts hold a deposit the account can have back, and what a
//! refund refuses, against a NEAR node that answers as each kind of
//! contract does.

use super::*;
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const ACCOUNT: &str = "alice.near";
const DEPOSIT: u128 = 1_250_000_000_000_000_000_000;

/// A node where each contract answers `storage_balance_of` and
/// `ft_balance_of` the way its name says.
async fn node() -> (MockServer, NearClient) {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let params = &body["params"];
            assert_eq!(params["request_type"], "call_function", "{body}");
            use base64::Engine;
            let args: Value = serde_json::from_slice(
                &base64::engine::general_purpose::STANDARD
                    .decode(params["args_base64"].as_str().unwrap())
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(args, json!({"account_id": ACCOUNT}));
            let registered = json!({"total": DEPOSIT.to_string(), "available": "0"});
            let answer = match (
                params["account_id"].as_str().unwrap(),
                params["method_name"].as_str().unwrap(),
            ) {
                ("gone.near", _) => {
                    return ResponseTemplate::new(200).set_body_json(json!({"jsonrpc": "2.0",
                        "id": body["id"], "error": {"name": "HANDLER_ERROR",
                        "cause": {"name": "UNKNOWN_ACCOUNT"}}}));
                }
                // nearcore's answer for a method the contract lacks.
                ("nostorage.near", "storage_balance_of") | ("notoken.near", "ft_balance_of") => {
                    return ResponseTemplate::new(200).set_body_json(json!({"jsonrpc": "2.0",
                        "id": body["id"], "result": {"error": "wasm execution failed with error: \
                        FunctionCallError(MethodResolveError(MethodNotFound))", "logs": [],
                        "block_height": 1}}));
                }
                ("unregistered.near", "storage_balance_of") => Value::Null,
                ("free.near", "storage_balance_of") => json!({"total": "0", "available": "0"}),
                (_, "storage_balance_of") => registered,
                ("held.near", "ft_balance_of") => json!("5"),
                (_, "ft_balance_of") => json!("0"),
                other => panic!("unexpected call {other:?}"),
            };
            ResponseTemplate::new(200).set_body_json(json!({"jsonrpc": "2.0", "id": body["id"],
                "result": {"result": serde_json::to_vec(&answer).unwrap(), "logs": [],
                "block_height": 1}}))
        })
        .mount(&server)
        .await;
    let client = NearClient::new(Arc::new(vec![server.uri()]));
    (server, client)
}

/// Listed: a registered contract whose token the account holds none of.
/// Passed over: one that holds no deposit, a zero one, one the account holds
/// tokens of, and one that answers no NEP-145 or NEP-141 query or does not
/// exist.
#[tokio::test]
async fn only_a_deposit_beside_an_empty_balance_is_refundable() {
    let (_server, node) = node().await;
    assert_eq!(
        refundable_deposit(&node, "empty.near", ACCOUNT)
            .await
            .unwrap(),
        Some(DEPOSIT)
    );
    for contract in [
        "held.near",
        "unregistered.near",
        "free.near",
        "nostorage.near",
        "notoken.near",
        "gone.near",
    ] {
        assert_eq!(
            refundable_deposit(&node, contract, ACCOUNT).await.unwrap(),
            None,
            "{contract}"
        );
    }
}

/// A refund is built only for a deposit beside an empty balance; each other
/// contract is refused with what stands in the way.
#[tokio::test]
async fn a_refund_refuses_held_tokens_and_missing_deposits() {
    let (_server, node) = node().await;
    assert_eq!(
        refund_preconditions(&node, "empty.near", ACCOUNT)
            .await
            .unwrap(),
        DEPOSIT
    );
    for (contract, words) in [
        ("held.near", "still holds this token"),
        ("notoken.near", "reports no token balance"),
        ("unregistered.near", "holds no storage deposit"),
        ("free.near", "holds no storage deposit"),
        ("nostorage.near", "holds no storage deposit"),
        ("gone.near", "holds no storage deposit"),
    ] {
        let error = refund_preconditions(&node, contract, ACCOUNT)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(words), "{contract}: {error}");
    }
}

mod service {
    //! What the wallet's account lists and refunds, read through the service
    //! from a node and indexer answering for one account.

    use super::super::*;
    use super::DEPOSIT;
    use crate::service::wallet_near_keys::tests::{NearAccount, access_key, ed25519, near_wallet};
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use std::sync::Mutex;

    fn registered(deposit: u128) -> Value {
        json!({"total": deposit.to_string(), "available": "0"})
    }

    fn token(storage: Value, balance: &str) -> HashMap<&'static str, Value> {
        HashMap::from([
            ("storage_balance_of", storage),
            ("ft_balance_of", json!(balance)),
        ])
    }

    /// An account the indexer says held four tokens, whose wallet tracks a
    /// fifth and whose history names a sixth.
    async fn account() -> (
        Arc<Mutex<NearAccount>>,
        crate::service::loopback_service::OpenService,
        wiremock::MockServer,
        String,
        String,
    ) {
        let state = Arc::new(Mutex::new(NearAccount {
            inventory: vec![
                ("empty.near", "0"),
                ("held.near", "5"),
                ("unregistered.near", "0"),
                ("nostorage.near", "0"),
            ],
            tokens: HashMap::from([
                ("empty.near", token(registered(DEPOSIT), "0")),
                ("held.near", token(registered(DEPOSIT), "5")),
                ("unregistered.near", token(Value::Null, "0")),
                (
                    "nostorage.near",
                    HashMap::from([("ft_balance_of", json!("0"))]),
                ),
                ("holding.near", token(registered(2 * DEPOSIT), "0")),
                ("history.near", token(registered(DEPOSIT), "0")),
            ]),
            ..NearAccount::default()
        }));
        let (service, server, wallet, account) = near_wallet(&state).await;
        state.lock().unwrap().keys = vec![access_key(&ed25519(&account), json!("FullAccess"))];
        service
            .use_endpoint(
                Chain::Near,
                crate::EndpointApi::Nearblocks,
                &[
                    EndpointCapability::History,
                    EndpointCapability::TokenHistory,
                    EndpointCapability::TokenDiscovery,
                ],
                &server.uri(),
            )
            .await;
        let mut stored = service
            .app_state()
            .await
            .wallets
            .into_iter()
            .find(|w| w.id == wallet)
            .unwrap();
        stored.holdings.push(
            serde_json::from_value(json!({"name": "Holding", "symbol": "HOLD",
                "coingeckoId": "", "chainId": "near", "tokenStandard": "NEP-141",
                "contractAddress": "holding.near", "amount": "0"}))
            .unwrap(),
        );
        service
            .apply_state_command(StateCommand::UpsertWallet { wallet: stored })
            .await
            .unwrap();
        let received: crate::store::persistence_models::TransactionRecord =
            serde_json::from_value(json!({"id": "received", "walletId": wallet,
                "kind": "receive", "status": "confirmed", "walletName": "NEAR",
                "assetDisplayName": "History", "symbol": "HIST", "chainId": "near",
                "amount": "1", "address": account,
                "deploymentId": "near:nep-141:history.near", "createdAtUnix": 1.0}))
            .unwrap();
        service
            .upsert_history_records(vec![crate::wallet_db::history_record_from_payload(
                received,
            )])
            .await
            .unwrap();
        (state, service, server, wallet, account)
    }

    /// The contracts the indexer's inventory, the wallet's holdings and its
    /// history name, each asked for its deposit: those holding one beside an
    /// empty balance are listed with the token's symbol where the wallet
    /// knows it, and their deposits summed.
    #[tokio::test]
    async fn deposits_are_found_from_the_inventory_holdings_and_history() {
        let (_state, service, _server, wallet, account) = account().await;
        let storage = service.wallet_token_storage(wallet).await.unwrap();
        let listed: Vec<_> = storage
            .deposits
            .iter()
            .map(|d| (d.contract.as_str(), d.symbol.as_str(), d.refund.as_str()))
            .collect();
        assert_eq!(
            listed,
            [
                ("empty.near", "empty.near", "0.00125"),
                ("history.near", "history.near", "0.00125"),
                ("holding.near", "HOLD", "0.0025"),
            ]
        );
        assert_eq!(
            (storage.refundable.as_str(), storage.account.as_str()),
            ("0.005", account.as_str())
        );
    }

    /// What is not a token contract, and any refund from a watched account,
    /// is refused before anything is built.
    #[tokio::test]
    async fn a_refund_needs_a_token_contract_and_a_signing_wallet() {
        let (_state, service, _server, wallet, _account) = account().await;
        let refusal = service
            .build_token_storage_refund(wallet, "Not A Contract".into())
            .await
            .unwrap_err()
            .to_string();
        assert!(
            refusal.contains("is not a NEAR token contract"),
            "{refusal}"
        );
        let mut watch = crate::derivation::setup::tests::fixture(
            Chain::Near,
            crate::derivation::setup::WalletSetupMethod::WatchAddresses,
        );
        watch.request.kind = crate::derivation::import::WalletImportKind::WatchAddresses {
            addresses: vec!["22".repeat(32)],
        };
        let watched = service.import(watch).await;
        let refusal = service
            .build_token_storage_refund(watched, "history.near".into())
            .await
            .unwrap_err()
            .to_string();
        assert!(
            refusal.contains("a watch-only wallet cannot send"),
            "{refusal}"
        );
        assert!(service.list_sends().await.unwrap().is_empty());
    }

    /// Unregistering refuses while the account holds the token, so a token
    /// that arrives between the review and signing stops the signature.
    #[tokio::test]
    async fn a_token_arriving_before_signing_stops_the_refund() {
        let (state, service, _server, wallet, _account) = account().await;
        let built = service
            .build_token_storage_refund(wallet, "empty.near".into())
            .await
            .unwrap();
        let balance = |amount: &str| {
            state
                .lock()
                .unwrap()
                .tokens
                .get_mut("empty.near")
                .unwrap()
                .insert("ft_balance_of", json!(amount));
        };
        balance("1");
        let refusal = service
            .sign_send(built.id.clone(), built.review_digest.clone(), None)
            .await
            .unwrap_err()
            .to_string();
        assert!(refusal.contains("still holds this token"), "{refusal}");
        let stored = service.inspect_send(built.id.clone()).await.unwrap();
        assert_eq!(stored.stage, crate::send::stages::SendStage::Prepared);
        balance("0");
        let signed = service
            .sign_send(built.id, built.review_digest, None)
            .await
            .unwrap();
        assert_eq!(signed.stage, crate::send::stages::SendStage::Signed);
    }
}
