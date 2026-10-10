//! A staged EVM send: what a build refuses, what a stored send must still be
//! when it is read back, and what signing and broadcasting check before
//! anything leaves the device.
use super::*;
use crate::send::SendExecutionRequest;
use crate::send::ethereum::{EvmCustomFeeConfiguration, EvmSendOverridesInput};
use crate::send::stages::{SendStage, SubmissionOutcome};
use crate::store::secret_backends::InMemorySecretStore;
use crate::store::state::{WalletSigning, WalletState};
use crate::store::wallet_secrets::store_seed_phrase;
use serde_json::{Value, json};
use sha3::Digest;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const PATH: &str = "m/44'/60'/0'/0/0";
/// The phrase's first Ethereum address: the wallet's own.
const SENDER: &str = "0x9858EfFD232B4033E47d90003D41EC34EcaEda94";
const PASSWORD: &str = "stage-password";

fn recipient() -> String {
    format!("0x{}", "22".repeat(20))
}

/// What every node reports about the chain, shared so a test can move the
/// chain on under a stored send.
#[derive(Default)]
struct ChainView {
    nonce: u64,
    token_metadata_fails: bool,
}

/// How a node answers what is not a read of the chain.
#[derive(Clone, Copy, PartialEq)]
enum Node {
    Accepts,
    Rejects,
    OnAnotherNetwork,
}

async fn node(kind: Node, chain: Arc<std::sync::Mutex<ChainView>>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let body: Value = request.body_json().unwrap();
            let answer = |call: &Value| {
                let error = |message: &str| {
                    json!({"jsonrpc":"2.0","id":call["id"],"error":{"code":-32000,"message":message}})
                };
                let view = chain.lock().unwrap();
                let result = match call["method"].as_str().unwrap() {
                    "eth_chainId" if kind == Node::OnAnotherNetwork => json!("0x2"),
                    "eth_chainId" => json!("0x1"),
                    "eth_getTransactionCount" => json!(format!("0x{:x}", view.nonce)),
                    "eth_getBalance" => json!(format!("0x{:x}", 10u128.pow(30))),
                    "eth_estimateGas" => json!("0x5208"),
                    "eth_getCode" => json!("0x"),
                    "eth_feeHistory" => {
                        json!({"baseFeePerGas":["0x3b9aca00"],"reward":[["0x77359400"]]})
                    }
                    "eth_call" if view.token_metadata_fails => {
                        return error("metadata unavailable");
                    }
                    "eth_call" => {
                        let data = call["params"][0]["data"].as_str().unwrap();
                        match &data[2..10] {
                            // The contract's own precision is 6.
                            "313ce567" => json!(format!("0x{:064x}", 6)),
                            "70a08231" => json!(format!("0x{:064x}", 10u128.pow(30))),
                            "95d89b41" => json!(format!("0x{:0<64}", hex::encode("TOKEN"))),
                            "01ffc9a7" => json!(format!("0x{:064x}", 0)),
                            other => panic!("unexpected eth_call selector {other}"),
                        }
                    }
                    "eth_sendRawTransaction" if kind == Node::Rejects => {
                        return error("transaction rejected");
                    }
                    "eth_sendRawTransaction" => {
                        let raw = call["params"][0].as_str().unwrap();
                        json!(format!(
                            "0x{}",
                            hex::encode(sha3::Keccak256::digest(hex::decode(&raw[2..]).unwrap()))
                        ))
                    }
                    other => panic!("unexpected RPC {other}"),
                };
                json!({"jsonrpc":"2.0","id":call["id"],"result":result})
            };
            ResponseTemplate::new(200).set_body_json(match body.as_array() {
                Some(batch) => json!(batch.iter().map(answer).collect::<Vec<_>>()),
                None => answer(&body),
            })
        })
        .mount(&server)
        .await;
    server
}

/// Every raw transaction `server` was sent.
async fn submitted(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .flat_map(|request| {
            let body: Value = request.body_json().unwrap();
            body.as_array().cloned().unwrap_or_else(|| vec![body])
        })
        .filter(|call| call["method"] == "eth_sendRawTransaction")
        .map(|call| call["params"][0].as_str().unwrap().to_string())
        .collect()
}

