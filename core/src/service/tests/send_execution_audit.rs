//! Real stored mnemonic -> identity -> params -> signer -> mock submission.
use super::*;
use crate::store::{
    secret_backends::InMemorySecretStore, state::WalletState, wallet_secrets::store_seed_phrase,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

#[tokio::test]
async fn audit_stored_wallets_reach_solana_sui_aptos_and_tron_submission() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/send-audit-vectors.json"
    ))
    .unwrap();
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
        let transaction_hash = Arc::new(std::sync::Mutex::new(None::<String>));
        let response_hash = transaction_hash.clone();
        let wrong_response = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let response_is_wrong = wrong_response.clone();
        Mock::given(any()).respond_with(move |request: &Request| {
            let body:Value=serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            let path=request.url.path();
            let result = match body["method"].as_str().unwrap_or(path) {
                "getGenesisHash" => json!(Chain::Solana.solana_genesis_hash().unwrap()),
                "getAccountInfo" => {
                    // The mint, the owner's funded account, and no account
                    // yet for the recipient.
                    let program = if token2022 {"TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"} else {"TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"};
                    let mint = bs58::encode([0x44; 32]).into_string();
                    let owner = v["solana"]["address"].as_str().unwrap();
                    let source = crate::send::solana::derive_associated_token_account(
                        &crate::derivation::solana::decode_b58_32(owner).unwrap(),
                        &[0x44; 32],
                        &crate::derivation::solana::decode_b58_32(program).unwrap(),
                    ).unwrap();
                    let address = body["params"][0].as_str().unwrap();
                    if address == mint {
                        json!({"value":{"owner":program,"data":{"parsed":{"type":"mint","info":{"isInitialized":true,"decimals":6,"extensions":[]}}}}})
                    } else if address == bs58::encode(source).into_string() {
                        json!({"value":{"owner":program,"data":{"parsed":{"type":"account","info":{"mint":mint,"owner":owner,"state":"initialized","tokenAmount":{"amount":"1000000000","decimals":6}}}}}})
                    } else {
                        json!({"value":null})
                    }
                },
                "isBlockhashValid" => json!({"value":true}),
                "getLatestBlockhash" => json!({"value":{"blockhash":v["solana"]["blockhash"]}}),
                "sendTransaction" => {
                    let tx=STANDARD.decode(body["params"][0].as_str().unwrap()).unwrap();
                    if token {
                        let program = crate::derivation::solana::decode_b58_32(if token2022 {"TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"} else {"TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"}).unwrap();
                        assert!(tx[69..69 + tx[68] as usize * 32].as_chunks::<32>().0.contains(&program));
                    }
                    let public:[u8;32]=tx[69..101].try_into().unwrap();
                    ed25519_dalek::VerifyingKey::from_bytes(&public).unwrap().verify_strict(&tx[65..],&ed25519_dalek::Signature::from_slice(&tx[1..65]).unwrap()).unwrap();
                    json!(bs58::encode(&tx[1..65]).into_string())
                },
                "suix_getReferenceGasPrice" => json!("1000"),
                "sui_getChainIdentifier" => json!(Chain::Sui.sui_network_identity().unwrap().0),
                "sui_getCheckpoint" => {
                    assert_eq!(body["params"],json!(["0"]));
                    json!({"sequenceNumber":"0","digest":Chain::Sui.sui_network_identity().unwrap().1})
                },
                "suix_getCoins" => json!({"data":[{"coinObjectId":format!("0x{}","33".repeat(32)),"version":"7","digest":"11111111111111111111111111111111","balance":"200000000"}],"hasNextPage":false,"nextCursor":null}),
                "sui_executeTransactionBlock" => {
                    let bytes=STANDARD.decode(body["params"][0].as_str().unwrap()).unwrap();
                    assert_eq!(hex::encode(&bytes),v["sui"]["raw"]);
                    assert_eq!(body["params"][1][0],v["sui"]["signature"]);
                    let digest = if response_is_wrong.load(std::sync::atomic::Ordering::Relaxed) {
                        "11111111111111111111111111111111".to_string()
                    } else {
                        response_hash.lock().unwrap().clone().unwrap()
                    };
                    json!({"digest":digest,"effects":{"status":{"status":"success"}}})
                },
                "/" => json!({"chain_id":1,"ledger_version":"1"}),
                "/estimate_gas_price" => json!({"gas_estimate":100}),
                "/view" => json!(["10000000000"]),
                "/transactions" => {
                    assert_eq!(body["sender"],v["aptos"]["address"]);
                    assert_eq!(body["signature"]["public_key"],format!("0x{}",v["aptos"]["public_key"].as_str().unwrap()));
                    let hash = if response_is_wrong.load(std::sync::atomic::Ordering::Relaxed) {
                        format!("0x{}","ab".repeat(32))
                    } else {
                        response_hash.lock().unwrap().clone().unwrap()
                    };
                    json!({"hash":hash})
                },
                "/wallet/getnowblock" => json!({"blockID":format!("0000000000000007{}","33".repeat(24)),"block_header":{"raw_data":{"number":7}}}),
                // An account that never set its permissions: its own key.
                "/wallet/getaccount" => json!({"address":body["address"],"balance":10_000_000_000u64}),
                "/wallet/getblockbynum" => {
                    assert_eq!(body,json!({"num":0}));
                    json!({"blockID":Chain::Tron.tron_genesis_block_id().unwrap()})
                },
                "/wallet/broadcasttransaction" => {
                    assert_eq!(body["raw_data"]["contract"][0]["parameter"]["value"]["owner_address"],v["tron"]["transactions"][0]["raw_data"]["contract"][0]["parameter"]["value"]["owner_address"]);
                    json!({"result":true})
                },
                "/wallet/triggerconstantcontract" => {
                    let result=if body["function_selector"]=="decimals()" {format!("{:064x}",6)} else {format!("{:064x}{:064x}{:0<64}",32,4,hex::encode("TEST"))};
                    json!({"result":{"result":true},"constant_result":[result]})
                },
                p if p.starts_with("/accounts/") => json!({"sequence_number":"7"}),
                p if p.starts_with("/transactions/by_hash/") => {
                    let hash = response_hash.lock().unwrap().clone().unwrap();
                    assert_eq!(p,format!("/transactions/by_hash/{hash}"));
                    json!({"hash":hash,"type":"pending_transaction"})
                },
                other => panic!("unexpected provider request: {other} ({body})"),
            };
            ResponseTemplate::new(200).set_body_json(if body["method"].is_string(){json!({"jsonrpc":"2.0","id":body["id"],"result":result})}else{result})
        }).mount(&server).await;
        let service = WalletService::new(vec![ChainEndpoints {
            capabilities: EndpointCapability::ALL.to_vec(),
            chain_id: chain,
            endpoints: vec![server.uri()],
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
                wallet: WalletState::single_address(
                    "w",
                    "Test",
                    chain,
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
            token_standard: None,
            wallet_id: "w".into(),
            chain_id: chain,
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
            sign_only: false,
            memo: None,
        };
        let prepared = service.build_send(request).await.unwrap();
        let signed = service
            .sign_send(prepared.id.clone(), prepared.review_digest, None)
            .await
            .unwrap_or_else(|e| panic!("{chain:?}: {e}"));
        *transaction_hash.lock().unwrap() = signed.transaction_hash.clone();
        if chain == Chain::Sui {
            assert_eq!(
                signed.transaction_hash.as_deref(),
                fixture["sui"]["transaction_digest"].as_str()
            );
        }
        assert!(server.received_requests().await.unwrap().iter().all(|r| {
            let text = String::from_utf8_lossy(&r.body);
            !text.contains("sendTransaction")
                && !text.contains("executeTransactionBlock")
                && !r.url.path().ends_with("/transactions")
                && !r.url.path().contains("broadcasttransaction")
        }));
        if matches!(chain, Chain::Sui | Chain::Aptos) {
            assert!(signed.transaction_hash.is_some());
            wrong_response.store(true, std::sync::atomic::Ordering::Relaxed);
            let uncertain = service
                .broadcast_send(signed.id.clone(), vec![server.uri()])
                .await
                .unwrap();
            assert_eq!(
                uncertain.attempts[0].outcome,
                crate::send::stages::SubmissionOutcome::Uncertain
            );
            assert_eq!(uncertain.transaction_hash, signed.transaction_hash);
            let history = service.fetch_all_history_records().await.unwrap();
            assert_eq!(history.len(), 1);
            assert_eq!(
                history[0].payload.status,
                crate::store::wallet_domain::TransactionStatus::Pending
            );
            assert_eq!(history[0].payload.transaction_hash, signed.transaction_hash);
            wrong_response.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        let result = service
            .broadcast_send(signed.id.clone(), vec![server.uri()])
            .await
            .unwrap();
        assert_eq!(
            result.attempts.last().unwrap().outcome,
            crate::send::stages::SubmissionOutcome::Accepted
        );
        assert!(
            result
                .transaction_hash
                .as_ref()
                .is_some_and(|hash| !hash.is_empty())
        );
        assert_eq!(
            result.transaction_hash,
            result.attempts.last().unwrap().transaction_hash
        );
        let history = service.fetch_all_history_records().await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].payload.transaction_hash, result.transaction_hash);
        let reopened = WalletService::new(vec![]).unwrap();
        reopened
            .open_state(db.to_string_lossy().into())
            .await
            .unwrap();
        let restored = reopened.inspect_send(signed.id).await.unwrap();
        assert_eq!(restored.signed_payload, signed.signed_payload);
        assert_eq!(restored.review_digest, signed.review_digest);
        assert_eq!(restored.transaction_hash, result.transaction_hash);
        assert_eq!(
            reopened.fetch_all_history_records().await.unwrap()[0]
                .payload
                .transaction_hash,
            result.transaction_hash
        );
        let requests = server.received_requests().await.unwrap();
        assert!(
            !requests
                .iter()
                .any(|r| r.url.path().contains("encode_submission")
                    || r.url.path().contains("createtransaction")
                    || r.url.path().contains("triggersmartcontract")
                    || String::from_utf8_lossy(&r.body).contains("unsafe_transferSui"))
        );
    }
}

