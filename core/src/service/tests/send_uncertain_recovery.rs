//! A real expired TON signature stays resolvable after a lost submission reply.
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

fn replace_strings(value: &mut Value, replacements: &[(String, String)]) {
    match value {
        Value::String(text) => {
            if let Some((_, replacement)) = replacements.iter().find(|(source, _)| source == text) {
                *text = replacement.clone();
            }
        }
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| replace_strings(value, replacements)),
        Value::Object(values) => values
            .values_mut()
            .for_each(|value| replace_strings(value, replacements)),
        _ => {}
    }
}

#[tokio::test]
async fn ton_prepared_expiry_matches_the_signed_deployment_and_active_wallet_message() {
    use crate::derivation::ton::TonWalletVersion;
    for (version, seqno) in TonWalletVersion::ALL
        .into_iter()
        .flat_map(|version| [(version, 0), (version, 7)])
    {
        let chain = Chain::Ton;
        let public = ed25519_dalek::SigningKey::from_bytes(&[1; 32])
            .verifying_key()
            .to_bytes();
        let sender = version.address(&public, chain).unwrap();
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(move |request: &Request| {
                let response = match request.url.path() {
                    "/getMasterchainInfo" => {
                        let (root, file) = chain.ton_zero_state().unwrap();
                        json!({"ok":true,"result":{"init":{"workchain":-1,"seqno":0,"root_hash":root,"file_hash":file}}})
                    }
                    "/getAddressBalance" => json!({"ok":true,"result":"1000000000"}),
                    "/getAddressInformation" => {
                        json!({"ok":true,"result":{"state":if seqno == 0 {"uninitialized"} else {"active"}}})
                    }
                    "/runGetMethod" => {
                        json!({"ok":true,"result":{"exit_code":0,"stack":[["num",format!("0x{seqno:x}")]]}})
                    }
                    other => panic!("Build and sign must never broadcast: {other}"),
                };
                ResponseTemplate::new(200).set_body_json(response)
            })
            .mount(&server)
            .await;
        let service = WalletService::new(vec![ChainEndpoints {
            chain_id: chain,
            capabilities: EndpointCapability::ALL.to_vec(),
            endpoints: vec![server.uri()],
        }])
        .unwrap();
        let secrets = Arc::new(crate::store::secret_backends::InMemorySecretStore::new());
        service.set_secret_store(secrets.clone());
        service
            .open_state(
                std::env::temp_dir()
                    .join(format!(
                        "ton-expiry-{}.sqlite",
                        crate::store::new_event_id()
                    ))
                    .to_string_lossy()
                    .into(),
            )
            .await
            .unwrap();
        service
            .apply_state_command(crate::store::state::StateCommand::UpsertWallet {
                wallet: crate::store::state::WalletState::single_address(
                    "w", "TON", chain, &sender, None, false,
                ),
            })
            .await
            .unwrap();
        crate::store::wallet_secrets::store_private_key(
            &*secrets,
            "w",
            &hex::encode([1; 32]),
            None,
        )
        .unwrap();
        let artifact = service
            .build_send(crate::send::SendExecutionRequest {
                chain_id: chain,
                wallet_id: "w".into(),
                password: None,
                to_address: format!("0:{}", "33".repeat(32)),
                amount_str: "0.05".into(),
                contract_address: None,
                token_standard: None,
                token_decimals: None,
                fee_rate_svb: None,
                fee_sat: None,
                gas_budget: None,
                fee_amount: None,
                evm_overrides: None,
                sign_only: false,
                memo: None,
            })
            .await
            .unwrap();
        let details: Value = serde_json::from_str(&artifact.prepared_details).unwrap();
        let declared = details["Ton"]["valid_until"].as_u64().unwrap() as u32;
        if seqno == 0 {
            assert_eq!(declared, u32::MAX);
        } else {
            assert!(
                (crate::store::now_unix() as u32 + 30..=crate::store::now_unix() as u32 + 60)
                    .contains(&declared)
            );
        }
        let signed = service
            .sign_send(artifact.id, artifact.review_digest, None)
            .await
            .unwrap();
        let payload: Value = serde_json::from_str(signed.signed_payload.as_ref().unwrap()).unwrap();
        let boc = STANDARD
            .decode(payload["boc_b64"].as_str().unwrap())
            .unwrap();
        // Independently read the emitted BOC's signed body, including its
        // expiry bytes; comparing two calls to the builder could hide an override.
        assert_eq!(&boc[..6], &[0xb5, 0xee, 0x9c, 0x72, 2, 4]);
        let count = u16::from_be_bytes(boc[6..8].try_into().unwrap());
        let mut offset = 18;
        let mut cells = Vec::new();
        for _ in 0..count {
            let refs = usize::from(boc[offset] & 7);
            let length = usize::from(boc[offset + 1]).div_ceil(2);
            let start = offset + 2;
            let data = &boc[start..start + length];
            let children: Vec<_> = boc[start + length..start + length + refs * 2]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|bytes| usize::from(u16::from_be_bytes(*bytes)))
                .collect();
            cells.push((data, children));
            offset = start + length + refs * 2;
        }
        let body = cells[*cells[0].1.last().unwrap()].0;
        let word = |at: usize| u32::from_be_bytes(body[at..at + 4].try_into().unwrap());
        // The wallet's own layout: v4R2 signs in front of its wallet id; W5
        // opens with the signed-external opcode and signs at the tail.
        let expiry_at = match version {
            TonWalletVersion::V4R2 => 68,
            TonWalletVersion::W5 => {
                assert_eq!(word(0), 0x7369_676e);
                assert_eq!(word(4), version.wallet_id(chain).unwrap());
                8
            }
        };
        assert_eq!(word(expiry_at), declared, "{version:?}");
        assert_eq!(word(expiry_at + 4), seqno, "{version:?}");
    }
}

