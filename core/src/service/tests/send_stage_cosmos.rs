//! A Cosmos Hub transfer built, signed and broadcast against a mock node.
use super::*;
use crate::service::send_stage_support::{Secret, Wallet, node_at, refusal};
use base64::Engine as _;
use serde_json::json;
use sha2::Digest as _;
use std::sync::atomic::{AtomicU64, Ordering};

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const SENDER: &str = "cosmos19rl4cm2hmr8afy4kldpxz3fka4jguq0auqdal4";
const RECIPIENT: &str = "cosmos1tehv5km5e9y706rc2gzk9yyun9dljjjnvyt3u0";

/// What the mock node holds, which a test moves between stages.
struct Chain_ {
    network: &'static str,
    sequence: AtomicU64,
    height: AtomicU64,
    spendable: AtomicU64,
    account: bool,
    /// The `timeout_height` the last simulated transaction carried.
    simulated_timeout: AtomicU64,
}

impl Chain_ {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            network: "cosmoshub-4",
            sequence: AtomicU64::new(7),
            height: AtomicU64::new(1_000),
            spendable: AtomicU64::new(5_000_000),
            account: true,
            simulated_timeout: AtomicU64::new(0),
        })
    }
}

async fn mock(state: Arc<Chain_>) -> wiremock::MockServer {
    node_at(move |url, body| {
        let path = url.path();
        Some(match path {
            "/cosmos/base/tendermint/v1beta1/blocks/latest" => json!({
                "block": {"header": {
                    "chain_id": state.network,
                    "height": state.height.load(Ordering::SeqCst).to_string(),
                }}
            }),
            "/feemarket/v1/gas_price/uatom" => {
                json!({"price": {"denom": "uatom", "amount": "0.010000000000000000"}})
            }
            "/cosmos/tx/v1beta1/simulate" => {
                use prost::Message as _;
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(body["tx_bytes"].as_str().unwrap())
                    .unwrap();
                let tx = crate::send::cosmos::proto::TxRaw::decode(raw.as_slice()).unwrap();
                let tx_body =
                    crate::send::cosmos::proto::TxBody::decode(tx.body_bytes.as_slice()).unwrap();
                state
                    .simulated_timeout
                    .store(tx_body.timeout_height, Ordering::SeqCst);
                json!({"gas_info": {"gas_wanted": "0", "gas_used": "70000"}})
            }
            "/cosmos/tx/v1beta1/txs" => {
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(body["tx_bytes"].as_str().unwrap())
                    .unwrap();
                assert_eq!(body["mode"], "BROADCAST_MODE_SYNC");
                json!({"tx_response": {
                    "code": 0,
                    "txhash": hex::encode_upper(sha2::Sha256::digest(&raw)),
                }})
            }
            _ if path == format!("/cosmos/auth/v1beta1/accounts/{SENDER}") => {
                if !state.account {
                    return None;
                }
                json!({"account": {
                    "@type": "/cosmos.auth.v1beta1.BaseAccount",
                    "address": SENDER,
                    "account_number": "789012",
                    "sequence": state.sequence.load(Ordering::SeqCst).to_string(),
                }})
            }
            _ if path == format!("/cosmos/bank/v1beta1/spendable_balances/{SENDER}/by_denom") => {
                assert_eq!(url.query(), Some("denom=uatom"));
                json!({"balance": {
                    "denom": "uatom",
                    "amount": state.spendable.load(Ordering::SeqCst).to_string(),
                }})
            }
            other => panic!("unexpected request {other}"),
        })
    })
    .await
}

fn memo(value: &str) -> Option<crate::send::payment_memo::PaymentMemo> {
    Some(crate::send::payment_memo::PaymentMemo {
        kind: crate::registry::PaymentMemoKind::MemoText,
        value: value.into(),
    })
}

