//! A `sortedmulti` account's sessions through the service, against a
//! loopback indexer: Bitcoin's and Litecoin's PSBTs (`multisig-psbt.json`)
//! and Dogecoin's partially signed transactions (`p2sh-multisig.json`).
//! Each service is a data directory of its own.

use super::*;
use crate::derivation::import::{WalletImportCommit, WalletImportKind, WalletImportRequest};
use crate::service::multisig::MultisigInput;
use crate::store::secret_backends::InMemorySecretStore;
use crate::store::state::AppSettingUpdate;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

/// The unspent outputs (txid, vout, value) the indexer holds per address.
type Funds = Arc<std::sync::Mutex<HashMap<String, Vec<(String, u32, u64)>>>>;
/// What was posted to the indexer: broadcasts.
type Posted = Arc<std::sync::Mutex<Vec<String>>>;

/// A loopback indexer answering for `funds`, as Esplora at its root and as
/// Blockbook under `/api/v2`, keeping whatever is posted to it.
async fn indexer(funds: Funds, posted: Posted) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(move |request: &Request| {
            if request.method.as_str() == "POST" {
                posted
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&request.body).into_owned());
                return ResponseTemplate::new(200).set_body_json(json!({"result": "00".repeat(32)}));
            }
            let path = request.url.path().to_string();
            let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
            let held = |address: &str| {
                funds
                    .lock()
                    .unwrap()
                    .get(address)
                    .cloned()
                    .unwrap_or_default()
            };
            let body = match parts.as_slice() {
                ["fee-estimates"] => json!({"6": 2.5, "144": 1.0}),
                ["address", address] => {
                    let held = held(address);
                    let funded: u64 = held.iter().map(|(_, _, value)| value).sum();
                    json!({
                        "address": address,
                        "chain_stats": {"funded_txo_sum": funded, "spent_txo_sum": 0, "tx_count": held.len()},
                        "mempool_stats": {"funded_txo_sum": 0, "spent_txo_sum": 0, "tx_count": 0},
                    })
                }
                ["address", address, "utxo"] => json!(
                    held(address)
                        .iter()
                        .map(|(txid, vout, value)| json!({
                            "txid": txid, "vout": vout, "value": value,
                            "status": {"confirmed": true, "block_height": 1},
                        }))
                        .collect::<Vec<_>>()
                ),
                ["api", "v2", "address", address] => {
                    let held = held(address);
                    let balance: u64 = held.iter().map(|(_, _, value)| value).sum();
                    json!({
                        "address": address, "balance": balance.to_string(),
                        "unconfirmedBalance": "0", "txs": held.len(), "unconfirmedTxs": 0,
                    })
                }
                ["api", "v2", "utxo", address] => json!(
                    held(address)
                        .iter()
                        .map(|(txid, vout, value)| json!({
                            "txid": txid, "vout": vout, "value": value.to_string(),
                            "confirmations": 6, "height": 1,
                        }))
                        .collect::<Vec<_>>()
                ),
                _ => return ResponseTemplate::new(404),
            };
            ResponseTemplate::new(200).set_body_json(body)
        })
        .mount(&server)
        .await;
    server
}

/// A fresh data directory on `chain` reading `server`: Dogecoin's through
/// Blockbook, one of its indexers, rather than its default.
async fn service(chain: Chain, server: &MockServer) -> Arc<WalletService> {
    let service = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: chain,
        endpoints: vec![server.uri()],
    }])
    .unwrap();
    service.set_secret_store(Arc::new(InMemorySecretStore::new()));
    let database = std::env::temp_dir().join(format!(
        "multisig-psbt-{}.sqlite",
        crate::store::new_event_id()
    ));
    service
        .open_state(database.to_string_lossy().into_owned())
        .await
        .unwrap();
    if chain.mainnet_counterpart() == Chain::Dogecoin {
        service
            .apply_state_command(StateCommand::SetAppSetting {
                update: AppSettingUpdate::AddCustomEndpoint {
                    capabilities: crate::endpoint_api::endpoint_capability_options(
                        chain,
                        crate::EndpointApi::Blockbook,
                    ),
                    chain_id: chain,
                    api: "blockbook".into(),
                    endpoint: server.uri(),
                },
            })
            .await
            .unwrap();
    }
    service
}

