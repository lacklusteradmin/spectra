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

/// Transparent funds pay a transparent address its own script and a TEX
/// address (ZIP-320's reference pair) the P2PKH script of the same key
/// hash; a shielded address is paid from shielded funds, and an address of
/// the other network is none here.
#[test]
fn transparent_funds_pay_transparent_and_tex_addresses() {
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/zcash-addresses.json")).unwrap();
    let pair = &vectors["tex"]["vectors"][0];
    let transparent = address_script(pair["transparent"].as_str().unwrap(), Chain::Zcash).unwrap();
    assert_eq!(transparent.len(), 25);
    assert_eq!(
        address_script(pair["tex"].as_str().unwrap(), Chain::Zcash).unwrap(),
        transparent
    );
    let mut payload = vec![0x1c, 0xbd];
    payload.extend([2; 20]);
    let p2sh = bs58::encode(payload).with_check().into_string();
    let mut expected = vec![0xa9, 0x14];
    expected.extend([2; 20]);
    expected.push(0x87);
    assert_eq!(address_script(&p2sh, Chain::Zcash).unwrap(), expected);

    let unified = vectors["unified"]["vectors"][0]["unified_addr"]
        .as_str()
        .unwrap();
    assert_eq!(
        address_script(unified, Chain::Zcash)
            .unwrap_err()
            .to_string(),
        "A shielded address is paid from the wallet's shielded funds."
    );
    for (address, chain) in [
        (pair["tex"].as_str().unwrap(), Chain::ZcashTestnet),
        (pair["transparent"].as_str().unwrap(), Chain::ZcashTestnet),
        ("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa", Chain::Zcash),
    ] {
        assert_eq!(
            address_script(address, chain).unwrap_err().to_string(),
            "Not a Zcash address on this network"
        );
    }
}
