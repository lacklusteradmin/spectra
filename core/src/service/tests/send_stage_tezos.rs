//! A Tezos transfer built, signed and broadcast against a mock node.
use super::*;
use crate::derivation::primitives::blake2b;
use crate::derivation::tezos::{base58check, prefix};
use crate::service::send_stage_support::{Secret, Wallet, node_at, refusal};
use serde_json::{Value, json};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
/// The phrase's accounts 0, 1 and 2 (`tezos-vectors.json`).
const SENDER: &str = "tz1VQA4RP4fLjEEMW2FR4pE9kAg5abb5h5GL";
const SENDER_KEY: &str = "edpku4US3ZykcZifjzSGFCmFr3zRgCKndE82estE4irj4d5oqDNDvf";
const RECIPIENT: &str = "tz1gvekQVEwFFdxT2KrcY6kZgFK1qMN3mmWF";
const EMPTY: &str = "tz1gud9W8wmESYFsjbyqUcD8ZPC3GHQy3zhD";
const CONTRACT: &str = "KT1H5b7LxEExkFd2Tng77TfuWbM5aPvHstPr";
const PROTOCOL: &str = "PsUshuai9QapM5TGj1JpuVGkdxz5GykdnEvS6Rh8SUVrARvZLCY";
const BRANCH: &str = "BLXuuBECtE1bYReydLQTSD1FgYwmgg6VbEBSRf6nHwxrhdNvff5";
const HEAD: &str = "BMURKx3R8bQUpHZAruDcUgEMKCKxmXGKRiwGSJtkt2Lc3C3r2Vm";

/// What the mock node holds, which a test moves between stages.
struct Node {
    chain_id: &'static str,
    protocol: &'static str,
    head: AtomicU64,
    counter: AtomicU64,
    revealed: AtomicBool,
    balance: AtomicU64,
    /// Milligas a simulation adds to every transfer it runs.
    extra_milligas: AtomicU64,
    /// Whether a simulation answers that the network would not apply it.
    fails: AtomicBool,
    /// The keys the simulated reveals named.
    revealed_keys: Mutex<Vec<String>>,
}

impl Node {
    fn new(revealed: bool) -> Arc<Self> {
        Arc::new(Self {
            chain_id: "NetXdQprcVkpaWU",
            protocol: PROTOCOL,
            head: AtomicU64::new(1_000),
            counter: AtomicU64::new(7),
            revealed: AtomicBool::new(revealed),
            balance: AtomicU64::new(5_000_000),
            extra_milligas: AtomicU64::new(0),
            fails: AtomicBool::new(false),
            revealed_keys: Mutex::new(Vec::new()),
        })
    }

    fn simulate(&self, body: &Value) -> Value {
        assert_eq!(body["chain_id"], self.chain_id);
        assert_eq!(body["operation"]["branch"], BRANCH);
        let contents = body["operation"]["contents"].as_array().unwrap();
        let answered: Vec<Value> = contents
            .iter()
            .enumerate()
            .map(|(index, content)| {
                let mut answered = content.clone();
                let result = if self.fails.load(Ordering::SeqCst) {
                    json!({"status": "failed", "errors": [
                        {"kind": "temporary", "id": "proto.025-PsUshuai.contract.balance_too_low"},
                    ]})
                } else if content["kind"] == "reveal" {
                    self.revealed_keys
                        .lock()
                        .unwrap()
                        .push(content["public_key"].as_str().unwrap().into());
                    json!({"status": "applied", "consumed_milligas": "171000"})
                } else {
                    // The first operation of a batch bears its signature check.
                    let milligas = if index == 0 { 2_168_788 } else { 2_101_000 }
                        + self.extra_milligas.load(Ordering::SeqCst);
                    json!({
                        "status": "applied",
                        "consumed_milligas": milligas.to_string(),
                        "allocated_destination_contract": content["destination"] == EMPTY,
                    })
                };
                answered["metadata"] = json!({"operation_result": result});
                answered
            })
            .collect();
        json!({"contents": answered, "signature": body["operation"]["signature"]})
    }
}

