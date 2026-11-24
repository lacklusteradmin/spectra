//! A Hedera transfer built, signed and broadcast against a mock mirror node
//! and loopback consensus nodes, and the imports that find its account.
use super::*;
use crate::api::hedera_hapi::test_node;
use crate::derivation::import::{WalletImportCommit, WalletImportKind, WalletImportRequest};
use crate::service::send_stage_support::{Secret, Wallet, node_at, refusal};
use prost::Message as _;
use serde_json::{Value, json};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
/// The phrase's Ed25519 key on the standard profile, account 0
/// (`hedera-vectors.json`).
const KEY: &str = "10fcec8c5db29aeab62272a08be39b1ec75eba50624351fba9948ff5d3d78ecf";
const SENDER: &str = "0.0.1234";
const RECIPIENT: &str = "0.0.98765";
const NOW: u64 = 1_795_522_576;

/// What the mock network holds, which a test moves between stages.
struct Ledger {
    genesis: &'static str,
    balance: AtomicU64,
    now: AtomicU64,
    /// The accounts the payer's key holds.
    holders: Mutex<Vec<&'static str>>,
    /// The payer's key, raw hex.
    key: Mutex<String>,
    recipient_exists: AtomicBool,
    recipient_signs: AtomicBool,
    /// The loopback port every node of the address book answers on.
    node_port: AtomicU64,
}

impl Ledger {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            genesis: Chain::Hedera.hedera_network().unwrap().genesis_block_hash,
            balance: AtomicU64::new(500_000_000),
            now: AtomicU64::new(NOW),
            holders: Mutex::new(vec![SENDER]),
            key: Mutex::new(KEY.into()),
            recipient_exists: AtomicBool::new(true),
            recipient_signs: AtomicBool::new(false),
            node_port: AtomicU64::new(0),
        })
    }
}

fn account(id: &str, key: &str, balance: u64, receiver_sig_required: bool) -> Value {
    json!({
        "account": id, "deleted": false, "receiver_sig_required": receiver_sig_required,
        "key": {"_type": if key.len() == 64 { "ED25519" } else { "ECDSA_SECP256K1" }, "key": key},
        "balance": {"balance": balance, "timestamp": "1795522000.000000000", "tokens": []},
    })
}

async fn mirror(state: Arc<Ledger>) -> wiremock::MockServer {
    node_at(move |url, _| {
        let path = url.path();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        Some(match path {
            "/api/v1/blocks/0" => json!({"hash": state.genesis, "number": 0}),
            "/api/v1/blocks" => json!({"blocks": [{
                "number": 102722050,
                "timestamp": {"from": "1795522500.000000000",
                    "to": format!("{}.931957752", state.now.load(Ordering::SeqCst))},
            }]}),
            "/api/v1/network/exchangerate" => json!({
                "current_rate": {"cent_equivalent": 153925, "hbar_equivalent": 30000, "expiration_time": 1795525200u64},
                "next_rate": {"cent_equivalent": 154411, "hbar_equivalent": 30000, "expiration_time": 1795528800u64},
            }),
            "/api/v1/network/nodes" => {
                let port = state.node_port.load(Ordering::SeqCst);
                let nodes: Vec<Value> = ["0.0.3", "0.0.4", "0.0.5"]
                    .into_iter()
                    .map(|node| json!({
                        "node_account_id": node,
                        "node_cert_hash": format!("0x{}", "00".repeat(48)),
                        "service_endpoints": [{"domain_name": "", "ip_address_v4": "127.0.0.1", "port": port}],
                    }))
                    .collect();
                json!({"nodes": nodes, "links": {"next": null}})
            }
            "/api/v1/accounts" => {
                let key = state.key.lock().unwrap().clone();
                let holders: Vec<Value> = if query.get("account.publickey") == Some(&key) {
                    state
                        .holders
                        .lock()
                        .unwrap()
                        .iter()
                        .map(|id| account(id, &key, 0, false))
                        .collect()
                } else {
                    Vec::new()
                };
                json!({"accounts": holders, "links": {"next": null}})
            }
            _ if path == format!("/api/v1/accounts/{SENDER}") => account(
                SENDER,
                &state.key.lock().unwrap(),
                state.balance.load(Ordering::SeqCst),
                false,
            ),
            _ if path == format!("/api/v1/accounts/{RECIPIENT}") => {
                if !state.recipient_exists.load(Ordering::SeqCst) {
                    return None;
                }
                account(
                    RECIPIENT,
                    "f3ee93a679b115bffd753b068cf027535905c59cc6df45dd8eeb4a9f6f743112",
                    7,
                    state.recipient_signs.load(Ordering::SeqCst),
                )
            }
            _ if path.starts_with("/api/v1/accounts/") => return None,
            other => panic!("unexpected request {other}"),
        })
    })
    .await
}

