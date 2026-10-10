//! XRP Ledger issued currencies and Stellar credit assets are each their
//! code and issuer: a wallet's trust lines say which can come off and what
//! one locks, its balances stay apart by issuer, and an issuer's fee raised
//! between review and signing is a different payment.
use super::*;
use crate::send::stages::SendStage;
use crate::service::send_stage_protocols::send_stage_support::{Secret, Wallet, node, refusal};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;

fn vectors() -> Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/issued-assets.json")).unwrap()
}

const OTHER_ISSUER: &str = "rDsbeomae4FXwgQTJp9Rs64Qg9vDiTCdBv";
const DESTINATION: &str = "rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe";
const SOLO: &str = "534F4C4F00000000000000000000000000000000";
const OTHER_STELLAR_ISSUER: &str = "GBUXQE5RNV267EEVS6COJSHRKIE52GFVVA66TMM7UNAYLAOZP36PZ7YX";

/// The XRP Ledger as a node's validated ledger shows it: account roots and
/// each account's trust lines.
#[derive(Default)]
struct Ledger {
    roots: HashMap<String, Value>,
    lines: HashMap<String, Vec<Value>>,
}

fn root(sequence: u32) -> Value {
    json!({"Balance": "50000000", "Sequence": sequence, "OwnerCount": 1, "Flags": 0})
}

fn line(peer: &str, currency: &str, balance: &str, limit: &str, limit_peer: &str) -> Value {
    json!({"account": peer, "currency": currency, "balance": balance, "limit": limit,
        "limit_peer": limit_peer})
}

async fn xrpl(ledger: Arc<Mutex<Ledger>>) -> wiremock::MockServer {
    node(move |_, body| {
        let ledger = ledger.lock().unwrap();
        let params = &body["params"][0];
        let account = params["account"].as_str().unwrap_or_default();
        let missing = json!({"error": "actNotFound", "status": "error"});
        let result = match body["method"].as_str().unwrap() {
            "server_info" => json!({"info": {"network_id": 0}}),
            "server_state" => json!({"state": {"validated_ledger":
                {"seq": 1_000, "reserve_base": 1_000_000, "reserve_inc": 200_000}}}),
            "fee" => json!({"drops": {"open_ledger_fee": "12"}}),
            "account_info" => ledger
                .roots
                .get(account)
                .map_or(missing, |root| json!({"account_data": root})),
            "account_lines" if ledger.roots.contains_key(account) => {
                let peer = params["peer"].as_str();
                let lines: Vec<_> = ledger
                    .lines
                    .get(account)
                    .into_iter()
                    .flatten()
                    .filter(|line| peer.is_none_or(|peer| line["account"] == peer))
                    .collect();
                json!({"account": account, "lines": lines})
            }
            "account_lines" => missing,
            "account_objects" => json!({"account_objects": []}),
            other => panic!("unexpected XRPL request {other}"),
        };
        Some(json!({"result": result}))
    })
    .await
}

fn stellar_line(code: &str, issuer: &str, balance: &str, buying: &str) -> Value {
    json!({"balance": balance, "limit": "922337203685.4775807", "buying_liabilities": buying,
        "selling_liabilities": "0.0000000", "is_authorized": true,
        "asset_type": if code.len() <= 4 {"credit_alphanum4"} else {"credit_alphanum12"},
        "asset_code": code, "asset_issuer": issuer})
}

/// A Horizon on the test network where the wallet's account holds `lines`.
async fn horizon(lines: Vec<Value>) -> wiremock::MockServer {
    node(move |path, _| {
        Some(match path {
            "/" => json!({"network_passphrase": "Test SDF Network ; September 2015"}),
            "/ledgers" => json!({"_embedded": {"records":
                [{"sequence": 1_000, "base_reserve_in_stroops": 5_000_000}]}}),
            path if path.starts_with("/accounts/") => {
                let mut balances = lines.clone();
                balances.push(json!({"balance": "100.0000000", "asset_type": "native"}));
                json!({"balances": balances, "sequence": "123456789012",
                    "subentry_count": lines.len(), "num_sponsoring": 0, "num_sponsored": 0})
            }
            other => panic!("unexpected Horizon request {other}"),
        })
    })
    .await
}

async fn xrp_wallet(ledger: Arc<Mutex<Ledger>>) -> (Wallet, wiremock::MockServer) {
    let server = xrpl(ledger).await;
    let wallet = Wallet::import(
        Chain::Xrp,
        &server.uri(),
        Secret::Key(vectors()["xrpl"]["key"].as_str().unwrap()),
    )
    .await;
    (wallet, server)
}

