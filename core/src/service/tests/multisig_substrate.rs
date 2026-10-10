//! A pallet-multisig account on Asset Hub, against a node whose
//! `System.Account` and `Multisig.Multisigs` storage the test sets: each
//! transfer and approval the runtime would refuse is refused before
//! anything is submitted.

use super::*;
use crate::derivation::substrate_multisig::SubstrateMultisig;
use crate::store::secret_backends::InMemorySecretStore;
use crate::store::wallet_secrets::store_seed_phrase;
use parity_scale_codec::{Compact, Decode, Encode};
use serde_json::Value;
use std::sync::Mutex;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const CHAIN: Chain = Chain::Polkadot;
const DOT: u128 = 10_000_000_000;
const PHRASES: [(&str, &str); 4] = [
    (
        "s0",
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    ),
    (
        "s1",
        "legal winner thank year wave sausage worth useful legal winner thank yellow",
    ),
    (
        "s2",
        "letter advice cage absurd amount doctor acoustic avoid letter advice cage above",
    ),
    (
        "outsider",
        "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong",
    ),
];
/// `twox128("System") ++ twox128("Account")`.
const SYSTEM_ACCOUNT: &str = "26aa394eea5630e07c48ae0c9558cef7b99d880ec681799c0cf30e8886371da9";

/// An operation in flight: its timepoint, depositor and approvals.
struct Pending {
    when: (u32, u32),
    depositor: [u8; 32],
    approvals: Vec<[u8; 32]>,
}

/// What the node holds, and the signer of every extrinsic submitted.
struct Pallet {
    free: HashMap<[u8; 32], u128>,
    pending: Option<Pending>,
    submitted: Vec<[u8; 32]>,
}

fn block_hash() -> String {
    format!("0x{}", "11".repeat(32))
}

async fn node() -> (MockServer, Arc<Mutex<Pallet>>) {
    let live = Arc::new(Mutex::new(Pallet {
        free: HashMap::new(),
        pending: None,
        submitted: Vec::new(),
    }));
    let state = live.clone();
    let metadata = format!(
        "0x{}",
        hex::encode(crate::api::substrate_json_rpc::tests::fixture(CHAIN))
    );
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let mut live = state.lock().unwrap();
            let params = &body["params"];
            let result = match body["method"].as_str().unwrap() {
                "chain_getBlockHash" if params[0] == 0 => {
                    json!(CHAIN.substrate_genesis_hash().unwrap())
                }
                "chain_getBlockHash" | "chain_getFinalizedHead" => json!(block_hash()),
                "chain_getHeader" => json!({"number": "0xc8"}),
                "state_getRuntimeVersion" => {
                    json!({"specVersion": 2_005_000, "transactionVersion": 15})
                }
                "state_getMetadata" => json!(metadata),
                "system_accountNextIndex" => json!(0),
                "payment_queryInfo" => json!({"partialFee": "10000000",
                    "weight": {"refTime": 1_234_567, "proofSize": 8_910}}),
                "state_getStorage" => {
                    let key =
                        hex::decode(params[0].as_str().unwrap().trim_start_matches("0x")).unwrap();
                    if hex::encode(&key).starts_with(SYSTEM_ACCOUNT) {
                        let account: [u8; 32] = key[key.len() - 32..].try_into().unwrap();
                        let free = live.free.get(&account).copied().unwrap_or(0);
                        let mut record = vec![0; 16];
                        record.extend(free.to_le_bytes());
                        record.extend([0; 48]);
                        json!(format!("0x{}", hex::encode(record)))
                    } else {
                        match &live.pending {
                            None => Value::Null,
                            Some(pending) => {
                                let mut record = pending.when.0.encode();
                                record.extend(pending.when.1.encode());
                                record.extend(2_015_200_000u128.to_le_bytes());
                                record.extend(pending.depositor);
                                record.extend(pending.approvals.encode());
                                json!(format!("0x{}", hex::encode(record)))
                            }
                        }
                    }
                }
                "author_submitExtrinsic" => {
                    let raw =
                        hex::decode(params[0].as_str().unwrap().trim_start_matches("0x")).unwrap();
                    let mut input = raw.as_slice();
                    Compact::<u32>::decode(&mut input).unwrap();
                    assert_eq!(input[..2], [0x84, 0x00]);
                    live.submitted.push(input[2..34].try_into().unwrap());
                    use blake2::{Blake2b, Digest, digest::consts::U32};
                    json!(format!("0x{}", hex::encode(Blake2b::<U32>::digest(&raw))))
                }
                other => panic!("unexpected {other}"),
            };
            ResponseTemplate::new(200)
                .set_body_json(json!({"jsonrpc": "2.0", "id": body["id"], "result": result}))
        })
        .mount(&server)
        .await;
    (server, live)
}

