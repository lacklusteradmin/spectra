//! An account transfer as the service builds and signs it, against each
//! account network's own indexer: the outputs it spends come from every
//! address the account holds, each signed for by that address's key, and an
//! output gone since the review is not signed for. Zcash's transfer is bound
//! to its network upgrade and expiry height as well.
use super::*;
use crate::send::SendExecutionRequest;
use crate::send::account_utxo::AccountProtocol;
use crate::send::stages::{SendStage, SubmissionOutcome};
use crate::service::address_discovery::UtxoDerivation;
use crate::store::secret_backends::InMemorySecretStore;
use crate::store::state::WalletState;
use crate::store::wallet_secrets::store_seed_phrase;
use serde_json::{Value, json};
use std::sync::Mutex;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

/// What an account network's indexer holds and reports.
#[derive(Default)]
struct Ledger {
    /// Each address's confirmed outputs: (txid, vout, value).
    outputs: HashMap<String, Vec<(String, u32, u64)>>,
    /// Zcash: the tip, and a consensus branch the indexer claims in place
    /// of the network's own.
    height: u32,
    claimed_branch: Option<u32>,
    /// What a submission is answered with, and every one received.
    accepted_hash: String,
    submitted: Vec<String>,
}

impl Ledger {
    /// The outputs of the address a path names, whatever form of it the
    /// client sends (Bitcoin Cash's without its prefix).
    fn held(&self, address: &str) -> Vec<(String, u32, u64)> {
        self.outputs
            .iter()
            .find(|(held, _)| *held == address || held.rsplit(':').next() == Some(address))
            .map(|(_, outputs)| outputs.clone())
            .unwrap_or_default()
    }
}

/// The answer `chain`'s default indexer gives to `request`.
fn indexer_answer(chain: Chain, ledger: &mut Ledger, request: &Request) -> Value {
    let path = request.url.path().to_string();
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    let query = request.url.query().unwrap_or_default();
    match (chain.default_api().unwrap(), parts.as_slice()) {
        (crate::EndpointApi::Blockbook, ["api", "v2", "block-index", "0"]) => {
            json!({"blockHash": chain.zcash_genesis().unwrap()})
        }
        (crate::EndpointApi::Blockbook, ["api", "v2"]) => {
            let height = ledger.height;
            let branch = |height| {
                let branch = chain.zcash_consensus_branch(height).unwrap();
                format!("{:08x}", ledger.claimed_branch.unwrap_or(branch))
            };
            json!({"backend": {"blocks": height,
                "consensus": {"chaintip": branch(height), "nextblock": branch(height + 1)}}})
        }
        (crate::EndpointApi::Blockbook, ["api", "v2", "utxo", address]) => json!(
            ledger
                .held(address)
                .iter()
                .map(|(txid, vout, value)| json!({
                    "txid": txid, "vout": vout, "value": value.to_string(),
                    "confirmations": 6, "height": 1,
                }))
                .collect::<Vec<_>>()
        ),
        (crate::EndpointApi::Blockbook, ["api", "v2", "sendtx", ..]) => {
            ledger
                .submitted
                .push(String::from_utf8(request.body.clone()).unwrap());
            json!({"result": ledger.accepted_hash})
        }
        (crate::EndpointApi::Esplora, ["fee-estimates"]) => {
            json!({"1": 3.0, "6": 2.0, "144": 1.0})
        }
        (crate::EndpointApi::Esplora, ["address", address, "utxo"]) => json!(
            ledger
                .held(address)
                .iter()
                .map(|(txid, vout, value)| json!({
                    "txid": txid, "vout": vout, "value": value,
                    "status": {"confirmed": true, "block_height": 1},
                }))
                .collect::<Vec<_>>()
        ),
        (crate::EndpointApi::Whatsonchain, ["address", address, "unspent"]) => json!(
            ledger
                .held(address)
                .iter()
                .map(|(txid, vout, value)| json!({
                    "tx_hash": txid, "tx_pos": vout, "value": value, "height": 1,
                }))
                .collect::<Vec<_>>()
        ),
        (crate::EndpointApi::Blockcypher, ["addrs", address]) if query.contains("unspentOnly") => {
            json!({"txrefs": ledger.held(address).iter().map(|(txid, vout, value)| json!({
                "tx_hash": txid, "tx_output_n": vout, "value": value,
                "block_height": 1, "confirmations": 6,
            })).collect::<Vec<_>>()})
        }
        (crate::EndpointApi::Insight, ["addr", address, "utxo"]) => json!(
            ledger
                .held(address)
                .iter()
                .map(|(txid, vout, value)| json!({
                    "txid": txid, "vout": vout, "satoshis": value, "confirmations": 6,
                }))
                .collect::<Vec<_>>()
        ),
        (crate::EndpointApi::KaspaRest, ["addresses", address, "utxos"]) => {
            let script = crate::send::account_utxo::source_script(chain, address).unwrap();
            json!(ledger.held(address).iter().map(|(txid, vout, value)| json!({
                "address": address,
                "outpoint": {"transactionId": txid, "index": vout},
                "utxoEntry": {"amount": value.to_string(),
                    "scriptPublicKey": {"version": 0, "scriptPublicKey": hex::encode(&script)},
                    "blockDaaScore": "1", "isCoinbase": false},
            })).collect::<Vec<_>>())
        }
        (api, _) => panic!("unexpected {} read {path}?{query}", api.as_str()),
    }
}

