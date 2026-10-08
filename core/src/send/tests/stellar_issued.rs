use super::*;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const HOLDER: &str = "GDVEU3DD4KOFECV66VIHWEZOYX4ZKR3WV27L464SIIPOU2IUI3JCZA57";
const RECIPIENT: &str = "GBUXQE5RNV267EEVS6COJSHRKIE52GFVVA66TMM7UNAYLAOZP36PZ7YX";
const ISSUER: &str = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN";
const PASSPHRASE: &str = "Test SDF Network ; September 2015";

fn usdc() -> String {
    format!("USDC:{ISSUER}")
}

fn trustline(balance: &str, limit: &str, authorized: bool) -> Value {
    json!({"balance": balance, "limit": limit, "buying_liabilities": "0.0000000",
           "selling_liabilities": "0.0000000", "is_authorized": authorized,
           "asset_type": "credit_alphanum4", "asset_code": "USDC", "asset_issuer": ISSUER})
}

fn account(native: &str, subentries: u64, lines: Vec<Value>) -> Value {
    let mut balances = lines;
    balances.push(json!({"balance": native, "asset_type": "native",
                         "buying_liabilities": "0.0000000", "selling_liabilities": "0.0000000"}));
    json!({"balances": balances, "sequence": "1000", "subentry_count": subentries,
           "num_sponsoring": 0, "num_sponsored": 0})
}

type Accounts = std::collections::HashMap<&'static str, Value>;

fn accounts() -> Accounts {
    [
        (
            HOLDER,
            account(
                "10.0000000",
                1,
                vec![trustline("100.0000000", "1000.0000000", true)],
            ),
        ),
        (
            RECIPIENT,
            account(
                "5.0000000",
                1,
                vec![trustline("10.0000000", "1000.0000000", true)],
            ),
        ),
        (ISSUER, account("50.0000000", 0, vec![])),
    ]
    .into()
}

async fn horizon(accounts: Accounts) -> (MockServer, HorizonClient) {
    let server = MockServer::start().await;
    let accounts = Arc::new(Mutex::new(accounts));
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let path = request.url.path();
            if path == "/" {
                return ResponseTemplate::new(200)
                    .set_body_json(json!({"network_passphrase": PASSPHRASE}));
            }
            if path == "/ledgers" {
                return ResponseTemplate::new(200).set_body_json(json!({"_embedded": {"records": [
                    {"sequence": 1000, "base_reserve_in_stroops": 5_000_000}]}}));
            }
            let id = path.strip_prefix("/accounts/").expect("an account read");
            match accounts.lock().unwrap().get(id) {
                Some(account) => ResponseTemplate::new(200).set_body_json(account),
                None => ResponseTemplate::new(404).set_body_json(json!({"status": 404})),
            }
        })
        .mount(&server)
        .await;
    let client = HorizonClient::new(Arc::new(vec![server.uri()]));
    (server, client)
}

async fn pay(
    accounts: Accounts,
    recipient: &str,
    stroops: i64,
) -> Result<PreparedStellarAssetPayment, SendError> {
    let (_server, client) = horizon(accounts).await;
    PreparedStellarAssetPayment::plan(
        &client,
        crate::registry::Chain::StellarTestnet,
        HOLDER,
        recipient,
        &usdc(),
        stroops,
        100,
    )
    .await
}

fn edit(accounts: &mut Accounts, id: &'static str, change: impl FnOnce(&mut Value)) {
    change(accounts.get_mut(id).unwrap());
}

