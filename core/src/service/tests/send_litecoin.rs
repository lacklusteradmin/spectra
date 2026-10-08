use super::*;
use crate::send::SendExecutionRequest;
use crate::send::stages::SendStage;
use crate::store::secret_backends::InMemorySecretStore;
use crate::store::state::{WalletSigning, WalletState};
use crate::store::wallet_secrets::store_seed_phrase;
use serde_json::json;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

fn address(chain: Chain, path: &str) -> String {
    crate::derivation::dispatch::derive_for_chain(
        chain, SEED, path, None, None, None, true, false, false,
    )
    .unwrap()
    .address
    .unwrap()
}

fn request(chain: Chain, recipient: String) -> SendExecutionRequest {
    SendExecutionRequest {
        token_standard: None,
        chain_id: chain,
        wallet_id: "w".into(),
        password: None,
        to_address: recipient,
        amount_str: "0.0001".into(),
        contract_address: None,
        token_decimals: None,
        fee_rate_svb: Some("1.001".into()),
        fee_sat: None,
        gas_budget: None,
        fee_amount: None,
        evm_overrides: None,
        sign_only: false,
    }
}

async fn service(
    chain: Chain,
    endpoint: String,
    path: &str,
    password: Option<&str>,
) -> (Arc<WalletService>, String, Arc<InMemorySecretStore>) {
    let service = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: chain,
        endpoints: vec![endpoint],
    }])
    .unwrap();
    let db = std::env::temp_dir()
        .join(format!("ltc-send-{}.sqlite", crate::store::new_event_id()))
        .to_string_lossy()
        .into_owned();
    service.open_state(db.clone()).await.unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    store_seed_phrase(&*secrets, "w", SEED, password).unwrap();
    service.set_secret_store(secrets.clone());
    let mut wallet = WalletState::single_address(
        "w",
        "Litecoin",
        chain,
        address(chain, path),
        Some(path.into()),
        false,
    );
    wallet.signing = WalletSigning::SeedPhrase {
        password_protected: password.is_some(),
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
    (service, db, secrets)
}

#[test]
fn litecoin_fee_rate_uses_exact_checked_rounding() {
    assert_eq!(litecoin_fee_for_vsize("1.001", 141).unwrap(), 142);
    assert_eq!(
        litecoin_fee_for_vsize("0.0000000000000000001", 141).unwrap(),
        1
    );
    assert_eq!(
        litecoin_fee_for_vsize("18446744073709551615", 1).unwrap(),
        u64::MAX
    );
    for rate in ["0", "-1", "NaN", "1e3", "18446744073709551616"] {
        assert!(litecoin_fee_for_vsize(rate, 2).is_err(), "{rate}");
    }
}

#[tokio::test]
async fn protected_litecoin_wallet_builds_and_signs_known_receive_and_change_after_restart() {
    for chain in [Chain::Litecoin, Chain::LitecoinTestnet] {
        for purpose in [44, 49, 84] {
            let server = MockServer::start().await;
            let coin = if chain == Chain::Litecoin { 2 } else { 1 };
            let root_path = format!("m/{purpose}'/{coin}'/0'/0/0");
            let receive_path = format!("m/{purpose}'/{coin}'/0'/0/4");
            let change_path = format!("m/{purpose}'/{coin}'/0'/1/3");
            let receive = address(chain, &receive_path);
            let change = address(chain, &change_path);
            let known = [receive.clone(), change.clone()];
            Mock::given(any()).respond_with(move |r: &Request| {
                let path = r.url.path();
                let body = known.iter().position(|a| path == format!("/api/v2/utxo/{a}"))
                    .map(|i| json!([{"txid": format!("{:064x}", i + 1), "vout": 0, "value": "100000", "confirmations": 1, "height": 1}]))
                    .unwrap_or(json!([]));
                ResponseTemplate::new(200).set_body_json(body)
            }).mount(&server).await;
            let (wallet, db, secrets) =
                service(chain, server.uri(), &root_path, Some("secret")).await;
            for (a, path, branch, index) in [
                (&receive, &receive_path, "external", 4),
                (&change, &change_path, "change", 3),
            ] {
                wallet
                    .register_owned_address(
                        "w".into(),
                        chain,
                        a.clone(),
                        Some(path.clone()),
                        Some(branch.into()),
                        Some(index),
                    )
                    .await
                    .unwrap();
            }
            let built = wallet
                .build_send(request(
                    chain,
                    address(chain, &format!("m/{purpose}'/{coin}'/0'/0/9")),
                ))
                .await
                .unwrap();
            let prepared = wallet.load_send_artifact(built.id.clone()).await.unwrap();
            let PreparedPayload::Litecoin(prepared) = &prepared.prepared else {
                panic!("not Litecoin")
            };
            assert_eq!(prepared.inputs.len(), 2);
            assert!(
                prepared
                    .inputs
                    .iter()
                    .all(|input| input.source.derivation_path.as_ref().is_some())
            );
            assert!(
                prepared
                    .inputs
                    .iter()
                    .any(|input| input.source.address == receive)
            );
            assert!(
                prepared
                    .inputs
                    .iter()
                    .any(|input| input.source.address == change)
            );
            drop(wallet);
            let resumed = WalletService::new(vec![ChainEndpoints {
                capabilities: EndpointCapability::ALL.to_vec(),
                chain_id: chain,
                endpoints: vec![server.uri()],
            }])
            .unwrap();
            resumed.open_state(db).await.unwrap();
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
            let tx: bitcoin::Transaction = bitcoin::consensus::deserialize(
                &hex::decode(signed.signed_payload.as_ref().unwrap()).unwrap(),
            )
            .unwrap();
            assert_eq!(tx.input.len(), 2);
            assert_eq!(tx.output.len(), 2);
            assert_eq!(signed.transaction_hash, Some(tx.compute_txid().to_string()));
            assert_eq!(
                tx.input.iter().all(|i| !i.witness.is_empty()),
                purpose != 44
            );
        }
    }
}

