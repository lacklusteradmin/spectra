//! A Safe's sessions through the service, against a loopback Sepolia node
//! that answers for an official 1.4.1 proxy whose owners are
//! `safe-multisig.json`'s. The node is read again before each signature and
//! execution; what it says is changed between them.

use super::*;
use crate::derivation::import::{WalletImportCommit, WalletImportKind, WalletImportRequest};
use crate::store::secret_backends::InMemorySecretStore;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const CHAIN: Chain = Chain::EthereumSepolia;
/// The 1.4.1 factory's proxy, its singleton in slot 0.
const PROXY: &str = "0x608060405273ffffffffffffffffffffffffffffffffffffffff600054167fa619486e0000000000000000000000000000000000000000000000000000000060003514156050578060005260206000f35b3660008037600080366000845af43d6000803e60008114156070573d6000fd5b3d6000f3fea264697066735822122003d1488ee65e08fa41e58e888a9865554c535f2c77126a82cb4c0f917f31441364736f6c63430007060033";
const SINGLETON: &str = "41675c099f32341bf84bfc5382af534df5c7461a";
const GUARD_SLOT: &str = "0x4a204f620c8c5ccdca3fd54d003badd85ba500436a431f0cbda4f558c93c34c8";

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/safe-multisig.json")).unwrap()
}

/// What the node says of the Safe, and the transactions sent to it.
#[derive(Clone)]
struct Safe {
    owners: Vec<String>,
    threshold: u64,
    nonce: u64,
    module: Option<String>,
    guard: Option<String>,
    sent: Vec<String>,
}

type Node = Arc<std::sync::Mutex<Safe>>;

fn word(value: u64) -> String {
    format!("{value:064x}")
}

fn address_word(address: &str) -> String {
    format!(
        "{:0>64}",
        address.trim_start_matches("0x").to_ascii_lowercase()
    )
}

/// One JSON-RPC call answered from `safe`.
fn answer(safe: &mut Safe, address: &str, call: &serde_json::Value) -> serde_json::Value {
    let params = &call["params"];
    let to_safe = |value: &serde_json::Value| {
        value
            .as_str()
            .is_some_and(|value| value.eq_ignore_ascii_case(address))
    };
    let result = match call["method"].as_str().unwrap_or_default() {
        "eth_chainId" => json!(format!("0x{:x}", CHAIN.evm_chain_id().unwrap())),
        "eth_getBalance" => json!(format!("0x{:x}", 10u128 * 10u128.pow(18))),
        "eth_getCode" if to_safe(&params[0]) => json!(PROXY),
        "eth_getStorageAt" if to_safe(&params[0]) => {
            let slot = params[1].as_str().unwrap_or_default();
            json!(match (slot, &safe.guard) {
                ("0x0", _) => format!("0x{}", address_word(SINGLETON)),
                (GUARD_SLOT, Some(guard)) => format!("0x{}", address_word(guard)),
                _ => format!("0x{}", word(0)),
            })
        }
        "eth_call" if to_safe(&params[0]["to"]) => {
            let data = params[0]["data"].as_str().unwrap_or_default();
            json!(match &data[2..10] {
                "a0e67e2b" => format!(
                    "0x{}{}{}",
                    word(32),
                    word(safe.owners.len() as u64),
                    safe.owners
                        .iter()
                        .map(|o| address_word(o))
                        .collect::<String>()
                ),
                "e75235b8" => format!("0x{}", word(safe.threshold)),
                "affed0e0" => format!("0x{}", word(safe.nonce)),
                "ffa1ad74" => format!("0x{}{}{:0<64}", word(32), word(5), hex::encode("1.4.1")),
                "cc2f8452" => format!(
                    "0x{}{}{}{}",
                    word(64),
                    word(1),
                    word(safe.module.iter().count() as u64),
                    safe.module
                        .iter()
                        .map(|m| address_word(m))
                        .collect::<String>()
                ),
                other => panic!("unexpected Safe call {other}"),
            })
        }
        "eth_sendRawTransaction" => {
            safe.sent.push(params[0].as_str().unwrap().to_string());
            json!(format!("0x{}", "00".repeat(32)))
        }
        _ => {
            return json!({"jsonrpc": "2.0", "id": call["id"],
                "error": {"code": -32601, "message": "not answered here"}});
        }
    };
    json!({"jsonrpc": "2.0", "id": call["id"], "result": result})
}

async fn node(safe: Safe) -> (MockServer, Node) {
    let node = Arc::new(std::sync::Mutex::new(safe));
    let address = fixture()["safe"].as_str().unwrap().to_string();
    let server = MockServer::start().await;
    let answering = node.clone();
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            let mut safe = answering.lock().unwrap();
            ResponseTemplate::new(200).set_body_json(match body.as_array() {
                Some(calls) => json!(
                    calls
                        .iter()
                        .map(|call| answer(&mut safe, &address, call))
                        .collect::<Vec<_>>()
                ),
                None => answer(&mut safe, &address, &body),
            })
        })
        .mount(&server)
        .await;
    (server, node)
}

