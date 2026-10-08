use super::*;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const HOLDER: &str = "rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe";
const RECIPIENT: &str = "rDsbeomae4FXwgQTJp9Rs64Qg9vDiTCdBv";
const ISSUER: &str = "rhub8VRN55s94qWKDv6jmDy1pUykJzF3wq";

/// A validated ledger: account roots and each account's trust lines.
#[derive(Clone)]
struct Ledger {
    roots: std::collections::HashMap<&'static str, Value>,
    lines: std::collections::HashMap<&'static str, Vec<Value>>,
    deposit_authorized: bool,
}

fn root(balance: &str, owner_count: u64, flags: u64) -> Value {
    json!({"Balance": balance, "OwnerCount": owner_count, "Flags": flags, "Sequence": 42})
}

fn line(peer: &str, balance: &str, limit: &str) -> Value {
    json!({"account": peer, "balance": balance, "currency": "USD", "limit": limit, "limit_peer": "0"})
}

fn ledger() -> Ledger {
    Ledger {
        roots: [
            (HOLDER, root("50000000", 3, 0)),
            (RECIPIENT, root("20000000", 1, 0)),
            (ISSUER, root("100000000", 9, 0)),
        ]
        .into(),
        lines: [
            (HOLDER, vec![line(ISSUER, "100", "1000")]),
            (RECIPIENT, vec![line(ISSUER, "10", "1000")]),
        ]
        .into(),
        deposit_authorized: true,
    }
}

async fn node(ledger: Ledger) -> (MockServer, XrplClient) {
    let server = MockServer::start().await;
    let ledger = Arc::new(Mutex::new(ledger));
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let params = &body["params"][0];
            let ledger = ledger.lock().unwrap();
            let result = match body["method"].as_str().unwrap() {
                "server_info" => json!({"info": {"network_id": 0}}),
                "server_state" => json!({"state": {"validated_ledger": {
                    "seq": 1000, "reserve_base": 1_000_000, "reserve_inc": 200_000}}}),
                "account_info" => {
                    let account = params["account"].as_str().unwrap();
                    match ledger.roots.get(account) {
                        Some(root) => json!({"account_data": root}),
                        None => json!({"error": "actNotFound", "status": "error"}),
                    }
                }
                "account_lines" => {
                    let account = params["account"].as_str().unwrap();
                    let peer = params["peer"].as_str();
                    if !ledger.roots.contains_key(account) {
                        json!({"error": "actNotFound", "status": "error"})
                    } else {
                        let lines: Vec<Value> = ledger
                            .lines
                            .get(account)
                            .cloned()
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|line| peer.is_none_or(|peer| line["account"] == peer))
                            .collect();
                        json!({"account": account, "lines": lines})
                    }
                }
                "deposit_authorized" => json!({"deposit_authorized": ledger.deposit_authorized}),
                other => panic!("unexpected request {other}"),
            };
            ResponseTemplate::new(200).set_body_json(json!({"result": result}))
        })
        .mount(&server)
        .await;
    let client = XrplClient::new(Arc::new(vec![server.uri()]));
    (server, client)
}

async fn pay(
    ledger: Ledger,
    recipient: &str,
    amount: &str,
) -> Result<PreparedXrpIssuedPayment, SendError> {
    let (_server, client) = node(ledger).await;
    PreparedXrpIssuedPayment::plan(
        &client,
        crate::registry::Chain::Xrp,
        HOLDER,
        recipient,
        &format!("USD.{ISSUER}"),
        amount,
        12,
    )
    .await
}

fn set_line(ledger: &mut Ledger, account: &'static str, field: &str, value: Value) {
    ledger.lines.get_mut(account).unwrap()[0][field] = value;
}

/// A payment between two holders delivers its amount; an issuer's rate
/// makes the most it spends larger, rounded up, and reviews the difference.
#[tokio::test]
async fn payments_deliver_the_amount_and_spend_the_rate() {
    let plain = pay(ledger(), RECIPIENT, "12.5").await.unwrap();
    assert_eq!(
        (plain.amount.as_str(), plain.send_max.as_deref()),
        ("12.5", None)
    );
    assert_eq!((plain.sequence, plain.transfer_rate), (42, 1_000_000_000));
    assert!(plain.terms().is_none());
    let mut rated = ledger();
    rated.roots.get_mut(ISSUER).unwrap()["TransferRate"] = json!(1_002_000_000u64);
    let payment = pay(rated.clone(), RECIPIENT, "12.5").await.unwrap();
    assert_eq!(payment.send_max.as_deref(), Some("12.525"));
    let terms = payment.terms().unwrap();
    assert_eq!(
        (
            terms.debited.as_str(),
            terms.received.as_str(),
            terms.fee.as_str()
        ),
        ("12.525", "12.5", "0.025")
    );
    // Paying the issuer back takes no fee and needs no line of the issuer's.
    let redeem = pay(rated, ISSUER, "12.5").await.unwrap();
    assert_eq!(redeem.send_max, None);
    // The rate counts against the balance: 99.9 × 1.002 is past 100.
    let mut short = ledger();
    short.roots.get_mut(ISSUER).unwrap()["TransferRate"] = json!(1_002_000_000u64);
    let error = pay(short, RECIPIENT, "99.9").await.unwrap_err();
    assert!(matches!(error, SendError::InsufficientFunds(_)), "{error}");
}

