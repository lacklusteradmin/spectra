use super::*;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn id(text: &str) -> HederaAccountId {
    HederaAccountId::parse(Chain::Hedera, text).unwrap()
}

/// `/api/v1/accounts/0.0.1000000`, read from mainnet-public on 2026-11-24.
fn account_answer() -> Value {
    json!({
        "account": "0.0.1000000", "alias": null, "auto_renew_period": 7776000,
        "balance": {"balance": 3205420013u64, "timestamp": "1782302410.934870000"},
        "created_timestamp": "1655145563.835648470", "decline_reward": false, "deleted": false,
        "ethereum_nonce": 0, "evm_address": "0x00000000000000000000000000000000000f4240",
        "expiry_timestamp": "1662921563.835648470",
        "key": {"_type": "ED25519", "key": "f3ee93a679b115bffd753b068cf027535905c59cc6df45dd8eeb4a9f6f743112"},
        "max_automatic_token_associations": 10, "memo": "", "pending_reward": 25489570,
        "receiver_sig_required": false, "staked_account_id": null, "staked_node_id": 25,
        "stake_period_start": "1782172800.000000000",
        "links": {"next": "/api/v1/accounts/0.0.1000000?timestamp=lt:1790338570.660000001"}
    })
}

#[test]
fn an_account_is_read_with_its_one_key_or_none_one_key_signs_for() {
    let account = account_from(&account_answer()).unwrap();
    assert_eq!(account.id, id("0.0.1000000"));
    assert_eq!(account.balance, Some(3_205_420_013));
    assert!(!account.deleted && !account.receiver_sig_required);
    let HederaAccountKey::Single(key) = account.key else {
        panic!("one Ed25519 key");
    };
    assert_eq!(key.scheme, HederaKeyScheme::Ed25519);
    assert_eq!(
        key.der_hex(),
        "302a300506032b6570032100f3ee93a679b115bffd753b068cf027535905c59cc6df45dd8eeb4a9f6f743112"
    );
    let ecdsa = json!({"account": "0.0.5", "deleted": true,
        "key": {"_type": "ECDSA_SECP256K1", "key": "021de0d6f31872bbda0cfd1680e41a5a88af2646b8d1d015c5e1541ab21a0f4499"}});
    let ecdsa = account_from(&ecdsa).unwrap();
    assert!(ecdsa.deleted);
    assert!(
        matches!(ecdsa.key, HederaAccountKey::Single(ref key) if key.scheme == HederaKeyScheme::EcdsaSecp256k1)
    );
    // A key list, as the mirror node writes one, and no key at all.
    for key in [
        json!({"_type": "ProtobufEncoded", "key": "2a240802122012200a20f3ee93a679b115bffd753b068cf027535905c59cc6df45dd8eeb4a9f6f743112"}),
        Value::Null,
    ] {
        let account = account_from(&json!({"account": "0.0.6", "key": key})).unwrap();
        assert_eq!(account.key, HederaAccountKey::Other);
    }
    // A key that is not a point of its curve is not read as one.
    let bad =
        json!({"account": "0.0.7", "key": {"_type": "ECDSA_SECP256K1", "key": "05".repeat(33)}});
    assert!(account_from(&bad).is_err());
}

#[test]
fn a_node_is_read_with_its_tls_endpoints_and_certificate_hash() {
    // Node 0.0.3 of `/api/v1/network/nodes`, mainnet, 2026-11-24.
    let node = json!({
        "node_account_id": "0.0.3", "node_id": 0,
        "node_cert_hash": "0x34244d50a8ed4d4cbadf25620d1ab88a32081977d2f8d70ba82ba52b3305a4560824cf6b0106d635ed5c39272fffabe8",
        "service_endpoints": [
            {"domain_name": "", "ip_address_v4": "34.64.136.190", "port": 50211},
            {"domain_name": "", "ip_address_v4": "34.64.136.190", "port": 50212},
            {"domain_name": "", "ip_address_v4": "43.202.249.148", "port": 50211},
            {"domain_name": "", "ip_address_v4": "43.202.249.148", "port": 50212}
        ]
    });
    let node = node_from(&node).unwrap().unwrap();
    assert_eq!(node.account, id("0.0.3"));
    assert_eq!(
        node.tls_endpoints,
        [
            ("34.64.136.190".to_string(), 50212),
            ("43.202.249.148".to_string(), 50212)
        ]
    );
    assert_eq!(node.cert_hash[..4], [0x34, 0x24, 0x4d, 0x50]);
    // Plaintext only, or no certificate hash: nothing a pinned channel reaches.
    let plaintext = json!({"node_account_id": "0.0.4", "node_cert_hash": format!("0x{}", "ab".repeat(48)),
        "service_endpoints": [{"domain_name": "", "ip_address_v4": "1.2.3.4", "port": 50211}]});
    assert!(node_from(&plaintext).unwrap().is_none());
    let unhashed = json!({"node_account_id": "0.0.4", "node_cert_hash": "0x",
        "service_endpoints": [{"domain_name": "node.example", "ip_address_v4": "", "port": 50212}]});
    assert!(node_from(&unhashed).unwrap().is_none());
}

