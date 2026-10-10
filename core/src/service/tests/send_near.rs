//! A NEAR send — native or NEP-141 — is quoted at the protocol's real
//! prepayment, built only when the account can pay it with its full-access
//! key, registers a recipient the token has not (NEP-145) at a reviewed
//! deposit, and is signed only while all of that still holds.
use super::*;
use crate::send::stages::SendStage;
use crate::service::send_stage_protocols::send_stage_support::{Secret, Wallet, node, refusal};
use serde_json::{Value, json};
use std::sync::Mutex;

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const NEAR: u128 = 1_000_000_000_000_000_000_000_000;
const CONTRACT: &str = "token.near";
const REGISTRATION: u128 = 1_250_000_000_000_000_000_000;
/// The token's raw amount `123.456789` sends at six places.
const TOKEN_AMOUNT: &str = "123456789";
/// The storage stake of the account's 1000 bytes, at 10^19 yoctoNEAR a byte.
const STORAGE_STAKE: u128 = 10_000_000_000_000_000_000_000;

#[derive(Clone, Copy, PartialEq)]
enum Nep145 {
    Registered,
    Unregistered,
    /// The contract answers no storage query at all.
    Absent,
}

/// What the NEAR node reads: the account, its key, the token and the gas
/// price.
struct Account {
    balance: u128,
    nonce: u64,
    full_access: bool,
    token_balance: u128,
    decimals: u8,
    gas_price: u128,
    nep145: Nep145,
}

impl Default for Account {
    fn default() -> Self {
        Self {
            balance: 100 * NEAR,
            nonce: 7,
            full_access: true,
            token_balance: 223_456_789,
            decimals: 6,
            gas_price: 100_000_000,
            nep145: Nep145::Registered,
        }
    }
}

fn protocol() -> Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/near-staking-fee-protocol86.json"
    ))
    .unwrap()
}

fn b58(byte: u8) -> String {
    bs58::encode([byte; 32]).into_string()
}

async fn near(account: Arc<Mutex<Account>>) -> wiremock::MockServer {
    node(move |_, body| {
        let live = account.lock().unwrap();
        let params = &body["params"];
        let view = |value: Value| {
            json!({"result": serde_json::to_vec(&value).unwrap(), "logs": [], "block_height": 100})
        };
        let result = match body["method"].as_str().unwrap() {
            "status" => json!({"chain_id": "mainnet"}),
            "block" if params["finality"] == "optimistic" => {
                json!({"header": {"hash": b58(0x66), "height": 10_001}})
            }
            "block" => json!({"header": {"hash": b58(0x33), "height": 10_000}}),
            "gas_price" => json!({"gas_price": live.gas_price.to_string()}),
            "EXPERIMENTAL_protocol_config" => {
                let mut config = protocol();
                config["transaction_validity_period"] = json!(86_400);
                config["runtime_config"]["storage_amount_per_byte"] =
                    json!("10000000000000000000");
                config
            }
            "query" => match params["request_type"].as_str().unwrap() {
                "view_access_key" => json!({"nonce": live.nonce, "permission":
                    if live.full_access { json!("FullAccess") } else { json!({"FunctionCall": {}}) }}),
                "view_account" => json!({"amount": live.balance.to_string(), "locked": "0",
                    "storage_usage": 1_000}),
                "call_function" => {
                    use base64::Engine;
                    let args: Value = serde_json::from_slice(
                        &base64::engine::general_purpose::STANDARD
                            .decode(params["args_base64"].as_str().unwrap())
                            .unwrap(),
                    )
                    .unwrap();
                    match params["method_name"].as_str().unwrap() {
                        "ft_metadata" => view(json!({"spec": "ft-1.0.0", "name": "Fixture",
                            "symbol": "TEST", "decimals": live.decimals})),
                        "ft_balance_of" if args["account_id"] == "22".repeat(32).as_str() => {
                            view(json!("0"))
                        }
                        "ft_balance_of" => view(json!(live.token_balance.to_string())),
                        // nearcore's answer for a method the contract lacks.
                        "storage_balance_of" | "storage_balance_bounds"
                            if live.nep145 == Nep145::Absent =>
                        {
                            json!({"error": "wasm execution failed with error: \
                                FunctionCallError(MethodResolveError(MethodNotFound))",
                                "logs": [], "block_height": 100})
                        }
                        "storage_balance_of" if live.nep145 == Nep145::Registered => {
                            view(json!({"total": REGISTRATION.to_string(), "available": "0"}))
                        }
                        "storage_balance_of" => view(Value::Null),
                        "storage_balance_bounds" => view(json!({"min": REGISTRATION.to_string(),
                            "max": REGISTRATION.to_string()})),
                        other => panic!("unexpected view {other}"),
                    }
                }
                other => panic!("unexpected query {other}"),
            },
            other => panic!("unexpected NEAR request {other}"),
        };
        Some(json!({"jsonrpc": "2.0", "id": body["id"], "result": result}))
    })
    .await
}

