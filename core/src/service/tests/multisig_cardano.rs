//! A Cardano native script watched at its address (`cardano-multisig.json`,
//! CSL's 2-of-3): the transfer a session builds from its outputs, the
//! wallets that may witness it, and the outputs and slot read again before
//! a witness and a submission.

use super::*;
use crate::store::secret_backends::InMemorySecretStore;
use crate::store::state::WalletSigning;
use crate::store::wallet_secrets::{store_private_key, store_seed_phrase};
use serde_json::Value;
use std::sync::Mutex;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const OUTSIDER: &str = "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong";
const INPUT: u64 = 10_000_000;

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/cardano-multisig.json"
    ))
    .unwrap()
}

fn script_address() -> String {
    fixture()["scripts"]["s1"]["addresses"]["cardano"]["address"]
        .as_str()
        .unwrap()
        .to_string()
}

fn recipient() -> String {
    fixture()["recipient"]["address"]
        .as_str()
        .unwrap()
        .to_string()
}

fn key_hash(index: usize) -> String {
    fixture()["cosigners"][index]["key_hash"]
        .as_str()
        .unwrap()
        .to_string()
}

/// What the loopback Koios holds: the tip's slot, whether the script's one
/// output is unspent, and every path asked.
struct Koios {
    slot: u64,
    unspent: bool,
    paths: Vec<String>,
}

async fn koios() -> (MockServer, Arc<Mutex<Koios>>) {
    let live = Arc::new(Mutex::new(Koios {
        slot: 1_000,
        unspent: true,
        paths: Vec::new(),
    }));
    let state = live.clone();
    let address = script_address();
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let mut live = state.lock().unwrap();
            let path = request.url.path().to_string();
            live.paths.push(path.clone());
            let body = match path.as_str() {
                "/genesis" => json!([{"networkmagic": "764824073", "networkid": "Mainnet"}]),
                "/tip" => json!([{"abs_slot": live.slot}]),
                "/epoch_params" => json!([{"epoch_no": 660, "min_fee_a": 44, "min_fee_b": 155381,
                    "coins_per_utxo_size": "4310", "max_tx_size": 16384, "max_val_size": 5000}]),
                "/address_utxos" => {
                    let query: Value = serde_json::from_slice(&request.body).unwrap();
                    assert_eq!(query["_addresses"], json!([address]));
                    let utxos = if live.unspent {
                        vec![json!({"tx_hash": "aa".repeat(32), "tx_index": 0,
                            "value": INPUT.to_string(), "is_spent": false, "asset_list": []})]
                    } else {
                        Vec::new()
                    };
                    json!(utxos)
                }
                other => panic!("unexpected {other}"),
            };
            ResponseTemplate::new(200).set_body_json(body)
        })
        .mount(&server)
        .await;
    (server, live)
}

/// The watched script; the wallets of its cosigners 0 and 2; a phrase
/// holding none of its keys; and a wallet holding cosigner 0's key alone.
async fn wallets(endpoint: String) -> Arc<WalletService> {
    let service = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: Chain::Cardano,
        endpoints: vec![endpoint],
    }])
    .unwrap();
    let db = std::env::temp_dir().join(format!(
        "cardano-multisig-{}.sqlite",
        crate::store::new_event_id()
    ));
    service
        .open_state(db.to_string_lossy().into())
        .await
        .unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    service.set_secret_store(secrets.clone());
    let fixture = fixture();
    let mut vault = WalletState::single_address(
        "vault",
        "Vault",
        Chain::Cardano,
        script_address(),
        None,
        true,
    );
    vault.multisig_policy = Some(fixture["scripts"]["s1"]["json"].to_string());
    let mut wallets = vec![vault];
    let path = crate::derivation::path::default_path_from_catalog(Chain::Cardano).unwrap();
    for (id, phrase) in [
        (
            "cosigner0",
            fixture["cosigners"][0]["phrase"].as_str().unwrap(),
        ),
        (
            "cosigner2",
            fixture["cosigners"][2]["phrase"].as_str().unwrap(),
        ),
        ("outsider", OUTSIDER),
    ] {
        let address = crate::derivation::dispatch::derive_for_chain(
            Chain::Cardano,
            phrase,
            &path,
            None,
            None,
            None,
            true,
            false,
            false,
        )
        .unwrap()
        .address
        .unwrap();
        wallets.push(WalletState::single_address(
            id,
            id,
            Chain::Cardano,
            address,
            Some(path.clone()),
            false,
        ));
        store_seed_phrase(&*secrets, id, phrase, None).unwrap();
    }
    let mut key_only = wallets[1].clone();
    key_only.id = "key-only".into();
    key_only.signing = WalletSigning::PrivateKey {
        password_protected: false,
    };
    store_private_key(
        &*secrets,
        "key-only",
        fixture["cosigners"][0]["private_key"].as_str().unwrap(),
        None,
    )
    .unwrap();
    wallets.push(key_only);
    for wallet in wallets {
        service
            .apply_state_command(StateCommand::UpsertWallet { wallet })
            .await
            .unwrap();
    }
    service
}

