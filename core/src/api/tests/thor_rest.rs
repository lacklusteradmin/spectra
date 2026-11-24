use super::*;

const ADDRESS: &str = "0xf9563630616fe66c57a9aad3364738b1c84b45d1";
const OTHER: &str = "0x1bb09429e42d78d1b5e0c8063dde8b4c87c6011c";
const ENERGY: &str = "0x0000000000000000000000000000456e65726779";

/// `GET /blocks/best` from mainnet.vechain.org, 2026-11-24.
fn best_block() -> Value {
    json!({
        "number": 26498961,
        "id": "0x019457919ffa9de1528c77d107e94b18c9a110db1af40a0f4fda62238bd249d5",
        "size": 1003,
        "parentID": "0x01945790c5a1b9f7d9c1e6c4b0d1b3a29b0d3b84a1c0d7b6a6c0b0a0e0d0c0b0",
        "timestamp": 1795522380,
        "gasLimit": 40000000,
        "baseFeePerGas": "0x9184e72a000",
        "isTrunk": true,
        "isFinalized": false,
        "transactions": [],
    })
}

/// One `meta` of a log, as `/logs/transfer` and `/logs/event` write it.
fn meta(txid: &str, block: u64) -> Value {
    json!({
        "blockID": format!("0x{block:08x}{}", "0".repeat(56)),
        "blockNumber": block,
        "blockTimestamp": 1795522430,
        "txID": txid,
        "txOrigin": ADDRESS,
        "clauseIndex": 0,
    })
}

fn txid(byte: &str) -> String {
    format!("0x{}", byte.repeat(32))
}

#[test]
fn the_best_block_is_read_with_its_base_fee_and_its_id_must_carry_its_number() {
    let block = block_from(&best_block()).unwrap();
    assert_eq!(block.number, 26_498_961);
    assert_eq!(block.base_fee_per_gas, Some(10_000_000_000_000));
    assert_eq!(hex::encode(&block.id[..8]), "019457919ffa9de1");
    // A block from before Galactica has no base fee.
    let mut old = best_block();
    old.as_object_mut().unwrap().remove("baseFeePerGas");
    assert_eq!(block_from(&old).unwrap().base_fee_per_gas, None);
    let mut lying = best_block();
    lying["number"] = json!(26_498_962);
    assert!(block_from(&lying).is_err());
}

#[test]
fn only_the_mainnet_genesis_is_vechain() {
    let genesis = Chain::Vechain.thor().unwrap().genesis_id;
    // `GET /blocks/0`, mainnet and testnet.
    let mainnet = json!({"number": 0, "id": genesis, "timestamp": 1530316800});
    assert!(verify_genesis(&mainnet, genesis).is_ok());
    let testnet = json!({
        "number": 0,
        "id": "0x000000000b2bce3c70bc649a02749e8687721b09ed2e15997f466536b20bb127",
    });
    assert!(verify_genesis(&testnet, genesis).is_err());
    assert!(verify_genesis(&json!({"number": 1, "id": genesis}), genesis).is_err());
    assert_eq!(Chain::Vechain.thor().unwrap().chain_tag(), 0x4a);
}

#[test]
fn an_account_holds_vet_and_vtho_in_hex_wei() {
    // `GET /accounts/{address}` from mainnet.vechain.org.
    let account = account_from(&json!({
        "balance": "0xb09667cc2e34ed48000",
        "energy": "0xd24b4b290615bdc73c8",
        "hasCode": false,
    }))
    .unwrap();
    assert_eq!(account.balance, 0xb09667cc2e34ed48000);
    assert_eq!(account.energy, 0xd24b4b290615bdc73c8);
    assert!(account_from(&json!({"balance": "12", "energy": "0x0"})).is_err());
}

#[test]
fn hex_quantities_and_transaction_ids_are_read_strictly() {
    assert_eq!(quantity("0x0").unwrap(), 0);
    assert_eq!(quantity("0x746a528800").unwrap(), 500_000_000_000);
    for bad in [
        "",
        "0x",
        "746a528800",
        "0x+1",
        "0xg",
        &format!("0x1{}", "0".repeat(32)),
    ] {
        assert!(quantity(bad).is_err(), "{bad}");
    }
    let upper = format!("0x{}", "AB".repeat(32));
    assert_eq!(transaction_id(&upper).unwrap(), txid("ab"));
    assert!(transaction_id(&"ab".repeat(32)).is_err());
    assert!(transaction_id("0x1234").is_err());
}