/// A password-protected phrase wallet's store, opened by a service per
/// step, as each CLI process opens it with the endpoints it was given.
struct Store {
    database: String,
    secrets: Arc<InMemorySecretStore>,
}

impl Store {
    async fn new() -> Self {
        let database = std::env::temp_dir()
            .join(format!(
                "evm-stages-{}.sqlite",
                crate::store::new_event_id()
            ))
            .to_string_lossy()
            .into_owned();
        let secrets = Arc::new(InMemorySecretStore::new());
        store_seed_phrase(&*secrets, "w", SEED, Some(PASSWORD)).unwrap();
        let store = Self { database, secrets };
        let service = store.open(&[]).await;
        let mut wallet = WalletState::single_address(
            "w",
            "Stages",
            Chain::Ethereum,
            SENDER,
            Some(PATH.into()),
            false,
        );
        wallet.signing = WalletSigning::SeedPhrase {
            password_protected: true,
        };
        service
            .apply_state_command(StateCommand::UpsertWallet { wallet })
            .await
            .unwrap();
        store
    }

    async fn open(&self, nodes: &[&MockServer]) -> Arc<WalletService> {
        let service = WalletService::new(vec![ChainEndpoints {
            capabilities: EndpointCapability::ALL.to_vec(),
            chain_id: Chain::Ethereum,
            endpoints: nodes.iter().map(|node| node.uri()).collect(),
        }])
        .unwrap();
        service.set_secret_store(self.secrets.clone());
        service.open_state(self.database.clone()).await.unwrap();
        service
    }

