use super::*;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The address whose history `unisat-fractal-answers.json` holds.
const HOLDER: &str = "bc1p26tlrfhgd84p7rxrtmt8gxxe06v4e92e2t3m7hnx36vhnzq8qs8qw9uykg";
/// The address whose available outputs it holds.
const HOLDER_OF_OUTPUTS: &str = "bc1qtx7rzkcs43xyg8r6wglqn2xsv3j5rhe0enwya7";

fn answer(name: &str) -> serde_json::Value {
    let answers: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/unisat-fractal-answers.json"
    ))
    .unwrap();
    answers[name].clone()
}

async fn unisat() -> (MockServer, UnisatOpenApiClient) {
    let server = MockServer::start().await;
    let client = UnisatOpenApiClient::new(Arc::new(vec![server.uri()]), Chain::FractalBitcoin);
    (server, client)
}

async fn serve(server: &MockServer, route: &str, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/indexer{route}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Fractal's block 1 is not Bitcoin's: the indexer names it.
#[tokio::test]
async fn the_indexer_names_fractals_block_one() {
    let (server, client) = unisat().await;
    serve(&server, "/height/1/block", answer("block1")).await;
    let (height, expected) = Chain::FractalBitcoin.identity_block().unwrap();
    assert_eq!(height, 1);
    assert_eq!(client.fetch_block_hash(1).await.unwrap(), expected);
    assert_ne!(
        expected,
        Chain::Bitcoin.identity_block().unwrap().1,
        "Bitcoin's block 1 differs"
    );
}

#[tokio::test]
async fn balance_reads_the_confirmed_and_pending_amounts() {
    let (server, client) = unisat().await;
    let mut balance = answer("balance");
    balance["data"]["satoshi"] = 88_343_892.into();
    balance["data"]["pendingSatoshi"] = (-1_092).into();
    serve(&server, &format!("/address/{HOLDER}/balance"), balance).await;
    let balance = client.fetch_balance(HOLDER).await.unwrap();
    assert_eq!(
        (balance.confirmed_sats, balance.unconfirmed_sats),
        (88_343_892, -1_092)
    );
}

/// Available outputs pay the address's own script; one carrying an
/// inscription, or already spent, is passed over; an output paying another
/// script is refused.
#[tokio::test]
async fn available_outputs_are_the_spendable_ones() {
    let (server, client) = unisat().await;
    let mut page = answer("utxo");
    page["data"]["total"] = 3.into();
    let mut inscribed = page["data"]["utxo"][0].clone();
    inscribed["vout"] = 7.into();
    inscribed["inscriptions"] = serde_json::json!([{"inscriptionId": "x"}]);
    page["data"]["utxo"].as_array_mut().unwrap().push(inscribed);
    Mock::given(method("GET"))
        .and(path(format!(
            "/v1/indexer/address/{HOLDER_OF_OUTPUTS}/available-utxo-data"
        )))
        .and(query_param("cursor", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page.clone()))
        .mount(&server)
        .await;
    let utxos = client.fetch_utxos(HOLDER_OF_OUTPUTS).await.unwrap();
    let read: Vec<_> = utxos
        .iter()
        .map(|utxo| {
            (
                utxo.txid.as_str(),
                utxo.vout,
                utxo.value,
                utxo.status.block_height,
            )
        })
        .collect();
    assert_eq!(
        read,
        [
            (
                "b53a069c3345ec557aff4cf94114253eb3b76849348221d195405e30ed81a84b",
                1,
                546,
                Some(2_031_615)
            ),
            (
                "85e0db5b327cc0b5a0f0259be7fe72970806415c6d4545532faf2bbea2c53f3d",
                1,
                546,
                Some(1_900_287)
            ),
        ]
    );

    let (server, client) = unisat().await;
    page["data"]["utxo"][0]["scriptPk"] = "0014aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();
    serve(
        &server,
        &format!("/address/{HOLDER_OF_OUTPUTS}/available-utxo-data"),
        page,
    )
    .await;
    assert!(client.fetch_utxos(HOLDER_OF_OUTPUTS).await.is_err());
}

/// The outputs are paged until the list ends, the last page sized to it.
#[tokio::test]
async fn outputs_are_paged_to_the_end() {
    let (server, client) = unisat().await;
    let template = answer("utxo")["data"]["utxo"][0].clone();
    let page = |cursor: usize, count: usize| {
        let utxo: Vec<_> = (cursor..cursor + count)
            .map(|vout| {
                let mut utxo = template.clone();
                utxo["vout"] = vout.into();
                utxo
            })
            .collect();
        serde_json::json!({"code": 0, "msg": "ok", "data": {"cursor": cursor, "total": 70, "utxo": utxo}})
    };
    for (cursor, size) in [(0, 64), (64, 6)] {
        Mock::given(method("GET"))
            .and(path(format!(
                "/v1/indexer/address/{HOLDER_OF_OUTPUTS}/available-utxo-data"
            )))
            .and(query_param("cursor", cursor.to_string()))
            .and(query_param("size", size.to_string()))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(cursor, size)))
            .expect(1)
            .mount(&server)
            .await;
    }
    assert_eq!(
        client.fetch_utxos(HOLDER_OF_OUTPUTS).await.unwrap().len(),
        70
    );
}

