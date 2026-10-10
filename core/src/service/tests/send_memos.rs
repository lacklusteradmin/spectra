//! A payment to an account that asks for a destination tag or memo — XRP's
//! `lsfRequireDestTag`, Stellar's SEP-29 `config.memo_required` — is refused
//! without one, before anything is read for the transaction and again before
//! signing; the one given is bound into the review.
use super::send_stage_support::{Secret, Wallet, methods, node, paths, refusal};
use super::*;
use crate::registry::PaymentMemoKind;
use crate::send::payment_memo::PaymentMemo;
use crate::send::stages::SendStage;
use serde_json::{Value, json};
use std::sync::Mutex;

const REQUIRE_DEST_TAG: u32 = 0x0002_0000;

fn vectors() -> Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/payment-memos.json")).unwrap()
}

/// An XRPL node holding the sender and a destination whose flags are
/// `flags`.
async fn xrpl(destination: String, flags: Arc<Mutex<u32>>) -> wiremock::MockServer {
    node(move |_, body| {
        let account = body["params"][0]["account"].as_str().unwrap_or_default();
        let result = match body["method"].as_str().unwrap() {
            "server_info" => json!({"info": {"network_id": 0}}),
            "fee" => json!({"drops": {"open_ledger_fee": "12"}}),
            "server_state" => json!({"state": {"validated_ledger":
                {"seq": 93_000_300, "reserve_base": 1_000_000, "reserve_inc": 200_000}}}),
            "account_info" if account == destination => json!({"account_data":
                {"Balance": "50000000", "Sequence": 5, "OwnerCount": 0, "Flags": *flags.lock().unwrap()}}),
            "account_info" => json!({"account_data":
                {"Balance": "25000000", "Sequence": 7, "OwnerCount": 0, "Flags": 0}}),
            "account_objects" => json!({"account_objects": []}),
            other => panic!("unexpected XRPL request {other}"),
        };
        Some(json!({"result": result}))
    })
    .await
}

/// A Horizon on the test network holding the sender and a destination that
/// asks for a memo while `required` holds.
async fn horizon(destination: String, required: Arc<Mutex<bool>>) -> wiremock::MockServer {
    node(move |path, _| {
        let account = |sequence: &str, data: Value| {
            json!({"balances": [{"balance": "5.0000000", "asset_type": "native"}],
                "sequence": sequence, "subentry_count": 0, "num_sponsoring": 0,
                "num_sponsored": 0, "flags": {"auth_immutable": false}, "data": data})
        };
        Some(match path {
            "/" => json!({"network_passphrase": "Test SDF Network ; September 2015"}),
            "/ledgers" => json!({"_embedded": {"records":
                [{"sequence": 1_000, "base_reserve_in_stroops": 5_000_000}]}}),
            "/fee_stats" => json!({"fee_charged": {"mode": "100"}}),
            path if path == format!("/accounts/{destination}") => account(
                "1",
                if *required.lock().unwrap() {
                    json!({"config.memo_required": "MQ=="})
                } else {
                    json!({})
                },
            ),
            path if path.starts_with("/accounts/") => account("123456789100", json!({})),
            other => panic!("unexpected Horizon request {other}"),
        })
    })
    .await
}

async fn xrp_wallet(flags: u32) -> (Wallet, wiremock::MockServer, Arc<Mutex<u32>>, String) {
    let vectors = vectors();
    let destination = vectors["xrpl"]["payments"][0]["transaction"]["Destination"]
        .as_str()
        .unwrap()
        .to_string();
    let flags = Arc::new(Mutex::new(flags));
    let server = xrpl(destination.clone(), flags.clone()).await;
    let wallet = Wallet::import(
        Chain::Xrp,
        &server.uri(),
        Secret::Key(vectors["xrpl"]["key"].as_str().unwrap()),
    )
    .await;
    (wallet, server, flags, destination)
}

async fn stellar_wallet(
    required: bool,
) -> (Wallet, wiremock::MockServer, Arc<Mutex<bool>>, String) {
    let vectors = vectors();
    let destination = vectors["stellar"]["destination"]
        .as_str()
        .unwrap()
        .to_string();
    let required = Arc::new(Mutex::new(required));
    let server = horizon(destination.clone(), required.clone()).await;
    let wallet = Wallet::import(
        Chain::StellarTestnet,
        &server.uri(),
        Secret::Key(vectors["stellar"]["seed"].as_str().unwrap()),
    )
    .await;
    (wallet, server, required, destination)
}

