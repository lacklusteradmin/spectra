use super::*;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const ME: &str = "0x7ab9a6a7109dcb9cb357a109f32dfcc78a7aa2d6029084eb924d95133fc71cec";
const THEM: &str = "0xac5bceec1b789ff840d7d4e6ce4ce61c90d190a7f8c4f4ddf0bff6ee2413c33c";
const LONG_SUI: &str =
    "0x0000000000000000000000000000000000000000000000000000000000000002::sui::SUI";
const CETUS: &str =
    "0x06864a6f921804860930db6ddbe2e16acdf8504495ea7481637a1c8b9a8fe54b::cetus::CETUS";

/// A node answering each operation, by name, with `answer(operation,
/// variables)`'s `data`.
async fn node(answer: impl Fn(&str, &Value) -> Value + Send + Sync + 'static) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let operation = body["operationName"].as_str().unwrap();
            assert_eq!(operation_name(body["query"].as_str().unwrap()), operation);
            ResponseTemplate::new(200)
                .set_body_json(json!({"data": answer(operation, &body["variables"])}))
        })
        .mount(&server)
        .await;
    server
}

fn client(server: &MockServer) -> SuiClient {
    SuiClient::new(Arc::new(vec![server.uri()]))
}

fn transaction(digest: &str, sender: &str, gas: [u64; 3], changes: Value) -> Value {
    json!({
        "digest": digest,
        "sender": {"address": sender},
        "gasInput": {"gasSponsor": {"address": sender}},
        "effects": {
            "timestamp": "2026-11-24T06:01:52.067Z",
            "checkpoint": {"sequenceNumber": 348606189},
            "gasEffects": {"gasSummary": {
                "computationCost": gas[0], "storageCost": gas[1], "storageRebate": gas[2]}},
            "balanceChanges": {"pageInfo": {"hasNextPage": false, "endCursor": null},
                "nodes": changes},
        },
    })
}

fn change(owner: &str, coin: &str, amount: &str) -> Value {
    json!({"owner": {"address": owner}, "coinType": {"repr": coin}, "amount": amount})
}

/// Shapes taken from mainnet `History` answers: a transfer is the owner's
/// balance change with its gas added back, dated by its checkpoint.
#[test]
fn transfers_are_read_from_balance_changes_without_gas() {
    let received = transaction(
        "in",
        THEM,
        [100000, 988000, 978120],
        json!([
            change(ME, LONG_SUI, "4770000000000"),
            change(THEM, LONG_SUI, "-4770000109880"),
        ]),
    );
    let sent = transaction(
        "out",
        ME,
        [100000, 1976000, 978120],
        json!([
            change(THEM, LONG_SUI, "2500100000000000"),
            change(ME, LONG_SUI, "-2500100001097880"),
        ]),
    );
    let entries = history_entries(&[received, sent], ME).unwrap();
    let incoming = entries.iter().find(|e| e.digest == "in").unwrap();
    assert!(incoming.is_incoming);
    assert_eq!(incoming.amount_mist, "4770000000000");
    assert_eq!(incoming.amount_display.as_deref(), Some("4770"));
    assert_eq!(incoming.contract, None);
    assert_eq!(incoming.from, THEM);
    // To the second, as every provider's stamp is read.
    assert_eq!(incoming.timestamp_ms, Some(1_795_500_112_000));
    let outgoing = entries.iter().find(|e| e.digest == "out").unwrap();
    assert!(!outgoing.is_incoming);
    assert_eq!(
        outgoing.amount_mist, "2500100000000000",
        "gas is not part of it"
    );
    assert_eq!(outgoing.to, THEM);
}

/// A transaction not yet in a checkpoint has no time and is undated; one in
/// a checkpoint without a time was read wrongly.
#[test]
fn only_an_uncheckpointed_transaction_is_undated() {
    let mut pending = transaction(
        "pending",
        THEM,
        [0, 0, 0],
        json!([change(ME, LONG_SUI, "5")]),
    );
    pending["effects"]["timestamp"] = Value::Null;
    pending["effects"]["checkpoint"] = Value::Null;
    let entries = history_entries(std::slice::from_ref(&pending), ME).unwrap();
    assert_eq!(entries[0].timestamp_ms, None);
    pending["effects"]["checkpoint"] = json!({"sequenceNumber": 9});
    assert!(history_entries(&[pending], ME).is_err());
}