/// Everything the network would refuse is refused before anything is built.
#[tokio::test]
async fn what_the_ledger_refuses_is_refused_first() {
    let mut cases: Vec<(Ledger, &str, &str, &str)> = Vec::new();
    let mut no_line = ledger();
    no_line.lines.insert(HOLDER, vec![]);
    cases.push((no_line, RECIPIENT, "1", "no trust line"));
    let mut frozen = ledger();
    set_line(&mut frozen, HOLDER, "freeze_peer", json!(true));
    cases.push((frozen, RECIPIENT, "1", "frozen"));
    let mut global = ledger();
    global.roots.get_mut(ISSUER).unwrap()["Flags"] = json!(0x0040_0000u64);
    cases.push((global, RECIPIENT, "1", "frozen"));
    let mut unauthorized = ledger();
    unauthorized.roots.get_mut(ISSUER).unwrap()["Flags"] = json!(0x0004_0000u64);
    cases.push((
        unauthorized.clone(),
        RECIPIENT,
        "1",
        "not authorized this wallet",
    ));
    set_line(&mut unauthorized, HOLDER, "peer_authorized", json!(true));
    cases.push((unauthorized, RECIPIENT, "1", "not authorized the recipient"));
    let mut missing = ledger();
    missing.roots.remove(RECIPIENT);
    cases.push((missing, RECIPIENT, "1", "does not exist"));
    let mut untrusting = ledger();
    untrusting.lines.insert(RECIPIENT, vec![]);
    cases.push((untrusting, RECIPIENT, "1", "recipient has no trust line"));
    cases.push((ledger(), RECIPIENT, "990.0000000000001", "no room"));
    let mut no_ripple = ledger();
    set_line(&mut no_ripple, HOLDER, "no_ripple_peer", json!(true));
    set_line(&mut no_ripple, RECIPIENT, "no_ripple_peer", json!(true));
    cases.push((no_ripple, RECIPIENT, "1", "move between holders"));
    let mut guarded = ledger();
    guarded.roots.get_mut(RECIPIENT).unwrap()["Flags"] = json!(0x0100_0000u64);
    guarded.deposit_authorized = false;
    cases.push((guarded, RECIPIENT, "1", "authorized"));
    cases.push((
        ledger(),
        RECIPIENT,
        "100.0000000000000001",
        "16 significant digits",
    ));
    cases.push((ledger(), RECIPIENT, "0", "positive"));
    cases.push((ledger(), HOLDER, "1", "another account"));
    let mut broke = ledger();
    broke.roots.get_mut(HOLDER).unwrap()["Balance"] = json!("11");
    cases.push((broke, RECIPIENT, "1", "network fee"));
    for (ledger, recipient, amount, words) in cases {
        let error = pay(ledger, recipient, amount)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(words), "{words}: {error}");
    }
    // One side's NoRipple alone does not stop the currency.
    let mut one_side = ledger();
    set_line(&mut one_side, HOLDER, "no_ripple_peer", json!(true));
    assert!(pay(one_side, RECIPIENT, "1").await.is_ok());
    // A frozen line can still pay its issuer back.
    let mut frozen = ledger();
    set_line(&mut frozen, HOLDER, "freeze_peer", json!(true));
    assert!(pay(frozen, ISSUER, "1").await.is_ok());
}

async fn trust(ledger: Ledger, remove: bool) -> Result<(PreparedXrpTrustSet, u64), SendError> {
    let (_server, client) = node(ledger).await;
    PreparedXrpTrustSet::plan(
        &client,
        crate::registry::Chain::Xrp,
        HOLDER,
        &format!("USD.{ISSUER}"),
        remove,
        12,
    )
    .await
}

/// A new line takes the largest limit and its reserve where the account
/// owns two objects or more; an empty line comes off with a limit of zero.
#[tokio::test]
async fn trust_lines_open_and_close_as_the_ledger_allows() {
    let mut fresh = ledger();
    fresh.lines.insert(HOLDER, vec![]);
    let (opened, reserve) = trust(fresh.clone(), false).await.unwrap();
    assert_eq!(reserve, 200_000);
    assert_eq!(
        opened.limit,
        IouValue::parse(MAX_TRUST_LIMIT).unwrap().to_decimal()
    );
    assert!(!opened.removes());
    // Three objects: the line needs the base and four increments.
    fresh.roots.get_mut(HOLDER).unwrap()["Balance"] = json!("1799999");
    let error = trust(fresh.clone(), false).await.unwrap_err();
    assert!(matches!(error, SendError::InsufficientFunds(_)), "{error}");
    // Under two objects the ledger waives the line's reserve.
    fresh.roots.get_mut(HOLDER).unwrap()["OwnerCount"] = json!(1);
    assert!(trust(fresh.clone(), false).await.is_ok());
    fresh.roots.get_mut(ISSUER).unwrap()["Flags"] = json!(0x2000_0000u64);
    assert!(
        trust(fresh, false)
            .await
            .unwrap_err()
            .to_string()
            .contains("no new trust lines")
    );
    assert!(
        trust(ledger(), false)
            .await
            .unwrap_err()
            .to_string()
            .contains("already trusts")
    );
    let mut missing = ledger();
    missing.roots.remove(ISSUER);
    assert!(
        trust(missing, false)
            .await
            .unwrap_err()
            .to_string()
            .contains("issuer does not exist")
    );

    assert!(
        trust(ledger(), true)
            .await
            .unwrap_err()
            .to_string()
            .contains("still holds")
    );
    let mut empty = ledger();
    set_line(&mut empty, HOLDER, "balance", json!("0"));
    let (removed, _) = trust(empty.clone(), true).await.unwrap();
    assert!(removed.removes() && removed.limit == "0");
    set_line(&mut empty, HOLDER, "limit_peer", json!("5"));
    assert!(
        trust(empty, true)
            .await
            .unwrap_err()
            .to_string()
            .contains("line stays")
    );
}
