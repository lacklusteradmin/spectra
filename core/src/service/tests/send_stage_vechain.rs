//! VeChainThor transfers of VET, VTHO and another VIP-180 token built,
//! signed and broadcast against a mock node, and every refusal before
//! signing.
use super::*;
use crate::send::stages::SubmissionOutcome;
use crate::service::send_stage_support::{Secret, Wallet, node_at, refusal};
use serde_json::{Value, json};
use std::sync::Mutex;

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
/// `m/44'/818'/0'/0/0` and `/1` of the phrase (`vechain-vectors.json`).
const SENDER: &str = "0xe5d846748409df0b23d5bd47ceb14afc1cbbbcb3";
const RECIPIENT: &str = "0xDb62FE39887b1b3b80a05e16596150CcDB12199e";
const ENERGY: &str = "0x0000000000000000000000000000456e65726779";
const B3TR: &str = "0x5ef79995fe8a89e0812330e4378eb2660cede699";
const VTHO: u128 = 1_000_000_000_000_000_000;
const BEST_ID: &str = "0x019457919ffa9de1528c77d107e94b18c9a110db1af40a0f4fda62238bd249d5";

/// What the mock node holds, which a test moves between stages.
struct Ledger {
    genesis: &'static str,
    best: u64,
    base_fee: Option<u128>,
    vet: u128,
    vtho: u128,
    b3tr: u128,
    /// The id the node answers a submission with.
    accepted: Option<String>,
    submitted: Vec<String>,
}

impl Default for Ledger {
    fn default() -> Self {
        Self {
            genesis: "0x00000000851caf3cfdb6e899cf5958bfb1ac3413d346d43539627e6be7ec1b4a",
            best: 26_498_961,
            base_fee: Some(10_000_000_000_000),
            vet: 2_000 * VTHO,
            vtho: 3 * VTHO,
            b3tr: 50,
            accepted: None,
            submitted: Vec::new(),
        }
    }
}

fn word(value: u128) -> String {
    format!("0x{value:064x}")
}

fn call(data: String, gas: u64, vm_error: &str) -> Value {
    json!({"data": data, "events": [], "transfers": [], "gasUsed": gas,
           "reverted": !vm_error.is_empty(), "vmError": vm_error})
}

/// One clause run by the node: a balance or decimals read, a token
/// transfer, or VET to an address with no code.
fn run(ledger: &Ledger, clause: &Value, caller: Option<&str>) -> Value {
    let to = clause["to"].as_str().unwrap().to_ascii_lowercase();
    let data = clause["data"].as_str().unwrap();
    let held = match to.as_str() {
        ENERGY => ledger.vtho,
        B3TR => ledger.b3tr,
        _ => 0,
    };
    if data.starts_with("0x70a08231") {
        assert!(data.ends_with(&SENDER[2..]));
        return call(word(held), 870, "");
    }
    if data == "0x313ce567" {
        return call(word(if to == B3TR { 0 } else { 18 }), 302, "");
    }
    assert_eq!(
        caller,
        Some(SENDER),
        "a transfer is simulated from its sender"
    );
    if let Some(arguments) = data.strip_prefix("0xa9059cbb") {
        assert_eq!(clause["value"], "0x0");
        let amount = u128::from_str_radix(&arguments[64..], 16).unwrap();
        return if amount > held {
            call("0x".into(), 0, "execution reverted")
        } else {
            call(word(1), 28_326, "")
        };
    }
    assert_eq!(data, "0x");
    let value = crate::api::thor_rest::quantity(clause["value"].as_str().unwrap()).unwrap();
    if value > ledger.vet {
        call("0x".into(), 0, "insufficient balance for transfer")
    } else {
        call("0x".into(), 0, "")
    }
}

async fn thor(ledger: Arc<Mutex<Ledger>>) -> wiremock::MockServer {
    node_at(move |url, body| {
        let mut ledger = ledger.lock().unwrap();
        Some(match url.path() {
            "/blocks/0" => json!({"number": 0, "id": ledger.genesis}),
            "/blocks/best" => {
                let id = format!("0x{:08x}{}", ledger.best, &BEST_ID[10..]);
                let mut block = json!({"number": ledger.best, "id": id, "isTrunk": true});
                if let Some(fee) = ledger.base_fee {
                    block["baseFeePerGas"] = json!(format!("0x{fee:x}"));
                }
                block
            }
            "/fees/priority" => json!({"maxPriorityFeePerGas": "0x746a528800"}),
            "/accounts/*" | "/accounts/%2A" => {
                let caller = body["caller"].as_str();
                Value::Array(
                    body["clauses"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|clause| run(&ledger, clause, caller))
                        .collect(),
                )
            }
            "/transactions" => {
                ledger
                    .submitted
                    .push(body["raw"].as_str().unwrap().to_string());
                json!({"id": ledger.accepted.clone().unwrap()})
            }
            path if path == format!("/accounts/{SENDER}") => json!({
                "balance": format!("0x{:x}", ledger.vet),
                "energy": format!("0x{:x}", ledger.vtho),
                "hasCode": false,
            }),
            other => panic!("unexpected request {other}"),
        })
    })
    .await
}