    /// The stored payload of every send, as the table holds it.
    fn rows(&self) -> Vec<String> {
        let db = rusqlite::Connection::open(&self.database).unwrap();
        let mut statement = db.prepare("SELECT payload FROM send_artifacts").unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn count(&self, table: &str) -> i64 {
        rusqlite::Connection::open(&self.database)
            .unwrap()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    /// Rewrite one stored send's payload behind the service's back, as
    /// another program with the file could.
    fn alter(&self, id: &str, change: impl FnOnce(&mut Value)) -> String {
        let db = rusqlite::Connection::open(&self.database).unwrap();
        let original: String = db
            .query_row(
                "SELECT payload FROM send_artifacts WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        let mut payload: Value = serde_json::from_str(&original).unwrap();
        change(&mut payload);
        self.restore(id, &payload.to_string());
        original
    }

    fn restore(&self, id: &str, payload: &str) {
        rusqlite::Connection::open(&self.database)
            .unwrap()
            .execute(
                "UPDATE send_artifacts SET payload=?2 WHERE id=?1",
                rusqlite::params![id, payload],
            )
            .unwrap();
    }
}

fn json(value: &impl serde::Serialize) -> Value {
    serde_json::to_value(value).unwrap()
}

fn request(to: &str, amount: &str) -> SendExecutionRequest {
    SendExecutionRequest {
        token_standard: None,
        chain_id: Chain::Ethereum,
        wallet_id: "w".into(),
        password: None,
        to_address: to.into(),
        amount_str: amount.into(),
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

/// A replacement at `nonce` paying `max_fee` and `priority_fee` gwei.
fn replacement(nonce: i64, max_fee: String, priority_fee: String) -> SendExecutionRequest {
    let mut request = request(&format!("0x{}", "33".repeat(20)), "0");
    request.evm_overrides = Some(EvmSendOverridesInput {
        nonce: Some(nonce),
        custom_fees: Some(EvmCustomFeeConfiguration {
            max_fee_per_gas_gwei: max_fee,
            max_priority_fee_per_gas_gwei: priority_fee,
        }),
        ..Default::default()
    });
    request
}

async fn sign(service: &WalletService, artifact: &SendArtifact) -> Result<SendArtifact, String> {
    service
        .sign_send(
            artifact.id.clone(),
            artifact.review_digest.clone(),
            Some(PASSWORD.into()),
        )
        .await
        .map_err(|error| error.to_string())
}

/// The token's precision is the contract's: a declared precision it does
/// not confirm, or a contract whose precision cannot be read, is refused
/// before anything is stored.
#[tokio::test]
async fn a_token_build_needs_the_contract_to_confirm_its_decimals() {
    let chain = Arc::new(std::sync::Mutex::new(ChainView {
        nonce: 7,
        ..Default::default()
    }));
    let accepts = node(Node::Accepts, chain.clone()).await;
    let store = Store::new().await;
    let service = store.open(&[&accepts]).await;
    let mut token = request(&recipient(), "1.5");
    token.contract_address = Some(format!("0x{}", "44".repeat(20)));
    token.token_decimals = Some(18);
    let error = service.build_send(token.clone()).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Token decimals do not match the selected network"),
        "{error}"
    );
    chain.lock().unwrap().token_metadata_fails = true;
    token.token_decimals = Some(6);
    let error = service.build_send(token.clone()).await.unwrap_err();
    assert!(
        error.to_string().contains("metadata unavailable"),
        "{error}"
    );
    assert!(service.list_sends().await.unwrap().is_empty());

    chain.lock().unwrap().token_metadata_fails = false;
    let built = service.build_send(token).await.unwrap();
    let PreparedPayload::Evm(prepared) =
        service.load_send_artifact(built.id).await.unwrap().prepared
    else {
        panic!("not an EVM transaction")
    };
    assert_eq!(prepared.value_wei, 0);
    assert_eq!(prepared.data[prepared.data.len() - 32..], {
        let mut amount = [0u8; 32];
        amount[24..].copy_from_slice(&1_500_000u64.to_be_bytes());
        amount
    });
    assert!(submitted(&accepts).await.is_empty());
}

/// A send to the wallet's own address is built to be confirmed as one, and
/// a send to anyone else is not.
#[tokio::test]
async fn a_send_to_the_wallets_own_address_is_built_for_self_send_confirmation() {
    let chain = Arc::new(std::sync::Mutex::new(ChainView {
        nonce: 7,
        ..Default::default()
    }));
    let accepts = node(Node::Accepts, chain).await;
    let store = Store::new().await;
    let service = store.open(&[&accepts]).await;
    let own = service
        .build_send(request(SENDER, "0.000000000000000001"))
        .await
        .unwrap();
    assert!(own.review.requires_self_send_confirmation);
    assert_eq!(
        json(&service.inspect_send(own.id.clone()).await.unwrap().review),
        json(&own.review)
    );
    let other = service
        .build_send(request(&recipient(), "0.01"))
        .await
        .unwrap();
    assert!(!other.review.requires_self_send_confirmation);
}

/// A stored send is read back only as it was reviewed and signed: a changed
/// name, advisory, amount or signed payload refuses it, and an altered
/// signed send is never broadcast.
#[tokio::test]
async fn an_altered_stored_send_is_refused_wherever_it_is_read() {
    let chain = Arc::new(std::sync::Mutex::new(ChainView {
        nonce: 7,
        ..Default::default()
    }));
    let accepts = node(Node::Accepts, chain).await;
    let store = Store::new().await;
    let service = store.open(&[&accepts]).await;
    let built = service
        .build_send(request(&recipient(), "0.01"))
        .await
        .unwrap();
    let alterations: [(&str, fn(&mut Value)); 3] = [
        ("symbol", |payload| {
            payload["view"]["symbol"] = json!("USDC")
        }),
        ("self-send advisory", |payload| {
            payload["view"]["review"]["requires_self_send_confirmation"] = json!(true)
        }),
        ("request amount", |payload| {
            payload["request"]["amount_str"] = json!("9")
        }),
    ];
    for (what, change) in alterations {
        let original = store.alter(&built.id, change);
        let refused = service.inspect_send(built.id.clone()).await.unwrap_err();
        assert!(
            refused
                .to_string()
                .contains("Prepared transaction was altered"),
            "{what}: {refused}"
        );
        let refused = sign(&service, &built).await.unwrap_err();
        assert!(
            refused.contains("Prepared transaction was altered"),
            "{what}: {refused}"
        );
        store.restore(&built.id, &original);
        assert_eq!(
            service.inspect_send(built.id.clone()).await.unwrap().stage,
            SendStage::Prepared,
            "{what}"
        );
    }
    let signed = sign(&service, &built).await.unwrap();
    store.alter(&signed.id, |payload| {
        payload["submission"]["payload"] = json!("0xdeadbeef")
    });
    let refused = service
        .broadcast_send(signed.id.clone(), vec![accepts.uri()])
        .await
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("Transaction artifact content was altered"),
        "{refused}"
    );
    assert!(submitted(&accepts).await.is_empty());
}

/// Signing confirms the digest of the reviewed send and happens once: a
/// different digest, or a nonce the account has used since the review,
/// signs nothing, and a signed send is not signed again.
#[tokio::test]
async fn signing_confirms_the_review_and_happens_once() {
    let chain = Arc::new(std::sync::Mutex::new(ChainView {
        nonce: 7,
        ..Default::default()
    }));
    let accepts = node(Node::Accepts, chain.clone()).await;
    let store = Store::new().await;
    let service = store.open(&[&accepts]).await;
    let built = service
        .build_send(request(&recipient(), "0.01"))
        .await
        .unwrap();
    let refused = service
        .sign_send(built.id.clone(), "altered".into(), Some(PASSWORD.into()))
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("review does not match"),
        "{refused}"
    );
    assert_eq!(
        service.inspect_send(built.id.clone()).await.unwrap().stage,
        SendStage::Prepared
    );

    chain.lock().unwrap().nonce = 9;
    let refused = sign(&service, &built).await.unwrap_err();
    assert!(
        refused.contains("Prepared nonce or network is stale"),
        "{refused}"
    );
    assert_eq!(
        service.inspect_send(built.id.clone()).await.unwrap().stage,
        SendStage::Prepared
    );

    chain.lock().unwrap().nonce = 7;
    let signed = sign(&service, &built).await.unwrap();
    assert_eq!(signed.stage, SendStage::Signed);
    let refused = sign(&service, &built).await.unwrap_err();
    assert!(refused.contains("already signed"), "{refused}");
    assert_eq!(
        json(&service.inspect_send(built.id).await.unwrap()),
        json(&signed)
    );
    assert!(submitted(&accepts).await.is_empty());
}

/// Every selected endpoint is proven on the send's network before the
/// signed bytes go to any of them; then each endpoint's answer is its own,
/// and a retry sends the same bytes again.
#[tokio::test]
async fn a_broadcast_proves_every_endpoint_before_submitting_to_any() {
    let chain = Arc::new(std::sync::Mutex::new(ChainView {
        nonce: 7,
        ..Default::default()
    }));
    let accepts = node(Node::Accepts, chain.clone()).await;
    let rejects = node(Node::Rejects, chain.clone()).await;
    let elsewhere = node(Node::OnAnotherNetwork, chain).await;
    let store = Store::new().await;
    let service = store.open(&[&accepts]).await;
    let built = service
        .build_send(request(&recipient(), "0.01"))
        .await
        .unwrap();
    let signed = sign(&service, &built).await.unwrap();
    let payload = signed.signed_payload.clone().unwrap();
    drop(service);

    let service = store.open(&[&accepts, &elsewhere]).await;
    let refused = service
        .broadcast_send(signed.id.clone(), vec![accepts.uri(), elsewhere.uri()])
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("wrong network"), "{refused}");
    assert!(submitted(&accepts).await.is_empty());
    assert!(submitted(&elsewhere).await.is_empty());
    assert!(
        service
            .inspect_send(signed.id.clone())
            .await
            .unwrap()
            .attempts
            .is_empty()
    );
    assert!(service.transactions().await.unwrap().is_empty());
    drop(service);

    let service = store.open(&[&accepts, &rejects]).await;
    let sent = service
        .broadcast_send(signed.id.clone(), vec![accepts.uri(), rejects.uri()])
        .await
        .unwrap();
    assert_eq!(sent.attempts.len(), 2);
    assert_eq!(sent.attempts[0].outcome, SubmissionOutcome::Accepted);
    assert_eq!(sent.attempts[0].transaction_hash, signed.transaction_hash);
    assert_ne!(sent.attempts[1].outcome, SubmissionOutcome::Accepted);
    assert!(
        sent.attempts[1].detail.contains("transaction rejected"),
        "{}",
        sent.attempts[1].detail
    );
    let records = service.transactions().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].transaction_hash, signed.transaction_hash);

    let retried = service
        .broadcast_send(signed.id.clone(), vec![accepts.uri()])
        .await
        .unwrap();
    assert_eq!(retried.attempts.len(), 3);
    assert_eq!(retried.attempts[2].outcome, SubmissionOutcome::Accepted);
    assert_eq!(
        submitted(&accepts).await,
        [payload.clone(), payload.clone()]
    );
    assert_eq!(submitted(&rejects).await, [payload]);
    assert_eq!(service.transactions().await.unwrap().len(), 1);
}

