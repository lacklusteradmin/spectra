use super::*;
use std::sync::Arc;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

#[tokio::test]
async fn unavailable_tip_refuses_preparation_before_reading_inputs_or_broadcasting() {
    let server = MockServer::start().await;
    Mock::given(path("/api/v2/block-index/0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "blockHash": Chain::Zcash.zcash_genesis().unwrap()
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/api/v2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"backend": {}})))
        .expect(1)
        .mount(&server)
        .await;
    let client = BlockbookClient::new(Arc::new(vec![server.uri()]), Chain::Zcash);
    let mut payload = vec![0x1c, 0xb8];
    payload.extend([1; 20]);
    let address = bs58::encode(payload).with_check().into_string();

    let error = prepare_zcash(&client, &address, &address, 100_000, None)
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        SendError::Api(crate::api::error::ApiError::Decode(_))
    ));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| { matches!(request.url.path(), "/api/v2/block-index/0" | "/api/v2") })
    );
}