fn memo(kind: PaymentMemoKind, value: &str) -> Option<PaymentMemo> {
    Some(PaymentMemo {
        kind,
        value: value.into(),
    })
}

/// A transaction refused at signing stays as it was reviewed: unsigned.
async fn still_unsigned(wallet: &Wallet, artifact: &crate::send::stages::SendArtifact) {
    let stored = wallet.stored(artifact).await;
    assert_eq!(stored.stage, SendStage::Prepared);
    assert!(stored.signed_payload.is_none());
}

#[tokio::test]
async fn an_xrp_destination_that_requires_a_tag_refuses_an_untagged_payment_having_read_only_that()
{
    let (wallet, server, _, destination) = xrp_wallet(REQUIRE_DEST_TAG).await;
    let error = refusal(wallet.build(wallet.request(&destination, "1")).await);
    assert!(error.contains("requires a destination tag"), "{error}");
    assert_eq!(methods(&server).await, ["server_info", "account_info"]);
    assert!(wallet.built_nothing().await);
}

#[tokio::test]
async fn an_xrp_destination_that_starts_requiring_a_tag_after_review_refuses_signing() {
    let (wallet, _server, flags, destination) = xrp_wallet(0).await;
    let built = wallet
        .build(wallet.request(&destination, "1"))
        .await
        .unwrap();
    assert_eq!(built.memo, None);
    *flags.lock().unwrap() = REQUIRE_DEST_TAG;
    let error = refusal(wallet.sign(&built).await);
    assert!(error.contains("requires a destination tag"), "{error}");
    still_unsigned(&wallet, &built).await;
}

#[tokio::test]
async fn a_stellar_destination_that_requires_a_memo_refuses_a_bare_payment_having_read_only_that() {
    let (wallet, server, _, destination) = stellar_wallet(true).await;
    let error = refusal(wallet.build(wallet.request(&destination, "1.5")).await);
    assert!(error.contains("requires a memo"), "{error}");
    assert_eq!(
        paths(&server).await,
        ["/".to_string(), format!("/accounts/{destination}")]
    );
    assert!(wallet.built_nothing().await);
}

#[tokio::test]
async fn a_stellar_destination_that_starts_requiring_a_memo_after_review_refuses_signing() {
    let (wallet, _server, required, destination) = stellar_wallet(false).await;
    let built = wallet
        .build(wallet.request(&destination, "1.5"))
        .await
        .unwrap();
    *required.lock().unwrap() = true;
    let error = refusal(wallet.sign(&built).await);
    assert!(error.contains("requires a memo"), "{error}");
    still_unsigned(&wallet, &built).await;
}

/// The tag or memo is part of what was reviewed: one changed in the store
/// afterwards, request and view alike, is refused on reading and signing.
#[tokio::test]
async fn a_tag_or_memo_changed_after_review_is_refused_as_altered() {
    let (xrp, _xrp_server, _, xrp_destination) = xrp_wallet(REQUIRE_DEST_TAG).await;
    let (stellar, _stellar_server, _, stellar_destination) = stellar_wallet(true).await;
    for (wallet, destination, given, changed) in [
        (
            &xrp,
            &xrp_destination,
            memo(PaymentMemoKind::DestinationTag, "77"),
            "78",
        ),
        (
            &stellar,
            &stellar_destination,
            memo(PaymentMemoKind::MemoText, "deposit"),
            "elsewhere",
        ),
    ] {
        let built = wallet
            .build(crate::send::SendExecutionRequest {
                memo: given.clone(),
                ..wallet.request(destination, "1")
            })
            .await
            .unwrap();
        assert_eq!(built.memo, given);
        wallet.rewrite(&built, |payload| {
            payload["view"]["memo"]["value"] = json!(changed);
            payload["request"]["memo"]["value"] = json!(changed);
        });
        let error = refusal(wallet.service.inspect_send(built.id.clone()).await);
        assert!(error.contains("altered"), "{error}");
        let error = refusal(wallet.sign(&built).await);
        assert!(error.contains("altered"), "{error}");
    }
}