/// A signed send reserves its nonce: another send at that nonce signs only
/// as a replacement, one that names the nonce and pays at least a tenth
/// more for both fees. The replaced send keeps its signed bytes.
#[tokio::test]
async fn only_an_explicit_nonce_with_higher_fees_replaces_a_signed_send() {
    let chain = Arc::new(std::sync::Mutex::new(ChainView {
        nonce: 7,
        ..Default::default()
    }));
    let accepts = node(Node::Accepts, chain).await;
    let store = Store::new().await;
    let service = store.open(&[&accepts]).await;
    let first = service
        .build_send(request(&recipient(), "0.01"))
        .await
        .unwrap();
    let competing = service
        .build_send(request(&recipient(), "0.02"))
        .await
        .unwrap();
    let signed = sign(&service, &first).await.unwrap();
    let PreparedPayload::Evm(reviewed) = service
        .load_send_artifact(first.id.clone())
        .await
        .unwrap()
        .prepared
    else {
        panic!("not an EVM transaction")
    };
    assert_eq!(reviewed.nonce, 7);
    let gwei = |wei: u128| crate::decimal::from_units(wei, 9);

    let refused = sign(&service, &competing).await.unwrap_err();
    assert!(
        refused.contains("already reserved by another signed transaction"),
        "{refused}"
    );
    let same_fees = service
        .build_send(replacement(
            7,
            gwei(reviewed.max_fee_per_gas),
            gwei(reviewed.max_priority_fee_per_gas),
        ))
        .await
        .unwrap();
    let refused = sign(&service, &same_fees).await.unwrap_err();
    assert!(
        refused.contains("already reserved by another signed transaction"),
        "{refused}"
    );
    for refused in [&competing, &same_fees] {
        assert_eq!(
            service
                .inspect_send(refused.id.clone())
                .await
                .unwrap()
                .stage,
            SendStage::Prepared
        );
    }

    let bumped = service
        .build_send(replacement(
            7,
            gwei(reviewed.max_fee_per_gas * 11 / 10),
            gwei(reviewed.max_priority_fee_per_gas * 11 / 10),
        ))
        .await
        .unwrap();
    let replaced = sign(&service, &bumped).await.unwrap();
    assert_eq!(replaced.stage, SendStage::Signed);
    assert_ne!(replaced.signed_payload, signed.signed_payload);
    assert_eq!(
        json(&service.inspect_send(first.id).await.unwrap()),
        json(&signed)
    );
    assert!(submitted(&accepts).await.is_empty());
}