/// A transaction whose only SUI effect is gas — here a net storage rebate
/// while another coin moved — transfers no SUI; the coin it moved is named
/// as the catalog writes it.
#[test]
fn gas_rebate_is_excluded_but_other_coin_transfers_are_kept() {
    let swap = transaction(
        "swap",
        ME,
        [102000, 3663200, 30111048],
        json!([
            change(ME, LONG_SUI, "26345848"),
            change(ME, CETUS, "-2699970876000000"),
            change(THEM, CETUS, "2699970876000000"),
        ]),
    );
    let entries = history_entries(&[swap], ME).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].contract.as_deref(),
        Some("0x6864a6f921804860930db6ddbe2e16acdf8504495ea7481637a1c8b9a8fe54b::cetus::CETUS")
    );
    assert_eq!(entries[0].amount_mist, "2699970876000000");
    assert_eq!(entries[0].to, THEM);
    assert!(!entries[0].is_incoming);
}

/// A transaction's balance changes past its first page are read before its
/// transfer is: here the owner's own change is on the second page. The page
/// cursor is the oldest transaction's, and the token's amount is in its own
/// decimals.
#[tokio::test]
async fn history_reads_every_page_of_a_transactions_balance_changes() {
    let server = node(|operation, variables| match operation {
        "History" => {
            assert_eq!(variables["owner"], ME);
            let mut airdrop = transaction(
                "airdrop",
                THEM,
                [1, 1, 0],
                json!([change(THEM, CETUS, "-7")]),
            );
            airdrop["effects"]["balanceChanges"]["pageInfo"] =
                json!({"hasNextPage": true, "endCursor": "p1"});
            json!({"address": {"transactions": {
                "pageInfo": {"hasPreviousPage": true, "startCursor": "older"},
                "nodes": [airdrop]}}})
        }
        "BalanceChanges" => {
            assert_eq!(variables, &json!({"digest": "airdrop", "after": "p1"}));
            json!({"transactionEffects": {"balanceChanges": {
                "pageInfo": {"hasNextPage": false, "endCursor": "p2"},
                "nodes": [change(ME, CETUS, "7")]}}})
        }
        "CoinMetadata" => json!({"coinMetadata": {"decimals": 6}}),
        other => panic!("unexpected {other}"),
    })
    .await;
    let page = client(&server).fetch_history_page(ME, None).await.unwrap();
    assert_eq!(page.next_cursor.as_deref(), Some("older"));
    assert_eq!(page.items.len(), 1);
    assert!(page.items[0].is_incoming);
    assert_eq!(page.items[0].amount_display.as_deref(), Some("0.000007"));
}

/// An endpoint is the network whose genesis checkpoint it names as its
/// chain identifier, and answers a latest checkpoint.
#[tokio::test]
async fn an_endpoint_is_the_network_its_chain_identifier_names() {
    for chain in [Chain::Sui, Chain::SuiTestnet] {
        let other = if chain == Chain::Sui {
            Chain::SuiTestnet
        } else {
            Chain::Sui
        };
        let server = node(move |operation, _| {
            assert_eq!(operation, "Chain");
            json!({"chainIdentifier": chain.sui_genesis_checkpoint(),
                "checkpoint": {"sequenceNumber": 42}})
        })
        .await;
        assert_eq!(client(&server).latest_checkpoint(chain).await, Ok(42));
        assert!(
            client(&server)
                .verify_network(other)
                .await
                .unwrap_err()
                .to_string()
                .contains("wrong network")
        );
    }
}

/// The health probe verifies the network through the same request, on the
/// configured URL.
#[tokio::test]
async fn the_health_probe_reads_the_configured_endpoints_network() {
    let server = node(|_, _| {
        json!({"chainIdentifier": Chain::Sui.sui_genesis_checkpoint(),
            "checkpoint": {"sequenceNumber": 7}})
    })
    .await;
    let endpoint = |url: String| crate::api::endpoint::Endpoint {
        api: crate::api::endpoint::EndpointApi::SuiGraphql,
        url,
    };
    let (checked, passed, detail) =
        crate::api::health::probe(Chain::Sui, &endpoint(server.uri()), &[]).await;
    assert!(checked && passed, "{detail}");
    let (checked, passed, detail) =
        crate::api::health::probe(Chain::SuiTestnet, &endpoint(server.uri()), &[]).await;
    assert!(
        checked && !passed && detail.contains("wrong network"),
        "{detail}"
    );
}

