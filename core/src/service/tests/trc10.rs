//! TRC-10 transfers against a Tron node: history read page by page through
//! TronGrid's fingerprints, and a send refused — at build, at signing and at
//! broadcast — for whatever the network would refuse it for at that moment.
use super::send_stage_support::{Secret, Wallet, node_at, refusal};
use super::*;
use crate::send::stages::SendStage;
use crate::service::HistoryRefreshScope;
use serde_json::{Value, json};
use std::sync::Mutex;

const TOKEN: &str = "1009999";

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/trc10-send-vectors.json"
    ))
    .unwrap()
}

/// The `41`-prefixed hex of a base58 Tron address.
fn hex_address(address: &str) -> String {
    format!(
        "41{}",
        crate::derivation::tron::tron_base58_to_evm_hex(address).unwrap()
    )
}

/// The ledger the node reads from: the token's precision, what the owner
/// holds of it and of TRX, and what the recipient holds of it.
struct Ledger {
    decimals: u8,
    balance: u64,
    native: u64,
    recipient_balance: u64,
}

impl Default for Ledger {
    fn default() -> Self {
        Self {
            decimals: 2,
            balance: 22_345,
            native: 10_000_000,
            recipient_balance: 0,
        }
    }
}

/// A TronGrid transfer of `amount` of the token, newest first by `time`.
fn transfer(hash: &str, time: u64, from: &str, to: &str, amount: u64) -> Value {
    json!({"txID": hash.repeat(32), "block_timestamp": time, "ret": [{"contractRet": "SUCCESS"}],
        "raw_data": {"contract": [{"type": "TransferAssetContract", "parameter": {"value": {
            "asset_name": hex::encode(TOKEN), "amount": amount,
            "owner_address": hex_address(from), "to_address": hex_address(to)}}}]}})
}

/// A Tron node, with its TronGrid account API under `/v1/accounts`.
async fn tron(ledger: Arc<Mutex<Ledger>>) -> wiremock::MockServer {
    let fixture = fixture();
    let owner = fixture["owner"].as_str().unwrap().to_string();
    let receiver = fixture["receiver"].as_str().unwrap().to_string();
    node_at(move |url, body| {
        let ledger = ledger.lock().unwrap();
        Some(match url.path() {
            "/wallet/getblockbynum" => {
                json!({"blockID": Chain::Tron.tron_genesis_block_id().unwrap()})
            }
            "/wallet/getnowblock" => json!({"blockID": fixture["block"]["id"],
                "block_header": {"raw_data": {"number": fixture["block"]["number"]}}}),
            "/wallet/getassetissuebyid" => json!({"id": TOKEN, "name": hex::encode("LegacyTest"),
                "abbr": hex::encode("T10"), "precision": ledger.decimals}),
            "/wallet/getaccount" if body["address"] == owner.as_str() => json!({"address": owner,
                "balance": ledger.native, "assetV2": [{"key": TOKEN, "value": ledger.balance}]}),
            "/wallet/getaccount" => json!({"address": body["address"], "balance": 1,
                "assetV2": [{"key": TOKEN, "value": ledger.recipient_balance}]}),
            "/wallet/getchainparameters" => json!({"chainParameter": [
                {"key": "getTransactionFee", "value": 1000},
                {"key": "getCreateAccountFee", "value": 100_000},
                {"key": "getCreateNewAccountFeeInSystemContract", "value": 1_000_000}]}),
            "/wallet/broadcasttransaction" => json!({"result": true, "txid": body["txID"]}),
            path if path == format!("/v1/accounts/{owner}/transactions/trc20") => {
                json!({"data": []})
            }
            // The newest transfer first; the one before it behind the
            // fingerprint the first page names, and nothing after that.
            path if path == format!("/v1/accounts/{owner}/transactions") => {
                if url
                    .query_pairs()
                    .any(|(key, value)| key == "fingerprint" && value == "older")
                {
                    json!({"data": [transfer("aa", 1_700_000_000_000, &owner, &receiver, 123)]})
                } else {
                    json!({"data": [transfer("bb", 1_700_000_001_000, &receiver, &owner, 250)],
                        "meta": {"fingerprint": "older"}})
                }
            }
            other => panic!("unexpected Tron request {other}"),
        })
    })
    .await
}