async fn imported(ledger: &Arc<Mutex<Ledger>>) -> (Wallet, wiremock::MockServer) {
    let server = thor(Arc::clone(ledger)).await;
    let wallet = Wallet::import(Chain::Vechain, &server.uri(), Secret::Phrase(PHRASE)).await;
    assert_eq!(wallet.address, SENDER);
    (wallet, server)
}

fn prepared(stored: &StoredSend) -> &crate::send::vechain::PreparedVechainTransfer {
    match &stored.prepared {
        PreparedPayload::Vechain(prepared) => prepared,
        _ => panic!("not a VeChainThor transfer"),
    }
}

/// Sign and broadcast `built`, the node accepting it under the id it was
/// signed with, exactly once.
async fn sign_and_broadcast(
    wallet: &Wallet,
    ledger: &Arc<Mutex<Ledger>>,
    built: &crate::send::stages::SendArtifact,
) -> String {
    let signed = wallet.sign(built).await.unwrap();
    let id = signed.transaction_hash.clone().unwrap();
    assert!(id.starts_with("0x") && id.len() == 66);
    ledger.lock().unwrap().accepted = Some(id.clone());
    let broadcast = wallet.broadcast(&signed).await.unwrap();
    assert_eq!(broadcast.transaction_hash.as_deref(), Some(id.as_str()));
    assert!(
        broadcast
            .attempts
            .iter()
            .any(|attempt| attempt.outcome == SubmissionOutcome::Accepted)
    );
    let submitted = ledger.lock().unwrap().submitted.clone();
    assert_eq!(submitted.len(), 1);
    assert!(submitted[0].starts_with("0x51"));
    id
}

#[tokio::test]
async fn a_vet_transfer_is_priced_in_vtho_signed_and_accepted() {
    let ledger = Arc::new(Mutex::new(Ledger::default()));
    let (wallet, _server) = imported(&ledger).await;
    let built = wallet
        .build(wallet.request(RECIPIENT, "1234.5"))
        .await
        .unwrap();
    let stored = wallet
        .service
        .load_send_artifact(built.id.clone())
        .await
        .unwrap();
    let transfer = prepared(&stored);
    assert_eq!(transfer.chain_tag, 0x4a);
    assert_eq!(transfer.recipient, RECIPIENT.to_ascii_lowercase());
    assert_eq!(transfer.token, None);
    assert_eq!(transfer.amount, 1_234_500_000_000_000_000_000);
    // VET to an address with no code runs nothing: its intrinsic gas.
    assert_eq!(transfer.gas, 21_000);
    assert_eq!(transfer.max_priority_fee_per_gas, 500_000_000_000);
    assert_eq!(transfer.max_fee_per_gas, 20_500_000_000_000);
    assert_eq!(hex::encode(transfer.block_ref), &BEST_ID[2..18]);
    assert_eq!(transfer.expiration, 60);
    // 21,000 gas at the base fee and the tip.
    assert_eq!(stored.request.fee_amount.as_deref(), Some("0.2205"));
    assert_eq!(
        built.signing_payload_hex,
        hex::encode(transfer.unsigned().unwrap())
    );
    sign_and_broadcast(&wallet, &ledger, &built).await;
}

#[tokio::test]
async fn vtho_and_another_vip180_token_are_sent_with_a_transfer_clause() {
    let ledger = Arc::new(Mutex::new(Ledger::default()));
    let (wallet, _server) = imported(&ledger).await;
    let built = wallet
        .build(wallet.token_request(RECIPIENT, "1.5", ENERGY, 18))
        .await
        .unwrap();
    let stored = wallet
        .service
        .load_send_artifact(built.id.clone())
        .await
        .unwrap();
    let transfer = prepared(&stored);
    assert_eq!(transfer.token.as_deref(), Some(ENERGY));
    assert_eq!(transfer.amount, 1_500_000_000_000_000_000);
    let (to, value, data) = transfer.clause().unwrap();
    assert_eq!((to.as_str(), value), (ENERGY, 0));
    // Intrinsic gas, the simulation's 28,326 and the 15,000 margin.
    assert_eq!(
        transfer.gas,
        crate::send::vechain::intrinsic_gas(&data) + 28_326 + 15_000
    );
    sign_and_broadcast(&wallet, &ledger, &built).await;

    // A token Spectra's catalog does not list, at its own decimals.
    let ledger = Arc::new(Mutex::new(Ledger::default()));
    let (wallet, _server) = imported(&ledger).await;
    let built = wallet
        .build(wallet.token_request(RECIPIENT, "50", B3TR, 0))
        .await
        .unwrap();
    let stored = wallet
        .service
        .load_send_artifact(built.id.clone())
        .await
        .unwrap();
    assert_eq!(prepared(&stored).token.as_deref(), Some(B3TR));
    assert_eq!(prepared(&stored).amount, 50);
    assert_eq!(
        stored.request.token_standard,
        Some(crate::registry::TokenStandard::Vip180)
    );
    sign_and_broadcast(&wallet, &ledger, &built).await;
}