/// A CRYPTOTRANSFER of `/api/v1/transactions?account.id=0.0.1000000`,
/// mainnet, 2026-11-24: an NFT purchase paid by 0.0.10143848 that also paid
/// 0.0.1000000 its staking reward.
fn purchase() -> Value {
    json!({
        "transaction_id": "0.0.1064038-1782302393-941572588", "result": "SUCCESS",
        "charged_tx_fee": 3131726, "node": "0.0.35", "consensus_timestamp": "1782302410.934870000",
        "name": "CRYPTOTRANSFER", "token_transfers": [],
        "transfers": [
            {"account": "0.0.800", "amount": -13165049, "is_approval": false},
            {"account": "0.0.802", "amount": 3131726, "is_approval": false},
            {"account": "0.0.1000000", "amount": 22965049, "is_approval": false},
            {"account": "0.0.1064038", "amount": -3131726, "is_approval": false},
            {"account": "0.0.1117001", "amount": 88200000, "is_approval": false},
            {"account": "0.0.1223480", "amount": 250000, "is_approval": false},
            {"account": "0.0.10143848", "amount": -100000000, "is_approval": false},
            {"account": "0.0.10466510", "amount": 200000, "is_approval": false},
            {"account": "0.0.10489694", "amount": 1550000, "is_approval": false}
        ],
        "staking_reward_transfers": [{"account": "0.0.1000000", "amount": 13165049}]
    })
}

#[test]
fn history_reads_what_a_transfer_moved_with_fee_and_reward_aside() {
    // Received, less the staking reward that came with it.
    let entry = history_entry(&purchase(), id("0.0.1000000"))
        .unwrap()
        .unwrap();
    assert_eq!(entry.txid, "0.0.1064038-1782302393-941572588");
    assert_eq!(entry.amount, HistoryAmount::Units(9_800_000));
    assert!(entry.incoming);
    assert_eq!(entry.counterparty, "0.0.10143848");
    assert_eq!(entry.outcome, HistoryOutcome::Confirmed);
    assert_eq!(entry.timestamp, Some(1_782_302_410.934_87));
    // The payer of the fee alone moved nothing else.
    assert!(
        history_entry(&purchase(), id("0.0.1064038"))
            .unwrap()
            .is_none()
    );
    // A send, from the payer: the fee aside, the amount, to the recipient
    // rather than the fee account.
    let sent = json!({
        "transaction_id": "0.0.3229-1795522566-435000006", "result": "SUCCESS", "charged_tx_fee": 194900,
        "consensus_timestamp": "1795522576.874181104",
        "transfers": [
            {"account": "0.0.802", "amount": 194900, "is_approval": false},
            {"account": "0.0.3229", "amount": -194901, "is_approval": false},
            {"account": "0.0.3230", "amount": 1, "is_approval": false}
        ],
        "staking_reward_transfers": []
    });
    let entry = history_entry(&sent, id("0.0.3229")).unwrap().unwrap();
    assert_eq!(entry.amount, HistoryAmount::Units(1));
    assert!(!entry.incoming);
    assert_eq!(entry.counterparty, "0.0.3230");
    // A failed send paid only its fee.
    let mut failed = sent.clone();
    failed["result"] = json!("INSUFFICIENT_ACCOUNT_BALANCE");
    failed["transfers"] = json!([
        {"account": "0.0.802", "amount": 194900, "is_approval": false},
        {"account": "0.0.3229", "amount": -194900, "is_approval": false}
    ]);
    assert!(history_entry(&failed, id("0.0.3229")).unwrap().is_none());
}

#[test]
fn transaction_ids_timestamps_and_fee_bounds_read_as_the_network_writes_them() {
    assert!(is_transaction_id("0.0.3229-1795522566-435000006"));
    assert!(is_transaction_id("0.0.3229-1795522566-000000000"));
    for bad in [
        "0.0.3229-1795522566-0",
        "0.0.3229@1795522566.435000006",
        "0.0.3229-1795522566",
        "0.0.03229-1795522566-435000006",
        "0.0.3229-1795522566-435000006-1",
        "../0.0.3229-1795522566-435000006",
    ] {
        assert!(!is_transaction_id(bad), "{bad}");
    }
    assert_eq!(
        HederaTimestamp::parse("1795522576.874181104"),
        Some(HederaTimestamp {
            seconds: 1_795_522_576,
            nanos: 874_181_104
        })
    );
    assert_eq!(HederaTimestamp::parse("12.5").unwrap().nanos, 500_000_000);
    assert!(HederaTimestamp::parse("12.").is_none());
    assert!(HederaTimestamp::parse("-1.0").is_none());
    // `/api/v1/network/exchangerate`, 2026-11-24: 153925 cents buy 30000
    // HBAR, so $0.001 buys 1,949,000 tinybars after rounding up.
    let rate = HederaExchangeRate {
        cent_equivalent: 153_925,
        hbar_equivalent: 30_000,
    };
    assert_eq!(rate.tinybars_for_mills(1), Some(1_949_002));
    assert_eq!(
        HederaExchangeRate {
            cent_equivalent: 12,
            hbar_equivalent: 1
        }
        .tinybars_for_mills(1),
        Some(833_334)
    );
    assert!(
        HederaExchangeRate {
            cent_equivalent: 0,
            hbar_equivalent: 1
        }
        .tinybars_for_mills(1)
        .is_none()
    );
    assert_eq!(
        mirror_of_address_book("https://mirror.example/api/v1/network/nodes").unwrap(),
        "https://mirror.example"
    );
    assert!(mirror_of_address_book("https://mirror.example").is_err());
}

