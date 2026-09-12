//! Real stored mnemonic -> identity -> params -> signer -> mock submission.
use super::*;
use crate::store::{
    secret_backends::InMemorySecretStore, state::WalletSummary, wallet_secrets::store_seed_phrase,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};

#[tokio::test]
async fn audit_stored_wallets_reach_solana_sui_aptos_and_tron_submission() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../testdata/send-audit-vectors.json")).unwrap();
    for (chain, token, token2022) in [
        (Chain::Solana, false, false),
        (Chain::Solana, true, false),
        (Chain::Solana, true, true),
        (Chain::Sui, false, false),
        (Chain::Aptos, false, false),
        (Chain::Tron, false, false),
        (Chain::Tron, true, false),
    ] {
        let server = MockServer::start().await;
        let v = fixture.clone();
        Mock::given(any()).respond_with(move |request: &Request| {
            let body:Value=serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            let path=request.url.path();
            let result = match body["method"].as_str().unwrap_or(path) {
                "getAccountInfo" => json!({"value":{"owner":if token2022 {"TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"} else {"TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"},"data":{"parsed":{"type":"mint","info":{"isInitialized":true,"decimals":6,"extensions":[]}}}}}),
                "getLatestBlockhash" => json!({"value":{"blockhash":v["solana"]["blockhash"]}}),
                "sendTransaction" => {
                    let tx=STANDARD.decode(body["params"][0].as_str().unwrap()).unwrap();
                    if token {
                        let program = crate::derivation::chains::solana::decode_b58_32(if token2022 {"TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"} else {"TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"}).unwrap();
                        assert!(tx[69..69 + tx[68] as usize * 32].chunks_exact(32).any(|key| key == program));
                    }
                    let public:[u8;32]=tx[69..101].try_into().unwrap();
                    ed25519_dalek::VerifyingKey::from_bytes(&public).unwrap().verify_strict(&tx[65..],&ed25519_dalek::Signature::from_slice(&tx[1..65]).unwrap()).unwrap();
                    json!(bs58::encode(&tx[1..65]).into_string())
                },
                "suix_getReferenceGasPrice" => json!("1000"),
                "suix_getCoins" => json!({"data":[{"coinObjectId":format!("0x{}","33".repeat(32)),"version":"7","digest":"11111111111111111111111111111111","balance":"200000000"}],"hasNextPage":false,"nextCursor":null}),
                "sui_executeTransactionBlock" => {
                    let bytes=STANDARD.decode(body["params"][0].as_str().unwrap()).unwrap();
                    assert_eq!(hex::encode(&bytes),v["sui"]["raw"]);
                    assert_eq!(body["params"][1][0],v["sui"]["signature"]);
                    json!({"digest":"11111111111111111111111111111111","effects":{"status":{"status":"success"}}})
                },
                "/" => json!({"chain_id":1,"ledger_version":"1"}),
                "/estimate_gas_price" => json!({"gas_estimate":100}),
                "/transactions" => {
                    assert_eq!(body["sender"],v["aptos"]["address"]);
                    assert_eq!(body["signature"]["public_key"],format!("0x{}",v["aptos"]["public_key"].as_str().unwrap()));
                    json!({"hash":format!("0x{}","ab".repeat(32))})
                },
                "/wallet/getnowblock" => json!({"blockID":format!("0000000000000007{}","33".repeat(24)),"block_header":{"raw_data":{"number":7}}}),
                "/wallet/broadcasttransaction" => {
                    assert_eq!(body["raw_data"]["contract"][0]["parameter"]["value"]["owner_address"],v["tron"]["transactions"][0]["raw_data"]["contract"][0]["parameter"]["value"]["owner_address"]);
                    json!({"result":true})
                },
                "/wallet/triggerconstantcontract" => {
                    let result=if body["function_selector"]=="decimals()" {format!("{:064x}",6)} else {format!("{:064x}{:064x}{:0<64}",32,4,hex::encode("TEST"))};
                    json!({"result":{"result":true},"constant_result":[result]})
                },
                p if p.starts_with("/accounts/") => json!({"sequence_number":"7"}),
                other => panic!("unexpected provider request: {other} ({body})"),
            };
            ResponseTemplate::new(200).set_body_json(if body["method"].is_string(){json!({"jsonrpc":"2.0","id":body["id"],"result":result})}else{result})
        }).mount(&server).await;
        let service = WalletService::new_typed(vec![ChainEndpoints {
            chain_id: chain.str_id().into(),
            endpoints: vec![server.uri()],
            api_key: None,
        }])
        .unwrap();
        let db = std::env::temp_dir().join(format!(
            "send-record-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(db.to_string_lossy().into())
            .await
            .unwrap();
        let secrets = Arc::new(InMemorySecretStore::new());
        service.set_secret_store(secrets.clone());
        let (address, path) = match chain {
            Chain::Solana => (
                fixture["solana"]["address"].as_str().unwrap(),
                fixture["solana"]["path"].as_str().unwrap(),
            ),
            Chain::Sui => (
                fixture["sui"]["address"].as_str().unwrap(),
                "m/44'/784'/0'/0'/0'",
            ),
            Chain::Aptos => (
                fixture["aptos"]["address"].as_str().unwrap(),
                "m/44'/637'/0'/0'/0'",
            ),
            _ => (
                fixture["tron"]["from"].as_str().unwrap(),
                "m/44'/195'/0'/0/0",
            ),
        };
        service
            .apply_state_command(StateCommand::UpsertWallet {
                wallet: WalletSummary::single_address(
                    "w",
                    "Test",
                    chain.chain_display_name(),
                    address,
                    Some(path.into()),
                    false,
                ),
            })
            .await
            .unwrap();
        store_seed_phrase(&*secrets, "w", fixture["mnemonic"].as_str().unwrap(), None).unwrap();
        let destination = match chain {
            Chain::Solana => bs58::encode([0x22; 32]).into_string(),
            Chain::Tron => fixture["tron"]["to"].as_str().unwrap().into(),
            _ => format!("0x{}", "22".repeat(32)),
        };
        let request = crate::send::SendExecutionRequest {
            wallet_id: "w".into(),
            chain_id: chain.str_id().into(),
            password: None,
            to_address: destination,
            amount_str: if token || chain == Chain::Tron {
                "123.456789".into()
            } else if chain == Chain::Aptos {
                "1.23456789".into()
            } else {
                "0.123456789".into()
            },
            contract_address: if token {
                Some(if chain == Chain::Tron {
                    fixture["tron"]["contract"].as_str().unwrap().into()
                } else {
                    bs58::encode([0x44; 32]).into_string()
                })
            } else {
                None
            },
            token_decimals: token.then_some(6),
            fee_rate_svb: None,
            fee_sat: None,
            gas_budget: None,
            fee_amount: None,
            evm_overrides: None,
            monero_priority: None,
            sign_only: false,
        };
        let result = service
            .execute_send(request)
            .await
            .unwrap_or_else(|e| panic!("{chain:?} token={token}: {e}"));
        assert!(!result.transaction_hash.is_empty(), "{chain:?}");
        let rows = service.fetch_all_history_records_typed().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].id.len(),
            36,
            "native front ends parse the stored transaction ID as UUID"
        );
        assert_eq!(rows[0].id.chars().filter(|c| *c == '-').count(), 4);
        assert_eq!(
            rows[0].payload.transaction_hash.as_deref(),
            Some(result.transaction_hash.as_str())
        );
        assert!(rows[0].payload.signed_transaction_payload.is_some());
        let reopened = WalletService::new_typed(vec![]).unwrap();
        reopened
            .open_state(db.to_string_lossy().into())
            .await
            .unwrap();
        assert_eq!(
            reopened.fetch_all_history_records_typed().await.unwrap()[0].payload,
            rows[0].payload
        );
        let requests = server.received_requests().await.unwrap();
        assert!(!requests
            .iter()
            .any(|r| r.url.path().contains("encode_submission")
                || r.url.path().contains("createtransaction")
                || r.url.path().contains("triggersmartcontract")
                || String::from_utf8_lossy(&r.body).contains("unsafe_transferSui")));
    }
}

