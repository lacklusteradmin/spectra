//! An Asset Hub transfer is quoted keeping the existential deposit, signed
//! only against the runtime and nonce it was reviewed with, and named by the
//! BLAKE2b hash of the extrinsic it signs.
use super::send_stage_support::{Secret, Wallet, node, refusal};
use super::*;
use crate::send::stages::SendStage;
use serde_json::{Value, json};
use std::sync::Mutex;

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const RECIPIENT: &str = "13UVJyLnbVp9RBZYFwFGyDvVd1y27Tt8tkntv6Q7JVPhFsTB";
/// One DOT, in planck.
const DOT: u128 = 10_000_000_000;

struct Runtime {
    spec: u32,
    nonce: u32,
    fee: u128,
    free: u128,
}

/// An Asset Hub node: its genesis, runtime and metadata, and every account
/// holding `free`.
async fn asset_hub(live: Arc<Mutex<Runtime>>) -> wiremock::MockServer {
    let metadata = format!(
        "0x{}",
        hex::encode(crate::api::substrate_json_rpc::tests::fixture(
            Chain::Polkadot
        ))
    );
    node(move |_, body| {
        let live = live.lock().unwrap();
        let block = format!("0x{}", "11".repeat(32));
        let result = match body["method"].as_str().unwrap() {
            "chain_getBlockHash" if body["params"][0] == 0 => {
                json!(Chain::Polkadot.substrate_genesis_hash().unwrap())
            }
            "chain_getBlockHash" | "chain_getFinalizedHead" => json!(block),
            "chain_getHeader" => json!({"number": "0x64"}),
            "state_getRuntimeVersion" => {
                json!({"specVersion": live.spec, "transactionVersion": 15})
            }
            "state_getMetadata" => json!(metadata),
            "system_accountNextIndex" => json!(live.nonce),
            "payment_queryInfo" => json!({"partialFee": live.fee.to_string()}),
            // AccountInfo: four counters, then free, reserved and frozen.
            "state_getStorage" => {
                let mut record = vec![0; 16];
                record.extend(live.free.to_le_bytes());
                record.extend([0; 48]);
                json!(format!("0x{}", hex::encode(record)))
            }
            other => panic!("unexpected Substrate request {other}"),
        };
        Some(json!({"jsonrpc": "2.0", "id": body["id"], "result": result}))
    })
    .await
}

async fn wallet() -> (Wallet, wiremock::MockServer, Arc<Mutex<Runtime>>) {
    let live = Arc::new(Mutex::new(Runtime {
        spec: 2_005_000,
        nonce: 7,
        fee: DOT / 1_000,
        free: 10 * DOT,
    }));
    let server = asset_hub(live.clone()).await;
    let wallet = Wallet::import(Chain::Polkadot, &server.uri(), Secret::Phrase(PHRASE)).await;
    (wallet, server, live)
}

/// What the preview offers keeps the existential deposit (0.01 DOT) and the
/// quoted fee back from the free balance.
#[tokio::test]
async fn a_substrate_preview_keeps_the_existential_deposit_and_the_fee_back() {
    let (wallet, _server, _) = wallet().await;
    let preview: Value = serde_json::from_str(
        &wallet
            .service
            .fetch_simple_chain_send_preview_json(Chain::Polkadot, wallet.address.clone())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(preview["fee_display"], "0.001");
    assert_eq!(preview["balance_display"], "9.99");
    assert_eq!(preview["max_sendable"], "9.989");
}

/// A runtime upgrade or a nonce used since the review makes the reviewed
/// extrinsic one the network would read differently, or not at all.
#[tokio::test]
async fn a_runtime_or_nonce_changed_since_review_refuses_signing() {
    let (wallet, _server, live) = wallet().await;
    let built = wallet.build(wallet.request(RECIPIENT, "1")).await.unwrap();
    for (spec, nonce) in [(2_005_001, 7), (2_005_000, 8)] {
        {
            let mut live = live.lock().unwrap();
            live.spec = spec;
            live.nonce = nonce;
        }
        let error = refusal(wallet.sign(&built).await);
        assert!(
            error.contains("runtime, network or nonce changed"),
            "{error}"
        );
        assert_eq!(wallet.stored(&built).await.stage, SendStage::Prepared);
    }
    {
        let mut live = live.lock().unwrap();
        live.spec = 2_005_000;
        live.nonce = 7;
    }
    let signed = wallet.sign(&built).await.unwrap();
    assert_eq!(signed.stage, SendStage::Signed);
    // Its hash is BLAKE2b-256 of the extrinsic it signed, as the network
    // names it.
    let payload: Value = serde_json::from_str(signed.signed_payload.as_deref().unwrap()).unwrap();
    let extrinsic = hex::decode(
        payload["extrinsic_hex"]
            .as_str()
            .unwrap()
            .trim_start_matches("0x"),
    )
    .unwrap();
    let hash = blake2b_simd::Params::new().hash_length(32).hash(&extrinsic);
    assert_eq!(
        signed.transaction_hash.as_deref(),
        Some(format!("0x{}", hash.to_hex()).as_str())
    );
}