/// The fixture's Safe as the network holds it: three owners, two of whom
/// sign, at nonce 7.
fn safe() -> Safe {
    Safe {
        owners: fixture()["owners"]
            .as_array()
            .unwrap()
            .iter()
            .map(|owner| owner["address"].as_str().unwrap().to_ascii_lowercase())
            .collect(),
        threshold: 2,
        nonce: 7,
        module: None,
        guard: None,
        sent: Vec::new(),
    }
}

fn commit(name: &str, kind: WalletImportKind) -> WalletImportCommit {
    WalletImportCommit {
        password: None,
        request: WalletImportRequest {
            wallet_name: name.into(),
            chain: CHAIN,
            kind,
        },
        derivation_path: None,
        derivation_overrides: Default::default(),
        seed_phrase: None,
        private_key: None,
        restore_height: None,
        named_account: None,
        ton_wallet_version: None,
        upgrade_wallet_id: None,
    }
}

/// The wallets of one data directory: the Safe watched, owners 0 and 1,
/// and a key that is no owner.
struct Wallets {
    service: Arc<WalletService>,
    safe: String,
    owners: [String; 2],
    outsider: String,
}

async fn wallets(server: &MockServer) -> Wallets {
    let service = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: CHAIN,
        endpoints: vec![server.uri()],
    }])
    .unwrap();
    service.set_secret_store(Arc::new(InMemorySecretStore::new()));
    let database = std::env::temp_dir().join(format!(
        "multisig-safe-{}.sqlite",
        crate::store::new_event_id()
    ));
    service
        .open_state(database.to_string_lossy().into_owned())
        .await
        .unwrap();
    let fixture = fixture();
    let import = |commit: WalletImportCommit| {
        let service = service.clone();
        async move {
            service
                .import_wallets(commit)
                .await
                .unwrap()
                .wallets
                .remove(0)
                .id
        }
    };
    let key = |name: &str, key: &str| {
        let mut commit = commit(name, WalletImportKind::PrivateKey);
        commit.private_key = Some(key.trim_start_matches("0x").into());
        commit
    };
    let owner = |index: usize| fixture["owners"][index]["private_key"].as_str().unwrap();
    Wallets {
        safe: import(commit(
            "Safe",
            WalletImportKind::WatchAddresses {
                addresses: vec![fixture["safe"].as_str().unwrap().into()],
            },
        ))
        .await,
        owners: [
            import(key("Owner0", owner(0))).await,
            import(key("Owner1", owner(1))).await,
        ],
        outsider: import(key("Outsider", &"42".repeat(32))).await,
        service,
    }
}

impl Wallets {
    async fn create(&self) -> MultisigSession {
        self.service
            .create_multisig(
                self.safe.clone(),
                MultisigSpend {
                    to_address: fixture()["transactions"][0]["to"].as_str().unwrap().into(),
                    amount: "0.01".into(),
                    fee_rate: None,
                    expires_in_secs: None,
                    memo: None,
                },
            )
            .await
            .unwrap()
    }

    async fn sign(
        &self,
        session: &MultisigSession,
        signer: &str,
    ) -> Result<MultisigSession, String> {
        self.service
            .sign_multisig(
                session.id.clone(),
                session.review_digest.clone(),
                Some(signer.into()),
                None,
            )
            .await
            .map_err(|error| error.to_string())
    }

    async fn execute(&self, session: &MultisigSession, executor: &str) -> String {
        self.service
            .submit_multisig(session.id.clone(), Some(executor.into()), None)
            .await
            .unwrap_err()
            .to_string()
    }

    async fn signed_by(&self, session: &MultisigSession) -> usize {
        self.service
            .multisig_session(session.id.clone())
            .await
            .unwrap()
            .signers
            .iter()
            .filter(|signer| signer.signed)
            .count()
    }
}

/// An enabled module can move the Safe's funds and a guard can block its
/// transactions, neither needing its owners: the account names each, read
/// from the network, and names nothing when there is neither.
#[tokio::test]
async fn a_safes_modules_and_guard_are_warned_of() {
    let module = format!("0x{}", "77".repeat(20));
    let guard = format!("0x{}", "88".repeat(20));
    let (server, node) = node(Safe {
        module: Some(module.clone()),
        guard: Some(guard.clone()),
        ..safe()
    })
    .await;
    let wallets = wallets(&server).await;
    let warnings: Vec<String> = wallets
        .service
        .multisig_account(wallets.safe.clone())
        .await
        .unwrap()
        .warnings
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        warnings,
        [
            format!("Module {module} can move this Safe's funds without its owners' signatures."),
            format!("Guard {guard} checks every transaction and can block it."),
        ]
    );
    {
        let mut safe = node.lock().unwrap();
        safe.module = None;
        safe.guard = None;
    }
    let account = wallets
        .service
        .multisig_account(wallets.safe.clone())
        .await
        .unwrap();
    assert!(account.warnings.is_empty(), "{:?}", account.warnings);
}

