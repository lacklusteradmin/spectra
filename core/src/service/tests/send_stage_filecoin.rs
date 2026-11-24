//! A Filecoin transfer built, signed and broadcast against a mock Lotus node.
use super::*;
use crate::service::send_stage_support::{Secret, Wallet, node_at, refusal};
use serde_json::{Value, json};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const SENDER: &str = "f1qode47ievxlxzk6z2viuovedabmn3tq6t57uqhq";
const RECIPIENT: &str = "f12oiips7bvuo3owfeietpqwhb6jf2zlcyoitbvra";
const ETHEREUM: &str = "f410froq7cckvdpkdfabqcjsfvqjw3xle3otsxy5tvsy";
const GENESIS: &str = "bafy2bzacecnamqgqmifpluoeldx7zzglxcljo6oja4vrmtj7432rphldpdmm2";
const GAS_LIMIT: u64 = 1_526_328;
const PREMIUM: u64 = 100_022;

/// What the mock node holds, which a test moves between stages.
struct Node {
    network: &'static str,
    genesis: &'static str,
    /// The sender's balance in attoFIL.
    balance: AtomicU64,
    /// What `MpoolGetNonce` answers: the next nonce, pending messages counted.
    next_nonce: AtomicU64,
    /// What `StateGetActor` answers for the sender: executed messages only.
    executed_nonce: AtomicU64,
    fee_cap: AtomicU64,
    /// Whether `StateSearchMsg` finds the pushed message mined.
    mined: std::sync::atomic::AtomicBool,
    pushed: Mutex<Vec<Value>>,
}

impl Node {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            network: "mainnet",
            genesis: GENESIS,
            balance: AtomicU64::new(5_000_000_000_000_000_000),
            next_nonce: AtomicU64::new(7),
            executed_nonce: AtomicU64::new(7),
            fee_cap: AtomicU64::new(101_213),
            mined: false.into(),
            pushed: Mutex::new(Vec::new()),
        })
    }
}

/// The CID the message Lotus is handed lands under, as a node computes it.
fn pushed_cid(pushed: &Value) -> String {
    let message = &pushed["Message"];
    let digits = |field: &str| message[field].as_str().unwrap().parse::<u128>().unwrap();
    let prepared = crate::send::filecoin::PreparedFilecoinMessage {
        sender: message["From"].as_str().unwrap().into(),
        recipient: message["To"].as_str().unwrap().into(),
        nonce: message["Nonce"].as_u64().unwrap(),
        value: digits("Value"),
        method: message["Method"].as_u64().unwrap(),
        gas_limit: message["GasLimit"].as_u64().unwrap(),
        gas_fee_cap: digits("GasFeeCap"),
        gas_premium: digits("GasPremium"),
    };
    use base64::Engine as _;
    let signature = base64::engine::general_purpose::STANDARD
        .decode(pushed["Signature"]["Data"].as_str().unwrap())
        .unwrap();
    prepared.signed_cid(Chain::Filecoin, &signature).unwrap()
}

fn answer(node: &Node, method: &str, params: &Value) -> Value {
    match method {
        "Filecoin.StateNetworkName" => json!(node.network),
        "Filecoin.ChainGetGenesis" => json!({"Cids": [{"/": node.genesis}], "Blocks": []}),
        "Filecoin.WalletBalance" => {
            assert_eq!(params[0], SENDER);
            json!(node.balance.load(Ordering::SeqCst).to_string())
        }
        "Filecoin.MpoolGetNonce" => {
            assert_eq!(params[0], SENDER);
            json!(node.next_nonce.load(Ordering::SeqCst))
        }
        "Filecoin.StateGetActor" => match params[0].as_str().unwrap() {
            SENDER => json!({"Nonce": node.executed_nonce.load(Ordering::SeqCst),
                             "Balance": "0", "DelegatedAddress": null}),
            "f01234" => json!({"Nonce": 3, "Balance": "0", "DelegatedAddress": null}),
            "f05678" => json!({"Nonce": 0, "Balance": "0", "DelegatedAddress": ETHEREUM}),
            _ => Value::Null,
        },
        "Filecoin.GasEstimateMessageGas" => {
            assert_eq!(params[1]["MaxFee"], "70000000000000000");
            let mut message = params[0].clone();
            message["GasLimit"] = json!(GAS_LIMIT);
            message["GasFeeCap"] = json!(node.fee_cap.load(Ordering::SeqCst).to_string());
            message["GasPremium"] = json!(PREMIUM.to_string());
            message
        }
        "Filecoin.MpoolPush" => {
            let pushed = params[0].clone();
            node.pushed.lock().unwrap().push(pushed.clone());
            json!({"/": pushed_cid(&pushed)})
        }
        "Filecoin.StateSearchMsg" => {
            if !node.mined.load(Ordering::SeqCst) {
                return Value::Null;
            }
            json!({"Message": params[1].clone(), "Receipt": {"ExitCode": 0}, "Height": 9})
        }
        other => panic!("unexpected method {other}"),
    }
}

