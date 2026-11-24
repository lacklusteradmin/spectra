use super::*;
use std::sync::Arc;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::body_json};

#[tokio::test]
async fn account_sequence_refuses_overflow_instead_of_wrapping() {
    for sequence in [json!(u32::MAX), json!(u64::from(u32::MAX) + 1), json!(-1)] {
        let server = MockServer::start().await;
        Mock::given(body_json(json!({"method":"account_info", "params":[{
            "account":"sender", "ledger_index":"current"
        }]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "result":{"account_data":{"Sequence":sequence}}
        })))
        .expect(1)
        .mount(&server)
        .await;
        let result = XrplClient::new(Arc::new(vec![server.uri()]))
            .fetch_sequence("sender")
            .await;
        if sequence == json!(u32::MAX) {
            assert_eq!(result.unwrap(), u32::MAX);
        } else {
            assert!(matches!(result, Err(ApiError::Decode(_))), "{sequence}");
        }
    }
}

#[tokio::test]
async fn submit_distinguishes_accepted_payments_from_refusal_and_malformed_responses() {
    for (result, expected) in [
        (
            json!({"accepted":true,"engine_result":"tesSUCCESS","tx_json":{"hash":"ABC"}}),
            "accepted",
        ),
        (
            json!({"accepted":true,"engine_result":"terQUEUED","tx_json":{"hash":"ABC"}}),
            "accepted",
        ),
        (
            json!({"accepted":false,"engine_result":"tesSUCCESS","tx_json":{"hash":"ABC"}}),
            "rejected",
        ),
        (
            json!({"accepted":false,"engine_result":"temMALFORMED","engine_result_message":"Malformed transaction","tx_json":{"hash":"ABC"}}),
            "rejected",
        ),
        (
            json!({"accepted":true,"engine_result":"tecUNFUNDED_PAYMENT","tx_json":{"hash":"ABC"}}),
            "rejected",
        ),
        (
            json!({"accepted":false,"engine_result":"tefPAST_SEQ","tx_json":{"hash":"ABC"}}),
            "rejected",
        ),
        (
            json!({"accepted":false,"engine_result":"terPRE_SEQ","tx_json":{"hash":"ABC"}}),
            "rejected",
        ),
        (
            json!({"accepted":true,"engine_result":"tesSUCCESS","tx_json":{"hash":""}}),
            "decode",
        ),
        (json!({"accepted":true,"tx_json":{"hash":"ABC"}}), "decode"),
        (
            json!({"engine_result":"tesSUCCESS","tx_json":{"hash":"ABC"}}),
            "decode",
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(body_json(
            json!({"method":"submit","params":[{"tx_blob":"signed"}]}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result":result})))
        .expect(1)
        .mount(&server)
        .await;
        let submission = XrplClient::new(Arc::new(vec![server.uri()]))
            .submit_signed_blob("signed")
            .await;
        match expected {
            "accepted" => {
                assert_eq!(submission.unwrap(), "ABC");
            }
            "rejected" => assert!(
                matches!(submission, Err(ApiError::Rejected(_))),
                "{submission:?}"
            ),
            "decode" => assert!(
                matches!(submission, Err(ApiError::Decode(_))),
                "{submission:?}"
            ),
            _ => unreachable!(),
        }
    }
}

mod transaction_status_tests {
    use super::*;
    use crate::api::transaction_status::TransactionStatus;
    use std::sync::Arc;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method},
    };

    #[tokio::test]
    async fn hash_lookup_requires_validation_and_retains_committed_failure() {
        let hash = "AB".repeat(32);
        for (result, expected) in [
            (
                json!({"status":"error","error":"txnNotFound"}),
                TransactionStatus::Pending,
            ),
            (
                json!({"validated":false,"hash":hash,"meta":{"TransactionResult":"tesSUCCESS"}}),
                TransactionStatus::Pending,
            ),
            (
                json!({"validated":true,"hash":hash,"ledger_index":120,"meta":{"TransactionResult":"tesSUCCESS"}}),
                TransactionStatus::Confirmed {
                    succeeded: true,
                    block: Some(120),
                },
            ),
            (
                json!({"validated":true,"hash":hash.to_lowercase(),"ledger_index":121,"meta":{"TransactionResult":"tecUNFUNDED_PAYMENT"}}),
                TransactionStatus::Confirmed {
                    succeeded: false,
                    block: Some(121),
                },
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(body_json(
                    json!({"method":"tx","params":[{"transaction":hash,"binary":false}]}),
                ))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result":result})))
                .expect(1)
                .mount(&server)
                .await;
            assert_eq!(
                XrplClient::new(Arc::new(vec![server.uri()]))
                    .fetch_transaction_status(&hash)
                    .await
                    .unwrap(),
                expected
            );
        }
        for result in [
            json!({"validated":true,"hash":"CD".repeat(32),"ledger_index":120,"meta":{"TransactionResult":"tesSUCCESS"}}),
            json!({"validated":true,"hash":hash,"ledger_index":120}),
            json!({"validated":true,"hash":hash,"meta":{"TransactionResult":"tesSUCCESS"}}),
            json!({"validated":true,"hash":hash,"ledger_index":120,"meta":{"TransactionResult":"tefPAST_SEQ"}}),
            json!({"status":"error","error":"invalidParams"}),
        ] {
            assert!(xrp_transaction_status(json!({"result":result}), &hash).is_err());
        }
    }

    #[tokio::test]
    async fn server_network_identity_refuses_wrong_and_unknown_networks() {
        use crate::registry::Chain;
        for (actual, selected, expected_ok) in [
            (json!(0), Chain::Xrp, true),
            (json!(1), Chain::XrpTestnet, true),
            (json!(1), Chain::Xrp, false),
            (Value::Null, Chain::Xrp, false),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(body_json(json!({"method":"server_info","params":[{}]})))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"result":{"info":{"network_id":actual}}})),
                )
                .expect(1)
                .mount(&server)
                .await;
            assert_eq!(
                XrplClient::new(Arc::new(vec![server.uri()]))
                    .verify_network(selected)
                    .await
                    .is_ok(),
                expected_ok
            );
        }
    }
}