/// An account wallet on `chain` with its phrase, its own address the
/// account's first, and receive 7 and change 3 handed out; and its indexer.
struct Account {
    service: Arc<WalletService>,
    server: MockServer,
    ledger: Arc<Mutex<Ledger>>,
    context: UtxoDerivation,
    root: String,
    /// (address, path) of receive 7 and change 3.
    receive: (String, Option<String>),
    change: (String, Option<String>),
}

impl Account {
    async fn new(chain: Chain) -> Self {
        let ledger = Arc::new(Mutex::new(Ledger {
            height: 3_500_000,
            accepted_hash: "00".repeat(32),
            ..Default::default()
        }));
        let reads = ledger.clone();
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(move |request: &Request| {
                ResponseTemplate::new(200).set_body_json(indexer_answer(
                    chain,
                    &mut reads.lock().unwrap(),
                    request,
                ))
            })
            .mount(&server)
            .await;
        let service = WalletService::new(vec![ChainEndpoints {
            capabilities: EndpointCapability::ALL.to_vec(),
            chain_id: chain,
            endpoints: vec![server.uri()],
        }])
        .unwrap();
        let database = std::env::temp_dir()
            .join(format!(
                "account-send-{}.sqlite",
                crate::store::new_event_id()
            ))
            .to_string_lossy()
            .into_owned();
        service.open_state(database).await.unwrap();
        let secrets = Arc::new(InMemorySecretStore::new());
        store_seed_phrase(&*secrets, "w", SEED, None).unwrap();
        service.set_secret_store(secrets);
        let path = crate::derivation::path::default_path_from_catalog(chain).unwrap();
        let context = UtxoDerivation::new(chain, SEED, path.clone()).unwrap();
        let root = context.derive(0).unwrap().0;
        let mut wallet = WalletState::single_address(
            "w",
            "Account",
            chain,
            root.clone(),
            Some(path.clone()),
            false,
        );
        wallet.xpub =
            Some(UtxoDerivation::account_xpub(chain, SEED, &path, &Default::default()).unwrap());
        service
            .apply_state_command(StateCommand::UpsertWallet { wallet })
            .await
            .unwrap();
        let receive = context.derive(7).unwrap();
        let change = context.derive_on_branch(1, 3).unwrap();
        for ((address, path), branch, index) in [(&receive, "external", 7), (&change, "change", 3)]
        {
            service
                .register_owned_address(
                    "w".into(),
                    chain,
                    address.clone(),
                    path.clone(),
                    Some(branch.into()),
                    Some(index),
                )
                .await
                .unwrap();
        }
        Self {
            service,
            server,
            ledger,
            context,
            root,
            receive,
            change,
        }
    }

    fn fund(&self, address: &str, serial: u64, value: u64) {
        self.ledger
            .lock()
            .unwrap()
            .outputs
            .insert(address.into(), vec![(format!("{serial:064x}"), 1, value)]);
    }