async fn wallet() -> (Wallet, wiremock::MockServer, Arc<Mutex<Account>>) {
    let account = Arc::new(Mutex::new(Account::default()));
    let server = near(account.clone()).await;
    let wallet = Wallet::import(Chain::Near, &server.uri(), Secret::Phrase(PHRASE)).await;
    wallet.use_only("near-json-rpc", &server.uri()).await;
    (wallet, server, account)
}

fn receiver() -> String {
    "22".repeat(32)
}

fn native(wallet: &Wallet) -> crate::send::SendExecutionRequest {
    wallet.request(&receiver(), "1")
}

fn token(wallet: &Wallet) -> crate::send::SendExecutionRequest {
    wallet.token_request(&receiver(), "123.456789", CONTRACT, 6)
}

/// The prepayment the protocol charges for the token's calls, from the
/// protocol's own cost table: `storage_deposit` first when the recipient
/// needs registering, then `ft_transfer`, each with 30 Tgas, and the one
/// yoctoNEAR `ft_transfer` attaches.
fn token_budget(register: bool) -> u128 {
    let config = protocol();
    let costs = &config["runtime_config"]["transaction_costs"];
    let actions = &costs["action_creation_config"];
    let number = |value: &Value| {
        value
            .as_u64()
            .map(u128::from)
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
            .unwrap()
    };
    let mut calls = vec![(
        "ft_transfer",
        json!({"amount": TOKEN_AMOUNT, "receiver_id": receiver()}),
    )];
    if register {
        calls.insert(
            0,
            (
                "storage_deposit",
                json!({"account_id": receiver(), "registration_only": true}),
            ),
        );
    }
    let receipt = &costs["action_receipt_creation_config"];
    let (mut burnt, mut remaining) = (
        number(&receipt["send_not_sir"]),
        number(&receipt["execution"]),
    );
    for (method, args) in calls {
        let bytes = (method.len() + serde_json::to_string(&args).unwrap().len()) as u128;
        let call = &actions["function_call_cost"];
        let per_byte = &actions["function_call_cost_per_byte"];
        burnt += number(&call["send_not_sir"]) + number(&per_byte["send_not_sir"]) * bytes;
        remaining += number(&call["execution"])
            + number(&per_byte["execution"]) * bytes
            + 30_000_000_000_000;
    }
    let price = 100_000_000u128;
    let purchase = price.max(number(&config["runtime_config"]["min_gas_purchase_price"]));
    burnt * price + remaining * purchase + 1
}

fn prepared(artifact: &crate::send::stages::SendArtifact) -> Value {
    serde_json::from_str::<Value>(&artifact.prepared_details).unwrap()["Near"].clone()
}

/// The token's preview quotes the prepayment its calls cost, and the build
/// reviews and stores that same budget.
#[tokio::test]
async fn a_near_token_send_reviews_the_prepayment_its_calls_cost() {
    let (wallet, _server, _) = wallet().await;
    let preview = wallet
        .service
        .preview_near_send(
            Chain::Near,
            &wallet.address,
            &receiver(),
            123_456_789,
            Some(CONTRACT),
        )
        .await
        .unwrap();
    let budget = token_budget(false);
    assert_eq!(preview.feeBudgetYoctoNear, budget.to_string());
    let built = wallet.build(token(&wallet)).await.unwrap();
    let near = prepared(&built);
    assert_eq!(near["fee_budget"], budget.to_string());
    assert_eq!(near["registration_deposit"], Value::Null);
    assert_eq!(
        wallet
            .service
            .load_send_artifact(built.id.clone())
            .await
            .unwrap()
            .request
            .fee_amount
            .as_deref(),
        Some(preview.estimatedNetworkFee.as_str())
    );
    assert_eq!(built.review.transfer_terms, None);
}