mod history_tests {
    use super::*;

    const ME: &str = "rMeMeMeMeMeMeMeMeMeMeMeMeMeMeMeMe1";
    const THEM: &str = "rThemThemThemThemThemThemThemThem2";
    const ISSUER: &str = "rhub8VRN55s94qWKDv6jmDy1pUykJzF3wq";

    fn account_root(account: &str, before: &str, after: &str) -> Value {
        json!({"ModifiedNode": {
            "LedgerEntryType": "AccountRoot",
            "FinalFields": {"Account": account, "Balance": after},
            "PreviousFields": {"Balance": before}
        }})
    }

    fn payment(
        hash: &str,
        from: &str,
        to: &str,
        amount: Value,
        result: &str,
        nodes: Value,
    ) -> Value {
        json!({
            "tx": {
                "TransactionType": "Payment", "hash": hash, "Account": from,
                "Destination": to, "Amount": amount, "Fee": "12", "date": 800_000_000u64
            },
            "meta": {"TransactionResult": result, "AffectedNodes": nodes}
        })
    }

    #[test]
    fn amounts_are_the_accounts_own_balance_change() {
        let txs = [
            // An outgoing payment: the fee is not part of the transfer.
            payment(
                "sent",
                ME,
                THEM,
                json!("5000000"),
                "tesSUCCESS",
                json!([
                    account_root(ME, "20000000", "14999988"),
                    account_root(THEM, "0", "5000000")
                ]),
            ),
            // A partial payment claims 1,000 XRP and delivers 1.
            payment(
                "partial",
                THEM,
                ME,
                json!("1000000000"),
                "tesSUCCESS",
                json!([account_root(ME, "14999988", "15999988")]),
            ),
            // An issued currency moves no XRP for this account: its trust
            // line, where it is the high side, gains 5 USD.
            payment(
                "iou",
                THEM,
                ME,
                json!({"currency": "USD", "issuer": ISSUER, "value": "5"}),
                "tesSUCCESS",
                json!([
                    account_root(THEM, "100", "88"),
                    ripple_state(ISSUER, ME, "USD", "-1.5", "-6.5"),
                    ripple_state(THEM, ISSUER, "USD", "10", "5")
                ]),
            ),
            // A line created by the payment, where this account is the low side.
            payment(
                "first",
                THEM,
                ME,
                json!({"currency": "534F4C4F00000000000000000000000000000000", "issuer": ISSUER, "value": "1e-20"}),
                "tesSUCCESS",
                json!([{"CreatedNode": {"LedgerEntryType": "RippleState", "NewFields": {
                    "Balance": {"currency": "534F4C4F00000000000000000000000000000000", "issuer": "rrrrrrrrrrrrrrrrrrrrBZbvji", "value": "1e-20"},
                    "LowLimit": {"issuer": ME}, "HighLimit": {"issuer": ISSUER}}}}]),
            ),
            // A failed payment only burned its fee.
            payment(
                "failed",
                ME,
                THEM,
                json!("5000000"),
                "tecUNFUNDED_PAYMENT",
                json!([account_root(ME, "15999988", "15999976")]),
            ),
        ];
        let entries = xrp_history_from_transactions(&txs, ME).unwrap();
        assert_eq!(entries.len(), 4, "{entries:?}");
        assert_eq!(entries[0].txid, "sent");
        assert!(!entries[0].is_incoming);
        assert_eq!(entries[0].amount_drops, 5_000_000);
        assert_eq!(entries[1].txid, "partial");
        assert!(entries[1].is_incoming);
        assert_eq!(entries[1].amount_drops, 1_000_000);
        // Only this account's line counts, read from its own side.
        assert_eq!(entries[2].txid, "iou");
        assert!(entries[2].is_incoming);
        assert_eq!(
            entries[2].contract.as_deref(),
            Some(format!("USD.{ISSUER}").as_str())
        );
        assert_eq!(entries[2].amount_display.as_deref(), Some("5"));
        assert_eq!(
            entries[3].contract.as_deref(),
            Some(format!("534F4C4F00000000000000000000000000000000.{ISSUER}").as_str())
        );
        assert_eq!(
            entries[3].amount_display.as_deref(),
            Some("0.00000000000000000001")
        );
        // XRP rows name no contract when serialized, as the normalizer reads them.
        let json = serde_json::to_value(&entries[0]).unwrap();
        assert!(json.get("contract").is_none() && json.get("amount_display").is_none());
    }

    fn ripple_state(low: &str, high: &str, currency: &str, before: &str, after: &str) -> Value {
        let balance = |value: &str| json!({"currency": currency, "issuer": "rrrrrrrrrrrrrrrrrrrrBZbvji", "value": value});
        json!({"ModifiedNode": {
            "LedgerEntryType": "RippleState",
            "FinalFields": {"Balance": balance(after), "LowLimit": {"issuer": low}, "HighLimit": {"issuer": high}},
            "PreviousFields": {"Balance": balance(before)}
        }})
    }
}