async fn wallet() -> (Wallet, wiremock::MockServer, Arc<Mutex<Ledger>>) {
    let ledger = Arc::new(Mutex::new(Ledger::default()));
    let server = tron(ledger.clone()).await;
    let wallet = Wallet::import(
        Chain::Tron,
        &server.uri(),
        Secret::Key(fixture()["key"].as_str().unwrap()),
    )
    .await;
    (wallet, server, ledger)
}

/// A send of 123.45 of the token, at its two places.
fn send(wallet: &Wallet) -> crate::send::SendExecutionRequest {
    wallet.token_request(fixture()["receiver"].as_str().unwrap(), "123.45", TOKEN, 2)
}

/// Each page saves what it read and keeps TronGrid's fingerprint for the
/// next; the page with none is the last.
#[tokio::test]
async fn trc10_history_is_read_through_each_fingerprint_until_none_is_left() {
    let (wallet, _server, _) = wallet().await;
    let page = |load_more| {
        let service = wallet.service.clone();
        let id = wallet.id.clone();
        async move {
            service
                .refresh_history(
                    HistoryRefreshScope::Wallets {
                        wallet_ids: vec![id],
                    },
                    load_more,
                    Some(50),
                    0.0,
                )
                .await
                .unwrap()
                .remove(0)
                .outcome
                .unwrap()
        }
    };
    let first = page(false).await;
    assert_eq!((first.added, first.exhausted), (1, false));
    let second = page(true).await;
    assert_eq!((second.added, second.exhausted), (1, true));
    let mut amounts: Vec<_> = wallet
        .service
        .fetch_all_history_records()
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.payload.transaction_hash.unwrap(), row.payload.amount))
        .collect();
    amounts.sort();
    assert_eq!(
        amounts,
        [
            ("aa".repeat(32), "1.23".to_string()),
            ("bb".repeat(32), "2.5".to_string())
        ]
    );
}

/// A token whose precision is not the reviewed one, a balance short of the
/// amount, TRX short of the bandwidth and activation budget, or a recipient
/// the amount would carry past int64: each is refused before anything is
/// stored.
#[tokio::test]
async fn a_trc10_send_the_network_would_refuse_is_never_built() {
    let (wallet, _server, ledger) = wallet().await;
    type Change = fn(&mut Ledger);
    let cases: [(Change, &str); 4] = [
        (|l| l.decimals = 3, "decimals changed"),
        (|l| l.balance = 0, "Insufficient TRC-10 token balance"),
        (
            |l| l.native = 0,
            "Insufficient TRX for the TRC-10 bandwidth and account activation budget",
        ),
        (
            |l| l.recipient_balance = i64::MAX as u64,
            "TRC-10 recipient balance would exceed the protocol range",
        ),
    ];
    for (change, words) in cases {
        *ledger.lock().unwrap() = Ledger::default();
        change(&mut ledger.lock().unwrap());
        let error = refusal(wallet.build(send(&wallet)).await);
        assert!(error.contains(words), "{words}: {error}");
        assert!(wallet.built_nothing().await, "{words}");
    }
}

/// What was true at review is read again: at signing the precision and the
/// balance, at the first broadcast the TRX that pays for it.
#[tokio::test]
async fn a_reviewed_trc10_send_is_refused_when_its_funds_change_before_signing_or_broadcast() {
    let (wallet, server, ledger) = wallet().await;
    let built = wallet.build(send(&wallet)).await.unwrap();
    type Change = fn(&mut Ledger);
    let cases: [(Change, &str); 2] = [
        (|l| l.decimals = 3, "TRC-10 precision changed"),
        (|l| l.balance = 0, "Insufficient TRC-10 token balance"),
    ];
    for (change, words) in cases {
        *ledger.lock().unwrap() = Ledger::default();
        change(&mut ledger.lock().unwrap());
        let error = refusal(wallet.sign(&built).await);
        assert!(error.contains(words), "{words}: {error}");
        assert_eq!(wallet.stored(&built).await.stage, SendStage::Prepared);
    }
    *ledger.lock().unwrap() = Ledger::default();
    let signed = wallet.sign(&built).await.unwrap();
    ledger.lock().unwrap().native = 0;
    let error = refusal(wallet.broadcast(&signed).await);
    assert!(error.contains("Insufficient TRX"), "{error}");
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| request.url.path() != "/wallet/broadcasttransaction")
    );
    assert!(wallet.stored(&signed).await.attempts.is_empty());
}