#[tokio::test]
async fn stale_and_duplicate_litecoin_inputs_are_refused() {
    let server = MockServer::start().await;
    let path = "m/84'/2'/0'/0/0";
    let (wallet, _, _) = service(Chain::Litecoin, server.uri(), path, None).await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!([{"txid": "01".repeat(32), "vout": 0, "value": "100000", "confirmations": 1}]),
        ))
        .mount(&server)
        .await;
    let req = request(Chain::Litecoin, address(Chain::Litecoin, "m/84'/2'/0'/0/9"));
    let built = wallet.build_send(req.clone()).await.unwrap();
    server.reset().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!([{"txid": "01".repeat(32), "vout": 0, "value": "99999", "confirmations": 1}]),
        ))
        .mount(&server)
        .await;
    assert!(
        wallet
            .sign_send(built.id, built.review_digest, None)
            .await
            .unwrap_err()
            .to_string()
            .contains("changed or was spent")
    );
    server.reset().await;
    let utxo = json!({"txid": "01".repeat(32), "vout": 0, "value": "100000", "confirmations": 1});
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([utxo.clone(), utxo])))
        .mount(&server)
        .await;
    assert!(
        wallet
            .build_send(req)
            .await
            .unwrap_err()
            .to_string()
            .contains("Duplicate Litecoin outpoint")
    );
}

#[tokio::test]
async fn unsupported_and_other_account_litecoin_sources_fail_before_provider_reads() {
    let server = MockServer::start().await;
    let (wallet, _, _) = service(Chain::Litecoin, server.uri(), "m/84'/2'/0'/0/0", None).await;
    let path = "m/84'/2'/1'/0/0";
    wallet
        .register_owned_address(
            "w".into(),
            Chain::Litecoin,
            address(Chain::Litecoin, path),
            Some(path.into()),
            Some("external".into()),
            Some(0),
        )
        .await
        .unwrap();
    assert!(
        wallet
            .collect_litecoin_inputs(Chain::Litecoin, "w")
            .await
            .unwrap_err()
            .to_string()
            .contains("different wallet account")
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// An MWEB recipient is paid by a peg-in from the wallet's inputs: signed,
/// the transaction's MWEB part pays the address the amount, and its kernel's
/// fee rides beside it. A malformed MWEB address is refused before any
/// provider read.
#[tokio::test]
async fn an_mweb_destination_is_paid_by_a_peg_in() {
    use crate::send::litecoin_mweb::keys::ViewKeys;
    let server = MockServer::start().await;
    let path = "m/84'/2'/0'/0/0";
    let (wallet, _, _) = service(Chain::Litecoin, server.uri(), path, None).await;
    let malformed = request(Chain::Litecoin, "ltcmweb1unsupported".into());
    assert!(
        wallet
            .prepare_litecoin(
                Chain::Litecoin,
                &malformed,
                &address(Chain::Litecoin, path),
                1_000
            )
            .await
            .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());

    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!([{"txid": "01".repeat(32), "vout": 0, "value": "1000000", "confirmations": 1}]),
        ))
        .mount(&server)
        .await;
    let (view, _) = ViewKeys::from_seed(&[7; 64]).unwrap();
    let recipient = view.address(2).unwrap().encode(Chain::Litecoin).unwrap();
    let built = wallet
        .build_send(request(Chain::Litecoin, recipient.clone()))
        .await
        .unwrap();
    let stored = wallet.load_send_artifact(built.id.clone()).await.unwrap();
    let PreparedPayload::LitecoinPegIn(pegin) = &stored.prepared else {
        panic!("not a peg-in")
    };
    assert_eq!(
        (pegin.amount, pegin.mweb_fee, pegin.recipient.as_str()),
        (10_000, 2_100, recipient.as_str())
    );
    let signed = wallet
        .sign_send(built.id, built.review_digest, None)
        .await
        .unwrap();
    let raw = hex::decode(signed.signed_payload.unwrap()).unwrap();
    let outputs = crate::send::litecoin_mweb::transaction::mweb_outputs(&raw).unwrap();
    let keys = view.spend_keys().unwrap();
    let paid: Vec<_> = outputs
        .iter()
        .filter_map(|output| crate::send::litecoin_mweb::output::rewind(output, &view, &keys))
        .map(|coin| (coin.value, coin.address_index))
        .collect();
    assert_eq!(paid, [(10_000, 2)]);
}

