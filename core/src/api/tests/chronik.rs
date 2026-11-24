use super::*;
use wiremock::matchers::{body_bytes, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The address whose answers `chronik-answers.json` holds.
const HOLDER: &str = "ecash:qrrrkdwsgd9ntyvd4ah3kr37ccqvg9844quu47hc30";
const SCRIPT_PATH: &str = "/script/p2pkh/c63b35d0434b35918daf6f1b0e3ec600c414f5a8";

fn answer(name: &str) -> Vec<u8> {
    let answers: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/chronik-answers.json")).unwrap();
    hex::decode(answers[name].as_str().unwrap()).unwrap()
}

fn protobuf(bytes: Vec<u8>) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/x-protobuf")
        .set_body_bytes(bytes)
}

async fn chronik() -> (MockServer, ChronikClient) {
    let server = MockServer::start().await;
    let client = ChronikClient::new(Arc::new(vec![server.uri()]), Chain::Ecash);
    (server, client)
}

/// A server names eCash's block 896,800, after its split from Bitcoin Cash;
/// one naming another block there is refused.
#[tokio::test]
async fn a_server_proves_it_is_ecash_by_its_identity_block() {
    let (server, client) = chronik().await;
    Mock::given(method("GET"))
        .and(path("/block/896800"))
        .respond_with(protobuf(answer("block896800")))
        .mount(&server)
        .await;
    client.verify_network().await.unwrap();
    assert_eq!(
        client.fetch_block_hash(896_800).await.unwrap(),
        "0000000000000000297efb200794348b44bff4bfb31716cf64dc45bac0a251ea"
    );

    let (other, client) = chronik().await;
    let mut block = Block::decode(answer("block896800").as_slice()).unwrap();
    block.block_info.as_mut().unwrap().hash[0] ^= 1;
    Mock::given(method("GET"))
        .and(path("/block/896800"))
        .respond_with(protobuf(block.encode_to_vec()))
        .mount(&other)
        .await;
    assert!(client.verify_network().await.is_err());
}

#[tokio::test]
async fn the_tip_is_read_from_blockchain_info() {
    let (server, client) = chronik().await;
    Mock::given(method("GET"))
        .and(path("/blockchain-info"))
        .respond_with(protobuf(answer("info")))
        .mount(&server)
        .await;
    assert_eq!(client.fetch_tip_height().await.unwrap(), 976_956);
}

/// The balance counts every output the address holds, the eToken one too;
/// a send spends all but the one carrying an eToken, which it would burn.
#[tokio::test]
async fn outputs_carrying_an_etoken_count_but_are_not_spent() {
    let (server, client) = chronik().await;
    Mock::given(method("GET"))
        .and(path(format!("{SCRIPT_PATH}/utxos")))
        .respond_with(protobuf(answer("utxos")))
        .mount(&server)
        .await;
    let balance = client.fetch_balance(HOLDER).await.unwrap();
    assert_eq!(
        (balance.confirmed_sats, balance.unconfirmed_sats),
        (20_101_436_811, 0)
    );
    let utxos = client.fetch_utxos(HOLDER).await.unwrap();
    assert_eq!(utxos.len(), 15);
    assert_eq!(
        utxos.iter().map(|utxo| utxo.value).sum::<u64>(),
        20_101_436_811 - 546
    );
    assert!(!utxos.iter().any(
        |utxo| utxo.txid == "cee43bd4d03a841adb47d1f69ee5166baa30e527d15246b576e1f2c160f3c7fe"
    ));
    assert!(utxos.iter().all(|utxo| utxo.status.confirmed));
    // The same address, written without its prefix, is the same script.
    assert_eq!(
        client
            .fetch_utxos(HOLDER.trim_start_matches("ecash:"))
            .await
            .unwrap()
            .len(),
        15
    );
}

/// An answer for another script than the address pays is refused.
#[tokio::test]
async fn outputs_of_another_script_are_refused() {
    let (server, client) = chronik().await;
    let mut utxos = ScriptUtxos::decode(answer("utxos").as_slice()).unwrap();
    utxos.script[3] ^= 1;
    Mock::given(method("GET"))
        .and(path(format!("{SCRIPT_PATH}/utxos")))
        .respond_with(protobuf(utxos.encode_to_vec()))
        .mount(&server)
        .await;
    assert!(client.fetch_balance(HOLDER).await.is_err());
    assert!(client.fetch_utxos(HOLDER).await.is_err());
}