async fn mock(node: Arc<Node>) -> wiremock::MockServer {
    node_at(move |_, body| {
        let result = answer(&node, body["method"].as_str().unwrap(), &body["params"]);
        Some(json!({"jsonrpc": "2.0", "id": body["id"], "result": result}))
    })
    .await
}

async fn wallet(node: Arc<Node>) -> (Wallet, wiremock::MockServer) {
    let server = mock(node).await;
    let wallet = Wallet::import(Chain::Filecoin, &server.uri(), Secret::Phrase(PHRASE)).await;
    assert_eq!(wallet.address, SENDER);
    (wallet, server)
}

async fn prepared(
    wallet: &Wallet,
    artifact: &SendArtifact,
) -> crate::send::filecoin::PreparedFilecoinMessage {
    let stored = wallet
        .service
        .load_send_artifact(artifact.id.clone())
        .await
        .unwrap();
    let PreparedPayload::Filecoin(prepared) = stored.prepared else {
        panic!("not a Filecoin message");
    };
    prepared
}

#[tokio::test]
async fn a_transfer_is_estimated_signed_and_accepted() {
    let node = Node::new();
    let (wallet, _server) = wallet(node.clone()).await;
    let built = wallet
        .build(wallet.request(RECIPIENT, "1.5"))
        .await
        .unwrap();
    let message = prepared(&wallet, &built).await;
    assert_eq!(message.value, 1_500_000_000_000_000_000);
    assert_eq!(message.nonce, 7);
    assert_eq!(message.method, crate::send::filecoin::METHOD_SEND);
    assert_eq!(
        (message.gas_limit, message.gas_fee_cap, message.gas_premium),
        (GAS_LIMIT, 101_213, u128::from(PREMIUM))
    );
    let stored = wallet
        .service
        .load_send_artifact(built.id.clone())
        .await
        .unwrap();
    assert_eq!(
        stored.request.fee_amount.as_deref(),
        Some(crate::amount::decimal::from_units(u128::from(GAS_LIMIT) * 101_213, 18).as_str())
    );

    let signed = wallet.sign(&built).await.unwrap();
    let cid = signed.transaction_hash.clone().unwrap();
    assert!(crate::derivation::filecoin::is_message_cid(&cid), "{cid}");
    let broadcast = wallet.broadcast(&signed).await.unwrap();
    assert_eq!(broadcast.transaction_hash.as_deref(), Some(cid.as_str()));
    assert!(
        broadcast
            .attempts
            .iter()
            .any(|attempt| attempt.outcome == SubmissionOutcome::Accepted)
    );
    let pushed = node.pushed.lock().unwrap()[0].clone();
    assert_eq!(pushed["Message"]["Nonce"], 7);
    assert_eq!(pushed["Message"]["Value"], "1500000000000000000");
    assert_eq!(pushed["Signature"]["Type"], 1);
}

#[tokio::test]
async fn an_ethereum_style_recipient_is_paid_through_invoke_evm() {
    let node = Node::new();
    let (wallet, _server) = wallet(node).await;
    for (recipient, method) in [
        (ETHEREUM, crate::send::filecoin::METHOD_INVOKE_EVM),
        // An ID whose actor has an f410f address, and one whose has none.
        ("f05678", crate::send::filecoin::METHOD_INVOKE_EVM),
        ("f01234", crate::send::filecoin::METHOD_SEND),
    ] {
        let built = wallet.build(wallet.request(recipient, "1")).await.unwrap();
        assert_eq!(
            prepared(&wallet, &built).await.method,
            method,
            "{recipient}"
        );
    }
    // Nothing creates an actor at an ID.
    assert!(refusal(wallet.build(wallet.request("f09999", "1")).await).contains("No actor"));
}

