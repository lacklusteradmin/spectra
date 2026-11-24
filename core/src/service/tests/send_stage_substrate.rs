//! An Asset Hub transfer is quoted keeping the existential deposit, signed
//! only against the runtime and nonce it was reviewed with, and named by the
//! BLAKE2b hash of the extrinsic it signs. An `Assets` pallet transfer is
//! quoted as itself and refused, at build and again before signing, where
//! the runtime would reject it or remove what is left.
use super::*;
use crate::send::stages::SendStage;
use crate::service::send_stage_support::{Secret, Wallet, node, refusal};
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
    /// The sender's account id, hex: whose storage is the sender's.
    sender: String,
    /// Every other account's free balance.
    others_free: u128,
    /// Asset 1984's `Asset` value, and the sender's and the recipient's
    /// `Account` values, by their name in `asset-hub-assets.json`; `None`
    /// holds none.
    asset: &'static str,
    metadata: &'static str,
    sender_asset: Option<&'static str>,
    recipient_asset: Option<&'static str>,
}

/// The `Assets` pallet vectors @polkadot/types encoded.
fn assets() -> Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/asset-hub-assets.json"
    ))
    .unwrap()
}

/// An Asset Hub node: its genesis, runtime and metadata, the sender holding
/// `free` and everyone else `others_free`, and asset 1984 as `Runtime` names.
async fn asset_hub(live: Arc<Mutex<Runtime>>) -> wiremock::MockServer {
    let metadata = format!(
        "0x{}",
        hex::encode(crate::api::substrate_json_rpc::tests::fixture(
            Chain::Polkadot
        ))
    );
    let states = assets()["polkadot"]["state"].clone();
    let state = move |name: &str| states[name]["hex"].clone();
    // A storage item's prefix: the pallet's and the item's hashed names.
    let prefix =
        |name: &str| assets()["polkadot"]["state"][name]["key"].as_str().unwrap()[..66].to_string();
    let (asset, account, asset_metadata) = (prefix("asset"), prefix("account"), prefix("metadata"));
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
            "state_getStorage" => {
                let key = body["params"][0].as_str().unwrap();
                let senders = key.ends_with(&live.sender);
                if key.starts_with(&asset) {
                    state(live.asset)
                } else if key.starts_with(&account) {
                    match if senders {
                        live.sender_asset
                    } else {
                        live.recipient_asset
                    } {
                        Some(name) => state(name),
                        None => Value::Null,
                    }
                } else if key.starts_with(&asset_metadata) {
                    state(live.metadata)
                } else {
                    // AccountInfo: four counters, then free, reserved and frozen.
                    let mut record = vec![0; 16];
                    record.extend(if senders { live.free } else { live.others_free }.to_le_bytes());
                    record.extend([0; 48]);
                    json!(format!("0x{}", hex::encode(record)))
                }
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
        sender: String::new(),
        others_free: 10 * DOT,
        asset: "asset",
        metadata: "metadata",
        sender_asset: Some("account"),
        recipient_asset: Some("recipient"),
    }));
    let server = asset_hub(live.clone()).await;
    let wallet = Wallet::import(Chain::Polkadot, &server.uri(), Secret::Phrase(PHRASE)).await;
    live.lock().unwrap().sender =
        hex::encode(crate::derivation::polkadot::decode_ss58(&wallet.address).unwrap());
    (wallet, server, live)
}