/// A wallet imported from `secret` on a mock mirror node whose address book
/// names loopback nodes answering `precheck`.
async fn wallet(
    state: &Arc<Ledger>,
    secret: Secret<'_>,
    precheck: i32,
) -> (Wallet, wiremock::MockServer, test_node::TestNode) {
    let node = test_node::start(move |_| precheck).await;
    state
        .node_port
        .store(u64::from(node.port), Ordering::SeqCst);
    let server = mirror(state.clone()).await;
    let wallet = Wallet::import(Chain::Hedera, &server.uri(), secret).await;
    // The mirror node and its address book, each as the API it speaks.
    let book = format!("{}/api/v1/network/nodes", server.uri());
    wallet
        .service
        .update_endpoints(vec![ChainEndpoints {
            chain_id: Chain::Hedera,
            endpoints: vec![server.uri(), book.clone()],
            capabilities: EndpointCapability::ALL.to_vec(),
        }])
        .await
        .unwrap();
    for (api, url) in [
        (
            crate::api::endpoint::EndpointApi::HederaMirror,
            server.uri(),
        ),
        (crate::api::endpoint::EndpointApi::HederaHapi, book),
    ] {
        wallet
            .service
            .apply_state_command(crate::store::state::StateCommand::SetAppSetting {
                update: crate::store::state::AppSettingUpdate::AddCustomEndpoint {
                    chain_id: Chain::Hedera,
                    api,
                    endpoint: url,
                    capabilities: crate::api::endpoint::endpoint_capability_options(
                        Chain::Hedera,
                        api,
                    ),
                },
            })
            .await
            .unwrap();
    }
    (wallet, server, node)
}

fn memo(value: &str) -> Option<crate::send::payment_memo::PaymentMemo> {
    Some(crate::send::payment_memo::PaymentMemo {
        kind: crate::registry::PaymentMemoKind::MemoText,
        value: value.into(),
    })
}

#[tokio::test]
async fn a_transfer_is_bounded_signed_for_its_nodes_and_accepted() {
    let state = Ledger::new();
    let (wallet, _server, node) = wallet(&state, Secret::Phrase(PHRASE), 0).await;
    assert_eq!(wallet.address, SENDER);
    let mut request = wallet.request(RECIPIENT, "1.5");
    request.memo = memo("104857600");
    let built = wallet.build(request).await.unwrap();
    let stored = wallet
        .service
        .load_send_artifact(built.id.clone())
        .await
        .unwrap();
    let PreparedPayload::Hedera(prepared) = &stored.prepared else {
        panic!("not a Hedera transfer");
    };
    // $0.001 at the rate that buys the most HBAR: 30000 HBAR per 153925
    // cents.
    assert_eq!(prepared.max_fee, 1_949_002);
    assert_eq!(prepared.amount, 150_000_000);
    assert_eq!(prepared.memo, "104857600");
    assert_eq!(prepared.recipient, RECIPIENT);
    assert_eq!(
        prepared.public_key,
        format!("302a300506032b6570032100{KEY}")
    );
    let mut nodes = prepared.nodes.clone();
    nodes.sort();
    assert_eq!(nodes, ["0.0.3", "0.0.4", "0.0.5"]);
    assert_eq!(stored.request.fee_amount.as_deref(), Some("0.01949002"));

    let signed = wallet.sign(&built).await.unwrap();
    let id = signed.transaction_hash.clone().unwrap();
    // A second before the mirror node's newest consensus time.
    assert!(id.starts_with(&format!("{SENDER}-{}-", NOW - 1)), "{id}");
    let list = hex::decode(signed.signed_payload.as_deref().unwrap()).unwrap();
    let split = crate::send::hedera::node_transactions(&list).unwrap();
    assert_eq!(
        split
            .iter()
            .map(|node| node.node.to_string())
            .collect::<Vec<_>>(),
        prepared.nodes
    );
    // Each body is signed by the account's key.
    let verifying =
        ed25519_dalek::VerifyingKey::from_bytes(&hex::decode(KEY).unwrap().try_into().unwrap())
            .unwrap();
    for node in &split {
        let transaction =
            crate::send::hedera::proto::Transaction::decode(node.transaction.as_slice()).unwrap();
        let signed = crate::send::hedera::proto::SignedTransaction::decode(
            transaction.signed_transaction_bytes.as_slice(),
        )
        .unwrap();
        let pair = &signed.sig_map.unwrap().sig_pair[0];
        assert_eq!(hex::encode(&pair.pub_key_prefix), KEY);
        let signature = pair.ed25519.clone().unwrap();
        verifying
            .verify_strict(
                &signed.body_bytes,
                &ed25519_dalek::Signature::from_bytes(&signature.try_into().unwrap()),
            )
            .unwrap();
    }

    let broadcast = wallet.broadcast(&signed).await.unwrap();
    assert_eq!(broadcast.transaction_hash.as_deref(), Some(id.as_str()));
    assert!(
        broadcast
            .attempts
            .iter()
            .any(|attempt| attempt.outcome == SubmissionOutcome::Accepted)
    );
    // The first node took it.
    assert_eq!(
        *node.received.lock().unwrap(),
        [split[0].transaction.clone()]
    );
}