#[tokio::test]
async fn history_reads_past_empty_stretches_and_another_network_is_refused() {
    let server = MockServer::start().await;
    let genesis = Chain::Hedera.hedera_network().unwrap().genesis_block_hash;
    Mock::given(method("GET"))
        .and(path("/api/v1/blocks/0"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"hash": genesis, "number": 0})),
        )
        .mount(&server)
        .await;
    // The first stretch is empty and links to the one before it.
    Mock::given(method("GET"))
        .and(path("/api/v1/transactions"))
        .and(query_param("account.id", "0.0.1000000"))
        .and(query_param("order", "desc"))
        .respond_with(move |request: &wiremock::Request| {
            let older = request.url.query().unwrap_or_default().contains("timestamp=lt");
            ResponseTemplate::new(200).set_body_json(if older {
                json!({"transactions": [purchase()], "links": {"next": null}})
            } else {
                json!({"transactions": [], "links": {"next": "/api/v1/transactions?account.id=0.0.1000000&transactiontype=CRYPTOTRANSFER&limit=25&order=desc&timestamp=lt:1785155153.122000001"}})
            })
        })
        .mount(&server)
        .await;
    let client = HederaMirrorClient::new(Arc::new(vec![server.uri()]));
    client.verify_network(Chain::Hedera).await.unwrap();
    let page = client
        .fetch_history_page(id("0.0.1000000"), None)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.next_cursor, None);
    assert!(
        client
            .fetch_history_page(id("0.0.1000000"), Some("https://elsewhere.example/"))
            .await
            .is_err()
    );

    // Testnet's block 0.
    let testnet = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/blocks/0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "hash": "0x5699954170cc8177692691d15368cad54f1e7a90c9e16b782f989de1b8d193583ed6ea19eb3290f59b85891eb23e8883"
        })))
        .mount(&testnet)
        .await;
    assert!(
        HederaMirrorClient::new(Arc::new(vec![testnet.uri()]))
            .verify_network(Chain::Hedera)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_status_is_the_user_transactions_result_and_an_unseen_one_is_pending() {
    let server = MockServer::start().await;
    let id = "0.0.3229-1795522566-435000006";
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/transactions/{id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"transactions": [
            {"transaction_id": id, "result": "SUCCESS", "scheduled": false, "nonce": 1},
            {"transaction_id": id, "result": "INSUFFICIENT_PAYER_BALANCE", "scheduled": false, "nonce": 0}
        ]})))
        .mount(&server)
        .await;
    let client = HederaMirrorClient::new(Arc::new(vec![server.uri()]));
    assert_eq!(
        client.fetch_transaction_status(id).await.unwrap(),
        TransactionStatus::Confirmed {
            succeeded: false,
            block: None
        }
    );
    assert_eq!(
        client
            .fetch_transaction_status("0.0.3229-1795522566-435000007")
            .await
            .unwrap(),
        TransactionStatus::Pending
    );
    assert!(client.fetch_transaction_status("0.0.3229").await.is_err());
}

#[tokio::test]
async fn a_key_lookup_keeps_the_accounts_that_hold_it_alone() {
    let server = MockServer::start().await;
    let raw = "f3ee93a679b115bffd753b068cf027535905c59cc6df45dd8eeb4a9f6f743112";
    let key = |raw: &str| json!({"_type": "ED25519", "key": raw});
    Mock::given(method("GET"))
        .and(path("/api/v1/accounts"))
        .and(query_param("account.publickey", raw))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "accounts": [
                {"account": "0.0.999906", "key": key(raw), "deleted": false},
                {"account": "0.0.999919", "key": key(raw), "deleted": true},
                {"account": "0.0.999976", "key": {"_type": "ProtobufEncoded", "key": "2a00"}, "deleted": false},
                {"account": "0.0.999977", "key": key("10fcec8c5db29aeab62272a08be39b1ec75eba50624351fba9948ff5d3d78ecf"), "deleted": false}
            ],
            "links": {"next": null}
        })))
        .mount(&server)
        .await;
    let public =
        HederaPublicKey::from_raw(HederaKeyScheme::Ed25519, &hex::decode(raw).unwrap()).unwrap();
    let found = HederaMirrorClient::new(Arc::new(vec![server.uri()]))
        .accounts_holding_key(&public)
        .await
        .unwrap();
    assert_eq!(found, [id("0.0.999906")]);
}