async fn stellar_wallet(lines: Vec<Value>) -> (Wallet, wiremock::MockServer) {
    let server = horizon(lines).await;
    let wallet = Wallet::import(
        Chain::StellarTestnet,
        &server.uri(),
        Secret::Key(vectors()["stellar"]["seed"].as_str().unwrap()),
    )
    .await;
    (wallet, server)
}

fn blocked(lines: &WalletTrustLines) -> Vec<(&str, Option<&str>)> {
    lines
        .lines
        .iter()
        .map(|line| (line.asset.as_str(), line.removal_blocked.as_deref()))
        .collect()
}

/// A line comes off only empty, and on the XRP Ledger only when its issuer
/// extends no trust of its own; a line with neither a limit nor a balance
/// is no line. Each line locks one owner reserve.
#[tokio::test]
async fn an_xrp_trust_line_says_why_it_cannot_come_off_and_what_it_locks() {
    let issuer = vectors()["xrpl"]["issuer"].as_str().unwrap().to_string();
    let holder = vectors()["xrpl"]["account"].as_str().unwrap().to_string();
    let ledger = Arc::new(Mutex::new(Ledger::default()));
    {
        let mut ledger = ledger.lock().unwrap();
        ledger.roots.insert(holder.clone(), root(9));
        ledger.lines.insert(
            holder,
            vec![
                line(OTHER_ISSUER, "USD", "7.5", "1000", "0"),
                line(&issuer, "USD", "0", "1000", "5"),
                line(&issuer, SOLO, "0", "1000", "0"),
                line(OTHER_ISSUER, "EUR", "0", "0", "0"),
            ],
        );
    }
    let (wallet, _server) = xrp_wallet(ledger).await;
    let lines = wallet
        .service
        .wallet_trust_lines(wallet.id.clone())
        .await
        .unwrap();
    assert_eq!(
        blocked(&lines),
        [
            (
                format!("USD.{OTHER_ISSUER}").as_str(),
                Some("The trust line still holds the token")
            ),
            (
                format!("USD.{issuer}").as_str(),
                Some("The issuer extends its own trust on this line, so the line stays")
            ),
            (format!("{SOLO}.{issuer}").as_str(), None),
        ]
    );
    assert_eq!(lines.reserve_per_line, "0.2");
}

/// A Stellar trustline comes off only empty and with no open offer buying
/// its asset; each locks one base reserve.
#[tokio::test]
async fn a_stellar_trustline_says_why_it_cannot_come_off_and_what_it_locks() {
    let issuer = vectors()["stellar"]["issuer"].as_str().unwrap().to_string();
    let (wallet, _server) = stellar_wallet(vec![
        stellar_line("USDC", OTHER_STELLAR_ISSUER, "3.0000000", "0.0000000"),
        stellar_line("LONGASSET12", &issuer, "0.0000000", "1.0000000"),
        stellar_line("USDC", &issuer, "0.0000000", "0.0000000"),
    ])
    .await;
    let lines = wallet
        .service
        .wallet_trust_lines(wallet.id.clone())
        .await
        .unwrap();
    assert_eq!(
        blocked(&lines),
        [
            (
                format!("USDC:{OTHER_STELLAR_ISSUER}").as_str(),
                Some("The trust line still holds the token")
            ),
            (
                format!("LONGASSET12:{issuer}").as_str(),
                Some("Open offers still use this trustline")
            ),
            (format!("USDC:{issuer}").as_str(), None),
        ]
    );
    assert_eq!(lines.reserve_per_line, "0.5");
}

fn descriptor(contract: &str) -> TokenDescriptor {
    TokenDescriptor {
        standard: String::new(),
        contract: contract.into(),
        symbol: "USD".into(),
        decimals: 0,
        name: None,
    }
}