async fn mock(node: Arc<Node>) -> wiremock::MockServer {
    node_at(move |url, body| {
        let path = url.path();
        let contracts = "/chains/main/blocks/head/context/contracts/";
        Some(match path {
            "/chains/main/chain_id" => json!(node.chain_id),
            "/chains/main/blocks/head/protocols" => {
                json!({"protocol": node.protocol, "next_protocol": node.protocol})
            }
            "/chains/main/blocks/head~2/header" => json!({
                "hash": BRANCH, "level": node.head.load(Ordering::SeqCst) - 2,
            }),
            "/chains/main/blocks/head/header" => json!({
                "hash": HEAD, "level": node.head.load(Ordering::SeqCst),
            }),
            "/chains/main/blocks/head/helpers/scripts/simulate_operation" => node.simulate(body),
            "/injection/operation" => {
                assert_eq!(url.query(), Some("chain=main"));
                let signed = hex::decode(body.as_str().unwrap()).unwrap();
                json!(base58check(&prefix::OPERATION, &blake2b::<32>(&[&signed])))
            }
            _ if path == format!("{contracts}{SENDER}") => json!({
                "balance": node.balance.load(Ordering::SeqCst).to_string(),
                "counter": node.counter.load(Ordering::SeqCst).to_string(),
                "revealed": node.revealed.load(Ordering::SeqCst),
            }),
            _ if path == format!("{contracts}{RECIPIENT}") => {
                json!({"balance": "2500000", "counter": "93", "revealed": true})
            }
            _ if path == format!("{contracts}{EMPTY}") => {
                json!({"balance": "0", "counter": "246598707", "revealed": false})
            }
            other => panic!("unexpected request {other}"),
        })
    })
    .await
}

async fn wallet(node: &Arc<Node>) -> (Wallet, wiremock::MockServer) {
    let server = mock(node.clone()).await;
    let wallet = Wallet::import(Chain::Tezos, &server.uri(), Secret::Phrase(PHRASE)).await;
    assert_eq!(wallet.address, SENDER);
    (wallet, server)
}

async fn prepared(wallet: &Wallet, id: &str) -> (PreparedTezosTransfer, Option<String>) {
    let stored = wallet.service.load_send_artifact(id.into()).await.unwrap();
    let PreparedPayload::Tezos(prepared) = stored.prepared else {
        panic!("not a Tezos transfer");
    };
    (prepared, stored.request.fee_amount)
}

/// The signature `signed` carries verifies, under the generic-operation
/// watermark, with the sender's key.
fn assert_signed_by_sender(signed: &str) {
    use ed25519_dalek::Verifier as _;
    let signed = hex::decode(signed).unwrap();
    let (forged, signature) = signed.split_at(signed.len() - 64);
    let key: [u8; 32] = crate::derivation::tezos::decode_base58check(SENDER_KEY, &prefix::EDPK, 32)
        .unwrap()
        .try_into()
        .unwrap();
    ed25519_dalek::VerifyingKey::from_bytes(&key)
        .unwrap()
        .verify(
            &blake2b::<32>(&[&[0x03], forged]),
            &ed25519_dalek::Signature::from_slice(signature).unwrap(),
        )
        .unwrap();
}

#[tokio::test]
async fn a_revealed_account_sends_within_its_simulated_limits() {
    let node = Node::new(true);
    let (wallet, _server) = wallet(&node).await;
    let built = wallet
        .build(wallet.request(RECIPIENT, "1.5"))
        .await
        .unwrap();
    let (prepared, fee_amount) = prepared(&wallet, &built.id).await;
    assert_eq!(prepared.reveal, None);
    assert_eq!(prepared.amount, 1_500_000);
    assert_eq!(prepared.protocol, PROTOCOL);
    assert_eq!(prepared.branch, BRANCH);
    assert_eq!(prepared.expires_at_level, 998 + 600);
    // 2,168.788 gas simulated, and octez-client's 100 of headroom.
    assert_eq!(prepared.transaction.gas_limit, 2_269);
    assert_eq!(prepared.transaction.storage_limit, 0);
    assert_eq!(prepared.transaction.counter, 8);
    let size = prepared.forge(&[0; 32]).unwrap().len() as u64 + 64;
    let fee = 100 + size + 227 + 100;
    assert_eq!(prepared.total_fee(), u128::from(fee));
    assert_eq!(
        fee_amount.as_deref(),
        Some(crate::amount::decimal::from_units(u128::from(fee), 6).as_str())
    );
    assert!(node.revealed_keys.lock().unwrap().is_empty());

    let signed = wallet.sign(&built).await.unwrap();
    let hash = signed.transaction_hash.clone().unwrap();
    assert!(hash.starts_with('o'));
    assert_signed_by_sender(signed.signed_payload.as_deref().unwrap());
    let broadcast = wallet.broadcast(&signed).await.unwrap();
    assert_eq!(broadcast.transaction_hash.as_deref(), Some(hash.as_str()));
    assert!(
        broadcast
            .attempts
            .iter()
            .any(|attempt| attempt.outcome == SubmissionOutcome::Accepted)
    );
}