/// What the preview offers keeps the existential deposit (0.01 DOT) and the
/// quoted fee back from the free balance.
#[tokio::test]
async fn a_substrate_preview_keeps_the_existential_deposit_and_the_fee_back() {
    let (wallet, _server, _) = wallet().await;
    let preview = wallet
        .service
        .fetch_shared_send_preview(Chain::Polkadot, wallet.address.clone())
        .await
        .unwrap();
    assert_eq!(preview.estimated_network_fee, "0.001");
    assert_eq!(preview.spendable_balance, "9.99");
    assert_eq!(preview.max_sendable, "9.989");
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
    let extrinsic = hex::decode(
        signed
            .signed_payload
            .as_deref()
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

/// Asset 1984 as the vectors hold it: six decimals, a minimum balance of 0.7.
const USDT: &str = "1984";

/// A recipient whose account id is the vectors' recipient, `0x09` × 32.
fn vector_recipient() -> String {
    crate::derivation::primitives::encode_ss58(&[9; 32], 0)
}

/// The fee is the asset transfer's own quote, in DOT; all of the asset may
/// be sent.
#[tokio::test]
async fn an_asset_send_quotes_its_own_transfer_and_offers_all_of_it() {
    let (wallet, _server, _) = wallet().await;
    let holding = wallet
        .hold_token("Asset Hub Asset", USDT, 6, "123.456789")
        .await;
    let quote = wallet
        .service
        .preview_owned_send(
            wallet.id.clone(),
            holding,
            "1".into(),
            vector_recipient(),
            None,
            None,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(quote.preview.estimated_network_fee, "0.001");
    assert_eq!(quote.preview.spendable_balance, "123.456789");
    assert_eq!(quote.preview.max_sendable, "123.456789");
    assert_eq!(quote.total, None);
}

/// The transaction is `Assets.transfer` as polkadot.js encodes it, its fee
/// paid in DOT; a frozen asset since the review refuses signing.
#[tokio::test]
async fn an_asset_transfer_is_built_as_polkadot_js_encodes_it_and_checked_again_before_signing() {
    let (wallet, _server, live) = wallet().await;
    let fixture = assets();
    let built = wallet
        .build(wallet.token_request(&vector_recipient(), "1.234567", USDT, 6))
        .await
        .unwrap();
    assert!(
        built.signing_payload_hex.starts_with(
            fixture["polkadot"]["calls"]["usdt"]
                .as_str()
                .unwrap()
                .trim_start_matches("0x")
        ),
        "{}",
        built.signing_payload_hex
    );
    // Its fee is DOT's; no DOT moves beside it.
    let prepared: Value = serde_json::from_str(&built.prepared_details).unwrap();
    let prepared = &prepared["Substrate"];
    assert_eq!(
        (
            &prepared["fee"],
            &prepared["amount"],
            &prepared["asset"]["id"]
        ),
        (&json!("10000000"), &json!("0"), &json!(1984))
    );
    live.lock().unwrap().asset = "asset_frozen";
    let error = refusal(wallet.sign(&built).await);
    assert!(error.contains("asset is frozen"), "{error}");
    live.lock().unwrap().asset = "asset";
    // Its metadata now states another precision: the reviewed amount means
    // something else.
    live.lock().unwrap().metadata = "metadata_eight";
    let error = refusal(wallet.sign(&built).await);
    assert!(error.contains("precision changed"), "{error}");
    assert_eq!(wallet.stored(&built).await.stage, SendStage::Prepared);
    live.lock().unwrap().metadata = "metadata";
    assert_eq!(wallet.sign(&built).await.unwrap().stage, SendStage::Signed);
}

/// What the runtime would reject, and what it would take but remove, is
/// refused before anything is built.
#[tokio::test]
async fn an_asset_transfer_the_runtime_would_reject_or_lose_is_refused() {
    let (wallet, _server, live) = wallet().await;
    let recipient = vector_recipient();
    let cases: [(&str, fn(&mut Runtime), &str); 7] = [
        ("1", |l| l.asset = "asset_frozen", "asset is frozen"),
        (
            "1",
            |l| l.sender_asset = Some("account_frozen"),
            "holding of the asset is frozen",
        ),
        ("1", |l| l.sender_asset = None, "holds none of this asset"),
        ("200", |_| {}, "Insufficient asset balance"),
        // 0.456789 would be left, below the 0.7 minimum.
        ("123", |_| {}, "Send all of it"),
        (
            "1",
            |l| l.recipient_asset = Some("account_blocked"),
            "cannot receive it",
        ),
        (
            "0.5",
            |l| l.recipient_asset = None,
            "send at least its minimum balance",
        ),
    ];
    for (amount, set, expected) in cases {
        {
            let mut live = live.lock().unwrap();
            set(&mut live);
        }
        let error = refusal(
            wallet
                .build(wallet.token_request(&recipient, amount, USDT, 6))
                .await,
        );
        assert!(error.contains(expected), "{amount}: {error}");
        let mut live = live.lock().unwrap();
        (live.asset, live.sender_asset, live.recipient_asset) =
            ("asset", Some("account"), Some("recipient"));
    }
    // An asset that cannot open an account, to an account with no DOT.
    {
        let mut live = live.lock().unwrap();
        (live.asset, live.recipient_asset, live.others_free) = ("asset_insufficient", None, 0);
    }
    let error = refusal(
        wallet
            .build(wallet.token_request(&recipient, "1", USDT, 6))
            .await,
    );
    assert!(error.contains("cannot open an account"), "{error}");
    assert!(wallet.built_nothing().await);
    // All of it may go, to a new holder, leaving nothing behind.
    live.lock().unwrap().asset = "asset";
    wallet
        .build(wallet.token_request(&recipient, "123.456789", USDT, 6))
        .await
        .unwrap();
}
