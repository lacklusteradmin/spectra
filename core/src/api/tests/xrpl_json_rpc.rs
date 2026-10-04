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
                let submission = submission.unwrap();
                assert_eq!(submission.txid, "ABC");
                assert_eq!(submission.tx_blob_hex, "signed");
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