#[tokio::test]
async fn asset_payments_check_both_trustlines_first() {
    let payment = pay(accounts(), RECIPIENT, 50_000_000).await.unwrap();
    assert_eq!(
        (payment.sequence, payment.amount_stroops),
        (1001, 50_000_000)
    );
    assert_eq!(payment.asset, usdc());
    // Paying the issuer needs no trustline of the issuer's.
    assert!(pay(accounts(), ISSUER, 1).await.is_ok());

    let mut cases: Vec<(Accounts, &str)> = Vec::new();
    let mut no_line = accounts();
    edit(&mut no_line, HOLDER, |a| {
        *a = account("10.0000000", 0, vec![])
    });
    cases.push((no_line, "no trustline for the asset"));
    let mut unauthorized = accounts();
    edit(&mut unauthorized, HOLDER, |a| {
        a["balances"][0]["is_authorized"] = json!(false)
    });
    cases.push((unauthorized, "not authorized this wallet"));
    let mut missing = accounts();
    missing.remove(RECIPIENT);
    cases.push((missing, "does not exist"));
    let mut untrusting = accounts();
    edit(&mut untrusting, RECIPIENT, |a| {
        *a = account("5.0000000", 0, vec![])
    });
    cases.push((untrusting, "recipient has no trustline"));
    let mut recipient_unauthorized = accounts();
    edit(&mut recipient_unauthorized, RECIPIENT, |a| {
        a["balances"][0]["is_authorized"] = json!(false)
    });
    cases.push((recipient_unauthorized, "not authorized the recipient"));
    let mut full = accounts();
    edit(&mut full, RECIPIENT, |a| {
        a["balances"][0]["limit"] = json!("10.0000001")
    });
    cases.push((full, "no room"));
    let mut committed = accounts();
    edit(&mut committed, HOLDER, |a| {
        a["balances"][0]["selling_liabilities"] = json!("96.0000000")
    });
    cases.push((committed, "Insufficient token balance"));
    // Two base reserves and one for the line are 1.5 XLM; the fee is on top.
    let mut reserved = accounts();
    edit(&mut reserved, HOLDER, |a| {
        a["balances"][1]["balance"] = json!("1.5000099")
    });
    cases.push((reserved, "network fee"));
    for (accounts, words) in cases {
        let error = pay(accounts, RECIPIENT, 50_000_000)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(words), "{words}: {error}");
    }
    assert!(
        pay(accounts(), HOLDER, 1)
            .await
            .unwrap_err()
            .to_string()
            .contains("another account")
    );
    assert!(pay(accounts(), RECIPIENT, 0).await.is_err());
}

async fn trust(
    accounts: Accounts,
    remove: bool,
) -> Result<(PreparedStellarChangeTrust, u64), SendError> {
    let (_server, client) = horizon(accounts).await;
    PreparedStellarChangeTrust::plan(
        &client,
        crate::registry::Chain::StellarTestnet,
        HOLDER,
        &usdc(),
        remove,
        100,
    )
    .await
}

#[tokio::test]
async fn trustlines_open_and_close_as_the_ledger_allows() {
    let mut fresh = accounts();
    edit(&mut fresh, HOLDER, |a| *a = account("2.0000100", 0, vec![]));
    let (opened, reserve) = trust(fresh.clone(), false).await.unwrap();
    assert_eq!(reserve, 5_000_000);
    assert_eq!(opened.limit_stroops, crate::send::stellar::MAX_TRUST_LIMIT);
    // Two reserves now and one for the line: 1.5 XLM and the fee.
    edit(&mut fresh, HOLDER, |a| {
        a["balances"][0]["balance"] = json!("1.5000099")
    });
    let error = trust(fresh.clone(), false).await.unwrap_err();
    assert!(matches!(error, SendError::InsufficientFunds(_)), "{error}");
    let mut missing = fresh.clone();
    edit(&mut missing, HOLDER, |a| {
        a["balances"][0]["balance"] = json!("9.0000000")
    });
    missing.remove(ISSUER);
    assert!(
        trust(missing, false)
            .await
            .unwrap_err()
            .to_string()
            .contains("issuer does not exist")
    );
    assert!(
        trust(accounts(), false)
            .await
            .unwrap_err()
            .to_string()
            .contains("already trusts")
    );

    assert!(
        trust(accounts(), true)
            .await
            .unwrap_err()
            .to_string()
            .contains("still holds")
    );
    let mut empty = accounts();
    edit(&mut empty, HOLDER, |a| {
        a["balances"][0]["balance"] = json!("0.0000000")
    });
    let (removed, _) = trust(empty.clone(), true).await.unwrap();
    assert!(removed.removes());
    edit(&mut empty, HOLDER, |a| {
        a["balances"][0]["buying_liabilities"] = json!("1.0000000")
    });
    assert!(
        trust(empty, true)
            .await
            .unwrap_err()
            .to_string()
            .contains("Open offers")
    );
}