/// The account (2 of s0, s1 and s2), each signatory's wallet and an
/// outsider's, every one of them holding ten DOT.
struct Account {
    service: Arc<WalletService>,
    policy: SubstrateMultisig,
    keys: HashMap<&'static str, [u8; 32]>,
    live: Arc<Mutex<Pallet>>,
    _server: MockServer,
}

async fn account() -> Account {
    let (server, live) = node().await;
    let service = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: CHAIN,
        endpoints: vec![server.uri()],
    }])
    .unwrap();
    let db = std::env::temp_dir().join(format!(
        "substrate-multisig-{}.sqlite",
        crate::store::new_event_id()
    ));
    service
        .open_state(db.to_string_lossy().into())
        .await
        .unwrap();
    let secrets = Arc::new(InMemorySecretStore::new());
    service.set_secret_store(secrets.clone());
    let path = crate::derivation::path::default_path_from_catalog(CHAIN).unwrap();
    let mut addresses = HashMap::new();
    for (id, phrase) in PHRASES {
        let address = crate::derivation::dispatch::derive_for_chain(
            CHAIN, phrase, &path, None, None, None, true, false, false,
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
                    address.clone(),
                    Some(path.clone()),
                    false,
                ),
            })
            .await
            .unwrap();
        store_seed_phrase(&*secrets, id, phrase, None).unwrap();
        addresses.insert(id, address);
    }
    let text =
        json!({"threshold": 2, "signatories": [addresses["s2"], addresses["s0"], addresses["s1"]]})
            .to_string();
    let policy = SubstrateMultisig::parse(CHAIN, &text).unwrap();
    let mut vault =
        WalletState::single_address("vault", "Vault", CHAIN, policy.address(), None, true);
    vault.multisig_policy = Some(text);
    service
        .apply_state_command(StateCommand::UpsertWallet { wallet: vault })
        .await
        .unwrap();
    let keys: HashMap<_, _> = addresses
        .iter()
        .map(|(id, address)| {
            let (_, key) =
                crate::derivation::primitives::decode_ss58(address, CHAIN.ss58_prefix()).unwrap();
            (*id, key)
        })
        .collect();
    {
        let mut live = live.lock().unwrap();
        live.free.insert(policy.account_id(), 10 * DOT);
        for key in keys.values() {
            live.free.insert(*key, 10 * DOT);
        }
    }
    Account {
        service,
        policy,
        keys,
        live,
        _server: server,
    }
}

impl Account {
    fn spend(&self, amount: &str) -> MultisigSpend {
        MultisigSpend {
            to_address: self.policy.address_of(&[0xdd; 32]),
            amount: amount.into(),
            fee_rate: None,
            expires_in_secs: None,
            memo: None,
        }
    }