#[test]
fn a_call_answers_each_clause_and_a_simulation_its_gas() {
    // `POST /accounts/*` for a VTHO `balanceOf` and `decimals()`, then a
    // simulated VTHO transfer, from mainnet.vechain.org.
    let reads = call_results(&json!([
        {"data": "0x000000000000000000000000000000000000000000000d245875523f4688702c",
         "events": [], "transfers": [], "gasUsed": 870, "reverted": false, "vmError": ""},
        {"data": "0x0000000000000000000000000000000000000000000000000000000000000012",
         "events": [], "transfers": [], "gasUsed": 302, "reverted": false, "vmError": ""},
    ]))
    .unwrap();
    assert_eq!(abi_word(&reads[0].data).unwrap(), 0xd245875523f4688702c);
    assert_eq!(decimals_word(&reads[1].data).unwrap(), 18);
    let simulated = call_results(&json!([{
        "data": "0x0000000000000000000000000000000000000000000000000000000000000001",
        "events": [{"address": ENERGY, "topics": [TRANSFER_TOPIC], "data": "0x"}],
        "transfers": [], "gasUsed": 28326, "reverted": false, "vmError": "",
    }]))
    .unwrap();
    assert_eq!(simulated[0].gas_used, 28_326);
    let reverted = call_results(&json!([{
        "data": "0x", "events": [], "transfers": [], "gasUsed": 0,
        "reverted": true, "vmError": "insufficient balance for transfer",
    }]))
    .unwrap();
    assert!(reverted[0].reverted);
    assert_eq!(
        hex::encode(balance_of_call(ADDRESS).unwrap()),
        format!("70a08231{:0>64}", &ADDRESS[2..])
    );
    assert_eq!(
        address_topic(ADDRESS).unwrap(),
        format!("0x{:0>64}", &ADDRESS[2..])
    );
}

#[test]
fn a_receipt_says_whether_the_clauses_ran() {
    let id = "0x433a18e1b45f11bfce582ac19f7f9e428b0965a62fdfcf6e2f71ddbb92e70776";
    // `GET /transactions/{id}/receipt` from mainnet.vechain.org.
    let receipt = |reverted: bool, txid: &str| {
        json!({
            "gasUsed": 21000,
            "gasPayer": ADDRESS,
            "paid": "0x46091543145dc08",
            "reward": "0x1767f710670dc08",
            "reverted": reverted,
            "meta": {
                "blockID": "0x0194579646030c95b127d7381472f5edc982239ff2e4010f3eb5c25d9e3957f1",
                "blockNumber": 26498966,
                "blockTimestamp": 1795522430,
                "txID": txid,
                "txOrigin": ADDRESS,
            },
            "outputs": [],
        })
    };
    assert_eq!(
        receipt_status(&receipt(false, id), id).unwrap(),
        TransactionStatus::Confirmed {
            succeeded: true,
            block: Some(26_498_966)
        }
    );
    assert_eq!(
        receipt_status(&receipt(true, id), id).unwrap(),
        TransactionStatus::Confirmed {
            succeeded: false,
            block: Some(26_498_966)
        }
    );
    assert_eq!(
        receipt_status(&Value::Null, id).unwrap(),
        TransactionStatus::Pending
    );
    assert!(receipt_status(&receipt(false, &txid("ab")), id).is_err());
}

#[test]
fn vet_transfers_are_netted_per_transaction() {
    let logs = [
        // Two clauses of one transaction paying out.
        json!({"sender": ADDRESS, "recipient": OTHER, "amount": "0x2386f26fc10000", "meta": meta(&txid("01"), 9)}),
        json!({"sender": ADDRESS, "recipient": OTHER, "amount": "0x2386f26fc10000", "meta": meta(&txid("01"), 9)}),
        // A payment in, written in mixed case by nobody but this test.
        json!({"sender": OTHER.to_ascii_uppercase().replace("0X", "0x"), "recipient": ADDRESS, "amount": "0xde0b6b3a7640000", "meta": meta(&txid("02"), 8)}),
        // To itself: nothing moved.
        json!({"sender": ADDRESS, "recipient": ADDRESS, "amount": "0x1", "meta": meta(&txid("03"), 7)}),
    ];
    let entries = vet_entries(&logs, ADDRESS).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].txid, txid("01"));
    assert_eq!(
        entries[0].amount,
        HistoryAmount::Units(20_000_000_000_000_000)
    );
    assert!(!entries[0].incoming);
    assert_eq!(entries[0].counterparty, OTHER);
    assert_eq!(entries[0].block_height, Some(9));
    assert_eq!(entries[0].timestamp, Some(1_795_522_430.0));
    assert_eq!(entries[0].outcome, HistoryOutcome::Confirmed);
    assert_eq!(
        entries[1].amount,
        HistoryAmount::Units(1_000_000_000_000_000_000)
    );
    assert!(entries[1].incoming);
    assert_eq!(entries[1].contract, None);
    // A log that names neither side as the address is not its history.
    let stranger =
        json!({"sender": OTHER, "recipient": OTHER, "amount": "0x1", "meta": meta(&txid("04"), 1)});
    assert!(vet_entries(&[stranger], ADDRESS).is_err());
}

fn transfer_event(contract: &str, from: &str, to: &str, amount: u128, tx: &str) -> Value {
    json!({
        "address": contract,
        "topics": [
            TRANSFER_TOPIC,
            format!("0x{:0>64}", &from[2..]),
            format!("0x{:0>64}", &to[2..]),
        ],
        "data": format!("0x{amount:064x}"),
        "meta": meta(tx, 5),
    })
}