#[tokio::test]
async fn what_the_network_would_refuse_is_refused_before_signing() {
    let state = Ledger::new();
    let (wallet, _server, _node) = wallet(&state, Secret::Phrase(PHRASE), 0).await;

    // A memo over 100 bytes, a token, more than the balance and the fee
    // bound, the sender itself.
    let mut long = wallet.request(RECIPIENT, "1");
    long.memo = memo(&"9".repeat(101));
    assert!(wallet.build(long).await.is_err());
    let token = wallet.token_request(RECIPIENT, "1", "0.0.456858", 6);
    assert!(wallet.build(token).await.is_err());
    assert!(refusal(wallet.build(wallet.request(RECIPIENT, "5")).await).contains("Insufficient"));
    assert!(wallet.build(wallet.request(SENDER, "1")).await.is_err());
    // A recipient the network has no account for, or that must sign too.
    state.recipient_exists.store(false, Ordering::SeqCst);
    assert!(refusal(wallet.build(wallet.request(RECIPIENT, "1")).await).contains("no account"));
    state.recipient_exists.store(true, Ordering::SeqCst);
    state.recipient_signs.store(true, Ordering::SeqCst);
    assert!(refusal(wallet.build(wallet.request(RECIPIENT, "1")).await).contains("signature"));
    state.recipient_signs.store(false, Ordering::SeqCst);
    assert!(wallet.built_nothing().await);

    // The account is rekeyed between review and signing.
    let built = wallet.build(wallet.request(RECIPIENT, "1")).await.unwrap();
    *state.key.lock().unwrap() =
        "f3ee93a679b115bffd753b068cf027535905c59cc6df45dd8eeb4a9f6f743112".into();
    assert!(refusal(wallet.sign(&built).await).contains("key"));
    *state.key.lock().unwrap() = KEY.into();

    // The balance moves below the amount and fee bound before signing.
    state.balance.store(100_000_000, Ordering::SeqCst);
    assert!(refusal(wallet.sign(&built).await).contains("Insufficient"));
    state.balance.store(500_000_000, Ordering::SeqCst);

    // Signed, then left past its 120 seconds: no node gets it.
    let signed = wallet.sign(&built).await.unwrap();
    state.now.store(
        NOW + crate::registry::HEDERA_TRANSACTION_VALID_SECS as u64,
        Ordering::SeqCst,
    );
    assert!(refusal(wallet.broadcast(&signed).await).contains("expired"));
}

#[tokio::test]
async fn a_refusing_node_leaves_the_submission_unaccepted() {
    let state = Ledger::new();
    // INSUFFICIENT_PAYER_BALANCE.
    let (wallet, _server, node) = wallet(&state, Secret::Phrase(PHRASE), 10).await;
    let built = wallet.build(wallet.request(RECIPIENT, "1")).await.unwrap();
    let signed = wallet.sign(&built).await.unwrap();
    let broadcast = wallet.broadcast(&signed).await.unwrap();
    assert!(
        broadcast
            .attempts
            .iter()
            .all(|attempt| attempt.outcome != SubmissionOutcome::Accepted
                && attempt.detail.contains("INSUFFICIENT_PAYER_BALANCE"))
    );
    assert_eq!(node.received.lock().unwrap().len(), 1);
}

