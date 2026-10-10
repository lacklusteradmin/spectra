//! A Stellar account's signers and thresholds as Horizon shows them
//! (`stellar-multisig.json`'s account), and each rule a session is held to
//! before it is built, signed or submitted.

use super::*;
use crate::registry::PaymentMemoKind;
use crate::send::payment_memo::PaymentMemo;
use crate::store::secret_backends::InMemorySecretStore;
use crate::store::wallet_secrets::store_seed_phrase;
use serde_json::Value;
use std::sync::Mutex;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const CHAIN: Chain = Chain::StellarTestnet;

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/stellar-multisig.json"
    ))
    .unwrap()
}

fn key(name: &str, field: &str) -> String {
    fixture()["keys"][name][field].as_str().unwrap().to_string()
}

fn recipient() -> String {
    fixture()["transactions"][0]["payment"]["destination"]
        .as_str()
        .unwrap()
        .to_string()
}

/// What the loopback Horizon holds: the account as the fixture has it, and
/// every request made, as `METHOD path`.
struct Horizon {
    account: Value,
    requests: Vec<String>,
}

fn other_account(address: &str) -> Value {
    json!({"id": address, "account_id": address, "sequence": "700", "subentry_count": 0,
        "thresholds": {"low_threshold": 0, "med_threshold": 0, "high_threshold": 0},
        "balances": [{"balance": "50.0000000", "asset_type": "native",
            "buying_liabilities": "0.0000000", "selling_liabilities": "0.0000000"}],
        "signers": [{"weight": 1, "key": address, "type": "ed25519_public_key"}],
        "data": {}, "flags": {"auth_immutable": false}})
}

async fn horizon() -> (MockServer, Arc<Mutex<Horizon>>) {
    let account = key("p0", "address");
    let live = Arc::new(Mutex::new(Horizon {
        account: fixture()["horizon"]["account_response"].clone(),
        requests: Vec::new(),
    }));
    let state = live.clone();
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let mut live = state.lock().unwrap();
            let path = request.url.path().trim_end_matches('/').to_string();
            live.requests.push(format!("{} {path}", request.method));
            let body = match path.as_str() {
                "" => json!({"network_passphrase": "Test SDF Network ; September 2015"}),
                "/fee_stats" => json!({"fee_charged": {"mode": "100"}}),
                "/ledgers" => {
                    json!({"_embedded": {"records": [{"base_reserve_in_stroops": 5_000_000}]}})
                }
                path if path == format!("/accounts/{account}") => live.account.clone(),
                path if path.starts_with("/accounts/") => other_account(&path[10..]),
                _ => return ResponseTemplate::new(404).set_body_json(json!({"status": 404})),
            };
            ResponseTemplate::new(200).set_body_json(body)
        })
        .mount(&server)
        .await;
    (server, live)
}

/// The account (p0's key, which alone no longer meets a payment's medium
/// threshold) and its signers p1 and p2, each a wallet of its phrase.
async fn wallets() -> (Arc<WalletService>, MockServer, Arc<Mutex<Horizon>>) {
    let (server, live) = horizon().await;
    let service = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: CHAIN,
        endpoints: vec![server.uri()],
    }])
    .unwrap();
    let db = std::env::temp_dir().join(format!(
        "stellar-multisig-{}.sqlite",
        crate::store::new_event_id()
    ));
    service
        .open_state(db.to_string_lossy().into())
        .await
        .unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    service.set_secret_store(secrets.clone());
    let path = fixture()["derivation_path"].as_str().unwrap().to_string();
    for name in ["p0", "p1", "p2"] {
        let derived = crate::derivation::dispatch::derive_for_chain(
            CHAIN,
            &key(name, "phrase"),
            &path,
            None,
            None,
            None,
            true,
            false,
            false,
        )
        .unwrap();
        assert_eq!(derived.address, Some(key(name, "address")));
        service
            .apply_state_command(StateCommand::UpsertWallet {
                wallet: WalletState::single_address(
                    name,
                    name,
                    CHAIN,
                    key(name, "address"),
                    Some(path.clone()),
                    false,
                ),
            })
            .await
            .unwrap();
        store_seed_phrase(&*secrets, name, &key(name, "phrase"), None).unwrap();
    }
    (service, server, live)
}

