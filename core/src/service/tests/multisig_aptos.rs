//! An Aptos MultiKey account watched from its policy (`aptos-multikey.json`):
//! the sequence a session takes, and the account read again before a
//! signature and a submission.

use super::*;
use crate::store::secret_backends::InMemorySecretStore;
use crate::store::wallet_secrets::store_seed_phrase;
use serde_json::Value;
use std::sync::Mutex;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const CHAIN: Chain = Chain::AptosTestnet;

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/aptos-multikey.json")).unwrap()
}

/// What the loopback node holds: the account's sequence and authentication
/// key, and every request made, as `METHOD path`.
struct Node {
    sequence: u64,
    key: String,
    requests: Vec<String>,
}

async fn node() -> (MockServer, Arc<Mutex<Node>>) {
    let fixture = fixture();
    let live = Arc::new(Mutex::new(Node {
        sequence: 5,
        key: fixture["multi_key"]["authentication_key"]
            .as_str()
            .unwrap()
            .into(),
        requests: Vec::new(),
    }));
    let state = live.clone();
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let mut live = state.lock().unwrap();
            let path = request.url.path().to_string();
            live.requests.push(format!("{} {path}", request.method));
            let body = match path.as_str() {
                "/" => json!({"chain_id": 2, "ledger_version": "9"}),
                "/estimate_gas_price" => json!({"gas_estimate": 100}),
                "/view" => json!(["10000000000"]),
                path if path.starts_with("/accounts/") => json!({
                    "sequence_number": live.sequence.to_string(),
                    "authentication_key": live.key}),
                other => panic!("unexpected {other}"),
            };
            ResponseTemplate::new(200).set_body_json(body)
        })
        .mount(&server)
        .await;
    (server, live)
}

/// The watched account and the wallets of its two Ed25519 members.
async fn wallets(endpoint: String) -> Arc<WalletService> {
    let service = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: CHAIN,
        endpoints: vec![endpoint],
    }])
    .unwrap();
    let db = std::env::temp_dir().join(format!(
        "aptos-multikey-{}.sqlite",
        crate::store::new_event_id()
    ));
    service
        .open_state(db.to_string_lossy().into())
        .await
        .unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    service.set_secret_store(secrets.clone());
    let fixture = fixture();
    let keys = fixture["keys"].as_array().unwrap();
    let mut vault = WalletState::single_address(
        "vault",
        "Shared",
        CHAIN,
        fixture["multi_key"]["address"].as_str().unwrap(),
        None,
        true,
    );
    vault.multisig_policy = Some(
        json!({"signaturesRequired": 2, "publicKeys": keys
            .iter()
            .map(|key| format!("{}-pub-0x{}", key["scheme"].as_str().unwrap(),
                key["public_key"].as_str().unwrap()))
            .collect::<Vec<_>>()})
        .to_string(),
    );
    service
        .apply_state_command(StateCommand::UpsertWallet { wallet: vault })
        .await
        .unwrap();
    for (id, key) in [("signer0", &keys[0]), ("signer1", &keys[1])] {
        let (phrase, path) = (
            key["phrase"].as_str().unwrap(),
            key["path"].as_str().unwrap(),
        );
        let address = crate::derivation::dispatch::derive_for_chain(
            CHAIN, phrase, path, None, None, None, true, false, false,
        )
        .unwrap()
        .address
        .unwrap();
        service
            .apply_state_command(StateCommand::UpsertWallet {
                wallet: WalletState::single_address(
                    id,
                    id,
                    CHAIN,
                    address,
                    Some(path.into()),
                    false,
                ),
            })
            .await
            .unwrap();
        store_seed_phrase(&*secrets, id, phrase, None).unwrap();
    }
    service
}

fn spend() -> MultisigSpend {
    MultisigSpend {
        to_address: fixture()["transaction"]["recipient"]
            .as_str()
            .unwrap()
            .into(),
        amount: "0.01".into(),
        fee_rate: None,
        expires_in_secs: None,
        memo: None,
    }
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

#[tokio::test]
async fn a_session_takes_the_accounts_next_sequence_and_an_hour_to_expire() {
    let (server, live) = node().await;
    let service = wallets(server.uri()).await;
    let created = service
        .create_multisig("vault".into(), spend())
        .await
        .unwrap();
    assert_eq!(created.scheme, MultisigScheme::AptosMultiKey);
    assert_eq!(created.sequence.as_deref(), Some("5"));
    assert_eq!(created.threshold, 2);
    // The network's gas price for the chain's gas limit.
    let max_gas = CHAIN.aptos_max_gas_amount().unwrap();
    assert_eq!(created.fee, (max_gas * 100).to_string());
    let now = crate::store::now_unix() as u64;
    let expires = created.expires_at.unwrap();
    assert!((now + 3_590..=now + 3_600).contains(&expires), "{expires}");
    live.lock().unwrap().sequence = 9;
    let later = service
        .create_multisig("vault".into(), spend())
        .await
        .unwrap();
    assert_eq!(later.sequence.as_deref(), Some("9"));
}

#[tokio::test]
async fn a_session_is_refused_once_the_key_rotated_or_the_sequence_moved_on() {
    let (server, live) = node().await;
    let service = wallets(server.uri()).await;
    let created = service
        .create_multisig("vault".into(), spend())
        .await
        .unwrap();
    let by_p0 = sign(&service, &created, "signer0").await.unwrap();
    let authentication_key = live.lock().unwrap().key.clone();
    let rotated = format!("0x{}", "77".repeat(32));
    live.lock().unwrap().key = rotated.clone();
    let refusal = sign(&service, &by_p0, "signer1")
        .await
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("rotated"), "{refusal}");
    live.lock().unwrap().key = authentication_key.clone();
    let complete = sign(&service, &by_p0, "signer1").await.unwrap();
    assert!(complete.complete);

    live.lock().unwrap().key = rotated;
    let refusal = service
        .submit_multisig(created.id.clone(), None, None)
        .await
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("rotated"), "{refusal}");
    live.lock().unwrap().key = authentication_key;
    live.lock().unwrap().sequence = 6;
    let refusal = service
        .submit_multisig(created.id.clone(), None, None)
        .await
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("sequence moved on"), "{refusal}");
    assert!(
        !live
            .lock()
            .unwrap()
            .requests
            .iter()
            .any(|request| request == "POST /transactions")
    );
    let stored = service.multisig_session(created.id).await.unwrap();
    assert_eq!(stored.submitted_txid, None);
}