#[tokio::test]
async fn aptos_build_and_sign_bind_the_reviewed_total_gas_budget() {
    use std::sync::atomic::{AtomicU64, Ordering};

    let fixture: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/send-audit-vectors.json"
    ))
    .unwrap();
    let address = fixture["aptos"]["address"].as_str().unwrap();
    let destination = format!("0x{}", "22".repeat(32));
    for chain in [Chain::Aptos, Chain::AptosTestnet] {
        let price = Arc::new(AtomicU64::new(100));
        let balance = Arc::new(AtomicU64::new(100_000_000));
        let server = MockServer::start().await;
        let live_price = price.clone();
        let live_balance = balance.clone();
        Mock::given(any())
            .respond_with(move |request: &Request| {
                let response = match request.url.path() {
                    "/" => json!({"chain_id":chain.aptos_chain_id().unwrap(),"ledger_version":"1"}),
                    "/estimate_gas_price" => {
                        json!({"gas_estimate":live_price.load(Ordering::Relaxed)})
                    }
                    "/view" => json!([live_balance.load(Ordering::Relaxed).to_string()]),
                    path if path.starts_with("/accounts/") => json!({"sequence_number":"7"}),
                    other => panic!("unexpected Aptos provider request: {other}"),
                };
                ResponseTemplate::new(200).set_body_json(response)
            })
            .mount(&server)
            .await;
        let service = WalletService::new(vec![ChainEndpoints {
            capabilities: EndpointCapability::ALL.to_vec(),
            chain_id: chain,
            endpoints: vec![server.uri()],
        }])
        .unwrap();
        let db = std::env::temp_dir().join(format!(
            "aptos-gas-review-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(db.to_string_lossy().into())
            .await
            .unwrap();
        let secrets = Arc::new(InMemorySecretStore::new());
        service.set_secret_store(secrets.clone());
        let mut wallet = WalletState::single_address(
            "w",
            "APT",
            chain,
            address,
            Some("m/44'/637'/0'/0'/0'".into()),
            false,
        );
        let mut holding = chain.native_holding_template();
        holding.amount = "1".into();
        wallet.holdings = vec![holding];
        service
            .apply_state_command(StateCommand::UpsertWallet { wallet })
            .await
            .unwrap();
        store_seed_phrase(&*secrets, "w", fixture["mnemonic"].as_str().unwrap(), None).unwrap();
        let quote = service
            .quote_owned_send(
                "w".into(),
                format!("{}:native", chain.str_id()),
                "0.99".into(),
                destination.clone(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(quote.request.fee_amount.as_deref(), Some("0.01"));

        // A changed provider quote cannot silently increase a reviewed transaction's fee.
        price.store(500, Ordering::Relaxed);
        balance.store(99_999_999, Ordering::Relaxed);
        assert!(
            service
                .build_send(quote.request.clone())
                .await
                .unwrap_err()
                .to_string()
                .contains("Insufficient")
        );
        assert!(service.list_sends().await.unwrap().is_empty());
        balance.store(100_000_000, Ordering::Relaxed);
        for fee in ["0", "-1", "NaN", "0.01000001"] {
            let mut invalid = quote.request.clone();
            invalid.fee_amount = Some(fee.into());
            assert!(service.build_send(invalid).await.is_err(), "{fee}");
        }
        let mut direct = quote.request.clone();
        direct.fee_amount = None;
        assert!(
            service
                .build_send(direct)
                .await
                .unwrap_err()
                .to_string()
                .contains("Insufficient")
        );
        assert!(service.list_sends().await.unwrap().is_empty());

        let prepared = service.build_send(quote.request).await.unwrap();
        let details: Value = serde_json::from_str(&prepared.prepared_details).unwrap();
        assert_eq!(details["Aptos"]["body"]["max_gas_amount"], "10000");
        assert_eq!(details["Aptos"]["body"]["gas_unit_price"], "100");
        let signed = service
            .sign_send(prepared.id, prepared.review_digest, None)
            .await
            .unwrap();
        let outer: Value = serde_json::from_str(signed.signed_payload.as_deref().unwrap()).unwrap();
        let body: Value =
            serde_json::from_str(outer["signed_body_json"].as_str().unwrap()).unwrap();
        assert_eq!(body["max_gas_amount"], "10000");
        assert_eq!(body["gas_unit_price"], "100");
        assert_eq!(body["payload"]["arguments"][1], "99000000");
    }
}

#[tokio::test]
async fn nonce_journal_survives_response_loss_restart_and_concurrent_sends() {
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
    let service = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: crate::registry::Chain::Ethereum,
        endpoints: vec![server.uri()],
    }])
    .unwrap();
    service.open_state(db.clone()).await.unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    service.set_secret_store(secrets.clone());
    service
        .apply_state_command(StateCommand::UpsertWallet {
            wallet: WalletState::single_address(
                "w",
                "W",
                crate::registry::Chain::Ethereum,
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
                "eth_chainId" => json!("0x1"),
                "eth_getBalance" => json!("0x56bc75e2d63100000"),
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
                    let records = crate::wallet_db::history_fetch_all(
                        &crate::wallet_db::WalletDatabase::new(&database),
                    )
                    .unwrap();
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
    let mut request = super::tests::request_fixture::req(crate::registry::Chain::Ethereum);
    request.to_address = "0x1111111111111111111111111111111111111111".into();
    request.evm_overrides = Some(EvmSendOverridesInput {
        gas_limit: Some(21_000),
        custom_fees: Some(EvmCustomFeeConfiguration {
            max_fee_per_gas_gwei: "2".into(),
            max_priority_fee_per_gas_gwei: "1".into(),
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
    assert!(
        !lose_response.load(Ordering::SeqCst),
        "the failed send must reach the simulated lost broadcast response"
    );
    let first = service.transactions().await.unwrap().remove(0);
    assert_eq!(first.nonce, Some(7));
    assert!(first.failure_reason.is_some());
    drop(service);
    let reopened = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: crate::registry::Chain::Ethereum,
        endpoints: vec![server.uri()],
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
    old.created_at_unix = 0.0;
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
            .apply_resolved_pending_statuses(crate::registry::Chain::Ethereum, vec![])
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

#[cfg(test)]
mod signed_world_chain_fee_budget {
    use super::*;
    use crate::send::ethereum::{EvmCustomFeeConfiguration, EvmSendOverridesInput};
    use crate::send::stages::{SendStage, SubmissionOutcome};
    use sha3::Digest;
    use std::sync::atomic::{AtomicU64, Ordering};

    async fn broadcasts(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| {
                let body: Value = request.body_json().unwrap();
                body["method"] == "eth_sendRawTransaction"
            })
            .count()
    }

    async fn snapshot(service: &WalletService, id: &str) -> (Value, Value) {
        (
            serde_json::to_value(service.load_send_artifact(id.into()).await.unwrap()).unwrap(),
            serde_json::to_value(service.transactions().await.unwrap()).unwrap(),
        )
    }

    #[tokio::test]
    async fn increased_oracle_budget_refuses_first_broadcast_but_replays_submitted_bytes() {
        let chain = Chain::WorldChain;
        let server = MockServer::start().await;
        let l1_fee = Arc::new(AtomicU64::new(100_000));
        let operator_fee = Arc::new(AtomicU64::new(200_000));
        let l1_quote = l1_fee.clone();
        let operator_quote = operator_fee.clone();
        Mock::given(any())
            .respond_with(move |request: &Request| {
                let body: Value = request.body_json().unwrap();
                let result = match body["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x1e0"),
                    "eth_getTransactionCount" => json!("0x7"),
                    "eth_getBalance" => json!(format!("0x{:x}", 100_000_000_000_000_000_000_u128)),
                    "eth_getCode" => json!("0x"),
                    "eth_call" => {
                        assert_eq!(
                            body["params"][0]["to"].as_str().unwrap().to_lowercase(),
                            "0x420000000000000000000000000000000000000f"
                        );
                        let data = body["params"][0]["data"].as_str().unwrap();
                        let amount = match &data[2..10] {
                            "f1c7a58b" => l1_quote.load(Ordering::SeqCst),
                            "275aedd2" => operator_quote.load(Ordering::SeqCst),
                            other => panic!("unexpected oracle selector {other}"),
                        };
                        json!(format!("0x{amount:064x}"))
                    }
                    "eth_sendRawTransaction" => {
                        let raw = body["params"][0].as_str().unwrap();
                        json!(format!(
                            "0x{}",
                            hex::encode(sha3::Keccak256::digest(hex::decode(&raw[2..]).unwrap()))
                        ))
                    }
                    other => panic!("unexpected RPC {other}"),
                };
                ResponseTemplate::new(200)
                    .set_body_json(json!({"jsonrpc":"2.0", "id":body["id"], "result":result}))
            })
            .mount(&server)
            .await;
        let service = WalletService::new(vec![ChainEndpoints {
            capabilities: EndpointCapability::ALL.to_vec(),
            chain_id: chain,
            endpoints: vec![server.uri()],
        }])
        .unwrap();
        let db = std::env::temp_dir().join(format!(
            "world-signed-fee-budget-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(db.to_string_lossy().into())
            .await
            .unwrap();
        let secrets = Arc::new(InMemorySecretStore::new());
        service.set_secret_store(secrets.clone());
        service
            .apply_state_command(StateCommand::UpsertWallet {
                wallet: WalletState::single_address(
                    "w",
                    "World Chain",
                    chain,
                    "0x9858EfFD232B4033E47d90003D41EC34EcaEda94",
                    Some("m/44'/60'/0'/0/0".into()),
                    false,
                ),
            })
            .await
            .unwrap();
        store_seed_phrase(
            &*secrets,
            "w",
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            None,
        )
        .unwrap();
        let mut request = super::super::tests::request_fixture::req(chain);
        request.to_address = "0x1111111111111111111111111111111111111111".into();
        request.evm_overrides = Some(EvmSendOverridesInput {
            nonce: Some(7),
            gas_limit: Some(21_000),
            custom_fees: Some(EvmCustomFeeConfiguration {
                max_fee_per_gas_gwei: "2".into(),
                max_priority_fee_per_gas_gwei: "1".into(),
            }),
            ..Default::default()
        });
        let prepared = service.build_send(request).await.unwrap();
        let signed = service
            .sign_send(prepared.id, prepared.review_digest, None)
            .await
            .unwrap();
        assert_eq!(signed.stage, SendStage::Signed);
        assert!(signed.attempts.is_empty());
        assert!(service.transactions().await.unwrap().is_empty());
        let signed_snapshot = snapshot(&service, &signed.id).await;

        l1_fee.store(100_001, Ordering::SeqCst);
        let error = service
            .broadcast_send(signed.id.clone(), vec![server.uri()])
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Network fee changed"), "{error}");
        assert_eq!(snapshot(&service, &signed.id).await, signed_snapshot);
        assert_eq!(broadcasts(&server).await, 0);

        l1_fee.store(100_000, Ordering::SeqCst);
        let submitted = service
            .broadcast_send(signed.id.clone(), vec![server.uri()])
            .await
            .unwrap();
        assert_eq!(submitted.attempts.len(), 1);
        assert_eq!(submitted.attempts[0].outcome, SubmissionOutcome::Accepted);
        assert_eq!(service.transactions().await.unwrap().len(), 1);
        assert_eq!(broadcasts(&server).await, 1);
        let signed_payload = submitted.signed_payload.clone();
        let signed_hash = submitted.transaction_hash.clone();

        operator_fee.store(200_001, Ordering::SeqCst);
        service
            .rebroadcast_transaction(signed.id.clone())
            .await
            .unwrap();
        let replayed = service.inspect_send(signed.id.clone()).await.unwrap();
        assert_eq!(replayed.signed_payload, signed_payload);
        assert_eq!(replayed.transaction_hash, signed_hash);
        assert_eq!(replayed.attempts.len(), 2);
        assert_eq!(broadcasts(&server).await, 2);
    }
}

#[cfg(test)]
mod owned_world_chain_nonce_rollover {
    use super::*;
    use crate::send::ethereum::EvmSendOverridesInput;
    use crate::send::stages::SendStage;
    use crate::store::wallet_domain::AssetHolding;

    #[tokio::test]
    async fn signed_nonce_127_is_reserved_before_preview_prices_nonce_128_rlp() {
        let chain = Chain::WorldChain;
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(|request: &Request| {
                let body: Value = request.body_json().unwrap();
                let answer = |call: &Value| {
                    let result = match call["method"].as_str().unwrap() {
                        "eth_chainId" => json!("0x1e0"),
                        "eth_getTransactionCount" => json!("0x7f"),
                        "eth_getBalance" => json!("0xde0b6b3a7640000"),
                        "eth_estimateGas" => json!("0x5208"),
                        "eth_getCode" => json!("0x"),
                        "eth_feeHistory" => json!({
                            "baseFeePerGas": ["0x3b9aca00"],
                            "reward": [["0x77359400"]]
                        }),
                        "eth_call" => {
                            assert_eq!(
                                call["params"][0]["to"].as_str().unwrap().to_lowercase(),
                                "0x420000000000000000000000000000000000000f"
                            );
                            assert_eq!(call["params"][1], "latest");
                            let data = call["params"][0]["data"].as_str().unwrap();
                            let argument = u64::from_str_radix(&data[10..], 16).unwrap();
                            let fee = match &data[2..10] {
                                "f1c7a58b" => argument * 1_000,
                                "275aedd2" => {
                                    assert_eq!(argument, 21_000);
                                    1_000
                                }
                                selector => panic!("unexpected oracle selector {selector}"),
                            };
                            json!(format!("0x{fee:064x}"))
                        }
                        method => panic!("unexpected RPC {method}"),
                    };
                    json!({"jsonrpc":"2.0", "id":call["id"], "result":result})
                };
                ResponseTemplate::new(200).set_body_json(match body.as_array() {
                    Some(batch) => json!(batch.iter().map(answer).collect::<Vec<_>>()),
                    None => answer(&body),
                })
            })
            .mount(&server)
            .await;
        let service = WalletService::new(vec![ChainEndpoints {
            capabilities: EndpointCapability::ALL.to_vec(),
            chain_id: chain,
            endpoints: vec![server.uri()],
        }])
        .unwrap();
        let db = std::env::temp_dir().join(format!(
            "owned-preview-rollover-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(db.to_string_lossy().into())
            .await
            .unwrap();
        let secrets = Arc::new(InMemorySecretStore::new());
        service.set_secret_store(secrets.clone());
        let mut wallet = WalletState::single_address(
            "w",
            "World",
            chain,
            "0x9858effd232b4033e47d90003d41ec34ecaeda94",
            Some("m/44'/60'/0'/0/0".into()),
            false,
        );
        wallet.holdings.push(AssetHolding {
            id: String::new(),
            name: "Ether".into(),
            symbol: "ETH".into(),
            coingecko_id: "ethereum".into(),
            chain_id: chain,
            token_standard: "Native".into(),
            contract_address: None,
            amount: "1".into(),
        });
        let holding_key = wallet.holdings[0].deployment_id();
        service
            .apply_state_command(StateCommand::UpsertWallet { wallet })
            .await
            .unwrap();
        store_seed_phrase(
            &*secrets, "w",
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            None,
        ).unwrap();
        let destination = format!("0x{}", "22".repeat(20));
        let mut request = super::super::tests::request_fixture::req(chain);
        request.to_address = destination.clone();
        request.amount_str = "0.001".into();
        request.evm_overrides = Some(EvmSendOverridesInput {
            nonce: Some(127),
            ..Default::default()
        });
        let prepared = service.build_send(request.clone()).await.unwrap();
        let size_at_127 = hex::decode(&prepared.signing_payload_hex).unwrap().len();
        let signed = service
            .sign_send(prepared.id, prepared.review_digest, None)
            .await
            .unwrap();
        assert_eq!(signed.stage, SendStage::Signed);
        assert!(service.transactions().await.unwrap().is_empty());

        let preview = service
            .preview_owned_evm_send(
                "w".into(),
                holding_key,
                "0.001".into(),
                destination,
                None,
                None,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(preview.nonce, 128);
        assert_eq!(preview.gasLimit, 21_000);
        let preview_requests = server.received_requests().await.unwrap();
        let oracle_sizes = preview_requests
            .iter()
            .flat_map(|request| {
                let body: Value = request.body_json().unwrap();
                body.as_array().cloned().unwrap_or_else(|| vec![body])
            })
            .filter_map(|call| {
                let data = call["params"][0]["data"].as_str()?;
                data.strip_prefix("0xf1c7a58b")
                    .map(|argument| usize::from_str_radix(argument, 16).unwrap())
            })
            .collect::<Vec<_>>();
        assert_eq!(oracle_sizes.last().copied(), Some(size_at_127 + 1));
        assert_eq!(
            preview.estimatedNetworkFee,
            crate::decimal::from_units(
                21_000 * 4_000_000_000_u128 + (size_at_127 as u128 + 1) * 1_000 + 1_000,
                18,
            )
        );

        request.evm_overrides = None;
        let next_prepared = service.build_send(request).await.unwrap();
        assert_eq!(
            hex::decode(&next_prepared.signing_payload_hex)
                .unwrap()
                .len(),
            size_at_127 + 1
        );
        let unsigned = hex::decode(&next_prepared.signing_payload_hex).unwrap();
        let mut fields = &unsigned[1..];
        assert!(alloy_rlp::Header::decode(&mut fields).unwrap().list);
        assert_eq!(
            <u64 as alloy_rlp::Decodable>::decode(&mut fields).unwrap(),
            480
        );
        assert_eq!(
            <u64 as alloy_rlp::Decodable>::decode(&mut fields).unwrap(),
            128
        );
    }
}
