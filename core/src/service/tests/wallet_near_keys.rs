//! A NEAR account's access keys as its node lists them, and which one a
//! wallet may delete.

use super::*;
use crate::derivation::setup::WalletSetupMethod;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

/// A NEAR account as its node and indexer report it.
#[derive(Default)]
pub(in crate::service) struct NearAccount {
    pub account: String,
    /// `view_access_key_list`'s entries.
    pub keys: Vec<Value>,
    /// The indexer's token inventory: each contract and its balance.
    pub inventory: Vec<(&'static str, &'static str)>,
    /// What each token contract answers, by method; a method missing is
    /// one the contract does not implement.
    pub tokens: HashMap<&'static str, HashMap<&'static str, Value>>,
}

/// An access-key entry.
pub(in crate::service) fn access_key(public_key: &str, permission: Value) -> Value {
    json!({"public_key": public_key, "access_key": {"nonce": 7, "permission": permission}})
}

/// A key's `ed25519:` text from its hex.
pub(in crate::service) fn ed25519(hex_key: &str) -> String {
    format!(
        "ed25519:{}",
        bs58::encode(hex::decode(hex_key).unwrap()).into_string()
    )
}

/// A NEAR node, which is also the account's indexer, answering for `state`.
pub(in crate::service) async fn near_node(state: Arc<Mutex<NearAccount>>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let state = state.lock().unwrap();
            if request.method.as_str() == "GET" {
                assert_eq!(
                    request.url.path(),
                    format!("/accounts/{}/assets/fts", state.account)
                );
                let rows: Vec<Value> = state
                    .inventory
                    .iter()
                    .map(|(contract, amount)| {
                        json!({"contract": contract, "amount": amount, "meta": {"decimals": 6}})
                    })
                    .collect();
                return ResponseTemplate::new(200).set_body_json(json!({"data": rows, "meta": {}}));
            }
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let params = &body["params"];
            let result = match body["method"].as_str().unwrap() {
                "status" => json!({"chain_id": "mainnet"}),
                "block" => json!({"header": {"hash": bs58::encode([2; 32]).into_string(),
                    "height": 100}}),
                "gas_price" => json!({"gas_price": "100000000"}),
                "EXPERIMENTAL_protocol_config" => {
                    let mut config: Value = serde_json::from_str(include_str!(
                        "../../../tests/fixtures/near-staking-fee-protocol86.json"
                    ))
                    .unwrap();
                    config["transaction_validity_period"] = json!(86_400);
                    config["runtime_config"]["storage_amount_per_byte"] =
                        json!("10000000000000000000");
                    config
                }
                "query" => match params["request_type"].as_str().unwrap() {
                    "view_access_key_list" => {
                        assert_eq!(params["account_id"], state.account.as_str());
                        json!({"keys": state.keys})
                    }
                    "view_access_key" => state
                        .keys
                        .iter()
                        .find(|key| key["public_key"] == params["public_key"])
                        .map(|key| key["access_key"].clone())
                        .unwrap_or_else(|| panic!("no key {}", params["public_key"])),
                    "view_account" => {
                        json!({"amount": 10u128.pow(24).to_string(), "locked": "0",
                            "storage_usage": 500})
                    }
                    "call_function" => {
                        use base64::Engine;
                        let args: Value = serde_json::from_slice(
                            &base64::engine::general_purpose::STANDARD
                                .decode(params["args_base64"].as_str().unwrap())
                                .unwrap(),
                        )
                        .unwrap();
                        assert_eq!(args, json!({"account_id": state.account}));
                        match state
                            .tokens
                            .get(params["account_id"].as_str().unwrap())
                            .and_then(|methods| {
                                methods.get(params["method_name"].as_str().unwrap())
                            }) {
                            Some(answer) => json!({"result": serde_json::to_vec(answer).unwrap(),
                                "logs": [], "block_height": 100}),
                            // nearcore's answer for a method the contract lacks.
                            None => json!({"error": "wasm execution failed with error: \
                                FunctionCallError(MethodResolveError(MethodNotFound))",
                                "logs": [], "block_height": 100}),
                        }
                    }
                    other => panic!("unexpected NEAR query {other}"),
                },
                other => panic!("unexpected NEAR call {other}"),
            };
            ResponseTemplate::new(200)
                .set_body_json(json!({"jsonrpc": "2.0", "id": body["id"], "result": result}))
        })
        .mount(&server)
        .await;
    server
}

/// A service whose NEAR wallet, restored from the shared phrase, reads
/// `state` from its node; the wallet's id and its implicit account.
pub(in crate::service) async fn near_wallet(
    state: &Arc<Mutex<NearAccount>>,
) -> (
    crate::service::loopback_service::OpenService,
    MockServer,
    String,
    String,
) {
    let service = crate::service::loopback_service::open().await;
    let wallet = service
        .import(crate::derivation::setup::tests::fixture(
            Chain::Near,
            WalletSetupMethod::ImportPhrase,
        ))
        .await;
    let account = service.address(&wallet, Chain::Near).await;
    state.lock().unwrap().account = account.clone();
    let server = near_node(state.clone()).await;
    use EndpointCapability::*;
    service
        .use_endpoint(
            Chain::Near,
            crate::EndpointApi::NearJsonRpc,
            &[Balance, Fee, Broadcast, Verification],
            &server.uri(),
        )
        .await;
    (service, server, wallet, account)
}