#[test]
fn token_transfers_are_read_at_their_own_decimals() {
    let tokens = vec![ENERGY.to_string()];
    let logs = [
        transfer_event(
            ENERGY,
            OTHER,
            ADDRESS,
            25_500_000_000_000_000_000,
            &txid("0a"),
        ),
        transfer_event(ENERGY, ADDRESS, OTHER, 1, &txid("0b")),
    ];
    let transfers = token_transfers(&logs, ADDRESS, &tokens).unwrap();
    let decimals = HashMap::from([(ENERGY.to_string(), 18)]);
    let entries = token_entries(transfers, ADDRESS, &decimals).unwrap();
    assert_eq!(entries[0].contract.as_deref(), Some(ENERGY));
    assert_eq!(entries[0].amount, HistoryAmount::Decimal("25.5".into()));
    assert!(entries[0].incoming);
    assert_eq!(entries[0].counterparty, OTHER);
    assert_eq!(
        entries[1].amount,
        HistoryAmount::Decimal("0.000000000000000001".into())
    );
    assert!(!entries[1].incoming);

    // An NFT's `Transfer` has a fourth topic; a contract nobody asked for,
    // or a transfer between others, is not this address's token history.
    let mut nft = transfer_event(ENERGY, OTHER, ADDRESS, 1, &txid("0c"));
    nft["topics"]
        .as_array_mut()
        .unwrap()
        .push(json!(format!("0x{:064x}", 7)));
    let untracked = transfer_event(
        "0x5ef79995fe8a89e0812330e4378eb2660cede699",
        OTHER,
        ADDRESS,
        1,
        &txid("0d"),
    );
    let between_others = transfer_event(ENERGY, OTHER, OTHER, 1, &txid("0e"));
    for log in [nft, untracked, between_others] {
        assert!(token_transfers(&[log], ADDRESS, &tokens).is_err());
    }
    // Decimals that were never read are not guessed.
    let transfers = token_transfers(&logs[..1], ADDRESS, &tokens).unwrap();
    assert!(token_entries(transfers, ADDRESS, &HashMap::new()).is_err());
}

#[tokio::test]
async fn history_pages_stop_at_the_first_pages_block_and_walk_both_logs() {
    use std::sync::Mutex;
    use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};
    let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
    let server = MockServer::start().await;
    let recorded = Arc::clone(&seen);
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            let answer = match request.url.path() {
                "/blocks/best" => best_block(),
                "/logs/transfer" => {
                    recorded.lock().unwrap().push(body.clone());
                    let offset = body["options"]["offset"].as_u64().unwrap();
                    // A full first page, then one more transfer.
                    let count = if offset == 0 {
                        HISTORY_PAGE_SIZE as u64
                    } else {
                        1
                    };
                    Value::Array(
                        (0..count)
                            .map(|n| {
                                json!({"sender": OTHER, "recipient": ADDRESS, "amount": "0x1",
                                       "meta": meta(&format!("0x{:064x}", offset + n), 10)})
                            })
                            .collect(),
                    )
                }
                "/logs/event" => {
                    recorded.lock().unwrap().push(body.clone());
                    json!([transfer_event(ENERGY, OTHER, ADDRESS, 7, &txid("ee"))])
                }
                "/accounts/*" => json!([{
                    "data": format!("0x{:064x}", 18), "events": [], "transfers": [],
                    "gasUsed": 302, "reverted": false, "vmError": "",
                }]),
                other => panic!("unexpected request {other}"),
            };
            ResponseTemplate::new(200).set_body_json(answer)
        })
        .mount(&server)
        .await;
    let client = ThorClient::new(Arc::new(vec![server.uri()]));
    let tokens = [ENERGY.to_string()];
    let first = client
        .fetch_history_page(ADDRESS, &tokens, None)
        .await
        .unwrap();
    assert_eq!(first.items.len(), HISTORY_PAGE_SIZE as usize + 1);
    let cursor = first.next_cursor.expect("the transfer log has more");
    let second = client
        .fetch_history_page(ADDRESS, &tokens, Some(&cursor))
        .await
        .unwrap();
    assert_eq!(second.items.len(), 1);
    assert!(second.next_cursor.is_none());
    // Transfer log, event log, then the transfer log's second page; the
    // event log, done on the first page, is not asked again.
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 3);
    for request in seen.iter() {
        assert_eq!(request["range"]["to"], 26_498_961);
        assert_eq!(request["order"], "desc");
    }
    assert_eq!(seen[1]["criteriaSet"][0]["address"], ENERGY);
    assert_eq!(
        seen[1]["criteriaSet"][1]["topic2"],
        json!(address_topic(ADDRESS).unwrap())
    );
    assert_eq!(seen[2]["options"]["offset"], HISTORY_PAGE_SIZE);

    // No token is no event query: an empty criteria set matches every event.
    let before = server.received_requests().await.unwrap().len();
    client
        .fetch_history_page(ADDRESS, &[], Some(&cursor))
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests[before..]
            .iter()
            .all(|r| r.url.path() != "/logs/event")
    );
}