#[tokio::test]
async fn audit_fix5_nonce_journal_survives_response_loss_restart_and_concurrent_sends() {
    use crate::send::ethereum::{EvmCustomFeeConfiguration, EvmSendOverridesInput};
    use sha3::Digest;
    use std::sync::atomic::{AtomicBool, Ordering};
    let server = MockServer::start().await;
    let db = std::env::temp_dir()
        .join(format!(
            "audit-fix5-{}.sqlite",
            crate::store::new_event_id()
        ))
        .to_string_lossy()
        .into_owned();
    let service = WalletService::new_typed(vec![ChainEndpoints {
        chain_id: "ethereum".into(),
        endpoints: vec![server.uri()],
        api_key: None,
    }])
    .unwrap();
    service.open_state(db.clone()).await.unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    service.set_secret_store(secrets.clone());
    service
        .apply_state_command(StateCommand::UpsertWallet {
            wallet: WalletSummary::single_address(
                "w",
                "W",
                "Ethereum",
                "0x9858EfFD232B4033E47d90003D41EC34EcaEda94",
                Some("m/44'/60'/0'/0/0".into()),
                false,
            ),
        })
        .await
        .unwrap();
    store_seed_phrase(&*secrets, "w", "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about", None).unwrap();
    let lose_response = Arc::new(AtomicBool::new(true));
    let fault = lose_response.clone();
    let database = db.clone();
    Mock::given(any())
        .respond_with(move |r: &Request| {
            let body: Value = r.body_json().unwrap();
            let result = match body["method"].as_str().unwrap() {
                "eth_getTransactionCount" => {
                    assert_eq!(body["params"][1], "pending");
                    json!("0x7") // A stale provider never advances: the journal must reserve nonces.
                }
                "eth_sendRawTransaction" => {
                    let raw = body["params"][0].as_str().unwrap();
                    let hash = format!(
                        "0x{}",
                        hex::encode(sha3::Keccak256::digest(hex::decode(&raw[2..]).unwrap()))
                    );
                    let records = crate::wallet_db::history_fetch_all(&database).unwrap();
                    let row = records
                        .iter()
                        .find(|r| r.payload.transaction_hash.as_deref() == Some(&hash))
                        .expect("durable signed record must exist before submission");
                    let saved: crate::send::payload::PreparedSubmission = serde_json::from_str(
                        row.payload.signed_transaction_payload.as_ref().unwrap(),
                    )
                    .unwrap();
                    assert_eq!(saved.payload, raw);
                    assert!(saved.nonce.is_some());
                    if fault.swap(false, Ordering::SeqCst) {
                        // The node received the bytes, but its response cannot be read.
                        return ResponseTemplate::new(200).set_body_string("{");
                    }
                    json!(hash)
                }
                other => panic!("unexpected RPC {other}"),
            };
            ResponseTemplate::new(200)
                .set_body_json(json!({"jsonrpc":"2.0", "id":body["id"], "result":result}))
        })
        .mount(&server)
        .await;
    let mut request = super::tests::build_send_params_tests::req("ethereum", "Ethereum");
    request.to_address = "0x1111111111111111111111111111111111111111".into();
    request.evm_overrides = Some(EvmSendOverridesInput {
        gas_limit: Some(21_000),
        custom_fees: Some(EvmCustomFeeConfiguration {
            max_fee_per_gas_gwei: 2.0,
            max_priority_fee_per_gas_gwei: 1.0,
        }),
        ..Default::default()
    });
    let mut invalid = request.clone();
    invalid.to_address = "invalid".into();
    assert!(service.execute_send(invalid).await.is_err());
    assert!(
        service.transactions().await.unwrap().is_empty(),
        "no placeholder for signing failures"
    );
    assert!(service.execute_send(request.clone()).await.is_err());
    let first = service.transactions().await.unwrap().remove(0);
    assert_eq!(first.ethereum_nonce, Some(7));
    assert!(first.failure_reason.is_some());
    drop(service);
    let reopened = WalletService::new_typed(vec![ChainEndpoints {
        chain_id: "ethereum".into(),
        endpoints: vec![server.uri()],
        api_key: None,
    }])
    .unwrap();
    reopened.open_state(db.clone()).await.unwrap();
    reopened.set_secret_store(secrets);
    let next = reopened.execute_send(request.clone()).await.unwrap();
    assert_eq!(next.evm.unwrap().nonce, 8);
    let (a, b) = tokio::join!(
        reopened.execute_send(request.clone()),
        reopened.execute_send(request.clone())
    );
    let mut nonces = vec![a.unwrap().evm.unwrap().nonce, b.unwrap().evm.unwrap().nonce];
    nonces.sort();
    assert_eq!(nonces, vec![9, 10]);
    let hash = reopened
        .rebroadcast_transaction(first.id.clone())
        .await
        .unwrap();
    assert_eq!(Some(hash), first.transaction_hash.clone());
    assert_eq!(
        reopened.transactions().await.unwrap().len(),
        4,
        "rebroadcast reuses the existing record"
    );
    let mut competing = first.clone();
    competing.id = crate::store::new_transaction_id();
    let conflict = reopened
        .save_prepared_send_record(competing, true)
        .await
        .unwrap_err();
    assert!(
        conflict.to_string().contains("reserved by another send"),
        "{conflict}"
    );
    assert_eq!(reopened.transactions().await.unwrap().len(), 4);
    let mut old = first.clone();
    old.created_at = 0.0;
    reopened
        .apply_transaction_command(TransactionCommand::Upsert { records: vec![old] })
        .await
        .unwrap();
    for _ in 0..20 {
        reopened
            .record_status_poll(first.id.clone(), StatusPollOutcome::Failed)
            .await;
    }
    assert!(
        reopened
            .stale_pending_failure_ids("Ethereum".into())
            .await
            .unwrap()
            .is_empty(),
        "missing receipts cannot free an uncertain nonce"
    );

    let broadcasts = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.body_json::<Value>().unwrap()["method"] == "eth_sendRawTransaction")
        .count();
    rusqlite::Connection::open(&db).unwrap().execute_batch("CREATE TRIGGER reject_send BEFORE INSERT ON history_records BEGIN SELECT RAISE(ABORT, 'simulated disk failure'); END;").unwrap();
    let error = reopened.execute_send(request).await.unwrap_err();
    assert!(
        error.to_string().contains("simulated disk failure"),
        "{error}"
    );
    let after = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.body_json::<Value>().unwrap()["method"] == "eth_sendRawTransaction")
        .count();
    assert_eq!(
        after, broadcasts,
        "a failed journal write must prevent submission"
    );
}
