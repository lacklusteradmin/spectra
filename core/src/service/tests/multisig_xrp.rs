//! An XRP Ledger account's signer list as a node shows it (`account_info`
//! with `signer_lists`, in both API versions, from `xrp-multisig.json`), and
//! each rule a session is held to before it is built, signed or submitted.

use super::*;
use crate::registry::PaymentMemoKind;
use crate::send::payment_memo::PaymentMemo;
use crate::store::secret_backends::InMemorySecretStore;
use crate::store::wallet_secrets::store_seed_phrase;
use serde_json::Value;
use std::sync::Mutex;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const DISABLE_MASTER: u64 = 0x0010_0000;

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/xrp-multisig.json")).unwrap()
}

fn key(name: &str, field: &str) -> String {
    fixture()["keys"][name][field].as_str().unwrap().to_string()
}

fn recipient() -> String {
    fixture()["transactions"][0]["tx"]["Destination"]
        .as_str()
        .unwrap()
        .to_string()
}

/// What the loopback node holds: the account's flags, sequence and list
/// quorum, the open ledger, each signer's own flags, and every method asked.
struct Ledger {
    v2: bool,
    flags: u64,
    sequence: u64,
    ledger: u64,
    quorum: u64,
    signer_flags: HashMap<String, u64>,
    methods: Vec<String>,
}

/// The fixture's `account_info` answer for the account, as the ledger now
/// holds it, its list where `v2` puts it.
fn account_info(live: &Ledger) -> Value {
    let version = if live.v2 {
        "account_info_v2"
    } else {
        "account_info_v1"
    };
    let mut result = fixture()["rpc"][version]["response"]["result"].clone();
    result["ledger_index"] = json!(live.ledger);
    result["account_data"]["Flags"] = json!(live.flags);
    result["account_data"]["Sequence"] = json!(live.sequence);
    let list = if live.v2 {
        &mut result["signer_lists"][0]
    } else {
        &mut result["account_data"]["signer_lists"][0]
    };
    list["SignerQuorum"] = json!(live.quorum);
    result
}

async fn node(v2: bool) -> (MockServer, Arc<Mutex<Ledger>>) {
    let account = key("p0", "address");
    let live = Arc::new(Mutex::new(Ledger {
        v2,
        flags: DISABLE_MASTER,
        sequence: 9,
        ledger: 94_999_990,
        quorum: 2,
        signer_flags: HashMap::new(),
        methods: Vec::new(),
    }));
    let state = live.clone();
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let mut live = state.lock().unwrap();
            let method = body["method"].as_str().unwrap().to_string();
            live.methods.push(method.clone());
            let params = &body["params"][0];
            let result = match method.as_str() {
                "server_info" => json!({"info": {"network_id": 0}}),
                "fee" => json!({"drops": {"open_ledger_fee": "10"}}),
                "server_state" => json!({"state": {"validated_ledger": {
                    "seq": live.ledger - 1, "reserve_base": 1_000_000, "reserve_inc": 200_000}}}),
                "account_info" if params["account"] == account.as_str() => account_info(&live),
                "account_info" => {
                    let address = params["account"].as_str().unwrap();
                    json!({"account_data": {"Account": address, "Balance": "30000000",
                        "Flags": live.signer_flags.get(address).copied().unwrap_or(0),
                        "OwnerCount": 0, "Sequence": 3},
                        "ledger_current_index": live.ledger})
                }
                other => {
                    json!({"status": "error", "error_message": format!("unexpected {other}")})
                }
            };
            ResponseTemplate::new(200).set_body_json(json!({ "result": result }))
        })
        .mount(&server)
        .await;
    (server, live)
}

async fn service(endpoint: String) -> (Arc<WalletService>, Arc<InMemorySecretStore>) {
    let service = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: Chain::Xrp,
        endpoints: vec![endpoint],
    }])
    .unwrap();
    let db = std::env::temp_dir().join(format!(
        "xrp-multisig-{}.sqlite",
        crate::store::new_event_id()
    ));
    service
        .open_state(db.to_string_lossy().into())
        .await
        .unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    service.set_secret_store(secrets.clone());
    (service, secrets)
}