#[tokio::test]
async fn expired_uncertain_submission_queries_execution_before_refusing_rebroadcast() {
    for status in ["pending", "confirmed", "failed"] {
        let chain = Chain::Ton;
        let public = ed25519_dalek::SigningKey::from_bytes(&[1; 32])
            .verifying_key()
            .to_bytes();
        let version = crate::derivation::ton::TonWalletVersion::default();
        let sender = version.address(&public, chain).unwrap();
        let parsed = crate::derivation::ton::parse_ton_address(&sender).unwrap();
        let owner_raw = format!("{}:{}", parsed.workchain, hex::encode(parsed.account_id));
        let recipient = format!("0:{}", "33".repeat(32));
        let expiry = crate::store::now_unix() as u32 - 60;
        let raw = crate::send::ton::build_transfer_for_address(
            &crate::send::ton::TonSigner {
                version,
                chain,
                private_key: &[1; 32],
                public_key: &public,
            },
            crate::derivation::ton::parse_ton_address(&recipient).unwrap(),
            50_000_000,
            7,
            None,
            expiry,
            3,
        )
        .unwrap();
        let hash = STANDARD.encode(crate::derivation::ton::boc_root_hash(&raw).unwrap());
        let payload = json!({"boc_b64":STANDARD.encode(&raw)}).to_string();
        let mut trace: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/ton-status-v3.json"))
                .unwrap();
        let action = &trace["traces"][0]["actions"][0]["details"];
        let replacements = vec![
            (action["sender"].as_str().unwrap().into(), owner_raw),
            (
                action["sender_jetton_wallet"].as_str().unwrap().into(),
                recipient.clone(),
            ),
            (
                trace["traces"][0]["external_hash"].as_str().unwrap().into(),
                hash.clone(),
            ),
        ];
        replace_strings(&mut trace, &replacements);
        if status == "pending" {
            trace = json!({"traces":[]});
        }
        if status == "failed" {
            let root = trace["traces"][0]["trace"]["tx_hash"]
                .as_str()
                .unwrap()
                .to_string();
            trace["traces"][0]["transactions"][root]["description"]["aborted"] = json!(true);
        }
        let server = MockServer::start().await;
        Mock::given(any()).respond_with(move |request: &Request| {
            let response = match request.url.path() {
                "/getMasterchainInfo" => { let (root,file)=chain.ton_zero_state().unwrap(); json!({"ok":true,"result":{"init":{"workchain":-1,"seqno":0,"root_hash":root,"file_hash":file}}}) },
                "/v3/masterchainInfo" => { let (id,root,file)=chain.ton_first_block().unwrap(); json!({"first":{"workchain":-1,"shard":"8000000000000000","seqno":1,"global_id":id,"root_hash":root,"file_hash":file}}) },
                "/v3/traces" => trace.clone(),
                other => panic!("Expired recovery must never submit or read fresh funds: {other}"),
            };
            ResponseTemplate::new(200).set_body_json(response)
        }).mount(&server).await;
        let service = WalletService::new(vec![ChainEndpoints {
            chain_id: chain,
            capabilities: EndpointCapability::ALL.to_vec(),
            endpoints: vec![server.uri()],
        }])
        .unwrap();
        let path = std::env::temp_dir().join(format!(
            "ton-expired-recovery-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(path.to_string_lossy().into())
            .await
            .unwrap();
        service
            .apply_state_command(crate::store::state::StateCommand::UpsertWallet {
                wallet: crate::store::state::WalletState::single_address(
                    "w", "TON", chain, &sender, None, false,
                ),
            })
            .await
            .unwrap();
        service
            .apply_state_command(crate::store::state::StateCommand::SetAppSetting {
                update: crate::store::state::AppSettingUpdate::AddCustomEndpoint {
                    chain_id: chain,
                    api: "toncenter-v3".into(),
                    endpoint: format!("{}/v3", server.uri()),
                    capabilities: vec![EndpointCapability::Verification],
                },
            })
            .await
            .unwrap();
        let request = crate::send::SendExecutionRequest {
            chain_id: chain,
            wallet_id: "w".into(),
            password: None,
            to_address: recipient.clone(),
            amount_str: "0.05".into(),
            contract_address: None,
            token_standard: None,
            token_decimals: None,
            fee_rate_svb: None,
            fee_sat: None,
            gas_budget: None,
            fee_amount: Some("0.007".into()),
            evm_overrides: None,
            sign_only: false,
            memo: None,
        };
        let prepared = PreparedPayload::Ton {
            seqno: 7,
            amount: 50_000_000,
            valid_until: expiry,
            jetton: None,
        };
        let id = crate::store::new_event_id();
        let mut stored = StoredSend {
            view: SendArtifact {
                id: id.clone(),
                revision: 0,
                stage: SendStage::Signed,
                wallet_id: "w".into(),
                chain_id: chain,
                sender: sender.clone(),
                recipient,
                amount: "0.05".into(),
                asset: "TON".into(),
                symbol: "TON".into(),
                staking: None,
                operation: None,
                created_at: f64::from(expiry) - 60.0,
                review_digest: String::new(),
                review: SendArtifactReview::default(),
                prepared_details: serde_json::to_string_pretty(&prepared).unwrap(),
                signing_payload_hex: String::new(),
                signed_payload: Some(payload.clone()),
                transaction_hash: Some(hash.clone()),
                attempts: vec![BroadcastAttempt {
                    endpoint: server.uri(),
                    attempted_at: f64::from(expiry) - 30.0,
                    outcome: SubmissionOutcome::Uncertain,
                    transaction_hash: None,
                    detail: "Response lost".into(),
                }],
                selected_endpoints: vec![server.uri()],
                memo: None,
            },
            request: request.clone(),
            prepared,
            submission: Some(crate::send::payload::PreparedSubmission {
                payload,
                result_field: "message_hash".into(),
                transaction_hash: Some(hash.clone()),
                nonce: Some(7),
            }),
            signed_digest: None,
            substrate_verified_through: None,
            icp_staking_receipts: vec![],
        };
        stored.view.review_digest = stored.digest().unwrap();
        stored.signed_digest = stored.submission_digest().unwrap();
        service.save_send_artifact(&stored, vec![]).await.unwrap();
        let mut record = service
            .begin_send_record(chain, &request, &sender)
            .await
            .unwrap();
        record.id = id.clone();
        record.transaction_hash = Some(hash);
        record.nonce = Some(7);
        service.save_send_record(record).await.unwrap();
        let error = service
            .broadcast_send(id.clone(), vec![server.uri()])
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains(if status == "pending" {
                "expired"
            } else if status == "confirmed" {
                "already confirmed"
            } else {
                "execution failed"
            }),
            "{status}: {error}"
        );
        let record = service
            .fetch_all_history_records()
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.id == id)
            .unwrap()
            .payload;
        assert_eq!(
            record.status,
            match status {
                "pending" => TransactionStatus::Pending,
                "confirmed" => TransactionStatus::Confirmed,
                _ => TransactionStatus::Failed,
            }
        );
        assert_eq!(
            service
                .load_send_artifact(id)
                .await
                .unwrap()
                .view
                .signed_payload,
            stored.view.signed_payload
        );
        let requests = server.received_requests().await.unwrap();
        assert!(
            requests
                .iter()
                .any(|request| request.url.path() == "/v3/traces")
        );
        assert!(requests.iter().all(|request| request.method == "GET"));
    }
}
