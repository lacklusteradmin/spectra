use super::*;
use crate::send::{SendExecutionRequest, stages::SendStage};
use crate::store::secret_backends::InMemorySecretStore;
use crate::store::state::{WalletSigning, WalletState};
use crate::store::wallet_secrets::store_seed_phrase;
use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness};
use serde_json::json;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

struct TestDirectory(std::path::PathBuf);
impl TestDirectory {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn address(chain: Chain, path: &str) -> String {
    crate::derivation::dispatch::derive_for_chain(
        chain, SEED, path, None, None, None, true, false, false,
    )
    .unwrap()
    .address
    .unwrap()
}

fn parent(script: Vec<u8>, reward: bool, nonce: u32) -> Transaction {
    let mut outputs = Vec::new();
    if reward {
        outputs.push(TxOut {
            value: Amount::ZERO,
            script_pubkey: ScriptBuf::new(),
        });
    }
    outputs.push(TxOut {
        value: Amount::from_sat(1_000_000),
        script_pubkey: ScriptBuf::from_bytes(script),
    });
    Transaction {
        version: bitcoin::transaction::Version(3),
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint {
                txid: "11".repeat(32).parse().unwrap(),
                vout: nonce,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output: outputs,
    }
}

fn request(chain: Chain, recipient: String) -> SendExecutionRequest {
    SendExecutionRequest {
        token_standard: None,
        chain_id: chain,
        wallet_id: "w".into(),
        password: None,
        to_address: recipient,
        amount_str: "0.5".into(),
        contract_address: None,
        token_decimals: None,
        fee_rate_svb: None,
        fee_sat: None,
        gas_budget: None,
        fee_amount: None,
        evm_overrides: None,
        sign_only: false,
        memo: None,
    }
}

async fn service(
    chain: Chain,
    endpoint: String,
    path: &str,
) -> (Arc<WalletService>, TestDirectory, Arc<InMemorySecretStore>) {
    let directory = TestDirectory(
        std::env::temp_dir().join(format!("spectra-peercoin-{}", crate::store::new_event_id())),
    );
    std::fs::create_dir_all(directory.path()).unwrap();
    let service = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: chain,
        endpoints: vec![endpoint],
    }])
    .unwrap();
    service
        .open_state(
            directory
                .path()
                .join("wallet.sqlite")
                .to_string_lossy()
                .into(),
        )
        .await
        .unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    store_seed_phrase(&*secrets, "w", SEED, Some("secret")).unwrap();
    service.set_secret_store(secrets.clone());
    let mut wallet = WalletState::single_address(
        "w",
        "Peercoin",
        chain,
        address(chain, path),
        Some(path.into()),
        false,
    );
    wallet.signing = WalletSigning::SeedPhrase {
        password_protected: true,
    };
    wallet.xpub = Some(
        crate::service::address_discovery::UtxoDerivation::account_xpub(
            chain,
            SEED,
            path,
            &Default::default(),
        )
        .unwrap(),
    );
    service
        .apply_state_command(StateCommand::UpsertWallet { wallet })
        .await
        .unwrap();
    (service, directory, secrets)
}