/// A wallet holding fixture key `name`'s phrase, at its address.
async fn key_wallet(service: &WalletService, secrets: &InMemorySecretStore, name: &str) {
    let path = crate::derivation::path::default_path_from_catalog(Chain::Xrp).unwrap();
    let address = key(name, "address");
    let derived = crate::derivation::dispatch::derive_for_chain(
        Chain::Xrp,
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
    assert_eq!(derived.address.as_deref(), Some(address.as_str()));
    service
        .apply_state_command(StateCommand::UpsertWallet {
            wallet: WalletState::single_address(name, name, Chain::Xrp, address, Some(path), false),
        })
        .await
        .unwrap();
    store_seed_phrase(secrets, name, &key(name, "phrase"), None).unwrap();
}

/// The account (p0's key, its master disabled) and the list's p1 and p2.
async fn wallets(v2: bool) -> (Arc<WalletService>, MockServer, Arc<Mutex<Ledger>>) {
    let (server, live) = node(v2).await;
    let (service, secrets) = service(server.uri()).await;
    for name in ["p0", "p1", "p2"] {
        key_wallet(&service, &secrets, name).await;
    }
    (service, server, live)
}

fn spend() -> MultisigSpend {
    MultisigSpend {
        to_address: recipient(),
        amount: "1".into(),
        fee_rate: None,
        expires_in_secs: None,
        memo: Some(PaymentMemo {
            kind: PaymentMemoKind::DestinationTag,
            value: "7".into(),
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

fn signed(session: &MultisigSession) -> Vec<String> {
    session
        .signers
        .iter()
        .filter(|signer| signer.signed)
        .map(|signer| signer.signer.clone())
        .collect()
}

#[tokio::test]
async fn the_signer_list_and_a_disabled_master_key_read_from_either_api_version() {
    for v2 in [false, true] {
        let (service, _server, _) = wallets(v2).await;
        let account = service.multisig_account("p0".into()).await.unwrap();
        // The master key signs for nothing: only the list is a permission.
        let [listed] = account.permissions.as_slice() else {
            panic!("{account:?}");
        };
        assert_eq!(listed.name, "signer list");
        assert_eq!(listed.threshold, 2);
        assert_eq!(
            listed
                .signers
                .iter()
                .map(|signer| (signer.signer.as_str(), signer.weight))
                .collect::<Vec<_>>(),
            [
                (key("p1", "address").as_str(), 1),
                ("rN7n7otQDd6FczFgLdSqtcsAUxDkw6fzRH", 2),
                (key("p2", "address").as_str(), 1),
            ]
        );
        assert_eq!(account.signer_wallet_ids, ["p1", "p2"]);
        assert_eq!(account.warnings.len(), 1, "{v2}");
        assert!(
            account.warnings[0]
                .to_string()
                .contains("master key is disabled")
        );
    }
}

#[tokio::test]
async fn an_ordinary_send_is_refused_before_it_is_built_once_the_master_key_is_disabled() {
    let (service, _server, live) = wallets(false).await;
    let refusal = service
        .build_send(crate::send::SendExecutionRequest {
            token_standard: None,
            chain_id: Chain::Xrp,
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
    assert!(refusal.contains("master key is disabled"), "{refusal}");
    assert!(service.list_sends().await.unwrap().is_empty());
    // An account whose master key still signs passes.
    live.lock().unwrap().flags = 0;
    service
        .validate_xrp_master_key(Chain::Xrp, &key("p0", "address"))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_session_fixes_its_sequence_fee_tag_and_last_ledger_when_built() {
    let (service, _server, _) = wallets(false).await;
    let created = service.create_multisig("p0".into(), spend()).await.unwrap();
    assert_eq!(created.scheme, MultisigScheme::XrplSignerList);
    assert_eq!(created.sequence.as_deref(), Some("9"));
    // One base fee for the transaction and one for each of the list's three
    // signers.
    assert_eq!(created.fee, "40");
    // An hour of ledgers, about four seconds each.
    assert_eq!(created.expires_at_height, Some(94_999_990 + 900));
    let [output] = created.outputs.as_slice() else {
        panic!("{created:?}");
    };
    assert_eq!(output.address, recipient());
    assert_eq!(output.value, "1000000");
    assert_eq!(output.memo.as_deref(), Some("7"));
    assert_eq!((created.threshold, created.signed_weight), (2, 0));
    assert!(!created.complete);
    assert_eq!(
        service.multisig_sessions("p0".into()).await.unwrap(),
        [created]
    );
}

#[tokio::test]
async fn a_submission_short_of_the_quorum_or_counting_a_disabled_signer_is_refused() {
    let (service, _server, live) = wallets(false).await;
    let created = service.create_multisig("p0".into(), spend()).await.unwrap();
    let by_p1 = sign(&service, &created, "p1").await.unwrap();
    assert_eq!(signed(&by_p1), [key("p1", "address")]);
    let refusal = service
        .submit_multisig(created.id.clone(), None, None)
        .await
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("do not yet meet"), "{refusal}");
    // A signer whose master key is disabled since it signed no longer counts.
    let complete = sign(&service, &by_p1, "p2").await.unwrap();
    assert!(complete.complete);
    live.lock()
        .unwrap()
        .signer_flags
        .insert(key("p1", "address"), DISABLE_MASTER);
    let refusal = service
        .submit_multisig(created.id.clone(), None, None)
        .await
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("disabled since it signed"), "{refusal}");
    assert!(!live.lock().unwrap().methods.iter().any(|m| m == "submit"));
    let stored = service.multisig_session(created.id).await.unwrap();
    assert_eq!(stored.submitted_txid, None);
}

#[tokio::test]
async fn a_signer_whose_own_master_key_is_disabled_cannot_sign() {
    let (service, _server, live) = wallets(false).await;
    let created = service.create_multisig("p0".into(), spend()).await.unwrap();
    live.lock()
        .unwrap()
        .signer_flags
        .insert(key("p2", "address"), DISABLE_MASTER);
    let refusal = sign(&service, &created, "p2")
        .await
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("cannot sign as a signer"), "{refusal}");
    let stored = service.multisig_session(created.id).await.unwrap();
    assert!(signed(&stored).is_empty());
}

#[tokio::test]
async fn a_session_is_refused_once_its_list_sequence_or_last_ledger_moved_on() {
    let (service, _server, live) = wallets(false).await;
    let created = service.create_multisig("p0".into(), spend()).await.unwrap();
    type Change = fn(&mut Ledger);
    let changes: [(Change, Change, &str); 3] = [
        (|l| l.quorum = 1, |l| l.quorum = 2, "signer list changed"),
        (|l| l.sequence = 10, |l| l.sequence = 9, "sequence moved on"),
        (
            |l| l.ledger += 1_000,
            |l| l.ledger -= 1_000,
            "last ledger has passed",
        ),
    ];
    for (change, restore, expected) in changes {
        change(&mut live.lock().unwrap());
        let refusal = sign(&service, &created, "p1")
            .await
            .unwrap_err()
            .to_string();
        assert!(refusal.contains(expected), "{refusal}");
        restore(&mut live.lock().unwrap());
    }
    let stored = service.multisig_session(created.id).await.unwrap();
    assert!(signed(&stored).is_empty());
    // Restored, the same session signs.
    sign(&service, &stored, "p1").await.unwrap();
}