/// A digest settles once a checkpoint holds it, by its effects; executed
/// but not yet checkpointed, or not yet executed, it is still pending. A
/// reply for another digest is not this transaction's.
#[test]
fn a_digest_settles_at_its_checkpoint_and_only_for_itself() {
    let digest = "4btiuiMPvEENsttpZC7CZ53DruC3MAgfznDbASZ7DR6S";
    let effects = |digest: &str, checkpoint: Value, status: &str| json!({"digest": digest, "checkpoint": checkpoint, "status": status});
    let read = |effects: Value| transaction_status(&effects, digest);
    let at = json!({"sequenceNumber": 42});
    assert_eq!(
        read(effects(digest, at.clone(), "SUCCESS")),
        Ok(TransactionStatus::Confirmed {
            succeeded: true,
            block: Some(42)
        })
    );
    assert_eq!(
        read(effects(digest, at.clone(), "FAILURE")),
        Ok(TransactionStatus::Confirmed {
            succeeded: false,
            block: Some(42)
        })
    );
    assert_eq!(
        read(effects(digest, Value::Null, "FAILURE")),
        Ok(TransactionStatus::Pending)
    );
    assert_eq!(read(Value::Null), Ok(TransactionStatus::Pending));
    assert_eq!(
        read(effects("wrong-digest", at, "SUCCESS")),
        Err(ApiError::decode("Sui status: transaction digest mismatch"))
    );
}

/// An answer with errors refuses the request even beside data; one without
/// data is malformed.
#[test]
fn errors_refuse_and_missing_data_is_malformed() {
    let refused = answer(json!({"data": {"x": 1}, "errors": [{"message": "stale object"}]}));
    assert_eq!(
        refused,
        Err(ApiError::Rejected("Sui GraphQL: stale object".into()))
    );
    assert!(matches!(
        answer(json!({"data": null})),
        Err(ApiError::Decode(_))
    ));
    assert_eq!(
        answer(json!({"data": {"x": 1}, "errors": []})),
        Ok(json!({"x": 1}))
    );
}

/// GraphQL's 64-digit types are read as the catalog writes them, and a
/// coin object's type names its coin.
#[test]
fn types_are_read_as_the_catalog_writes_them() {
    assert_eq!(short_type(LONG_SUI), SUI_COIN_TYPE);
    assert_eq!(
        coin_of(&short_type(&format!(
            "0x{:0>64}::coin::Coin<{LONG_SUI}>",
            "2"
        )))
        .as_deref(),
        Some(SUI_COIN_TYPE)
    );
    assert_eq!(coin_of("0x3::staking_pool::StakedSui"), None);
    assert_eq!(long_address("0x5"), format!("0x{:0>64}", "5"));
    assert_eq!(long_address(&ME.to_uppercase().replace("0X", "0x")), ME);
}

/// A listing whose service repeats a cursor would page forever, and is
/// refused.
#[tokio::test]
async fn a_repeated_page_cursor_is_refused() {
    let server = node(|operation, _| {
        assert_eq!(operation, "Balances");
        json!({"address": {"balances": {
            "pageInfo": {"hasNextPage": true, "endCursor": "same"},
            "nodes": [{"coinType": {"repr": CETUS}, "totalBalance": "5"}]}}})
    })
    .await;
    assert!(
        client(&server)
            .fetch_all_coin_balances(ME)
            .await
            .unwrap_err()
            .to_string()
            .contains("repeated page cursor")
    );
}

/// Coin objects are counted per coin type across pages, from the coins the
/// address holds as objects.
#[tokio::test]
async fn coin_objects_are_counted_per_type_across_pages() {
    let server = node(|operation, variables| {
        assert_eq!(operation, "Objects");
        assert_eq!(variables["type"], "0x2::coin::Coin");
        let coin = |byte: &str, coin: &str, balance: &str| {
            json!({"address": format!("0x{}", byte.repeat(32)), "version": 7,
                "digest": "1".repeat(32),
                "owner": {"__typename": "AddressOwner", "address": {"address": ME}},
                "contents": {"type": {"repr": format!("0x{:0>64}::coin::Coin<{coin}>", "2")},
                    "json": {"balance": balance}}})
        };
        if variables["after"].is_null() {
            json!({"address": {"objects": {
                "pageInfo": {"hasNextPage": true, "endCursor": "next"},
                "nodes": [coin("33", LONG_SUI, "4"), coin("44", CETUS, "1")]}}})
        } else {
            json!({"address": {"objects": {
                "pageInfo": {"hasNextPage": false, "endCursor": null},
                "nodes": [coin("55", LONG_SUI, "6")]}}})
        }
    })
    .await;
    let counts = client(&server).fetch_coin_object_counts(ME).await.unwrap();
    assert_eq!(
        counts,
        [
            ("0x2::sui::SUI".to_string(), 2, 10),
            (short_type(CETUS), 1, 1),
        ]
    );
}