#[tokio::test]
async fn protected_peercoin_account_quotes_and_signs_all_sources_after_restart() {
    for chain in [Chain::Peercoin, Chain::PeercoinTestnet] {
        for purpose in [44, 49, 84, 86] {
            let coin = if chain.is_testnet() { 1 } else { 6 };
            let root_path = format!("m/{purpose}'/{coin}'/0'/0/0");
            let receive_path = format!("m/{purpose}'/{coin}'/0'/0/4");
            let change_path = format!("m/{purpose}'/{coin}'/0'/1/3");
            let paths = [&receive_path, &change_path];
            let mut parents = Vec::new();
            for (index, path) in paths.iter().enumerate() {
                let a = address(chain, path);
                let script = crate::derivation::utxo_address::parse_utxo_address(chain, &a)
                    .unwrap()
                    .script_pubkey();
                parents.push((a, parent(script, false, index as u32), 0));
            }
            // A mature P2PK reward and an immature reward on the root address.
            let root = address(chain, &root_path);
            let root_script = crate::derivation::utxo_address::parse_utxo_address(chain, &root)
                .unwrap()
                .script_pubkey();
            if purpose == 44 {
                let derived = crate::derivation::dispatch::derive_for_chain(
                    chain, SEED, &root_path, None, None, None, true, false, true,
                )
                .unwrap();
                let key = bitcoin::secp256k1::SecretKey::from_slice(
                    &hex::decode(derived.private_key_hex.unwrap()).unwrap(),
                )
                .unwrap();
                let public = bitcoin::secp256k1::PublicKey::from_secret_key(
                    &bitcoin::secp256k1::Secp256k1::new(),
                    &key,
                );
                let mut p2pk = vec![33];
                p2pk.extend(public.serialize());
                p2pk.push(0xac);
                parents.push((root.clone(), parent(p2pk, true, 8), 1));
            }
            parents.push((root.clone(), parent(root_script, true, 9), 1));
            let fixture = parents.clone();
            let server = MockServer::start().await;
            Mock::given(any()).respond_with(move |r: &Request| {
                let path = r.url.path();
                let body = if path == "/api/v2" {
                    json!({"blockbook":{"coin": if chain.is_testnet() {"Peercoin Testnet"} else {"Peercoin"}, "decimals":6},
                        "backend":{"chain": if chain.is_testnet() {"testnet"} else {"livenet"}, "blocks":900000}})
                } else if let Some(a) = path.strip_prefix("/api/v2/utxo/") {
                    json!(fixture.iter().filter(|(address, _, _)| address == a).map(|(_, tx, vout)| {
                        let confirmations = if tx.input[0].previous_output.vout == 9 { 1 } else { 500 };
                        json!({"txid":tx.compute_txid().to_string(),"vout":vout,"value":"1000000","confirmations":confirmations,"height":1})
                    }).collect::<Vec<_>>())
                } else if let Some(id) = path.strip_prefix("/api/v2/tx/") {
                    let (_, tx, _) = fixture.iter().find(|(_, tx, _)| tx.compute_txid().to_string() == id).unwrap();
                    json!({"txid":id,"hex":hex::encode(bitcoin::consensus::serialize(tx)),
                        "confirmations":if tx.input[0].previous_output.vout == 9 {1} else {500}, "blockHeight":1})
                } else { panic!("unexpected provider request {path}"); };
                ResponseTemplate::new(200).set_body_json(body)
            }).mount(&server).await;
            let (wallet, directory, secrets) = service(chain, server.uri(), &root_path).await;
            for (path, branch, index) in
                [(&receive_path, "external", 4), (&change_path, "change", 3)]
            {
                wallet
                    .register_owned_address(
                        "w".into(),
                        chain,
                        address(chain, path),
                        Some(path.clone()),
                        Some(branch.into()),
                        Some(index),
                    )
                    .await
                    .unwrap();
            }
            let to = address(chain, &format!("m/{purpose}'/{coin}'/0'/0/9"));
            let small_preview = wallet
                .preview_peercoin_owned_send(chain, "w", "0.5", &to)
                .await
                .unwrap();
            assert_eq!(small_preview.selectedInputCount, Some(1));
            let amount = if purpose == 44 { "2.9" } else { "1.9" };
            let preview = wallet
                .preview_peercoin_owned_send(chain, "w", amount, &to)
                .await
                .unwrap();
            let expected_inputs = if purpose == 44 { 3 } else { 2 };
            assert_eq!(preview.selectedInputCount, Some(expected_inputs));
            assert_eq!(
                preview.spendableBalance,
                Some(if purpose == 44 { "3" } else { "2" }.into())
            );
            let mut req = request(chain, to);
            req.amount_str = amount.into();
            req.fee_sat =
                Some(crate::send::payload::fee_units(&preview.estimatedNetworkFee, 6).unwrap());
            let built = wallet.build_send(req).await.unwrap();
            let persisted = wallet.load_send_artifact(built.id.clone()).await.unwrap();
            let PreparedPayload::Peercoin(prepared) = &persisted.prepared else {
                panic!("wrong protocol");
            };
            assert_eq!(prepared.inputs.len() as i64, expected_inputs);
            assert_eq!(
                preview.estimatedNetworkFee,
                crate::decimal::from_units(prepared.fee.into(), 6)
            );
            assert!(
                prepared
                    .inputs
                    .iter()
                    .any(|input| input.source.address == parents[0].0)
            );
            drop(wallet);
            let resumed = WalletService::new(vec![ChainEndpoints {
                capabilities: EndpointCapability::ALL.to_vec(),
                chain_id: chain,
                endpoints: vec![server.uri()],
            }])
            .unwrap();
            resumed
                .open_state(
                    directory
                        .path()
                        .join("wallet.sqlite")
                        .to_string_lossy()
                        .into(),
                )
                .await
                .unwrap();
            resumed.set_secret_store(secrets);
            assert!(
                resumed
                    .sign_send(
                        built.id.clone(),
                        built.review_digest.clone(),
                        Some("wrong".into())
                    )
                    .await
                    .is_err()
            );
            let signed = resumed
                .sign_send(built.id, built.review_digest, Some("secret".into()))
                .await
                .unwrap();
            assert_eq!(signed.stage, SendStage::Signed);
            let tx: Transaction = bitcoin::consensus::deserialize(
                &hex::decode(signed.signed_payload.unwrap()).unwrap(),
            )
            .unwrap();
            assert_eq!(tx.version.0, 3);
            assert_eq!(tx.input.len() as i64, expected_inputs);
            assert_eq!(
                tx.output[0].value.to_sat(),
                crate::decimal::to_units(amount, 6).unwrap() as u64
            );
            assert_eq!(signed.transaction_hash, Some(tx.compute_txid().to_string()));
            assert_eq!(
                tx.input.iter().all(|input| input.witness.is_empty()),
                purpose == 44
            );
        }
    }
}