/// A coinbase output is spent only once it has matured: 100 blocks.
#[tokio::test]
async fn a_coinbase_output_waits_for_maturity() {
    let (server, client) = chronik().await;
    let utxo = |height: i32, coinbase: bool, vout: u32| ScriptUtxo {
        outpoint: Some(OutPoint {
            txid: vec![7; 32],
            out_idx: vout,
        }),
        block_height: height,
        is_coinbase: coinbase,
        sats: 1_000,
        token: None,
    };
    let answer = ScriptUtxos {
        script: hex::decode("76a914c63b35d0434b35918daf6f1b0e3ec600c414f5a888ac").unwrap(),
        utxos: vec![
            utxo(900, true, 0),
            utxo(901, true, 1),
            utxo(990, false, 2),
            utxo(-1, false, 3),
        ],
    };
    Mock::given(method("GET"))
        .and(path(format!("{SCRIPT_PATH}/utxos")))
        .respond_with(protobuf(answer.encode_to_vec()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/blockchain-info"))
        .respond_with(protobuf(
            BlockchainInfo {
                tip_hash: vec![0; 32],
                tip_height: 999,
            }
            .encode_to_vec(),
        ))
        .mount(&server)
        .await;
    let utxos = client.fetch_utxos(HOLDER).await.unwrap();
    let vouts: Vec<_> = utxos.iter().map(|utxo| utxo.vout).collect();
    assert_eq!(vouts, [0, 2, 3]);
    assert!(!utxos[2].status.confirmed);
    assert_eq!(utxos[2].status.block_height, None);
}

/// History is newest first, each row the address's net change and the
/// transaction's fee; the next page follows while there are pages.
#[tokio::test]
async fn history_reads_the_address_net_change() {
    let (server, client) = chronik().await;
    Mock::given(method("GET"))
        .and(path(format!("{SCRIPT_PATH}/history")))
        .and(query_param("page", "0"))
        .respond_with(protobuf(answer("history")))
        .mount(&server)
        .await;
    let page = client.fetch_history_page(HOLDER, None).await.unwrap();
    let rows: Vec<_> = page
        .items
        .iter()
        .map(|row| {
            (
                row.txid.as_str(),
                row.net_sats,
                row.fee_sats,
                row.block_height,
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (
                "0812731a85f60a876515c7d22659033c433a17f0bbd1a92315349556cd4f45a2",
                -101_244,
                Some(2_260),
                Some(976_950)
            ),
            (
                "1ccc0d170f00b492dec82ba3fb5a27edcf2e18c70d7e7a76f98239580c13dafb",
                397_498_080,
                Some(1_920),
                Some(976_918)
            ),
        ]
    );
    assert_eq!(page.items[0].block_time, Some(1_795_519_353));
    // The captured page was asked for two rows of 2,974 pages; the client
    // asks fifty, and goes on while the server says there are more.
    assert_eq!(page.next_cursor.as_deref(), Some("2"));
}

#[tokio::test]
async fn a_transaction_status_names_its_block() {
    let (server, client) = chronik().await;
    let txid = "0812731a85f60a876515c7d22659033c433a17f0bbd1a92315349556cd4f45a2";
    Mock::given(method("GET"))
        .and(path(format!("/tx/{txid}")))
        .respond_with(protobuf(answer("tx")))
        .mount(&server)
        .await;
    let status = client.fetch_tx_status(txid).await.unwrap();
    assert!(status.confirmed);
    assert_eq!(status.block_height, Some(976_950));
    // An answer for another transaction is refused.
    let other = format!("{}00", &txid[..62]);
    Mock::given(method("GET"))
        .and(path(format!("/tx/{other}")))
        .respond_with(protobuf(answer("tx")))
        .mount(&server)
        .await;
    assert!(client.fetch_tx_status(&other).await.is_err());
}

/// A broadcast posts the raw transaction in protobuf, its token checks on,
/// and answers the txid; a refusal carries Chronik's message.
#[tokio::test]
async fn a_broadcast_posts_protobuf_and_reads_the_txid() {
    let (server, client) = chronik().await;
    let mut txid = vec![0u8; 32];
    txid[0] = 0xab;
    Mock::given(method("POST"))
        .and(path("/broadcast-tx"))
        .and(header("content-type", "application/x-protobuf"))
        .and(body_bytes(
            BroadcastTxRequest {
                raw_tx: vec![1, 2, 3],
                skip_token_checks: false,
            }
            .encode_to_vec(),
        ))
        .respond_with(protobuf(BroadcastTxResponse { txid }.encode_to_vec()))
        .mount(&server)
        .await;
    assert_eq!(
        client.broadcast_raw_tx("010203").await.unwrap(),
        format!("{}ab", "00".repeat(31))
    );

    let (refusing, client) = chronik().await;
    Mock::given(method("POST"))
        .and(path("/broadcast-tx"))
        .respond_with(
            ResponseTemplate::new(400).set_body_bytes(
                ErrorMessage {
                    msg: "400: Broadcast failed: bad-txns-inputs-missingorspent".into(),
                }
                .encode_to_vec(),
            ),
        )
        .mount(&refusing)
        .await;
    match client.broadcast_raw_tx("010203").await {
        Err(ApiError::Rejected(message)) => {
            assert!(message.contains("missingorspent"), "{message}")
        }
        other => panic!("{other:?}"),
    }
}

/// Another network's spelling of the same key hash is refused before any
/// request: Bitcoin Cash's CashAddr and the legacy Base58 form.
#[tokio::test]
async fn another_networks_address_is_refused_before_asking() {
    let (_server, client) = chronik().await;
    for address in [
        "bitcoincash:qrrrkdwsgd9ntyvd4ah3kr37ccqvg9844q93p4vzhc",
        "1K59iyqJ9BoqzSHdyHawXk3dkfzae6rv22",
        "bc1qsv8xygwwrqrw5h9xh6ezl4djdzysdfnzfqf2ve",
    ] {
        assert!(matches!(
            client.fetch_utxos(address).await,
            Err(ApiError::InvalidInput(_))
        ));
    }
}
