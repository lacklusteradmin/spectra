//! A Cardano transaction lives a fixed span past the tip, is signed only
//! over the inputs it was reviewed with, and is broadcast only to a node on
//! its own network.
use super::send_stage_support::{Secret, Wallet, node, refusal};
use super::*;
use crate::send::stages::SendStage;
use serde_json::{Value, json};
use std::sync::Mutex;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path},
};

fn vector() -> Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/cardano-emurgo-witness.json"
    ))
    .unwrap()
}

/// What the Koios node says: its network's magic, the tip's slot, and
/// whether the wallet's 1.17 ADA output has since gained a native asset.
struct Koios {
    magic: &'static str,
    tip: u64,
    changed: bool,
}

const MAINNET: &str = "764824073";
const SUBMITTED: &str = "/submittx";

async fn koios(live: Arc<Mutex<Koios>>) -> wiremock::MockServer {
    let server = node(move |path, _| {
        let live = live.lock().unwrap();
        let asset = json!({"policy_id": "a".repeat(56), "asset_name": "01", "quantity": "1"});
        Some(match path {
            "/epoch_params" => json!([{"epoch_no": 660, "min_fee_a": 44, "min_fee_b": 155381,
                "coins_per_utxo_size": "4310", "max_tx_size": 16384, "max_val_size": 5000}]),
            "/genesis" => json!([{"networkmagic": live.magic, "networkid": "Mainnet"}]),
            "/tip" => json!([{"abs_slot": live.tip}]),
            "/address_utxos" => json!([
                {"tx_hash": "11".repeat(32), "tx_index": 0, "value": "2000000",
                    "is_spent": false, "asset_list": [asset]},
                {"tx_hash": "00".repeat(32), "tx_index": 0, "value": "1170000",
                    "is_spent": false, "asset_list": if live.changed { json!([asset]) } else { json!([]) }},
            ]),
            other => panic!("unexpected Koios request {other}"),
        })
    })
    .await;
    // Koios accepts a submission with 202 and the transaction's id.
    Mock::given(method("POST"))
        .and(path(SUBMITTED))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!("ab".repeat(32))))
        .with_priority(1)
        .mount(&server)
        .await;
    server
}

async fn wallet(tip: u64) -> (Wallet, wiremock::MockServer, Arc<Mutex<Koios>>) {
    let live = Arc::new(Mutex::new(Koios {
        magic: MAINNET,
        tip,
        changed: false,
    }));
    let server = koios(live.clone()).await;
    let wallet = Wallet::import(
        Chain::Cardano,
        &server.uri(),
        Secret::Phrase(vector()["mnemonic"].as_str().unwrap()),
    )
    .await;
    (wallet, server, live)
}

fn recipient() -> String {
    vector()["address"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_cardano_transaction_lives_7200_slots_past_the_tip() {
    let (wallet, _server, _) = wallet(1_000).await;
    let built = wallet
        .build(wallet.request(&recipient(), "1"))
        .await
        .unwrap();
    let prepared: Value = serde_json::from_str(&built.prepared_details).unwrap();
    assert_eq!(prepared["Cardano"]["ttl"], 8_200);
}

/// An input spent, or holding something other than it did, since the
/// review is a different transaction.
#[tokio::test]
async fn a_cardano_input_changed_since_review_refuses_signing() {
    let (wallet, _server, live) = wallet(0).await;
    let built = wallet
        .build(wallet.request(&recipient(), "1"))
        .await
        .unwrap();
    live.lock().unwrap().changed = true;
    let error = refusal(wallet.sign(&built).await);
    assert!(error.contains("Cardano input changed"), "{error}");
    let stored = wallet.stored(&built).await;
    assert_eq!(stored.stage, SendStage::Prepared);
    assert!(stored.signed_payload.is_none());
}

#[tokio::test]
async fn a_signed_cardano_transaction_is_broadcast_only_to_a_node_on_its_network() {
    let (wallet, server, live) = wallet(0).await;
    let built = wallet
        .build(wallet.request(&recipient(), "1"))
        .await
        .unwrap();
    let signed = wallet.sign(&built).await.unwrap();
    let submissions = || async {
        server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|request| request.url.path() == SUBMITTED)
            .map(|request| hex::encode(request.body))
            .collect::<Vec<_>>()
    };
    live.lock().unwrap().magic = "1";
    let error = refusal(wallet.broadcast(&signed).await);
    assert!(error.contains("wrong network"), "{error}");
    assert!(submissions().await.is_empty());
    live.lock().unwrap().magic = MAINNET;
    wallet.broadcast(&signed).await.unwrap();
    let payload: Value = serde_json::from_str(signed.signed_payload.as_deref().unwrap()).unwrap();
    assert_eq!(submissions().await, [payload["cbor_hex"].as_str().unwrap()]);
}