fn function_call(receiver: &str, allowance: Option<&str>) -> Value {
    json!({"FunctionCall": {"allowance": allowance, "receiver_id": receiver,
        "method_names": []}})
}

/// The key the wallet signs with first, then the other full-access keys,
/// then function-call keys by the contract they may call, each with the gas
/// allowance it has left.
#[tokio::test]
async fn keys_are_listed_signing_key_first_then_full_access_then_by_contract() {
    let state = Arc::new(Mutex::new(NearAccount::default()));
    let (service, _server, wallet, account) = near_wallet(&state).await;
    let signer = ed25519(&account);
    let (other, dapp, unlimited) = (
        ed25519(&"11".repeat(32)),
        ed25519(&"33".repeat(32)),
        ed25519(&"44".repeat(32)),
    );
    state.lock().unwrap().keys = vec![
        access_key(
            &dapp,
            function_call("zapp.near", Some("250000000000000000000000")),
        ),
        access_key(&other, json!("FullAccess")),
        access_key(&unlimited, function_call("app.near", None)),
        access_key(&signer, json!("FullAccess")),
    ];
    let keys = service.wallet_access_keys(wallet).await.unwrap();
    assert_eq!(keys.account, account);
    let listed: Vec<_> = keys
        .keys
        .iter()
        .map(|k| {
            (
                k.public_key.as_str(),
                k.signs,
                k.full_access,
                k.receiver.as_deref(),
                k.allowance.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            (signer.as_str(), true, true, None, None),
            (other.as_str(), false, true, None, None),
            (unlimited.as_str(), false, false, Some("app.near"), None),
            (dapp.as_str(), false, false, Some("zapp.near"), Some("0.25")),
        ]
    );
}

/// A function-call key is deleted for the protocol's DeleteKey fee: its
/// receipt and the action, the gas priced at no less than the minimum
/// purchase price. A full-access key, the key the wallet signs with and a
/// key the account does not hold are refused, and nothing is stored.
#[tokio::test]
async fn only_a_function_call_key_the_wallet_does_not_sign_with_is_deleted() {
    let state = Arc::new(Mutex::new(NearAccount::default()));
    let (service, _server, wallet, account) = near_wallet(&state).await;
    let signer = ed25519(&account);
    let (other, dapp) = (ed25519(&"11".repeat(32)), ed25519(&"33".repeat(32)));
    let keys = |signer_permission: Value| {
        vec![
            access_key(&dapp, function_call("zapp.near", None)),
            access_key(&other, json!("FullAccess")),
            access_key(&signer, signer_permission),
        ]
    };
    // Signing with a function-call key of its own: that one is not deleted.
    state.lock().unwrap().keys = keys(function_call("app.near", None));
    for (key, words) in [
        (&other, "full-access keys are listed only"),
        (&signer, "This is the key Spectra signs with"),
        (&ed25519(&"55".repeat(32)), "is not a key of this account"),
    ] {
        let error = service
            .build_access_key_deletion(wallet.clone(), key.clone())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(words), "{error}");
    }
    assert!(service.list_sends().await.unwrap().is_empty());

    state.lock().unwrap().keys = keys(json!("FullAccess"));
    let built = service
        .build_access_key_deletion(wallet, dapp.clone())
        .await
        .unwrap();
    let config: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/near-staking-fee-protocol86.json"
    ))
    .unwrap();
    let costs = &config["runtime_config"]["transaction_costs"];
    let parts = [
        &costs["action_receipt_creation_config"],
        &costs["action_creation_config"]["delete_key_cost"],
    ];
    let gas = |field: &str| -> u128 {
        parts
            .iter()
            .map(|part| u128::from(part[field].as_u64().unwrap()))
            .sum()
    };
    let price = 100_000_000u128;
    let minimum: u128 = config["runtime_config"]["min_gas_purchase_price"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let fee = gas("send_sir") * price + gas("execution") * price.max(minimum);
    assert_eq!(
        built.operation,
        Some(crate::send::stages::WalletOperation::DeleteAccessKey {
            public_key: dapp,
            receiver: "zapp.near".into(),
            network_fee: crate::decimal::from_units(fee, 24),
        })
    );
    assert_eq!((built.sender, built.recipient), (account.clone(), account));
}

/// A named account holds several full-access keys; the wallet signs with
/// the one its phrase derives, which the import recorded.
#[tokio::test]
async fn a_named_account_signs_with_the_key_its_import_recorded() {
    let state = Arc::new(Mutex::new(NearAccount::default()));
    let (service, _server, implicit, account) = near_wallet(&state).await;
    let signer = ed25519(&account);
    let other = ed25519(&"11".repeat(32));
    {
        let mut state = state.lock().unwrap();
        state.account = "alice.near".into();
        state.keys = vec![
            access_key(&other, json!("FullAccess")),
            access_key(&signer, json!("FullAccess")),
        ];
    }
    service
        .apply_state_command(StateCommand::RemoveWallet {
            wallet_id: implicit,
        })
        .await
        .unwrap();
    let mut commit =
        crate::derivation::setup::tests::fixture(Chain::Near, WalletSetupMethod::ImportPhrase);
    commit.named_account = Some("alice.near".into());
    let named = service.import(commit).await;
    let keys = service.wallet_access_keys(named).await.unwrap();
    let listed: Vec<_> = keys
        .keys
        .iter()
        .map(|k| (k.public_key.as_str(), k.signs))
        .collect();
    assert_eq!(listed, [(signer.as_str(), true), (other.as_str(), false)]);
}