#[tokio::test]
async fn litecoin_private_key_wallet_uses_its_actual_network_and_needs_no_path() {
    use crate::store::wallet_secrets::store_private_key;
    for chain in [Chain::Litecoin, Chain::LitecoinTestnet] {
        let server = MockServer::start().await;
        Mock::given(any()).respond_with(ResponseTemplate::new(200).set_body_json(json!([{"txid": "01".repeat(32), "vout": 0, "value": "100000", "confirmations": 1}]))).mount(&server).await;
        let coin = if chain == Chain::Litecoin { 2 } else { 1 };
        let (wallet, _, secrets) =
            service(chain, server.uri(), &format!("m/44'/{coin}'/0'/0/0"), None).await;
        crate::store::wallet_secrets::delete(&*secrets, "w").unwrap();
        let key = format!("{:064x}", 1);
        store_private_key(&*secrets, "w", &key, None).unwrap();
        let derived = crate::derivation::dispatch::derive_from_private_key(chain, key, true, false)
            .unwrap()
            .unwrap()
            .address
            .unwrap();
        let mut state =
            WalletState::single_address("w", "Private", chain, derived.clone(), None, false);
        state.signing = WalletSigning::PrivateKey {
            password_protected: false,
        };
        wallet
            .apply_state_command(StateCommand::UpsertWallet { wallet: state })
            .await
            .unwrap();
        assert_eq!(
            wallet
                .send_identity_address("w".into(), chain, None)
                .await
                .unwrap(),
            derived
        );
        let sources = wallet.account_utxo_send_sources("w", chain).await.unwrap();
        assert_eq!(sources.len(), 1);
        assert!(sources[0].derivation_path.is_none());
        let built = wallet
            .build_send(request(chain, address(chain, "m/84'/2'/0'/0/9")))
            .await
            .unwrap();
        let signed = wallet
            .sign_send(built.id, built.review_digest, None)
            .await
            .unwrap();
        assert_eq!(signed.stage, SendStage::Signed);
    }
}

#[tokio::test]
async fn same_account_address_path_mismatch_is_refused_before_sign_provider_reads() {
    let server = MockServer::start().await;
    let (wallet, _, _) = service(Chain::Litecoin, server.uri(), "m/84'/2'/0'/0/0", None).await;
    wallet
        .register_owned_address(
            "w".into(),
            Chain::Litecoin,
            address(Chain::Litecoin, "m/84'/2'/0'/0/4"),
            Some("m/84'/2'/0'/0/3".into()),
            Some("external".into()),
            Some(3),
        )
        .await
        .unwrap();
    assert!(
        wallet
            .send_identity_address("w".into(), Chain::Litecoin, None)
            .await
            .unwrap_err()
            .to_string()
            .contains("does not match its wallet derivation path")
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn unconfirmed_litecoin_inputs_are_excluded_and_zero_input_values_are_refused() {
    let server = MockServer::start().await;
    let (wallet, _, _) = service(Chain::Litecoin, server.uri(), "m/84'/2'/0'/0/0", None).await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!([{"txid": "01".repeat(32), "vout": 0, "value": "100000", "confirmations": 0}]),
        ))
        .mount(&server)
        .await;
    assert!(
        wallet
            .collect_litecoin_inputs(Chain::Litecoin, "w")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        wallet
            .build_send(request(
                Chain::Litecoin,
                address(Chain::Litecoin, "m/84'/2'/0'/0/9")
            ))
            .await
            .unwrap_err()
            .to_string()
            .contains("No spendable inputs")
    );
    server.reset().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!([{"txid": "01".repeat(32), "vout": 0, "value": "0", "confirmations": 1}]),
        ))
        .mount(&server)
        .await;
    assert!(
        wallet
            .collect_litecoin_inputs(Chain::Litecoin, "w")
            .await
            .unwrap_err()
            .to_string()
            .contains("must be positive")
    );
}