async fn create(service: &WalletService) -> MultisigSession {
    service
        .create_multisig(
            "vault".into(),
            MultisigSpend {
                to_address: recipient(),
                amount: "2".into(),
                fee_rate: None,
                expires_in_secs: None,
                memo: None,
            },
        )
        .await
        .unwrap()
}

async fn sign(
    service: &WalletService,
    session: &MultisigSession,
    signer: &str,
) -> Result<MultisigSession, SpectraBridgeError> {
    service
        .sign_multisig(
            session.id.clone(),
            session.review_digest.clone(),
            Some(signer.into()),
            None,
        )
        .await
}

fn signed(session: &MultisigSession) -> Vec<String> {
    session
        .signers
        .iter()
        .filter(|signer| signer.signed)
        .map(|signer| signer.signer.clone())
        .collect()
}

#[tokio::test]
async fn a_session_pays_from_the_scripts_outputs_until_its_last_slot() {
    let (server, _) = koios().await;
    let service = wallets(server.uri()).await;
    let created = create(&service).await;
    assert_eq!(created.scheme, MultisigScheme::CardanoNativeScript);
    assert_eq!(created.threshold, 2);
    let [input] = created.inputs.as_slice() else {
        panic!("{created:?}");
    };
    assert_eq!(
        (
            input.outpoint.as_str(),
            input.address.as_str(),
            input.value.as_str()
        ),
        (
            format!("{}#0", "aa".repeat(32)).as_str(),
            script_address().as_str(),
            "10000000"
        )
    );
    let [payment, change] = created.outputs.as_slice() else {
        panic!("{created:?}");
    };
    assert_eq!(
        (
            payment.address.clone(),
            payment.value.as_str(),
            payment.is_change
        ),
        (recipient(), "2000000", false)
    );
    // What is not paid comes back to the script, less the fee.
    assert_eq!(change.address, script_address());
    assert!(change.is_change);
    let fee: u64 = created.fee.parse().unwrap();
    assert!(fee > 0);
    assert_eq!(
        change.value.parse::<u64>().unwrap() + fee,
        INPUT - 2_000_000
    );
    // As long as an ordinary send allows, from the tip.
    assert_eq!(created.expires_at_height, Some(1_000 + 7_200));
    assert!(!created.complete);
}

#[tokio::test]
async fn only_a_phrase_holding_one_of_the_scripts_keys_witnesses() {
    let (server, _) = koios().await;
    let service = wallets(server.uri()).await;
    let created = create(&service).await;
    for (signer, expected) in [
        ("outsider", "holds none of the script's keys"),
        ("key-only", "holds no phrase"),
    ] {
        let refusal = sign(&service, &created, signer)
            .await
            .unwrap_err()
            .to_string();
        assert!(refusal.contains(expected), "{refusal}");
    }
    let stored = service.multisig_session(created.id.clone()).await.unwrap();
    assert!(signed(&stored).is_empty());
    let by_cosigner = sign(&service, &created, "cosigner0").await.unwrap();
    assert_eq!(signed(&by_cosigner), [key_hash(0)]);
}

#[tokio::test]
async fn a_spent_input_or_a_passed_last_slot_stops_a_witness_and_a_submission() {
    let (server, live) = koios().await;
    let service = wallets(server.uri()).await;
    let created = create(&service).await;
    type Change = fn(&mut Koios);
    let changes: [(Change, Change, &str); 2] = [
        (
            |k| k.unspent = false,
            |k| k.unspent = true,
            "spent or changed",
        ),
        (
            |k| k.slot = 1_000 + 7_200,
            |k| k.slot = 1_000,
            "last slot has passed",
        ),
    ];
    for (change, restore, expected) in changes {
        change(&mut live.lock().unwrap());
        let refusal = sign(&service, &created, "cosigner0")
            .await
            .unwrap_err()
            .to_string();
        assert!(refusal.contains(expected), "{refusal}");
        restore(&mut live.lock().unwrap());
    }
    assert!(signed(&service.multisig_session(created.id.clone()).await.unwrap()).is_empty());
    sign(&service, &created, "cosigner0").await.unwrap();
    let complete = sign(&service, &created, "cosigner2").await.unwrap();
    assert!(complete.complete);
    for (change, restore, expected) in changes {
        change(&mut live.lock().unwrap());
        let refusal = service
            .submit_multisig(created.id.clone(), None, None)
            .await
            .unwrap_err()
            .to_string();
        assert!(refusal.contains(expected), "{refusal}");
        restore(&mut live.lock().unwrap());
    }
    assert!(!live.lock().unwrap().paths.iter().any(|p| p == "/submittx"));
    let stored = service.multisig_session(created.id).await.unwrap();
    assert_eq!(stored.submitted_txid, None);
}
