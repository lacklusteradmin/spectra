//! Starknet transfers built, signed and broadcast against a mock node: from
//! a deployed Ready account, and from one the send deploys first.
use super::*;
use crate::service::send_stage_support::{Secret, Wallet, methods, node, refusal};
use serde_json::{Value, json};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
/// The phrase's first Ready account and its key, from starknet.js
/// (`core/tests/fixtures/starknet-vectors.json`).
const SENDER: &str = "0x0424e0202bce2d9abbee87ee8faa8fd416bcf02ad8fe18d66320f23b1b07fc02";
const PUBLIC_KEY: &str = "0x001f03432e214578b6ac859bd1d282e948a615bef54feaf8da02141d42a5f5fa";
const RECIPIENT: &str = "0x0591960a2eae4798de21771480cede3ce3c17cbeeda9b3b988cfadd2e8db16a4";
const STRK: &str = "0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d";
const ETH: &str = "0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7";
const READY_CLASS: &str = "0x36078334509b514626504edc9fb252328d1a240e4e948bef8d0c08dff45927f";

/// What the mock node holds, which a test moves between stages.
struct Node {
    chain_id: &'static str,
    class: Mutex<Option<&'static str>>,
    nonce: AtomicU64,
    strk: AtomicU64,
    eth: AtomicU64,
    guardian: AtomicBool,
    public_key: Mutex<String>,
    /// The hashes the node answers submissions with.
    hashes: Mutex<Vec<String>>,
    submitted: Mutex<Vec<Value>>,
}

impl Node {
    fn new(deployed: bool) -> Arc<Self> {
        Arc::new(Self {
            chain_id: "0x534e5f4d41494e",
            class: Mutex::new(deployed.then_some(READY_CLASS)),
            nonce: AtomicU64::new(if deployed { 7 } else { 0 }),
            // 10 STRK and 0.01 ETH.
            strk: AtomicU64::new(10_000_000_000_000_000_000),
            eth: AtomicU64::new(10_000_000_000_000_000),
            guardian: AtomicBool::new(false),
            public_key: Mutex::new(String::new()),
            hashes: Mutex::new(Vec::new()),
            submitted: Mutex::new(Vec::new()),
        })
    }
}

fn felt(text: &str) -> starknet_crypto::Felt {
    crate::derivation::starknet::parse_felt(text).unwrap()
}

fn rpc(text: &str) -> String {
    crate::api::starknet_json_rpc::rpc_felt(&felt(text))
}

fn selector(name: &str) -> String {
    crate::api::starknet_json_rpc::rpc_felt(&crate::derivation::starknet::selector(name))
}

/// As PublicNode estimated a STRK transfer from a Ready account on
/// 2026-11-24.
fn estimate() -> Value {
    json!({
        "l1_gas_consumed": "0x0", "l1_gas_price": "0x396eef64a169",
        "l2_gas_consumed": "0x187540", "l2_gas_price": "0x6bc235206",
        "l1_data_gas_consumed": "0xc0", "l1_data_gas_price": "0xdb0b15d8e",
        "overall_fee": "0xa4c33f62e76a00", "unit": "FRI",
    })
}

async fn mock(state: Arc<Node>) -> wiremock::MockServer {
    node(move |_, body| {
        let result = |result: Value| Some(json!({"jsonrpc": "2.0", "id": 1, "result": result}));
        let error = |code: i64, message: &str| {
            Some(json!({"jsonrpc": "2.0", "id": 1, "error": {"code": code, "message": message}}))
        };
        let params = &body["params"];
        match body["method"].as_str().unwrap() {
            "starknet_chainId" => result(json!(state.chain_id)),
            "starknet_getClassHashAt" => {
                assert_eq!(params[1], rpc(SENDER));
                match *state.class.lock().unwrap() {
                    Some(class) => result(json!(class)),
                    None => error(20, "Contract not found"),
                }
            }
            "starknet_getNonce" => {
                assert_eq!(params[1], rpc(SENDER));
                if state.class.lock().unwrap().is_none() {
                    return error(20, "Contract not found");
                }
                result(json!(format!("{:#x}", state.nonce.load(Ordering::SeqCst))))
            }
            "starknet_call" => {
                let call = &params[0];
                let selector_of = |name| call["entry_point_selector"] == selector(name);
                if selector_of("balanceOf") {
                    assert_eq!(call["calldata"], json!([rpc(SENDER)]));
                    let held = if call["contract_address"] == rpc(STRK) {
                        state.strk.load(Ordering::SeqCst)
                    } else {
                        assert_eq!(call["contract_address"], rpc(ETH));
                        state.eth.load(Ordering::SeqCst)
                    };
                    result(json!([format!("{held:#x}"), "0x0"]))
                } else if selector_of("decimals") {
                    result(json!(["0x12"]))
                } else if selector_of("get_owner") {
                    result(json!([rpc(&state.public_key.lock().unwrap())]))
                } else if selector_of("get_guardian") {
                    result(json!([if state.guardian.load(Ordering::SeqCst) {
                        "0x7"
                    } else {
                        "0x0"
                    }]))
                } else {
                    panic!("unexpected call {call}")
                }
            }
            "starknet_estimateFee" => {
                let request = body["params"]["request"].as_array().unwrap();
                for transaction in request {
                    assert_eq!(
                        transaction["version"],
                        "0x100000000000000000000000000000003"
                    );
                    assert_eq!(
                        transaction["resource_bounds"]["l2_gas"]["max_amount"],
                        "0x0"
                    );
                }
                result(Value::Array(request.iter().map(|_| estimate()).collect()))
            }
            "starknet_getTransactionStatus" => error(29, "Transaction hash not found"),
            "starknet_addDeployAccountTransaction" | "starknet_addInvokeTransaction" => {
                state.submitted.lock().unwrap().push(params.clone());
                let hash = state.hashes.lock().unwrap().remove(0);
                result(json!({"transaction_hash": hash}))
            }
            other => panic!("unexpected request {other}"),
        }
    })
    .await
}