#[tokio::test]
async fn what_the_network_would_refuse_is_refused_before_signing() {
    let node = Node::new();
    let (wallet, _server) = wallet(node.clone()).await;

    // More than the balance, all of it with no FIL left for gas, a token,
    // another network's address.
    assert!(refusal(wallet.build(wallet.request(RECIPIENT, "6")).await).contains("Insufficient"));
    assert!(refusal(wallet.build(wallet.request(RECIPIENT, "5")).await).contains("Insufficient"));
    let token = wallet.token_request(
        RECIPIENT,
        "1",
        "f410froq7cckvdpkdfabqcjsfvqjw3xle3otsxy5tvsy",
        18,
    );
    assert!(wallet.build(token).await.is_err());
    assert!(
        wallet
            .build(wallet.request("t12oiips7bvuo3owfeietpqwhb6jf2zlcyoitbvra", "1"))
            .await
            .is_err()
    );
    // Gas that would cost more than the 0.07 FIL a message may pay.
    node.fee_cap.store(100_000_000_000, Ordering::SeqCst);
    assert!(refusal(wallet.build(wallet.request(RECIPIENT, "1")).await).contains("0.07 FIL"));
    node.fee_cap.store(101_213, Ordering::SeqCst);
    assert!(wallet.built_nothing().await);

    // Another message from the account takes the nonce between review and
    // signing, or spends the FIL.
    let built = wallet.build(wallet.request(RECIPIENT, "1")).await.unwrap();
    node.next_nonce.store(8, Ordering::SeqCst);
    assert!(refusal(wallet.sign(&built).await).contains("nonce"));
    node.next_nonce.store(7, Ordering::SeqCst);
    node.balance
        .store(1_000_000_000_000_000_000, Ordering::SeqCst);
    assert!(refusal(wallet.sign(&built).await).contains("Insufficient"));
}

#[tokio::test]
async fn a_node_on_another_network_is_refused() {
    for node in [
        Arc::new(Node {
            network: "calibrationnet",
            ..Arc::try_unwrap(Node::new()).ok().unwrap()
        }),
        Arc::new(Node {
            genesis: "bafy2bzacecyaggy24wol5ruvs6qm73gjibs2l2iyhcqmvi7r7a4ph7zx3yqd4",
            ..Arc::try_unwrap(Node::new()).ok().unwrap()
        }),
    ] {
        let (wallet, _server) = wallet(node).await;
        assert!(wallet.build(wallet.request(RECIPIENT, "1")).await.is_err());
        assert!(wallet.built_nothing().await);
    }
}

#[tokio::test]
async fn a_message_whose_nonce_the_chain_used_is_not_sent_again() {
    let node = Node::new();
    let (wallet, _server) = wallet(node.clone()).await;
    let built = wallet.build(wallet.request(RECIPIENT, "1")).await.unwrap();
    let signed = wallet.sign(&built).await.unwrap();
    wallet.broadcast(&signed).await.unwrap();

    // Another message with its nonce executed: this one can never land.
    node.executed_nonce.store(8, Ordering::SeqCst);
    assert!(refusal(wallet.broadcast(&signed).await).contains("expired"));
    // It is the one that executed.
    node.mined.store(true, Ordering::SeqCst);
    assert!(refusal(wallet.broadcast(&signed).await).contains("already confirmed"));
    assert_eq!(node.pushed.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_quote_is_the_most_a_transfer_can_cost_and_what_is_left_after_it() {
    let node = Node::new();
    let (wallet, _server) = wallet(node.clone()).await;
    let units = |amount: u128| crate::amount::decimal::from_units(amount, 18);
    let balance = 5_000_000_000_000_000_000u128;
    let fee = u128::from(GAS_LIMIT) * 101_213;
    let preview = wallet
        .service
        .fetch_shared_send_preview(Chain::Filecoin, SENDER.into())
        .await
        .unwrap();
    assert_eq!(preview.estimated_network_fee, units(fee));
    assert_eq!(preview.spendable_balance, units(balance));
    assert_eq!(preview.max_sendable, units(balance - fee));
    // With no FIL there is nothing a node can run: the quote is the most
    // any message may pay.
    node.balance.store(0, Ordering::SeqCst);
    let empty = wallet
        .service
        .fetch_shared_send_preview(Chain::Filecoin, SENDER.into())
        .await
        .unwrap();
    assert_eq!(
        empty.estimated_network_fee,
        units(crate::send::filecoin::MAX_FEE)
    );
    assert_eq!(empty.max_sendable, units(0));
}