/// The account names, of the owners the network lists, those whose keys a
/// wallet on this device holds: those wallets may sign.
#[tokio::test]
async fn the_account_names_the_owner_wallets_this_device_holds() {
    let (server, _) = node(safe()).await;
    let wallets = wallets(&server).await;
    let account = wallets
        .service
        .multisig_account(wallets.safe.clone())
        .await
        .unwrap();
    let named: Vec<(String, Option<String>)> = account.permissions[0]
        .signers
        .iter()
        .map(|signer| (signer.signer.clone(), signer.wallet_id.clone()))
        .collect();
    let owners = safe().owners;
    assert_eq!(
        named,
        [
            (owners[0].clone(), Some(wallets.owners[0].clone())),
            (owners[1].clone(), Some(wallets.owners[1].clone())),
            (owners[2].clone(), None),
        ]
    );
    let mut signers = wallets.owners.to_vec();
    signers.sort();
    assert_eq!(account.signer_wallet_ids, signers);
}

/// A Safe transaction is executed only once it carries the threshold's
/// signatures, and only by a wallet that is one of the Safe's owners, who
/// pays the gas; refused, nothing is sent.
#[tokio::test]
async fn execution_is_refused_short_of_the_threshold_and_to_a_wallet_that_is_no_owner() {
    let (server, node) = node(safe()).await;
    let wallets = wallets(&server).await;
    let session = wallets.create().await;
    wallets.sign(&session, &wallets.owners[0]).await.unwrap();
    let short = wallets.execute(&session, &wallets.owners[0]).await;
    assert_eq!(
        short,
        "The session does not yet carry enough owners' signatures."
    );
    assert!(
        wallets
            .sign(&session, &wallets.owners[1])
            .await
            .unwrap()
            .complete
    );
    let outsider = wallets.execute(&session, &wallets.outsider).await;
    assert_eq!(outsider, "Outsider is not one of the Safe's owners.");
    assert!(node.lock().unwrap().sent.is_empty());
    let kept = wallets.service.multisig_session(session.id).await.unwrap();
    assert_eq!(kept.submitted_txid, None);
}

/// A session is built under the owners and threshold the Safe had then: an
/// owner signing after either changed is refused, and so is an execution,
/// and neither leaves a mark.
#[tokio::test]
async fn a_session_is_refused_once_the_safes_owners_or_threshold_change() {
    let (server, node) = node(safe()).await;
    let wallets = wallets(&server).await;
    let session = wallets.create().await;
    wallets.sign(&session, &wallets.owners[0]).await.unwrap();
    let changed = "The Safe's owners, threshold or version changed since this session was built; build it again.";

    node.lock().unwrap().threshold = 3;
    assert_eq!(
        wallets.sign(&session, &wallets.owners[1]).await,
        Err(changed.into())
    );
    assert_eq!(wallets.signed_by(&session).await, 1);

    node.lock().unwrap().threshold = 2;
    assert!(
        wallets
            .sign(&session, &wallets.owners[1])
            .await
            .unwrap()
            .complete
    );
    node.lock().unwrap().owners[2] = format!("0x{}", "99".repeat(20));
    assert_eq!(wallets.execute(&session, &wallets.owners[0]).await, changed);
    assert!(node.lock().unwrap().sent.is_empty());
}

/// A session takes the Safe's next nonce that no open session holds; once
/// the Safe has used a session's nonce, that session is neither signed nor
/// imported again, while one at a nonce still ahead is.
#[tokio::test]
async fn a_used_nonce_is_refused_and_a_new_session_takes_the_next_free_one() {
    let (server, node) = node(safe()).await;
    let wallets = wallets(&server).await;
    let first = wallets.create().await;
    let second = wallets.create().await;
    assert_eq!(
        (first.sequence.as_deref(), second.sequence.as_deref()),
        (Some("7"), Some("8"))
    );

    node.lock().unwrap().nonce = 8;
    let used = "This Safe transaction's nonce was already used.";
    assert_eq!(
        wallets.sign(&first, &wallets.owners[0]).await,
        Err(used.into())
    );
    let imported = wallets
        .service
        .import_multisig(wallets.safe.clone(), first.data.clone())
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(imported, used);
    assert_eq!(wallets.signed_by(&first).await, 0);
    wallets.sign(&second, &wallets.owners[0]).await.unwrap();

    assert_eq!(wallets.create().await.sequence.as_deref(), Some("9"));
}