async fn imported(state: &Arc<Node>) -> (wiremock::MockServer, Wallet) {
    let server = mock(state.clone()).await;
    let wallet = Wallet::import(Chain::Starknet, &server.uri(), Secret::Phrase(PHRASE)).await;
    assert_eq!(wallet.address, SENDER);
    let public_key = wallet
        .service
        .stored_wallet(&wallet.id)
        .await
        .unwrap()
        .starknet_public_key
        .unwrap();
    *state.public_key.lock().unwrap() = public_key;
    (server, wallet)
}

async fn prepared(wallet: &Wallet, id: &str) -> crate::send::starknet::PreparedStarknetTransfer {
    let stored = wallet.service.load_send_artifact(id.into()).await.unwrap();
    let PreparedPayload::Starknet(prepared) = stored.prepared else {
        panic!("not a Starknet transfer");
    };
    prepared
}

#[tokio::test]
async fn a_deployed_account_sends_strk_under_the_estimate_and_half_again() {
    let state = Node::new(true);
    let (server, wallet) = imported(&state).await;
    assert_eq!(*state.public_key.lock().unwrap(), PUBLIC_KEY);
    let built = wallet
        .build(wallet.request(RECIPIENT, "1.5"))
        .await
        .unwrap();
    let prepared = prepared(&wallet, &built.id).await;
    assert!(prepared.deployment.is_none());
    assert_eq!(prepared.nonce, 7);
    assert_eq!(prepared.token, felt(STRK));
    assert_eq!(prepared.amount, 1_500_000_000_000_000_000);
    assert_eq!(prepared.resource_bounds.l2_gas.max_amount, 0x187540 * 3 / 2);
    assert_eq!(
        prepared.resource_bounds.l2_gas.max_price_per_unit,
        0x6bc235206 * 3 / 2
    );
    let stored = wallet
        .service
        .load_send_artifact(built.id.clone())
        .await
        .unwrap();
    assert_eq!(
        stored.request.fee_amount.as_deref(),
        Some(crate::amount::decimal::from_units(prepared.max_fee().unwrap(), 18).as_str())
    );

    let signed = wallet.sign(&built).await.unwrap();
    let hash = signed.transaction_hash.clone().unwrap();
    assert_eq!(
        hash,
        crate::derivation::starknet::felt_hex(&prepared.invoke_hash())
    );
    state.hashes.lock().unwrap().push(rpc(&hash));
    let broadcast = wallet.broadcast(&signed).await.unwrap();
    assert!(
        broadcast
            .attempts
            .iter()
            .any(|attempt| attempt.outcome == SubmissionOutcome::Accepted
                && attempt.transaction_hash.as_deref() == Some(hash.as_str()))
    );
    let submitted = state.submitted.lock().unwrap().clone();
    assert_eq!(submitted.len(), 1);
    let invoke = &submitted[0]["invoke_transaction"];
    assert_eq!(invoke["version"], "0x3");
    assert_eq!(invoke["nonce"], "0x7");
    assert_eq!(invoke["sender_address"], rpc(SENDER));
    assert!(
        !methods(&server)
            .await
            .contains(&"starknet_addDeployAccountTransaction".to_string())
    );
}