/// A stored send holds no secret: not the phrase, its key or the password
/// that sealed them. Deleting the wallet deletes its sends and the
/// reservations they hold.
#[tokio::test]
async fn stored_sends_hold_no_secret_and_go_with_their_wallet() {
    let chain = Arc::new(std::sync::Mutex::new(ChainView {
        nonce: 7,
        ..Default::default()
    }));
    let accepts = node(Node::Accepts, chain).await;
    let store = Store::new().await;
    let service = store.open(&[&accepts]).await;
    let built = service
        .build_send(request(&recipient(), "0.01"))
        .await
        .unwrap();
    service
        .build_send(request(&recipient(), "0.02"))
        .await
        .unwrap();
    sign(&service, &built).await.unwrap();
    let key = crate::derivation::dispatch::derive_for_chain(
        Chain::Ethereum,
        SEED,
        PATH,
        None,
        None,
        None,
        false,
        false,
        true,
    )
    .unwrap()
    .private_key_hex
    .unwrap();
    let rows = store.rows();
    assert_eq!(rows.len(), 2);
    for row in &rows {
        for secret in ["abandon", key.as_str(), PASSWORD] {
            assert!(!row.contains(secret), "a stored send holds {secret}");
        }
    }
    assert_eq!(store.count("send_reservations"), 1);

    service
        .apply_state_command(StateCommand::RemoveWallet {
            wallet_id: "w".into(),
        })
        .await
        .unwrap();
    assert!(service.list_sends().await.unwrap().is_empty());
    assert_eq!(store.count("send_artifacts"), 0);
    assert_eq!(store.count("send_reservations"), 0);
}