/// Each history row is the address's net change, read from the
/// transaction's inputs and outputs, and the transaction's fee.
#[tokio::test]
async fn history_reads_each_transactions_inputs_and_outputs() {
    let (server, client) = unisat().await;
    serve(
        &server,
        &format!("/address/{HOLDER}/history"),
        answer("history"),
    )
    .await;
    for txid in [
        "12dd8923baaefa934db1484477fa1487890c603d85c647ff191f69c5f9af85ab",
        "9fbd6d8880e0a9c0e88abc21a8bbcd04b894841a10bc9ff95737640bdcac8557",
    ] {
        for side in ["ins", "outs"] {
            serve(
                &server,
                &format!("/tx/{txid}/{side}"),
                answer(&format!("{side}-{txid}")),
            )
            .await;
        }
    }
    let page = client.fetch_history_page(HOLDER, None).await.unwrap();
    let rows: Vec<_> = page
        .items
        .iter()
        .map(|row| (row.txid.as_str(), row.net_sats, row.fee_sats, row.confirmed))
        .collect();
    assert_eq!(
        rows,
        [
            (
                "12dd8923baaefa934db1484477fa1487890c603d85c647ff191f69c5f9af85ab",
                -3_625,
                Some(3_078),
                true
            ),
            (
                "9fbd6d8880e0a9c0e88abc21a8bbcd04b894841a10bc9ff95737640bdcac8557",
                3_625,
                Some(2_325),
                true
            ),
        ]
    );
    assert_eq!(page.next_cursor, None);
}

#[tokio::test]
async fn a_transaction_status_counts_its_confirmations() {
    let (server, client) = unisat().await;
    let txid = "12dd8923baaefa934db1484477fa1487890c603d85c647ff191f69c5f9af85ab";
    serve(&server, &format!("/tx/{txid}"), answer("tx")).await;
    let status = client.fetch_tx_status(txid).await.unwrap();
    assert!(status.confirmed);
    assert_eq!(status.block_height, Some(2_322_887));
    assert_eq!(status.confirmations, Some(40));
}

/// A refusal is the indexer's message, not a success with no data.
#[tokio::test]
async fn a_nonzero_code_is_a_refusal() {
    let (server, client) = unisat().await;
    serve(
        &server,
        &format!("/address/{HOLDER}/balance"),
        serde_json::json!({"code": -1, "msg": "address invalid", "data": null}),
    )
    .await;
    match client.fetch_balance(HOLDER).await {
        Err(ApiError::Rejected(message)) => assert!(message.contains("address invalid")),
        other => panic!("{other:?}"),
    }
    // Another network's address is refused before any request.
    assert!(matches!(
        client
            .fetch_balance("ltc1qg82g4kq2fssdcd8v3cfsqyygqfwvlsyknzexhw")
            .await,
        Err(ApiError::InvalidInput(_))
    ));
}

#[tokio::test]
async fn a_broadcast_posts_the_hex_and_reads_the_txid() {
    let (server, client) = unisat().await;
    let txid = "ab".repeat(32);
    Mock::given(method("POST"))
        .and(path("/v1/indexer/local_pushtx"))
        .and(body_json(serde_json::json!({"txHex": "0102"})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"code": 0, "msg": "ok", "data": txid})),
        )
        .mount(&server)
        .await;
    assert_eq!(client.broadcast_raw_tx("0102").await.unwrap(), txid);

    let (refusing, client) = unisat().await;
    Mock::given(method("POST"))
        .and(path("/v1/indexer/local_pushtx"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"code": -1, "msg": "bad-txns-inputs-missingorspent", "data": null}),
        ))
        .mount(&refusing)
        .await;
    assert!(matches!(
        client.broadcast_raw_tx("0102").await,
        Err(ApiError::Rejected(_))
    ));
}