#[tokio::test]
async fn an_undeployed_account_is_deployed_and_then_sends_eth() {
    let state = Node::new(false);
    let (server, wallet) = imported(&state).await;
    let request = wallet.token_request(RECIPIENT, "0.004", ETH, 18);
    let built = wallet.build(request).await.unwrap();
    let prepared = prepared(&wallet, &built.id).await;
    let deployment = prepared.deployment.clone().unwrap();
    assert_eq!(deployment.public_key, felt(PUBLIC_KEY));
    assert_eq!(prepared.nonce, 1);
    assert_eq!(prepared.token, felt(ETH));
    // Both transactions' fees are the review's, together.
    assert!(prepared.max_fee().unwrap() > prepared.resource_bounds.max_fee().unwrap());

    let signed = wallet.sign(&built).await.unwrap();
    // The deployment and the transfer reserve nonces zero and one: a second
    // transfer built now cannot be signed while this one stands.
    let second = wallet
        .build(wallet.token_request(RECIPIENT, "0.001", ETH, 18))
        .await
        .unwrap();
    assert!(wallet.sign(&second).await.is_err());

    state.hashes.lock().unwrap().extend([
        crate::api::starknet_json_rpc::rpc_felt(&prepared.deploy_hash().unwrap()),
        rpc(signed.transaction_hash.as_deref().unwrap()),
    ]);
    let broadcast = wallet.broadcast(&signed).await.unwrap();
    assert!(
        broadcast
            .attempts
            .iter()
            .any(|attempt| attempt.outcome == SubmissionOutcome::Accepted)
    );
    let submitted = state.submitted.lock().unwrap().clone();
    assert_eq!(submitted.len(), 2);
    let deploy = &submitted[0]["deploy_account_transaction"];
    assert_eq!(deploy["class_hash"], READY_CLASS);
    assert_eq!(deploy["contract_address_salt"], rpc(PUBLIC_KEY));
    assert_eq!(
        deploy["constructor_calldata"],
        json!(["0x0", rpc(PUBLIC_KEY), "0x1"])
    );
    let invoke = &submitted[1]["invoke_transaction"];
    assert_eq!(invoke["nonce"], "0x1");
    assert_eq!(invoke["calldata"][1], rpc(ETH));
    let sent = methods(&server).await;
    let position = |method: &str| sent.iter().position(|m| m == method).unwrap();
    assert!(
        position("starknet_addDeployAccountTransaction")
            < position("starknet_addInvokeTransaction")
    );
}

#[tokio::test]
async fn what_the_network_would_refuse_is_refused_before_signing() {
    let state = Node::new(true);
    let (_server, wallet) = imported(&state).await;
    // More than the account holds, of STRK with its fee and of ETH; another
    // network's address; a memo, which a transfer cannot carry.
    assert!(refusal(wallet.build(wallet.request(RECIPIENT, "10")).await).contains("Insufficient"));
    assert!(
        wallet
            .build(wallet.token_request(RECIPIENT, "1", ETH, 18))
            .await
            .is_err()
    );
    assert!(
        wallet
            .build(wallet.request("0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed", "1"))
            .await
            .is_err()
    );
    let mut memo = wallet.request(RECIPIENT, "1");
    memo.memo = Some(crate::send::payment_memo::PaymentMemo {
        kind: crate::registry::PaymentMemoKind::MemoText,
        value: "1".into(),
    });
    assert!(wallet.build(memo).await.is_err());
    // An account whose guardian must co-sign, or of another class.
    state.guardian.store(true, Ordering::SeqCst);
    assert!(refusal(wallet.build(wallet.request(RECIPIENT, "1")).await).contains("guardian"));
    state.guardian.store(false, Ordering::SeqCst);
    *state.class.lock().unwrap() =
        Some("0x29927c8af6bccf3f6fda035981e765a7bdbf18a2dc0d630494f8758aa908e2b");
    assert!(
        refusal(wallet.build(wallet.request(RECIPIENT, "1")).await).contains("does not sign for")
    );
    *state.class.lock().unwrap() = Some(READY_CLASS);
    assert!(wallet.built_nothing().await);

    // Another transaction from the account lands between review and
    // signing.
    let built = wallet.build(wallet.request(RECIPIENT, "1")).await.unwrap();
    state.nonce.store(8, Ordering::SeqCst);
    assert!(refusal(wallet.sign(&built).await).contains("nonce"));
    state.nonce.store(7, Ordering::SeqCst);

    // Signed, and then another transaction takes its nonce: it can no
    // longer land.
    let signed = wallet.sign(&built).await.unwrap();
    state.nonce.store(8, Ordering::SeqCst);
    assert!(refusal(wallet.broadcast(&signed).await).contains("expired"));
    assert!(state.submitted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_node_on_another_network_is_refused() {
    let state = Arc::new(Node {
        chain_id: "0x534e5f5345504f4c4941",
        ..Arc::try_unwrap(Node::new(true)).ok().unwrap()
    });
    let (_server, wallet) = imported(&state).await;
    assert!(wallet.build(wallet.request(RECIPIENT, "1")).await.is_err());
    assert!(wallet.built_nothing().await);
}

#[tokio::test]
async fn a_quote_prices_the_deployment_with_the_transfer() {
    let state = Node::new(false);
    let (_server, wallet) = imported(&state).await;
    let undeployed = wallet
        .service
        .fetch_shared_send_preview(Chain::Starknet, SENDER.into())
        .await
        .unwrap();
    *state.class.lock().unwrap() = Some(READY_CLASS);
    let deployed = wallet
        .service
        .fetch_shared_send_preview(Chain::Starknet, SENDER.into())
        .await
        .unwrap();
    let fee = |preview: &crate::send::preview_types::SendPreview| {
        crate::amount::decimal::to_units(&preview.estimated_network_fee, 18).unwrap()
    };
    assert_eq!(fee(&undeployed), 2 * fee(&deployed));
    assert_eq!(deployed.spendable_balance, "10");
    assert_eq!(
        crate::amount::decimal::to_units(&deployed.max_sendable, 18).unwrap(),
        10_000_000_000_000_000_000 - fee(&deployed)
    );
}