#[tokio::test]
async fn peercoin_rejects_precision_minimum_and_fee_before_network_or_storage() {
    let server = MockServer::start().await;
    let (wallet, _directory, _secrets) =
        service(Chain::Peercoin, server.uri(), "m/44'/6'/0'/0/0").await;
    for amount in ["0.000001", "0.009999", "0.0100001", "21000000.000001"] {
        let mut req = request(Chain::Peercoin, address(Chain::Peercoin, "m/44'/6'/0'/0/9"));
        req.amount_str = amount.into();
        assert!(wallet.build_send(req).await.is_err(), "{amount}");
    }
    let mut req = request(Chain::Peercoin, address(Chain::Peercoin, "m/44'/6'/0'/0/9"));
    req.fee_sat = Some(999);
    assert!(wallet.build_send(req).await.is_err());
    assert!(wallet.list_sends().await.unwrap().is_empty());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn peercoin_custom_broadcast_endpoint_proves_network_before_submission() {
    for chain in [Chain::Peercoin, Chain::PeercoinTestnet] {
        for correct_network in [true, false] {
            let server = MockServer::start().await;
            let reported_testnet = chain.is_testnet() == correct_network;
            Mock::given(any())
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "blockbook": {
                        "coin": if reported_testnet { "Peercoin Testnet" } else { "Peercoin" },
                        "decimals": 6
                    },
                    "backend": {
                        "chain": if reported_testnet { "testnet" } else { "livenet" },
                        "blocks": 900_000
                    }
                })))
                .mount(&server)
                .await;
            let wallet = WalletService::new(vec![ChainEndpoints {
                capabilities: vec![EndpointCapability::Broadcast],
                chain_id: chain,
                endpoints: vec![server.uri()],
            }])
            .unwrap();
            assert_eq!(
                wallet
                    .validate_broadcast_endpoint(chain, &server.uri())
                    .await
                    .is_ok(),
                correct_network,
                "{chain} reported testnet={reported_testnet}"
            );
            let requests = server.received_requests().await.unwrap();
            assert!(!requests.is_empty());
            assert!(
                requests
                    .iter()
                    .all(|request| { request.method == "GET" && request.url.path() == "/api/v2" })
            );
        }
    }
}