#[tokio::test]
async fn litecoin_build_boundaries_reject_null_outpoints_before_storing() {
    let server = MockServer::start().await;
    let (wallet, _, _) = service(Chain::Litecoin, server.uri(), "m/84'/2'/0'/0/0", None).await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "txid": "00".repeat(32), "vout": u32::MAX, "value": "100000", "confirmations": 1,
        }])))
        .mount(&server)
        .await;
    assert!(
        wallet
            .build_send(request(
                Chain::Litecoin,
                address(Chain::Litecoin, "m/84'/2'/0'/0/9")
            ))
            .await
            .unwrap_err()
            .to_string()
            .contains("Invalid Litecoin input outpoint")
    );
    assert!(wallet.list_sends().await.unwrap().is_empty());
}

#[tokio::test]
async fn litecoin_build_boundaries_default_fee_covers_many_inputs_and_respects_explicit_fee() {
    for count in [1, 20] {
        let server = MockServer::start().await;
        let utxos: Vec<_> = (1..=count)
            .map(|i| {
                json!({
                    "txid": format!("{i:064x}"), "vout": 0, "value": "100000", "confirmations": 1,
                })
            })
            .collect();
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200).set_body_json(utxos))
            .mount(&server)
            .await;
        let (wallet, _, _) = service(Chain::Litecoin, server.uri(), "m/44'/2'/0'/0/0", None).await;
        let mut req = request(Chain::Litecoin, address(Chain::Litecoin, "m/44'/2'/0'/0/9"));
        req.fee_rate_svb = None;
        let built = wallet.build_send(req.clone()).await.unwrap();
        let stored = wallet.load_send_artifact(built.id.clone()).await.unwrap();
        let PreparedPayload::Litecoin(prepared) = &stored.prepared else {
            panic!("not Litecoin")
        };
        assert_eq!(prepared.inputs.len(), count);
        assert!(prepared.fee >= 1_000);
        let signed = wallet
            .sign_send(built.id, built.review_digest, None)
            .await
            .unwrap();
        let tx: bitcoin::Transaction =
            bitcoin::consensus::deserialize(&hex::decode(signed.signed_payload.unwrap()).unwrap())
                .unwrap();
        let actual_fee = count as u64 * 100_000
            - tx.output
                .iter()
                .map(|output| output.value.to_sat())
                .sum::<u64>();
        assert_eq!(actual_fee, prepared.fee);
        assert!(actual_fee >= tx.vsize() as u64);
        if count > 1 {
            assert!(actual_fee > 1_000);
        }
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| request.url.path().starts_with("/api/v2/utxo/"))
        );
        req.fee_sat = Some(50);
        let explicit = wallet.build_send(req).await.unwrap();
        let explicit = wallet.load_send_artifact(explicit.id).await.unwrap();
        let PreparedPayload::Litecoin(prepared) = explicit.prepared else {
            panic!("not Litecoin")
        };
        assert_eq!(prepared.fee, 50);
    }
}

#[tokio::test]
async fn litecoin_build_boundaries_recipient_dust_is_checked_before_provider_reads() {
    for (chain, coin) in [(Chain::Litecoin, 2), (Chain::LitecoinTestnet, 1)] {
        for (purpose, dust) in [(44, 5460), (49, 5400), (84, 2940)] {
            let server = MockServer::start().await;
            let (wallet, _, _) =
                service(chain, server.uri(), &format!("m/84'/{coin}'/0'/0/0"), None).await;
            let mut req = request(
                chain,
                address(chain, &format!("m/{purpose}'/{coin}'/0'/0/9")),
            );
            req.amount_str = crate::decimal::from_units(dust - 1, 8);
            assert!(
                wallet
                    .build_send(req.clone())
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("below the dust threshold")
            );
            assert!(server.received_requests().await.unwrap().is_empty());
            assert!(wallet.list_sends().await.unwrap().is_empty());
            Mock::given(any())
                .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                    "txid": "01".repeat(32), "vout": 0, "value": "100000", "confirmations": 1,
                }])))
                .mount(&server)
                .await;
            req.amount_str = crate::decimal::from_units(dust, 8);
            assert!(wallet.build_send(req).await.is_ok());
        }
    }
}
