//! A Tron account whose permissions need two keys (`tron-multisig.json`,
//! TronWeb's), through the service against a loopback node: its owner
//! permission is 2 of 3, its active one 2 of 2 over keys 1 and 2. The
//! account is read again before each signature and broadcast; what the
//! node says is changed between them.

use super::*;
use crate::derivation::import::{WalletImportCommit, WalletImportKind, WalletImportRequest};
use crate::store::secret_backends::InMemorySecretStore;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/tron-multisig.json")).unwrap()
}

/// What the node says of the account, and what was broadcast to it.
struct Node {
    account: serde_json::Value,
    broadcasts: Vec<serde_json::Value>,
}

type Shared = Arc<std::sync::Mutex<Node>>;

async fn node() -> (MockServer, Shared) {
    let fixture = fixture();
    let node = Arc::new(std::sync::Mutex::new(Node {
        account: fixture["getaccount"].clone(),
        broadcasts: Vec::new(),
    }));
    let server = MockServer::start().await;
    let answering = node.clone();
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let body: serde_json::Value =
                serde_json::from_slice(&request.body).unwrap_or(json!({}));
            let mut node = answering.lock().unwrap();
            let answer = match request.url.path() {
                "/wallet/getblockbynum" => {
                    json!({"blockID": Chain::Tron.tron_genesis_block_id().unwrap()})
                }
                "/wallet/getnowblock" => json!({
                    "blockID": fixture["block"]["id"],
                    "block_header": {"raw_data": {"number": fixture["block"]["number"]}},
                }),
                "/wallet/getaccount" if body["address"] == fixture["account"] => {
                    node.account.clone()
                }
                "/wallet/getaccount" => json!({"address": body["address"], "balance": 10_000_000}),
                "/wallet/getchainparameters" => json!({"chainParameter": [
                    {"key": "getMultiSignFee", "value": 1_000_000},
                    {"key": "getTransactionFee", "value": 1000},
                    {"key": "getEnergyFee", "value": 210},
                ]}),
                "/wallet/broadcasttransaction" => {
                    let txid = body["txID"].clone();
                    node.broadcasts.push(body);
                    json!({"result": true, "txid": txid})
                }
                _ => return ResponseTemplate::new(404),
            };
            ResponseTemplate::new(200).set_body_json(answer)
        })
        .mount(&server)
        .await;
    (server, node)
}

/// One data directory holding the account's own key and keys 1 and 2.
struct Wallets {
    service: Arc<WalletService>,
    account: String,
    signers: [String; 2],
}

async fn wallets(server: &MockServer) -> Wallets {
    let service = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: Chain::Tron,
        endpoints: vec![server.uri()],
    }])
    .unwrap();
    service.set_secret_store(Arc::new(InMemorySecretStore::new()));
    let database = std::env::temp_dir().join(format!(
        "multisig-tron-{}.sqlite",
        crate::store::new_event_id()
    ));
    service
        .open_state(database.to_string_lossy().into_owned())
        .await
        .unwrap();
    let fixture = fixture();
    let key = |index: usize| {
        let service = service.clone();
        let key = fixture["keys"][index]["private_key"]
            .as_str()
            .unwrap()
            .to_string();
        async move {
            service
                .import_wallets(WalletImportCommit {
                    password: None,
                    request: WalletImportRequest {
                        wallet_name: format!("Key{index}"),
                        chain: Chain::Tron,
                        kind: WalletImportKind::PrivateKey,
                    },
                    derivation_path: None,
                    derivation_overrides: Default::default(),
                    seed_phrase: None,
                    private_key: Some(key),
                    restore_height: None,
                    named_account: None,
                    ton_wallet_version: None,
                    upgrade_wallet_id: None,
                })
                .await
                .unwrap()
                .wallets
                .remove(0)
                .id
        }
    };
    Wallets {
        account: key(0).await,
        signers: [key(1).await, key(2).await],
        service,
    }
}

impl Wallets {
    async fn create(&self, expires_in_secs: Option<u64>) -> Result<MultisigSession, String> {
        self.service
            .create_multisig(
                self.account.clone(),
                MultisigSpend {
                    to_address: fixture()["recipient"].as_str().unwrap().into(),
                    amount: "1".into(),
                    fee_rate: None,
                    expires_in_secs,
                    memo: None,
                },
            )
            .await
            .map_err(|error| error.to_string())
    }

    async fn sign(
        &self,
        session: &MultisigSession,
        signer: usize,
    ) -> Result<MultisigSession, String> {
        self.service
            .sign_multisig(
                session.id.clone(),
                session.review_digest.clone(),
                Some(self.signers[signer].clone()),
                None,
            )
            .await
            .map_err(|error| error.to_string())
    }

    async fn submit(&self, session: &MultisigSession) -> Result<MultisigSession, String> {
        self.service
            .submit_multisig(session.id.clone(), None, None)
            .await
            .map_err(|error| error.to_string())
    }

    async fn signed_weight(&self, session: &MultisigSession) -> u64 {
        self.service
            .multisig_session(session.id.clone())
            .await
            .unwrap()
            .signed_weight
    }
}