#[tokio::test]
async fn a_transfer_is_priced_by_simulation_signed_and_accepted() {
    let state = Chain_::new();
    let server = mock(state.clone()).await;
    let wallet = Wallet::import(Chain::CosmosHub, &server.uri(), Secret::Phrase(PHRASE)).await;
    assert_eq!(wallet.address, SENDER);
    let mut request = wallet.request(RECIPIENT, "1.5");
    request.memo = memo("104857600");
    let built = wallet.build(request).await.unwrap();
    // 70,000 gas used, 98,000 with the headroom, at the fee market's 0.01
    // uatom and a fifth more: above the network's 0.01 floor.
    let stored = wallet
        .service
        .load_send_artifact(built.id.clone())
        .await
        .unwrap();
    let PreparedPayload::Cosmos(prepared) = &stored.prepared else {
        panic!("not a Cosmos transfer");
    };
    assert_eq!(prepared.gas_limit, 98_000);
    assert_eq!(prepared.fee, 1_176);
    assert_eq!(prepared.amount, 1_500_000);
    assert_eq!(prepared.sequence, 7);
    assert_eq!(prepared.account_number, 789_012);
    assert_eq!(prepared.memo, "104857600");
    // Ten minutes of 5.6-second blocks.
    assert_eq!(prepared.timeout_height, 1_000 + 107);
    assert_eq!(stored.request.fee_amount.as_deref(), Some("0.001176"));

    let signed = wallet.sign(&built).await.unwrap();
    let hash = signed.transaction_hash.clone().unwrap();
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
async fn what_the_network_would_refuse_is_refused_before_signing() {
    let state = Chain_::new();
    let server = mock(state.clone()).await;
    let wallet = Wallet::import(Chain::CosmosHub, &server.uri(), Secret::Phrase(PHRASE)).await;

    // A memo over the Hub's 512 bytes, a token, more than can be spent.
    let mut long = wallet.request(RECIPIENT, "1");
    long.memo = memo(&"9".repeat(513));
    assert!(wallet.build(long).await.is_err());
    let token = wallet.token_request(
        RECIPIENT,
        "1",
        "ibc/27394FB092D2ECCD56123C74F36E4C1F926001CEADA9CA97EA622B25F41E5EB2",
        6,
    );
    assert!(wallet.build(token).await.is_err());
    assert!(refusal(wallet.build(wallet.request(RECIPIENT, "5")).await).contains("Insufficient"));
    // Another network's address.
    let osmosis = "osmo1tehv5km5e9y706rc2gzk9yyun9dljjjnylcp2a";
    assert!(wallet.build(wallet.request(osmosis, "1")).await.is_err());
    assert!(wallet.built_nothing().await);

    // The sequence moves on between review and signing: another
    // transaction from this account landed first.
    let built = wallet.build(wallet.request(RECIPIENT, "1")).await.unwrap();
    state.sequence.store(8, Ordering::SeqCst);
    assert!(refusal(wallet.sign(&built).await).contains("sequence"));
    state.sequence.store(7, Ordering::SeqCst);

    // The chain passes the timeout height before signing.
    state.height.store(1_000 + 107, Ordering::SeqCst);
    assert!(refusal(wallet.sign(&built).await).contains("timed out"));
}

#[tokio::test]
async fn an_account_the_chain_does_not_have_or_another_network_is_refused() {
    let state = Arc::new(Chain_ {
        account: false,
        ..Arc::try_unwrap(Chain_::new()).ok().unwrap()
    });
    let server = mock(state).await;
    let wallet = Wallet::import(Chain::CosmosHub, &server.uri(), Secret::Phrase(PHRASE)).await;
    assert!(refusal(wallet.build(wallet.request(RECIPIENT, "1")).await).contains("no account"));

    let state = Arc::new(Chain_ {
        network: "theta-testnet-001",
        ..Arc::try_unwrap(Chain_::new()).ok().unwrap()
    });
    let server = mock(state).await;
    let wallet = Wallet::import(Chain::CosmosHub, &server.uri(), Secret::Phrase(PHRASE)).await;
    assert!(wallet.build(wallet.request(RECIPIENT, "1")).await.is_err());
    assert!(wallet.built_nothing().await);
}

/// A quote simulates the transfer within the window the send itself gets: a
/// single block's margin expires inside the simulation on a fast network,
/// as BandChain's node refused one on 2026-11-26.
#[tokio::test]
async fn a_quote_simulates_within_the_sends_timeout() {
    let state = Chain_::new();
    let server = mock(state.clone()).await;
    let wallet = Wallet::import(Chain::CosmosHub, &server.uri(), Secret::Phrase(PHRASE)).await;
    let preview = wallet
        .service
        .preview_cosmos_send(Chain::CosmosHub, SENDER)
        .await
        .unwrap();
    assert_eq!(preview.estimated_network_fee, "0.001176");
    assert_eq!(state.simulated_timeout.load(Ordering::SeqCst), 1_000 + 107);
}