/// Each refusal names its reason and builds nothing: NEAR that covers the
/// amount but not the protocol's real prepayment, a key that is not the
/// account's full-access key, a token whose precision is not the reviewed
/// one, and a token balance short of the amount.
#[tokio::test]
async fn a_near_send_the_account_cannot_pay_or_sign_is_never_built() {
    let (wallet, _server, account) = wallet().await;
    type Change = fn(&mut Account);
    let cases: [(
        fn(&Wallet) -> crate::send::SendExecutionRequest,
        Change,
        &str,
    ); 5] = [
        // One NEAR, the storage stake and an old flat 0.001 fee.
        (
            native,
            |a| a.balance = NEAR + STORAGE_STAKE + NEAR / 1_000,
            "Insufficient spendable NEAR for amount, protocol fee and storage stake",
        ),
        (
            token,
            |a| a.balance = STORAGE_STAKE + NEAR / 1_000,
            "Insufficient spendable NEAR for amount, protocol fee and storage stake",
        ),
        (
            native,
            |a| a.full_access = false,
            "NEAR requires the wallet's full-access key",
        ),
        (token, |a| a.decimals = 7, "Token decimals changed"),
        (
            token,
            |a| a.token_balance = 0,
            "Insufficient NEP-141 balance",
        ),
    ];
    for (request, change, words) in cases {
        *account.lock().unwrap() = Account::default();
        change(&mut account.lock().unwrap());
        let error = refusal(wallet.build(request(&wallet)).await);
        assert!(error.contains(words), "{words}: {error}");
        assert!(wallet.built_nothing().await, "{words}");
    }
}

/// A recipient the token has not registered gets `storage_deposit` at the
/// contract's minimum first: a deposit reviewed with the send, counted
/// against the balance, and read again at signing. A contract that answers
/// no NEP-145 query is refused, since nothing can say whether the recipient
/// can hold the token.
#[tokio::test]
async fn a_recipient_the_token_has_not_registered_is_registered_at_the_reviewed_deposit() {
    let (wallet, _server, account) = wallet().await;
    account.lock().unwrap().nep145 = Nep145::Absent;
    let error = refusal(wallet.build(token(&wallet)).await);
    assert!(error.contains("NEP-145"), "{error}");
    assert!(wallet.built_nothing().await);

    let budget = token_budget(true);
    {
        let mut account = account.lock().unwrap();
        account.nep145 = Nep145::Unregistered;
        // The fee and the storage stake, but not the deposit too.
        account.balance = budget + REGISTRATION - 1 + STORAGE_STAKE;
    }
    let error = refusal(wallet.build(token(&wallet)).await);
    assert!(error.contains("Insufficient spendable NEAR"), "{error}");
    assert!(wallet.built_nothing().await);

    account.lock().unwrap().balance = 100 * NEAR;
    let built = wallet.build(token(&wallet)).await.unwrap();
    let near = prepared(&built);
    assert_eq!(near["fee_budget"], budget.to_string());
    assert_eq!(near["registration_deposit"], REGISTRATION.to_string());
    assert_eq!(
        built
            .review
            .transfer_terms
            .as_ref()
            .and_then(|terms| terms.recipient_registration.as_deref()),
        Some("0.00125")
    );
    // Registered between the review and signing: a different transfer.
    account.lock().unwrap().nep145 = Nep145::Registered;
    let error = refusal(wallet.sign(&built).await);
    assert!(
        error.contains("registration with the token changed"),
        "{error}"
    );
    assert_eq!(wallet.stored(&built).await.stage, SendStage::Prepared);
    account.lock().unwrap().nep145 = Nep145::Unregistered;
    assert_eq!(wallet.sign(&built).await.unwrap().stage, SendStage::Signed);
}

/// Between review and signing the gas price may double, the balance go,
/// the key lose its full access or its nonce be used: each is refused and
/// leaves the send unsigned.
#[tokio::test]
async fn a_reviewed_near_send_is_refused_at_signing_when_the_account_changed() {
    let (wallet, _server, account) = wallet().await;
    let built = wallet.build(native(&wallet)).await.unwrap();
    type Change = fn(&mut Account);
    let cases: [(Change, &str); 4] = [
        (|a| a.gas_price *= 2, "NEAR fee exceeds reviewed budget"),
        (|a| a.balance = 0, "Insufficient spendable NEAR"),
        (
            |a| a.full_access = false,
            "NEAR requires the wallet's full-access key",
        ),
        (|a| a.nonce += 1, "NEAR access-key nonce changed"),
    ];
    for (change, words) in cases {
        *account.lock().unwrap() = Account::default();
        change(&mut account.lock().unwrap());
        let error = refusal(wallet.sign(&built).await);
        assert!(error.contains(words), "{words}: {error}");
        assert_eq!(wallet.stored(&built).await.stage, SendStage::Prepared);
    }
    *account.lock().unwrap() = Account::default();
    assert_eq!(wallet.sign(&built).await.unwrap().stage, SendStage::Signed);
}