fn commit(kind: WalletImportKind, secret: &str, named: Option<&str>) -> WalletImportCommit {
    let (seed_phrase, private_key) = match kind {
        WalletImportKind::Phrase => (Some(secret.to_string()), None),
        _ => (None, Some(secret.to_string())),
    };
    WalletImportCommit {
        password: None,
        request: WalletImportRequest {
            wallet_name: String::new(),
            chain: Chain::Hedera,
            kind,
        },
        derivation_path: None,
        derivation_overrides: Default::default(),
        seed_phrase,
        private_key,
        restore_height: None,
        named_account: named.map(str::to_string),
        ton_wallet_version: None,
        upgrade_wallet_id: None,
        created: false,
    }
}

async fn service(endpoint: &str) -> Arc<WalletService> {
    let service = WalletService::new(vec![ChainEndpoints {
        chain_id: Chain::Hedera,
        endpoints: vec![endpoint.to_string()],
        capabilities: vec![
            EndpointCapability::Balance,
            EndpointCapability::History,
            EndpointCapability::Fee,
            EndpointCapability::Verification,
        ],
    }])
    .unwrap();
    service.set_secret_store(Arc::new(
        crate::secrets::backends::InMemorySecretStore::new(),
    ));
    let database =
        std::env::temp_dir().join(format!("hedera-import-{}.sqlite", crate::store::new_id()));
    service
        .opened(database.to_string_lossy().into())
        .await
        .unwrap();
    service
}

#[tokio::test]
async fn an_import_holds_the_one_account_the_key_holds_alone() {
    let state = Ledger::new();
    let server = mirror(state.clone()).await;
    let service = service(&server.uri()).await;

    // A key that holds no account yet: an account is created first.
    state.holders.lock().unwrap().clear();
    let none = service
        .preview_wallet_import(commit(WalletImportKind::Phrase, PHRASE, None))
        .await;
    assert!(refusal(none).contains("created by a transaction"));
    // Two accounts hold it: the import names one.
    *state.holders.lock().unwrap() = vec![SENDER, "0.0.5555"];
    assert!(
        refusal(
            service
                .preview_wallet_import(commit(WalletImportKind::Phrase, PHRASE, None))
                .await
        )
        .contains("0.0.1234, 0.0.5555")
    );
    let named = service
        .preview_wallet_import(commit(
            WalletImportKind::Phrase,
            PHRASE,
            Some("0.0.1234-pikcw"),
        ))
        .await
        .unwrap();
    assert_eq!(named.addresses, [SENDER]);
    // A named account another key holds, and one with a wrong checksum.
    assert!(
        refusal(
            service
                .preview_wallet_import(commit(WalletImportKind::Phrase, PHRASE, Some(RECIPIENT)))
                .await
        )
        .contains("not held by this key")
    );
    assert!(
        service
            .preview_wallet_import(commit(
                WalletImportKind::Phrase,
                PHRASE,
                Some("0.0.1234-abcde")
            ))
            .await
            .is_err()
    );
    // A watch takes the account itself, and no named account.
    let watch = |named| {
        commit(
            WalletImportKind::WatchAddresses {
                addresses: vec!["0.0.1234-pikcw".into()],
            },
            "",
            named,
        )
    };
    let mut watched = watch(None);
    watched.private_key = None;
    assert_eq!(
        service
            .preview_wallet_import(watched)
            .await
            .unwrap()
            .addresses,
        [SENDER]
    );
    let mut named_watch = watch(Some(SENDER));
    named_watch.private_key = None;
    assert!(service.preview_wallet_import(named_watch).await.is_err());
}

#[tokio::test]
async fn a_bare_key_is_sealed_as_the_scheme_its_account_holds() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/hedera-vectors.json")).unwrap();
    let raw = &fixture["raw_key"];
    let state = Ledger::new();
    // The account holds the key's secp256k1 reading.
    let ecdsa = crate::derivation::hedera::HederaPublicKey::from_der_hex(
        raw["ecdsa"]["public_key"].as_str().unwrap(),
    )
    .unwrap();
    *state.key.lock().unwrap() = hex::encode(&ecdsa.bytes);
    let server = mirror(state.clone()).await;
    let service = service(&server.uri()).await;
    let outcome = service
        .import_wallets(commit(
            WalletImportKind::PrivateKey,
            raw["hex"].as_str().unwrap(),
            None,
        ))
        .await
        .unwrap();
    let wallet = &outcome.wallets[0];
    assert_eq!(wallet.address.as_deref(), Some(SENDER));
    let store = service.secrets().unwrap();
    let sealed = crate::secrets::wallet::load_private_key(&*store, &wallet.id, None).unwrap();
    assert_eq!(
        sealed.as_str(),
        raw["ecdsa"]["private_key"].as_str().unwrap()
    );
}