fn spend() -> MultisigSpend {
    MultisigSpend {
        to_address: recipient(),
        amount: "10".into(),
        fee_rate: None,
        expires_in_secs: None,
        memo: Some(PaymentMemo {
            kind: PaymentMemoKind::MemoText,
            value: "rent".into(),
        }),
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
async fn each_threshold_reads_with_the_signers_it_weighs() {
    let (service, _server, _) = wallets().await;
    let account = service.multisig_account("p0".into()).await.unwrap();
    assert_eq!(
        account
            .permissions
            .iter()
            .map(|permission| (permission.name.as_str(), permission.threshold))
            .collect::<Vec<_>>(),
        [
            ("low threshold", 1),
            ("medium threshold (payments, trustlines)", 2),
            ("high threshold (signers, thresholds, merging)", 3),
        ]
    );
    for permission in &account.permissions {
        assert_eq!(
            permission
                .signers
                .iter()
                .map(|signer| (
                    signer.signer.clone(),
                    signer.weight,
                    signer.wallet_id.clone()
                ))
                .collect::<Vec<_>>(),
            [
                (key("p1", "address"), 1, Some("p1".into())),
                (key("p0", "address"), 1, None),
                (key("p2", "address"), 1, Some("p2".into())),
            ]
        );
    }
    assert_eq!(account.signer_wallet_ids, ["p1", "p2"]);
    let [warning] = account.warnings.as_slice() else {
        panic!("{account:?}");
    };
    assert!(warning.to_string().contains("alone no longer meets"));
}

#[tokio::test]
async fn an_ordinary_send_the_master_key_cannot_make_alone_is_refused_before_it_is_built() {
    let (service, _server, live) = wallets().await;
    let refusal = service
        .build_send(crate::send::SendExecutionRequest {
            token_standard: None,
            chain_id: CHAIN,
            wallet_id: "p0".into(),
            password: None,
            to_address: recipient(),
            amount_str: "1".into(),
            contract_address: None,
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
        .unwrap_err()
        .to_string();
    assert!(
        refusal.contains("cannot send from the account alone"),
        "{refusal}"
    );
    assert!(service.list_sends().await.unwrap().is_empty());
    // Once the key alone meets the medium threshold, a payment passes; a
    // merge, which needs the high one, still does not.
    live.lock().unwrap().account["thresholds"]["med_threshold"] = json!(1);
    let sender = key("p0", "address");
    service
        .validate_stellar_master_key(CHAIN, &sender, StellarThreshold::Medium)
        .await
        .unwrap();
    let refusal = service
        .validate_stellar_master_key(CHAIN, &sender, StellarThreshold::High)
        .await
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("needs weight 3"), "{refusal}");
}

#[tokio::test]
async fn a_session_fixes_its_sequence_fee_memo_and_threshold_when_built() {
    let (service, _server, _) = wallets().await;
    let created = service.create_multisig("p0".into(), spend()).await.unwrap();
    assert_eq!(created.scheme, MultisigScheme::StellarSigners);
    // The medium threshold, the sequence after the account's, the base fee.
    assert_eq!(created.threshold, 2);
    assert_eq!(created.sequence.as_deref(), Some("123456789013"));
    assert_eq!(created.fee, "100");
    let [output] = created.outputs.as_slice() else {
        panic!("{created:?}");
    };
    assert_eq!(output.address, recipient());
    assert_eq!(output.value, "100000000");
    assert_eq!(output.memo.as_deref(), Some("rent"));
    // An hour to gather signatures.
    let deadline = created.expires_at.unwrap();
    let now = crate::store::now_unix() as u64;
    assert!(
        (now + 3_590..=now + 3_600).contains(&deadline),
        "{deadline}"
    );
    assert_eq!(
        service.multisig_sessions("p0".into()).await.unwrap(),
        [created]
    );
}

#[tokio::test]
async fn a_submission_short_of_the_payment_threshold_is_refused() {
    let (service, _server, live) = wallets().await;
    let created = service.create_multisig("p0".into(), spend()).await.unwrap();
    let by_p1 = sign(&service, &created, "p1").await.unwrap();
    assert_eq!((by_p1.signed_weight, by_p1.complete), (1, false));
    let refusal = service
        .submit_multisig(created.id.clone(), None, None)
        .await
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("do not yet meet"), "{refusal}");
    assert!(
        !live
            .lock()
            .unwrap()
            .requests
            .iter()
            .any(|request| request.starts_with("POST"))
    );
    let stored = service.multisig_session(created.id).await.unwrap();
    assert_eq!(stored.submitted_txid, None);
}

#[tokio::test]
async fn a_session_is_refused_once_its_signers_sequence_or_deadline_moved_on() {
    let (service, _server, live) = wallets().await;
    let created = service.create_multisig("p0".into(), spend()).await.unwrap();
    for (pointer, changed, expected) in [
        (
            "/thresholds/med_threshold",
            json!(1),
            "signers or thresholds changed",
        ),
        ("/sequence", json!("123456789013"), "sequence moved on"),
    ] {
        let original = {
            let mut live = live.lock().unwrap();
            let field = live.account.pointer_mut(pointer).unwrap();
            std::mem::replace(field, changed)
        };
        let refusal = sign(&service, &created, "p1")
            .await
            .unwrap_err()
            .to_string();
        assert!(refusal.contains(expected), "{refusal}");
        *live.lock().unwrap().account.pointer_mut(pointer).unwrap() = original;
    }
    // The same payment, its deadline a second past.
    let mut stored = service.multisig_load(&created.id).await.unwrap();
    let SessionBody::Stellar(session) = &mut stored.body else {
        panic!("not a Stellar session");
    };
    let (mut payment, _) = decoded(session).unwrap();
    payment.max_time = crate::store::now_unix() as u64 - 1;
    session.envelope = engine().encode(payment.envelope(&[]).unwrap());
    service.multisig_save(&stored).await.unwrap();
    let expired = service.multisig_session(created.id.clone()).await.unwrap();
    let refusal = sign(&service, &expired, "p1")
        .await
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("deadline has passed"), "{refusal}");
    let stored = service.multisig_session(created.id).await.unwrap();
    assert_eq!(stored.signed_weight, 0);
}