/// Discovered and refreshed alike, one code from two issuers is two
/// balances, each under its own issuer.
#[tokio::test]
async fn one_code_from_two_issuers_is_two_balances_found_and_refreshed() {
    let issuer = vectors()["xrpl"]["issuer"].as_str().unwrap().to_string();
    let holder = vectors()["xrpl"]["account"].as_str().unwrap().to_string();
    let ledger = Arc::new(Mutex::new(Ledger::default()));
    {
        let mut ledger = ledger.lock().unwrap();
        ledger.roots.insert(holder.clone(), root(9));
        ledger.lines.insert(
            holder,
            vec![
                line(OTHER_ISSUER, "USD", "7.5", "1000", "0"),
                line(&issuer, "USD", "150", "1000", "0"),
            ],
        );
    }
    let (xrp, _xrp_server) = xrp_wallet(ledger).await;
    let stellar_issuer = vectors()["stellar"]["issuer"].as_str().unwrap().to_string();
    let (stellar, _stellar_server) = stellar_wallet(vec![
        stellar_line("USDC", OTHER_STELLAR_ISSUER, "3.0000000", "0.0000000"),
        stellar_line("USDC", &stellar_issuer, "100.0000000", "0.0000000"),
    ])
    .await;
    for (wallet, ours, theirs) in [
        (
            &xrp,
            (format!("USD.{issuer}"), "150"),
            (format!("USD.{OTHER_ISSUER}"), "7.5"),
        ),
        (
            &stellar,
            (format!("USDC:{stellar_issuer}"), "100"),
            (format!("USDC:{OTHER_STELLAR_ISSUER}"), "3"),
        ),
    ] {
        let expected = [
            (ours.0.clone(), ours.1.to_string()),
            (theirs.0.clone(), theirs.1.to_string()),
        ];
        let read = |rows: Vec<TokenBalanceResult>| {
            let mut rows: Vec<_> = rows
                .into_iter()
                .map(|row| (row.contract_address, row.balance_display))
                .collect();
            rows.sort();
            rows
        };
        let mut sorted = expected.to_vec();
        sorted.sort();
        let discovered = wallet
            .service
            .discover_token_balances(wallet.chain, wallet.address.clone())
            .await
            .unwrap();
        assert_eq!(read(discovered), sorted, "{:?}", wallet.chain);
        let refreshed = wallet
            .service
            .known_token_balances(
                wallet.chain,
                wallet.address.clone(),
                vec![descriptor(&ours.0), descriptor(&theirs.0)],
            )
            .await
            .unwrap();
        assert_eq!(read(refreshed), sorted, "{:?}", wallet.chain);
    }
}

/// The issuer's transfer fee is part of the reviewed payment: raised before
/// signing, the payment costs more than was reviewed, and is refused.
#[tokio::test]
async fn an_issuer_fee_raised_after_review_refuses_signing() {
    let vectors = vectors();
    let issuer = vectors["xrpl"]["issuer"].as_str().unwrap().to_string();
    let holder = vectors["xrpl"]["account"].as_str().unwrap().to_string();
    let ledger = Arc::new(Mutex::new(Ledger::default()));
    {
        let mut ledger = ledger.lock().unwrap();
        ledger.roots.insert(holder.clone(), root(7));
        ledger.roots.insert(
            issuer.clone(),
            json!({"Balance": "100000000", "Sequence": 1, "OwnerCount": 9, "Flags": 0,
                "TransferRate": 1_002_000_000u32}),
        );
        ledger.roots.insert(DESTINATION.into(), root(1));
        ledger.lines.insert(
            holder,
            vec![line(&issuer, "USD", "150", "9999999999999999e80", "0")],
        );
        ledger.lines.insert(
            DESTINATION.into(),
            vec![line(&issuer, "USD", "10", "1000", "0")],
        );
    }
    let (wallet, _server) = xrp_wallet(ledger.clone()).await;
    let built = wallet
        .build(crate::send::SendExecutionRequest {
            contract_address: Some(format!("USD.{issuer}")),
            ..wallet.request(DESTINATION, "123.456")
        })
        .await
        .unwrap();
    let terms = built.review.transfer_terms.clone().unwrap();
    assert_eq!(
        (terms.debited.as_str(), terms.fee.as_str()),
        ("123.702912", "0.246912")
    );
    let set_rate = |rate: u32| {
        ledger.lock().unwrap().roots.get_mut(&issuer).unwrap()["TransferRate"] = json!(rate);
    };
    set_rate(1_005_000_000);
    let error = refusal(wallet.sign(&built).await);
    assert!(error.contains("issuer fee changed"), "{error}");
    let stored = wallet.stored(&built).await;
    assert_eq!(stored.stage, SendStage::Prepared);
    assert!(stored.signed_payload.is_none());
    // At the reviewed rate it is the payment that was reviewed.
    set_rate(1_002_000_000);
    assert_eq!(wallet.sign(&built).await.unwrap().stage, SendStage::Signed);
}
