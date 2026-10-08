//! Which contracts hold a deposit the account can have back, and what a
//! refund refuses, against a NEAR node that answers as each kind of
//! contract does.

use super::*;
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const ACCOUNT: &str = "alice.near";
const DEPOSIT: u128 = 1_250_000_000_000_000_000_000;

/// A node where each contract answers `storage_balance_of` and
/// `ft_balance_of` the way its name says.
async fn node() -> (MockServer, NearClient) {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let params = &body["params"];
            assert_eq!(params["request_type"], "call_function", "{body}");
            use base64::Engine;
            let args: Value = serde_json::from_slice(
                &base64::engine::general_purpose::STANDARD
                    .decode(params["args_base64"].as_str().unwrap())
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(args, json!({"account_id": ACCOUNT}));
            let registered = json!({"total": DEPOSIT.to_string(), "available": "0"});
            let answer = match (
                params["account_id"].as_str().unwrap(),
                params["method_name"].as_str().unwrap(),
            ) {
                ("gone.near", _) => {
                    return ResponseTemplate::new(200).set_body_json(json!({"jsonrpc": "2.0",
                        "id": body["id"], "error": {"name": "HANDLER_ERROR",
                        "cause": {"name": "UNKNOWN_ACCOUNT"}}}));
                }
                // nearcore's answer for a method the contract lacks.
                ("nostorage.near", "storage_balance_of") | ("notoken.near", "ft_balance_of") => {
                    return ResponseTemplate::new(200).set_body_json(json!({"jsonrpc": "2.0",
                        "id": body["id"], "result": {"error": "wasm execution failed with error: \
                        FunctionCallError(MethodResolveError(MethodNotFound))", "logs": [],
                        "block_height": 1}}));
                }
                ("unregistered.near", "storage_balance_of") => Value::Null,
                ("free.near", "storage_balance_of") => json!({"total": "0", "available": "0"}),
                (_, "storage_balance_of") => registered,
                ("held.near", "ft_balance_of") => json!("5"),
                (_, "ft_balance_of") => json!("0"),
                other => panic!("unexpected call {other:?}"),
            };
            ResponseTemplate::new(200).set_body_json(json!({"jsonrpc": "2.0", "id": body["id"],
                "result": {"result": serde_json::to_vec(&answer).unwrap(), "logs": [],
                "block_height": 1}}))
        })
        .mount(&server)
        .await;
    let client = NearClient::new(Arc::new(vec![server.uri()]));
    (server, client)
}

/// Listed: a registered contract whose token the account holds none of.
/// Passed over: one that holds no deposit, a zero one, one the account holds
/// tokens of, and one that answers no NEP-145 or NEP-141 query or does not
/// exist.
#[tokio::test]
async fn only_a_deposit_beside_an_empty_balance_is_refundable() {
    let (_server, node) = node().await;
    assert_eq!(
        refundable_deposit(&node, "empty.near", ACCOUNT)
            .await
            .unwrap(),
        Some(DEPOSIT)
    );
    for contract in [
        "held.near",
        "unregistered.near",
        "free.near",
        "nostorage.near",
        "notoken.near",
        "gone.near",
    ] {
        assert_eq!(
            refundable_deposit(&node, contract, ACCOUNT).await.unwrap(),
            None,
            "{contract}"
        );
    }
}

/// A refund is built only for a deposit beside an empty balance; each other
/// contract is refused with what stands in the way.
#[tokio::test]
async fn a_refund_refuses_held_tokens_and_missing_deposits() {
    let (_server, node) = node().await;
    assert_eq!(
        refund_preconditions(&node, "empty.near", ACCOUNT)
            .await
            .unwrap(),
        DEPOSIT
    );
    for (contract, words) in [
        ("held.near", "still holds this token"),
        ("notoken.near", "reports no token balance"),
        ("unregistered.near", "holds no storage deposit"),
        ("free.near", "holds no storage deposit"),
        ("nostorage.near", "holds no storage deposit"),
        ("gone.near", "holds no storage deposit"),
    ] {
        let error = refund_preconditions(&node, contract, ACCOUNT)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(words), "{contract}: {error}");
    }
}