/// The account's own key holds weight 1 of its owner permission's 2, so an
/// ordinary send from it is refused when it is built, saying why, before
/// anything is signed or broadcast; the account warns of it.
#[tokio::test]
async fn an_ordinary_send_the_accounts_key_cannot_make_alone_is_refused_when_built() {
    let (server, node) = node().await;
    let wallets = wallets(&server).await;
    let warnings: Vec<String> = wallets
        .service
        .multisig_account(wallets.account.clone())
        .await
        .unwrap()
        .warnings
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        warnings,
        [
            "This wallet's key alone no longer meets any permission that can send TRX from the account."
        ]
    );
    let refused = wallets
        .service
        .build_send(crate::send::SendExecutionRequest {
            token_standard: None,
            chain_id: Chain::Tron,
            wallet_id: wallets.account.clone(),
            password: None,
            to_address: fixture()["recipient"].as_str().unwrap().into(),
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
    assert_eq!(
        refused,
        "This wallet's key cannot send from the account alone: its owner permission needs weight 2 and the key holds 1. Spend from it through its signers' Multisig page."
    );
    assert!(node.lock().unwrap().broadcasts.is_empty());
}

/// A session's transaction expires within a day of being built: a longer
/// or empty lifetime is refused before the network is read, and nothing is
/// kept; a day exactly is built.
#[tokio::test]
async fn a_session_expires_within_a_day_of_being_built() {
    let (server, _) = node().await;
    let wallets = wallets(&server).await;
    let read = server.received_requests().await.unwrap().len();
    for lifetime in [24 * 60 * 60 + 1, 0] {
        assert_eq!(
            wallets.create(Some(lifetime)).await.unwrap_err(),
            "A Tron transaction expires within a day of being built."
        );
    }
    assert_eq!(server.received_requests().await.unwrap().len(), read);
    let sessions = wallets.service.multisig_sessions(wallets.account.clone());
    assert!(sessions.await.unwrap().is_empty());
    let day = wallets.create(Some(24 * 60 * 60)).await.unwrap();
    let left = day.expires_at.unwrap() as f64 - crate::store::now_unix();
    assert!(
        (24.0 * 60.0 * 60.0 - 60.0..=24.0 * 60.0 * 60.0).contains(&left),
        "{left}"
    );
}

/// The transaction is broadcast only once its signers' weights meet the
/// permission's threshold: one of the active permission's two keys is
/// refused, and nothing reaches the node; both are broadcast once.
#[tokio::test]
async fn a_broadcast_short_of_the_threshold_is_refused() {
    let (server, node) = node().await;
    let wallets = wallets(&server).await;
    let session = wallets.create(None).await.unwrap();
    wallets.sign(&session, 0).await.unwrap();
    assert_eq!(
        wallets.submit(&session).await.unwrap_err(),
        "The signers' weights do not yet meet the permission's threshold."
    );
    assert!(node.lock().unwrap().broadcasts.is_empty());
    assert!(wallets.sign(&session, 1).await.unwrap().complete);
    let sent = wallets.submit(&session).await.unwrap();
    assert_eq!(sent.submitted_txid, Some(session.transaction_id));
    assert_eq!(node.lock().unwrap().broadcasts.len(), 1);
}

/// A session is built under the permission the account held then: after
/// it changed, no key signs the session and it is not broadcast, and the
/// session keeps only what it had.
#[tokio::test]
async fn a_changed_permission_stops_signing_and_broadcasting() {
    let (server, node) = node().await;
    let wallets = wallets(&server).await;
    let session = wallets.create(None).await.unwrap();
    wallets.sign(&session, 0).await.unwrap();
    node.lock().unwrap().account["active_permission"][0]["threshold"] = json!(1);
    let changed = "The account's permission changed since this session was built; build it again.";
    assert_eq!(wallets.sign(&session, 1).await.unwrap_err(), changed);
    assert_eq!(wallets.submit(&session).await.unwrap_err(), changed);
    assert_eq!(wallets.signed_weight(&session).await, 1);
    assert!(node.lock().unwrap().broadcasts.is_empty());
}

/// Past its deadline a transaction is neither signed nor broadcast: TronWeb's
/// transaction expired in 2025, and read in now it is refused both.
#[tokio::test]
async fn a_session_past_its_deadline_is_neither_signed_nor_broadcast() {
    let (server, node) = node().await;
    let wallets = wallets(&server).await;
    let vector = &fixture()["transactions"][0];
    let session = wallets
        .service
        .import_multisig(
            wallets.account.clone(),
            json!({"raw_data_hex": vector["raw_data_hex"], "txID": vector["txID"]}).to_string(),
        )
        .await
        .unwrap();
    assert!(session.expires_at.unwrap() < crate::store::now_unix() as u64);
    let passed = "The transaction's deadline has passed; build it again.";
    assert_eq!(wallets.sign(&session, 0).await.unwrap_err(), passed);
    assert_eq!(wallets.submit(&session).await.unwrap_err(), passed);
    assert_eq!(wallets.signed_weight(&session).await, 0);
    assert!(node.lock().unwrap().broadcasts.is_empty());
}