#[tokio::test]
async fn an_unrevealed_account_reveals_its_key_with_its_first_transfer() {
    let node = Node::new(false);
    let (wallet, _server) = wallet(&node).await;
    let built = wallet.build(wallet.request(EMPTY, "1")).await.unwrap();
    let (prepared, fee_amount) = prepared(&wallet, &built.id).await;
    // The protocol's own costs: the reveal's 171 gas and a transfer's 2,101,
    // each with 100 of headroom, and the empty recipient's 257 bytes.
    let reveal = prepared.reveal.unwrap();
    assert_eq!(
        (reveal.counter, reveal.gas_limit, reveal.storage_limit),
        (8, 271, 0)
    );
    assert_eq!(prepared.transaction.counter, 9);
    assert_eq!(prepared.transaction.gas_limit, 2_201);
    assert_eq!(prepared.transaction.storage_limit, 257);
    let burn = 257 * 250;
    assert_eq!(prepared.max_burn(Chain::Tezos).unwrap(), burn);
    assert_eq!(
        fee_amount.as_deref(),
        Some(crate::amount::decimal::from_units(prepared.total_fee() + burn, 6).as_str())
    );
    // Nothing was simulated before the key was at hand.
    assert!(node.revealed_keys.lock().unwrap().is_empty());

    let signed = wallet.sign(&built).await.unwrap();
    assert_eq!(*node.revealed_keys.lock().unwrap(), [SENDER_KEY]);
    assert_signed_by_sender(signed.signed_payload.as_deref().unwrap());
    let broadcast = wallet.broadcast(&signed).await.unwrap();
    assert_eq!(broadcast.transaction_hash, signed.transaction_hash);
}

#[tokio::test]
async fn what_the_network_would_refuse_is_refused_before_signing() {
    let node = Node::new(true);
    let (wallet, _server) = wallet(&node).await;

    // A token, more than the account holds, an address that is no Tezos
    // account's.
    let token = wallet.token_request(RECIPIENT, "1", CONTRACT, 6);
    assert!(wallet.build(token).await.is_err());
    assert!(refusal(wallet.build(wallet.request(RECIPIENT, "5")).await).contains("Insufficient"));
    assert!(
        wallet
            .build(wallet.request("tz1VQA4RP4fLjEEMW2FR4pE9kAg5abb5h5GM", "1"))
            .await
            .is_err()
    );
    assert!(wallet.built_nothing().await);

    // Another transaction from the account lands between review and
    // signing.
    let built = wallet.build(wallet.request(RECIPIENT, "1")).await.unwrap();
    node.counter.store(8, Ordering::SeqCst);
    assert!(refusal(wallet.sign(&built).await).contains("counter"));
    node.counter.store(7, Ordering::SeqCst);

    // The branch is no longer live.
    node.head.store(998 + 600, Ordering::SeqCst);
    assert!(refusal(wallet.sign(&built).await).contains("expired"));
    node.head.store(1_000, Ordering::SeqCst);

    // The exact batch now needs more gas than was reviewed, or would fail.
    node.extra_milligas.store(150_000, Ordering::SeqCst);
    assert!(refusal(wallet.sign(&built).await).contains("more gas"));
    node.extra_milligas.store(0, Ordering::SeqCst);
    node.fails.store(true, Ordering::SeqCst);
    assert!(refusal(wallet.sign(&built).await).contains("balance_too_low"));
    node.fails.store(false, Ordering::SeqCst);

    // The balance falls below what the transfer can cost.
    node.balance.store(1_000_000, Ordering::SeqCst);
    assert!(refusal(wallet.sign(&built).await).contains("Insufficient"));
    node.balance.store(5_000_000, Ordering::SeqCst);
    assert!(wallet.sign(&built).await.is_ok());
}

#[tokio::test]
async fn an_unrevealed_account_cannot_first_pay_a_contract() {
    let node = Node::new(false);
    let (wallet, _server) = wallet(&node).await;
    assert!(refusal(wallet.build(wallet.request(CONTRACT, "1")).await).contains("revealed"));
    assert!(wallet.built_nothing().await);
}

#[tokio::test]
async fn another_network_or_an_unknown_protocol_is_refused() {
    for node in [
        Arc::new(Node {
            chain_id: "NetXnHfVqm9iesp",
            ..Arc::try_unwrap(Node::new(true)).ok().unwrap()
        }),
        Arc::new(Node {
            protocol: "PtTALLiNtPec7mE7yY4m3k26J8Qukef3E3ehzhfXgFZKGtDdAXu",
            ..Arc::try_unwrap(Node::new(true)).ok().unwrap()
        }),
    ] {
        let protocol = node.protocol;
        let (wallet, _server) = wallet(&node).await;
        let refused = refusal(wallet.build(wallet.request(RECIPIENT, "1")).await);
        if protocol != PROTOCOL {
            assert!(refused.contains(protocol), "{refused}");
        }
        assert!(wallet.built_nothing().await);
    }
}