#[tokio::test]
async fn what_the_network_would_refuse_is_refused_before_signing() {
    let ledger = Arc::new(Mutex::new(Ledger::default()));
    let (wallet, _server) = imported(&ledger).await;

    // More VET than the account holds: the simulation fails.
    assert!(
        refusal(wallet.build(wallet.request(RECIPIENT, "2000.1")).await)
            .contains("insufficient balance")
    );
    // More of a token than it holds.
    assert!(
        wallet
            .build(wallet.token_request(RECIPIENT, "51", B3TR, 0))
            .await
            .is_err()
    );
    // VTHO for the amount and the fee together.
    assert!(
        refusal(
            wallet
                .build(wallet.token_request(RECIPIENT, "2.9", ENERGY, 18))
                .await
        )
        .contains("amount plus the network fee")
    );
    // Decimals that are not the token's own.
    assert!(
        wallet
            .build(wallet.token_request(RECIPIENT, "1", ENERGY, 6))
            .await
            .is_err()
    );
    // A memo, which VeChainThor transfers do not carry.
    let mut memo = wallet.request(RECIPIENT, "1");
    memo.memo = Some(crate::send::payment_memo::PaymentMemo {
        kind: crate::registry::PaymentMemoKind::MemoText,
        value: "deposit".into(),
    });
    assert!(wallet.build(memo).await.is_err());
    // A recipient with a checksum letter in the wrong case.
    assert!(
        wallet
            .build(wallet.request(&RECIPIENT.replacen('D', "d", 1), "1"))
            .await
            .is_err()
    );

    // VET is there; the VTHO its gas needs is not.
    ledger.lock().unwrap().vtho = VTHO / 10;
    assert!(
        refusal(wallet.build(wallet.request(RECIPIENT, "1")).await)
            .contains("Insufficient VTHO for the VeChain network fee")
    );
    assert!(wallet.built_nothing().await);

    // The VTHO is spent between review and signing.
    ledger.lock().unwrap().vtho = 3 * VTHO;
    let built = wallet.build(wallet.request(RECIPIENT, "1")).await.unwrap();
    ledger.lock().unwrap().vtho = 0;
    assert!(refusal(wallet.sign(&built).await).contains("VTHO"));
    ledger.lock().unwrap().vtho = 3 * VTHO;

    // The chain passes the transaction's expiration before signing.
    ledger.lock().unwrap().best += 60;
    assert!(refusal(wallet.sign(&built).await).contains("expired"));
}

#[tokio::test]
async fn another_network_or_a_block_without_a_base_fee_is_refused() {
    let ledger = Arc::new(Mutex::new(Ledger {
        // VeChainThor's testnet.
        genesis: "0x000000000b2bce3c70bc649a02749e8687721b09ed2e15997f466536b20bb127",
        ..Ledger::default()
    }));
    let (wallet, _server) = imported(&ledger).await;
    assert!(wallet.build(wallet.request(RECIPIENT, "1")).await.is_err());

    let ledger = Arc::new(Mutex::new(Ledger {
        base_fee: None,
        ..Ledger::default()
    }));
    let (wallet, _server) = imported(&ledger).await;
    assert!(refusal(wallet.build(wallet.request(RECIPIENT, "1")).await).contains("base fee"));
    assert!(wallet.built_nothing().await);
}

#[tokio::test]
async fn a_quote_prices_vet_and_vtho_in_vtho() {
    let ledger = Arc::new(Mutex::new(Ledger::default()));
    let (wallet, _server) = imported(&ledger).await;
    let vet = wallet
        .service
        .preview_vechain_send(Chain::Vechain, SENDER, RECIPIENT, None, 1)
        .await
        .unwrap();
    assert_eq!(vet.estimated_network_fee, "0.2205");
    // The fee is VTHO's, so all the VET can go.
    assert_eq!(vet.spendable_balance, "2000");
    assert_eq!(vet.max_sendable, "2000");
    let vtho = wallet
        .service
        .preview_vechain_send(Chain::Vechain, SENDER, RECIPIENT, Some((ENERGY, 18)), 1)
        .await
        .unwrap();
    assert_eq!(vtho.spendable_balance, "3");
    let fee: u128 = crate::amount::decimal::to_units(&vtho.estimated_network_fee, 18).unwrap();
    assert_eq!(
        crate::amount::decimal::to_units(&vtho.max_sendable, 18),
        Some(3 * VTHO - fee)
    );
    // A token whose transfer the balance cannot run is priced as a typical
    // one.
    ledger.lock().unwrap().vtho = 0;
    let empty = wallet
        .service
        .preview_vechain_send(Chain::Vechain, SENDER, RECIPIENT, Some((ENERGY, 18)), VTHO)
        .await
        .unwrap();
    assert_eq!(empty.max_sendable, "0");
    assert!(!crate::amount::decimal::is_zero(
        &empty.estimated_network_fee
    ));
}