    fn request(&self, chain: Chain, units: u64) -> SendExecutionRequest {
        SendExecutionRequest {
            token_standard: None,
            chain_id: chain,
            wallet_id: "w".into(),
            password: None,
            to_address: self.context.derive(9).unwrap().0,
            amount_str: crate::decimal::from_units(
                u128::from(units),
                u32::from(chain.native_decimals()),
            ),
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
}

/// Every account network but Litecoin and Peercoin, whose transfers have
/// stages of their own; with the amounts' scale (Dogecoin's static fee is a
/// hundred times the others').
fn account_networks() -> impl Iterator<Item = (Chain, u64)> {
    Chain::all()
        .filter(|chain| {
            chain.uses_account_utxo()
                && !matches!(
                    chain.mainnet_counterpart(),
                    Chain::Litecoin | Chain::Peercoin
                )
        })
        .map(|chain| {
            let scale = if chain.mainnet_counterpart() == Chain::Dogecoin {
                100
            } else {
                1
            };
            (chain, scale)
        })
}

/// A send spends the largest outputs it needs from any of the account's
/// addresses, each input carrying the source whose key signs it, and
/// returns its change to the wallet's own address.
#[tokio::test]
async fn an_account_transfer_spends_outputs_from_each_address_with_its_own_key() {
    for (chain, scale) in account_networks() {
        let account = Account::new(chain).await;
        account.fund(&account.root, 1, 100_000 * scale);
        account.fund(&account.receive.0, 2, 300_000 * scale);
        account.fund(&account.change.0, 3, 500_000 * scale);
        let built = account
            .service
            .build_send(account.request(chain, 600_000 * scale))
            .await
            .unwrap();
        let PreparedPayload::AccountTransfer(prepared) = account
            .service
            .load_send_artifact(built.id.clone())
            .await
            .unwrap()
            .prepared
        else {
            panic!("{chain}: not an account transfer")
        };
        let sources: Vec<_> = prepared
            .inputs
            .iter()
            .map(|input| {
                (
                    input.source.address.clone(),
                    input.source.derivation_path.clone(),
                    input.utxo.2,
                )
            })
            .collect();
        assert_eq!(
            sources,
            [
                (
                    account.change.0.clone(),
                    account.change.1.clone(),
                    500_000 * scale
                ),
                (
                    account.receive.0.clone(),
                    account.receive.1.clone(),
                    300_000 * scale
                ),
            ],
            "{chain}"
        );
        assert!(prepared.fee > 0, "{chain}");
        assert_eq!(
            prepared.outputs,
            [
                (
                    crate::send::account_utxo::recipient_script(
                        chain,
                        &account.context.derive(9).unwrap().0
                    )
                    .unwrap(),
                    600_000 * scale
                ),
                (
                    crate::send::account_utxo::source_script(chain, &account.root).unwrap(),
                    200_000 * scale - prepared.fee
                ),
            ],
            "{chain}"
        );
        let signed = account
            .service
            .sign_send(built.id, built.review_digest, None)
            .await
            .unwrap();
        assert_eq!(signed.stage, SendStage::Signed, "{chain}");
        assert!(signed.signed_payload.is_some(), "{chain}");
    }
}

/// An output spent since the review is not signed for.
#[tokio::test]
async fn an_output_spent_after_review_is_refused_at_signing() {
    for (chain, scale) in account_networks() {
        let account = Account::new(chain).await;
        account.fund(&account.root, 1, 100_000 * scale);
        account.fund(&account.change.0, 3, 500_000 * scale);
        let built = account
            .service
            .build_send(account.request(chain, 100_000 * scale))
            .await
            .unwrap();
        account
            .ledger
            .lock()
            .unwrap()
            .outputs
            .remove(&account.change.0);
        let refused = account
            .service
            .sign_send(built.id.clone(), built.review_digest, None)
            .await
            .unwrap_err();
        assert!(
            refused.to_string().contains("Input changed or was spent"),
            "{chain}: {refused}"
        );
        assert_eq!(
            account.service.inspect_send(built.id).await.unwrap().stage,
            SendStage::Prepared,
            "{chain}"
        );
    }
}

/// A Zcash transfer is bound to the network upgrade and expiry height it was
/// built under: an indexer on another upgrade's branch, or a tip at its
/// expiry, signs nothing, and a signed transfer past its expiry is not sent
/// again.
#[tokio::test]
async fn a_zcash_transfer_is_signed_and_sent_only_on_its_branch_before_its_expiry() {
    let chain = Chain::Zcash;
    let account = Account::new(chain).await;
    account.fund(&account.root, 1, 1_000_000);
    let built = account
        .service
        .build_send(account.request(chain, 100_000))
        .await
        .unwrap();
    let PreparedPayload::AccountTransfer(prepared) = account
        .service
        .load_send_artifact(built.id.clone())
        .await
        .unwrap()
        .prepared
    else {
        panic!("not an account transfer")
    };
    let AccountProtocol::Zcash { expiry_height, .. } = prepared.protocol else {
        panic!("not a Zcash transfer")
    };
    let sign = || {
        account
            .service
            .sign_send(built.id.clone(), built.review_digest.clone(), None)
    };
    // NU5's branch, and NU6.2's, which NU6.3 replaced.
    for stale in [0xc2d6_d0b4, 0x5437_f330] {
        account.ledger.lock().unwrap().claimed_branch = Some(stale);
        let refused = sign().await.unwrap_err();
        assert!(
            refused
                .to_string()
                .contains("Zcash consensus upgrade is unsupported or inconsistent"),
            "{stale:08x}: {refused}"
        );
    }
    account.ledger.lock().unwrap().claimed_branch = None;
    account.ledger.lock().unwrap().height = expiry_height;
    let refused = sign().await.unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("Zcash transaction expired or consensus changed"),
        "{refused}"
    );
    assert_eq!(
        account
            .service
            .inspect_send(built.id.clone())
            .await
            .unwrap()
            .stage,
        SendStage::Prepared
    );

    account.ledger.lock().unwrap().height = 3_500_000;
    let signed = sign().await.unwrap();
    account.ledger.lock().unwrap().accepted_hash = signed.transaction_hash.clone().unwrap();
    let sent = account
        .service
        .broadcast_send(signed.id.clone(), vec![account.server.uri()])
        .await
        .unwrap();
    assert_eq!(sent.attempts[0].outcome, SubmissionOutcome::Accepted);
    account.ledger.lock().unwrap().height = expiry_height;
    let refused = account
        .service
        .broadcast_send(signed.id.clone(), vec![account.server.uri()])
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("Signed transaction expired"),
        "{refused}"
    );
    assert_eq!(
        account.ledger.lock().unwrap().submitted,
        [signed.signed_payload.unwrap()]
    );
    assert_eq!(
        account
            .service
            .inspect_send(signed.id)
            .await
            .unwrap()
            .attempts
            .len(),
        1
    );
}