    async fn sign(&self, session: &MultisigSession, signer: &str) -> Result<(), String> {
        self.service
            .sign_multisig(
                session.id.clone(),
                session.review_digest.clone(),
                Some(signer.into()),
                None,
            )
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn submitted(&self) -> Vec<[u8; 32]> {
        self.live.lock().unwrap().submitted.clone()
    }
}

#[tokio::test]
async fn a_transfer_the_runtime_would_refuse_is_not_built() {
    let account = account().await;
    for (amount, expected) in [
        // All ten DOT would take the account's existential deposit.
        ("10", "keep its existential deposit"),
        // A thousandth of one would leave the recipient below its own.
        ("0.001", "less than the existential deposit"),
    ] {
        let refusal = account
            .service
            .create_multisig("vault".into(), account.spend(amount))
            .await
            .unwrap_err()
            .to_string();
        assert!(refusal.contains(expected), "{refusal}");
    }
    assert!(
        account
            .service
            .multisig_sessions("vault".into())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn the_same_transfer_is_built_once_while_a_session_holds_it() {
    let account = account().await;
    let created = account
        .service
        .create_multisig("vault".into(), account.spend("1"))
        .await
        .unwrap();
    let refusal = account
        .service
        .create_multisig("vault".into(), account.spend("1"))
        .await
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("already makes this transfer"), "{refusal}");
    assert_eq!(
        account
            .service
            .multisig_sessions("vault".into())
            .await
            .unwrap(),
        std::slice::from_ref(&created)
    );
    // Discarded, it is free to build again.
    account
        .service
        .discard_multisig(created.id.clone())
        .await
        .unwrap();
    let again = account
        .service
        .create_multisig("vault".into(), account.spend("1"))
        .await
        .unwrap();
    assert_eq!(again.transaction_id, created.transaction_id);
}

#[tokio::test]
async fn an_approval_the_pallet_would_refuse_is_never_submitted() {
    let account = account().await;
    let service = &account.service;
    let created = service
        .create_multisig("vault".into(), account.spend("1"))
        .await
        .unwrap();
    let refusal = account.sign(&created, "outsider").await.unwrap_err();
    assert!(
        refusal.contains("not one of the account's signatories"),
        "{refusal}"
    );
    assert!(account.submitted().is_empty());

    // The first approval, by hash, goes out; it is not in a block yet.
    account.sign(&created, "s0").await.unwrap();
    assert_eq!(account.submitted(), [account.keys["s0"]]);
    let approved = service.multisig_session(created.id.clone()).await.unwrap();
    assert_eq!(approved.signed_weight, 1);
    assert!(!approved.complete && approved.submitted_txid.is_none());
    let refusal = account.sign(&created, "s2").await.unwrap_err();
    assert!(refusal.contains("not in a block yet"), "{refusal}");

    // In a block, it is read back; its signatory cannot approve again.
    account.live.lock().unwrap().pending = Some(Pending {
        when: (200, 1),
        depositor: account.keys["s0"],
        approvals: vec![account.keys["s0"]],
    });
    let read = service
        .import_multisig("vault".into(), created.data.clone())
        .await
        .unwrap();
    assert_eq!(read.id, created.id);
    assert_eq!(read.sequence.as_deref(), Some("200-1"));
    let refusal = account.sign(&created, "s0").await.unwrap_err();
    assert!(refusal.contains("already approved"), "{refusal}");

    // Approvals are the submissions: nothing is left to finalize or submit.
    let refusal = service
        .submit_multisig(created.id.clone(), None, None)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        refusal.contains("goes to the network as it signs"),
        "{refusal}"
    );
    let refusal = service
        .finalize_multisig(created.id.clone())
        .await
        .unwrap_err()
        .to_string();
    assert!(
        refusal.contains("goes to the network as it signs"),
        "{refusal}"
    );

    // Executed elsewhere, the operation is gone; a late approval is refused.
    account.live.lock().unwrap().pending = None;
    let refusal = account.sign(&created, "s2").await.unwrap_err();
    assert!(refusal.contains("executed or cancelled"), "{refusal}");
    assert_eq!(account.submitted(), [account.keys["s0"]]);
}