fn commit(chain: Chain, kind: WalletImportKind) -> WalletImportCommit {
    WalletImportCommit {
        password: None,
        request: WalletImportRequest {
            wallet_name: "Vault".into(),
            chain,
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

/// A 2-of-3 account of a fixture's network: its descriptor, its addresses
/// by place, a recipient, and its cosigners' phrases and fingerprints.
struct Account {
    chain: Chain,
    descriptor: String,
    addresses: HashMap<(u64, u64), String>,
    recipient: String,
    phrases: Vec<String>,
    fingerprints: Vec<String>,
}

impl Account {
    fn of(fixture: &serde_json::Value, chain: Chain, script: &str, spend: &str) -> Self {
        let network = fixture["networks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|network| network["chain"] == chain.str_id())
            .unwrap();
        let cosigners = network["cosigners"].as_array().unwrap();
        let keys: Vec<String> = cosigners
            .iter()
            .map(|c| {
                format!(
                    "[{}/{}]{}/<0;1>/*",
                    c["fingerprint"].as_str().unwrap(),
                    c["origin"].as_str().unwrap(),
                    c["xpub"].as_str().unwrap()
                )
            })
            .collect();
        let text = |value: &serde_json::Value| value.as_str().unwrap().to_string();
        Self {
            chain,
            descriptor: format!("{script}(sortedmulti(2,{}))", keys.join(",")),
            addresses: network["addresses"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| {
                    (
                        (a["branch"].as_u64().unwrap(), a["index"].as_u64().unwrap()),
                        text(&a["address"]),
                    )
                })
                .collect(),
            recipient: text(&network[spend]["recipient"]["address"]),
            phrases: fixture["phrases"]
                .as_array()
                .unwrap()
                .iter()
                .map(text)
                .collect(),
            fingerprints: cosigners.iter().map(|c| text(&c["fingerprint"])).collect(),
        }
    }

    fn psbt(chain: Chain) -> Self {
        Self::of(
            &crate::derivation::multisig::tests::fixture(),
            chain,
            "wsh",
            "psbt",
        )
    }

    fn p2sh(chain: Chain) -> Self {
        let fixture =
            serde_json::from_str(include_str!("../../../tests/fixtures/p2sh-multisig.json"))
                .unwrap();
        Self::of(&fixture, chain, "sh", "transaction")
    }

    fn at(&self, branch: u64, index: u64) -> String {
        self.addresses[&(branch, index)].clone()
    }

    /// The account watched in `service`, as a coordinator holds it.
    async fn watch(&self, service: &WalletService) -> String {
        service
            .import_wallets(commit(
                self.chain,
                WalletImportKind::WatchMultisig {
                    policy: self.descriptor.clone(),
                },
            ))
            .await
            .unwrap()
            .wallets
            .remove(0)
            .id
    }

    /// The account watched in `service` and given cosigner `cosigner`'s
    /// phrase, as that cosigner holds it.
    async fn cosigner(&self, service: &WalletService, cosigner: usize) -> String {
        let wallet = self.watch(service).await;
        let mut phrase = commit(self.chain, WalletImportKind::Phrase);
        phrase.seed_phrase = Some(self.phrases[cosigner].clone());
        phrase.upgrade_wallet_id = Some(wallet.clone());
        assert!(service.import_wallets(phrase).await.unwrap().upgraded);
        wallet
    }
}

fn spend(to: &str, amount: &str, fee_rate: Option<&str>) -> MultisigSpend {
    MultisigSpend {
        to_address: to.into(),
        amount: amount.into(),
        fee_rate: fee_rate.map(Into::into),
        expires_in_secs: None,
        memo: None,
    }
}

fn addresses(inputs: &[MultisigInput]) -> Vec<String> {
    inputs.iter().map(|input| input.address.clone()).collect()
}

fn signed_by(session: &MultisigSession) -> Vec<String> {
    session
        .signers
        .iter()
        .filter(|signer| signer.signed)
        .map(|signer| signer.signer.clone())
        .collect()
}

/// A PSBT pays transparent outputs only: a Litecoin session to an MWEB
/// address is refused before the network is read, and nothing is kept.
#[tokio::test]
async fn a_litecoin_psbt_cannot_pay_an_mweb_address() {
    let account = Account::psbt(Chain::Litecoin);
    let server = indexer(Funds::default(), Posted::default()).await;
    let service = service(Chain::Litecoin, &server).await;
    let vault = account.watch(&service).await;
    let key = secp256k1::PublicKey::from_secret_key(
        &secp256k1::Secp256k1::new(),
        &secp256k1::SecretKey::from_slice(&[1; 32]).unwrap(),
    );
    let mweb = crate::send::litecoin_mweb::keys::StealthAddress {
        scan: key,
        spend: key,
    }
    .encode(Chain::Litecoin)
    .unwrap();
    let read = server.received_requests().await.unwrap().len();
    let refused = service
        .create_multisig(vault.clone(), spend(&mweb, "0.001", Some("2")))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("A PSBT cannot pay an MWEB address"),
        "{refused}"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), read);
    assert!(service.multisig_sessions(vault).await.unwrap().is_empty());
}

/// A PSBT spends the account's confirmed coins largest first, as few as
/// pay the amount and the fee at the rate asked, its change to the next
/// change address discovery leaves. A second session takes only coins no
/// open session spends, at the network's rate rounded up to a whole sat/vB.
#[tokio::test]
async fn a_psbt_spends_the_largest_free_coins_and_pays_change_to_the_next_change_address() {
    let account = Account::psbt(Chain::Bitcoin);
    let funds = Funds::default();
    funds.lock().unwrap().extend([
        (account.at(0, 0), vec![("aa".repeat(32), 0, 100_000)]),
        (account.at(0, 2), vec![("bb".repeat(32), 1, 300_000)]),
        (account.at(1, 0), vec![("cc".repeat(32), 2, 50_000)]),
    ]);
    let server = indexer(funds, Posted::default()).await;
    let service = service(Chain::Bitcoin, &server).await;
    let vault = account.watch(&service).await;
    service
        .discover_utxo_addresses(vault.clone(), Chain::Bitcoin)
        .await
        .unwrap();

    let first = service
        .create_multisig(
            vault.clone(),
            spend(&account.recipient, "0.0032", Some("2")),
        )
        .await
        .unwrap();
    assert_eq!(
        addresses(&first.inputs),
        [account.at(0, 2), account.at(0, 0)]
    );
    // Two inputs, a P2WPKH payment and P2WSH change: 294 vB at 2 sat/vB.
    assert_eq!(first.fee, "588");
    let [payment, change] = first.outputs.as_slice() else {
        panic!("{:?}", first.outputs)
    };
    assert_eq!(
        (
            payment.address.as_str(),
            payment.value.as_str(),
            payment.is_change
        ),
        (account.recipient.as_str(), "320000", false)
    );
    assert_eq!(
        (
            change.address.clone(),
            change.value.as_str(),
            change.is_change
        ),
        (account.at(1, 1), "79412", true)
    );

    // The network asks 2.5 sat/vB for six blocks; the one coin the first
    // session leaves pays 189 vB at 3.
    let second = service
        .create_multisig(vault, spend(&account.recipient, "0.0003", None))
        .await
        .unwrap();
    assert_eq!(addresses(&second.inputs), [account.at(1, 0)]);
    assert_eq!(second.fee, "567");
}

/// A signature is given only by a wallet holding a cosigner's key, and only
/// for the session as reviewed: the watching coordinator, which holds no
/// key, is refused, and so is a cosigner naming another review digest;
/// neither signs anything.
#[tokio::test]
async fn a_signature_needs_a_cosigners_key_and_the_reviewed_digest() {
    let account = Account::psbt(Chain::Bitcoin);
    let funds = Funds::default();
    funds
        .lock()
        .unwrap()
        .insert(account.at(0, 0), vec![("aa".repeat(32), 0, 100_000)]);
    let server = indexer(funds, Posted::default()).await;
    let service = service(Chain::Bitcoin, &server).await;
    let vault = account.watch(&service).await;
    let created = service
        .create_multisig(
            vault.clone(),
            spend(&account.recipient, "0.0005", Some("2")),
        )
        .await
        .unwrap();

    let keyless = service
        .sign_multisig(
            created.id.clone(),
            created.review_digest.clone(),
            None,
            None,
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(keyless.contains("holds no cosigner's key"), "{keyless}");

    let mut phrase = commit(Chain::Bitcoin, WalletImportKind::Phrase);
    phrase.seed_phrase = Some(account.phrases[0].clone());
    phrase.upgrade_wallet_id = Some(vault);
    service.import_wallets(phrase).await.unwrap();
    let stale = service
        .sign_multisig(created.id.clone(), "00".repeat(32), None, None)
        .await
        .unwrap_err()
        .to_string();
    assert!(stale.contains("not the one reviewed"), "{stale}");
    let kept = service.multisig_session(created.id.clone()).await.unwrap();
    assert!(signed_by(&kept).is_empty());
    assert_eq!(kept.data, created.data);

    let signed = service
        .sign_multisig(created.id, created.review_digest, None, None)
        .await
        .unwrap();
    assert_eq!(signed_by(&signed), [account.fingerprints[0].clone()]);
}

/// Each input must still be a confirmed output of the amount reviewed when
/// a cosigner signs: spent, or of another amount, it is refused and the
/// session is left unsigned.
#[tokio::test]
async fn a_spent_or_changed_input_stops_the_signature() {
    let account = Account::psbt(Chain::Bitcoin);
    let funds = Funds::default();
    funds.lock().unwrap().extend([
        (account.at(0, 0), vec![("aa".repeat(32), 0, 100_000)]),
        (account.at(0, 2), vec![("bb".repeat(32), 1, 300_000)]),
    ]);
    let server = indexer(funds.clone(), Posted::default()).await;
    let service = service(Chain::Bitcoin, &server).await;
    let vault = account.cosigner(&service, 0).await;
    service
        .discover_utxo_addresses(vault.clone(), Chain::Bitcoin)
        .await
        .unwrap();
    let created = service
        .create_multisig(vault, spend(&account.recipient, "0.0032", Some("2")))
        .await
        .unwrap();
    assert_eq!(created.inputs.len(), 2);

    let held = funds.lock().unwrap().remove(&account.at(0, 2)).unwrap();
    for (now, outputs) in [
        ("spent", vec![]),
        ("of another amount", vec![("bb".repeat(32), 1, 299_999)]),
    ] {
        funds.lock().unwrap().insert(account.at(0, 2), outputs);
        let refused = service
            .sign_multisig(
                created.id.clone(),
                created.review_digest.clone(),
                None,
                None,
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("An input is spent, unconfirmed or not of the amount the PSBT says"),
            "{now}: {refused}"
        );
        let kept = service.multisig_session(created.id.clone()).await.unwrap();
        assert!(signed_by(&kept).is_empty(), "{now}");
    }
    funds.lock().unwrap().insert(account.at(0, 2), held);
    let signed = service
        .sign_multisig(created.id, created.review_digest, None, None)
        .await
        .unwrap();
    assert_eq!(signed_by(&signed), [account.fingerprints[0].clone()]);
}

/// A P2SH spend made without a fee rate pays the network's static fee for
/// each kilobyte it starts, as Dogecoin's ordinary sends do: 0.01 DOGE for
/// one input's 375 bytes, 0.02 for five inputs' 1,571.
#[tokio::test]
async fn a_dogecoin_spend_without_a_rate_pays_the_static_fee_per_started_kilobyte() {
    let account = Account::p2sh(Chain::Dogecoin);
    let funds = Funds::default();
    funds.lock().unwrap().insert(
        account.at(0, 0),
        (0..6)
            .map(|vout| ("aa".repeat(32), vout, 100_000_000))
            .collect(),
    );
    let server = indexer(funds, Posted::default()).await;
    let service = service(Chain::Dogecoin, &server).await;
    let vault = account.watch(&service).await;
    for (amount, inputs, fee) in [("0.5", 1, "1000000"), ("4.5", 5, "2000000")] {
        let created = service
            .create_multisig(vault.clone(), spend(&account.recipient, amount, None))
            .await
            .unwrap();
        assert_eq!((created.inputs.len(), created.fee.as_str()), (inputs, fee));
    }
}

/// Dogecoin's partially signed transactions carry no amounts, so a cosigner
/// in a fresh data directory reads them from the network: its own
/// account's coins, found by discovery where it has seen no address used.
/// Two cosigners sign, each from the other's copy; the coordinator joins
/// both copies and finalizes. The submission is refused, as no node a user
/// adds can prove it serves Dogecoin, and nothing reaches it.
#[tokio::test]
async fn dogecoin_cosigners_in_fresh_data_directories_find_the_coins_and_finish_the_spend() {
    let account = Account::p2sh(Chain::Dogecoin);
    let coin = 100_000_000;
    let funds = Funds::default();
    funds.lock().unwrap().extend([
        (account.at(0, 0), vec![("aa".repeat(32), 0, coin)]),
        (account.at(0, 2), vec![("bb".repeat(32), 1, 3 * coin)]),
        (account.at(1, 0), vec![("cc".repeat(32), 2, coin / 2)]),
    ]);
    let posted = Posted::default();
    let server = indexer(funds, posted.clone()).await;
    let coordinator = service(Chain::Dogecoin, &server).await;
    let vault = account.watch(&coordinator).await;
    coordinator
        .discover_utxo_addresses(vault.clone(), Chain::Dogecoin)
        .await
        .unwrap();
    let created = coordinator
        .create_multisig(vault.clone(), spend(&account.recipient, "3.2", None))
        .await
        .unwrap();
    assert_eq!(
        addresses(&created.inputs),
        [account.at(0, 2), account.at(0, 0)]
    );

    let mut copy = created.data.clone();
    for cosigner in [0, 1] {
        let directory = service(Chain::Dogecoin, &server).await;
        let wallet = account.cosigner(&directory, cosigner).await;
        let imported = directory.import_multisig(wallet, copy).await.unwrap();
        assert_eq!(imported.transaction_id, created.transaction_id);
        assert_eq!(imported.review_digest, created.review_digest);
        assert_eq!(addresses(&imported.inputs), addresses(&created.inputs));
        assert_eq!(imported.outputs, created.outputs);
        let signed = directory
            .sign_multisig(imported.id, imported.review_digest, None, None)
            .await
            .unwrap();
        assert_eq!(signed.complete, cosigner == 1);
        coordinator
            .import_multisig(vault.clone(), signed.data.clone())
            .await
            .unwrap();
        copy = signed.data;
    }

    let joined = coordinator
        .multisig_session(created.id.clone())
        .await
        .unwrap();
    assert!(joined.complete);
    assert_eq!(
        signed_by(&joined),
        [
            account.fingerprints[0].clone(),
            account.fingerprints[1].clone()
        ]
    );
    let raw = coordinator
        .finalize_multisig(created.id.clone())
        .await
        .unwrap();
    let tx: bitcoin::Transaction =
        bitcoin::consensus::deserialize(&hex::decode(raw).unwrap()).unwrap();
    assert_eq!(tx.compute_txid().to_string(), joined.transaction_id);

    let refused = coordinator
        .submit_multisig(created.id.clone(), None, None)
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("cannot be verified"), "{refused}");
    assert!(posted.lock().unwrap().is_empty());
    assert_eq!(
        coordinator
            .multisig_session(created.id)
            .await
            .unwrap()
            .submitted_txid,
        None
    );
}
