//! Account input selection and each network's fee rule.
//!
//! An account transfer as the service builds and signs it, against each
//! account network's own indexer: the outputs it spends come from every
//! address the account holds, each signed for by that address's key, and an
//! output gone since the review is not signed for. Zcash's transfer is bound
//! to its network upgrade and expiry height as well.

use super::*;
use crate::api::endpoint::EndpointApi;
use crate::secrets::backends::InMemorySecretStore;
use crate::secrets::wallet::store_seed_phrase;
use crate::send::SendExecutionRequest;
use crate::send::account_utxo::AccountProtocol;
use crate::send::stages::{SendStage, SubmissionOutcome};
use crate::service::address_discovery::UtxoDerivation;
use crate::store::state::WalletState;
use serde_json::{Value, json};
use std::sync::Mutex;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

fn input(index: u8, value: u64, script: &[u8]) -> UtxoPreparedInput {
    UtxoPreparedInput {
        source: UtxoSendSource {
            address: format!("a{index}"),
            derivation_path: None,
            script_pubkey: script.to_vec(),
        },
        utxo: crate::api::utxo::UtxoInput::fixture(
            &hex::encode([index; 32]),
            u32::from(index),
            value,
            script.to_vec(),
        ),
    }
}

const P2PKH: [u8; 25] = {
    let mut script = [0u8; 25];
    script[0] = 0x76;
    script[1] = 0xa9;
    script[2] = 0x14;
    script[23] = 0x88;
    script[24] = 0xac;
    script
};

/// The largest outputs are spent first, only as many as the amount and
/// their fee need; change below the dust floor joins the fee; a reviewed fee
/// below what the inputs now take is refused, and too little is refused.
#[test]
fn selection_spends_the_largest_outputs_it_needs() {
    let policy = FeePolicy::PerKilobyte(1_000);
    let candidates = vec![
        input(1, 5_000, &P2PKH),
        input(2, 50_000, &P2PKH),
        input(3, 20_000, &P2PKH),
    ];
    let selection = select(
        candidates.clone(),
        60_000,
        &policy,
        &P2PKH,
        &P2PKH,
        546,
        None,
    )
    .unwrap();
    assert_eq!(
        selection
            .inputs
            .iter()
            .map(|input| input.utxo.value)
            .collect::<Vec<_>>(),
        [50_000, 20_000]
    );
    // Two inputs and two outputs fit one kilobyte.
    assert_eq!((selection.fee, selection.change), (1_000, 9_000));
    // 70,000 - 69,000 - 1,000 leaves nothing; 69,500 leaves 500, dust.
    let exact = select(
        candidates.clone(),
        69_000,
        &policy,
        &P2PKH,
        &P2PKH,
        546,
        None,
    )
    .unwrap();
    assert_eq!((exact.fee, exact.change), (1_000, 0));
    let dust = select(
        candidates.clone(),
        68_500,
        &policy,
        &P2PKH,
        &P2PKH,
        546,
        None,
    )
    .unwrap();
    assert_eq!((dust.fee, dust.change), (1_500, 0));
    assert!(
        select(
            candidates.clone(),
            74_500,
            &policy,
            &P2PKH,
            &P2PKH,
            546,
            None
        )
        .is_err()
    );
    // A reviewed fee: at least the policy's, and paid as reviewed.
    let fixed = select(
        candidates.clone(),
        60_000,
        &policy,
        &P2PKH,
        &P2PKH,
        546,
        Some(2_000),
    )
    .unwrap();
    assert_eq!((fixed.fee, fixed.change), (2_000, 8_000));
    assert!(select(candidates, 60_000, &policy, &P2PKH, &P2PKH, 546, Some(999)).is_err());
}

/// Each network's fee follows its size: Bitcoin's rate times the virtual
/// size, the legacy networks' fee per started kilobyte, ZIP-317's logical
/// actions, Decred's ten atoms a byte and Kaspa's mass, each at least the
/// network's static fee.
#[test]
fn fees_follow_each_networks_rule() {
    let shape = |inputs: usize| Shape {
        inputs: vec![P2PKH.as_slice(); inputs],
        outputs: vec![25, 25],
    };
    let size = |policy: &FeePolicy, inputs| policy.size(&shape(inputs)).unwrap();
    let fee = |policy: &FeePolicy, inputs| policy.fee(&shape(inputs)).unwrap();
    let rate = FeePolicy::Rate("2.5".into());
    assert_eq!(fee(&rate, 1), (size(&rate, 1) * 5).div_ceil(2));
    let legacy = FeePolicy::PerKilobyte(1_000_000);
    assert_eq!(size(&legacy, 1), 10 + 148 + 68);
    assert_eq!(fee(&legacy, 1), 1_000_000);
    assert_eq!(
        fee(&legacy, 7),
        2_000_000,
        "1,114 bytes start a second kilobyte"
    );
    assert_eq!(fee(&FeePolicy::Zip317, 1), 10_000);
    assert_eq!(fee(&FeePolicy::Zip317, 3), 15_000);
    let decred = FeePolicy::Decred(2_000);
    assert_eq!(fee(&decred, 1), (16 + 166 + 72) * 10);
    assert_eq!(
        FeePolicy::Decred(1_000_000).fee(&shape(1)).unwrap(),
        1_000_000
    );
    let kaspa = FeePolicy::Kaspa(1_000);
    assert_eq!(
        fee(&kaspa, 2),
        62 + 2 * 119 + 2 * 52 + 10 * 2 * 27 + 2 * 1_000
    );
}

/// A Kaspa spend is relayed only within KIP-9's storage mass limit, which
/// charges an output small beside the coins it spends: more inputs are
/// taken while their change is too small to stand, and a spend no
/// selection fits is refused rather than signed.
#[test]
fn kaspa_selection_keeps_within_the_storage_mass_limit() {
    // Pay-to-pubkey Schnorr: a push of 32 bytes and OP_CHECKSIG.
    let p2pk = [[0x20].as_slice(), &[7; 32], &[0xac]].concat();
    let policy = FeePolicy::Kaspa(1_000);
    let kas = 100_000_000;
    let fits = |selection: &Selection, amount: u64| {
        let values: Vec<u64> = selection.inputs.iter().map(|i| i.utxo.value).collect();
        let outputs: Vec<u64> = std::iter::once(amount)
            .chain((selection.change > 0).then_some(selection.change))
            .collect();
        crate::send::kaspa::storage_mass(&values, &outputs).unwrap()
            <= crate::send::kaspa::MAXIMUM_STANDARD_MASS
    };
    // One coin pays 10 KAS and returns about 90 as change.
    let candidates = vec![input(1, 100 * kas, &p2pk), input(2, kas, &p2pk)];
    let plain = select(
        candidates.clone(),
        10 * kas,
        &policy,
        &p2pk,
        &p2pk,
        1_000,
        None,
    )
    .unwrap();
    assert_eq!(plain.inputs.len(), 1);
    assert!(fits(&plain, 10 * kas));
    // 99.995 KAS from the large coin leaves change of about 0.005 KAS,
    // which storage mass refuses; the second coin raises it past one KAS.
    let amount = 100 * kas - 500_000;
    let widened = select(
        candidates.clone(),
        amount,
        &policy,
        &p2pk,
        &p2pk,
        1_000,
        None,
    )
    .unwrap();
    assert_eq!(widened.inputs.len(), 2);
    assert!(widened.change > kas);
    assert!(fits(&widened, amount));
    // 0.05 KAS beside a 100 KAS coin can never be relayed.
    let refused = select(candidates, 5_000_000, &policy, &p2pk, &p2pk, 1_000, None)
        .err()
        .unwrap();
    assert!(refused.to_string().contains("storage mass"), "{refused}");
    // Elsewhere the limit does not apply.
    assert!(FeePolicy::PerKilobyte(1_000).admits(&[100 * kas], &[1_000]));
}

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
    /// The raw transactions behind the outputs, where a send reads each
    /// output from its transaction: txid to hex and confirmations.
    transactions: HashMap<String, (String, u64)>,
    /// A block hash the indexer names in place of the network's identity
    /// block: another network's indexer.
    claimed_identity: Option<String>,
}

impl Ledger {
    /// The outputs of the address a path names, whatever form of it the
    /// client sends (Bitcoin Cash's CashAddr for a legacy address).
    fn held(&self, chain: Chain, address: &str) -> Vec<(String, u32, u64)> {
        self.outputs
            .iter()
            .find(|(held, _)| {
                crate::service::send_stage_support::same_utxo_address(chain, held, address)
            })
            .map(|(_, outputs)| outputs.clone())
            .unwrap_or_default()
    }
}

/// The answer `chain`'s default indexer gives to `request`: JSON, but for
/// a block hash Esplora writes as text, and Chronik's protobuf.
fn indexer_response(chain: Chain, ledger: &mut Ledger, request: &Request) -> ResponseTemplate {
    let path = request.url.path().to_string();
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    match (chain.default_api().unwrap(), parts.as_slice()) {
        (EndpointApi::Esplora, ["block-height", height]) => {
            let (identity, hash) = chain.identity_block().unwrap();
            assert_eq!(height.parse::<u64>().unwrap(), identity, "{chain}");
            ResponseTemplate::new(200)
                .set_body_string(ledger.claimed_identity.clone().unwrap_or(hash))
        }
        (EndpointApi::Esplora, ["tx"]) => {
            ledger
                .submitted
                .push(String::from_utf8(request.body.clone()).unwrap());
            ResponseTemplate::new(200).set_body_string(ledger.accepted_hash.clone())
        }
        (EndpointApi::Chronik, ["block", height]) => {
            let (identity, hash) = chain.identity_block().unwrap();
            assert_eq!(height.parse::<u64>().unwrap(), identity, "{chain}");
            ResponseTemplate::new(200).set_body_bytes(crate::api::chronik::block_answer(
                identity as i32,
                &ledger.claimed_identity.clone().unwrap_or(hash),
            ))
        }
        (EndpointApi::Chronik, ["broadcast-tx"]) => {
            let (raw, answer) = crate::api::chronik::accept_broadcast(&request.body);
            ledger.submitted.push(hex::encode(raw));
            ResponseTemplate::new(200).set_body_bytes(answer)
        }
        (EndpointApi::Chronik, ["script", "p2pkh", hash, "utxos"]) => {
            let script = crate::derivation::utxo_address::ParsedUtxoAddress::P2pkh(
                hex::decode(hash).unwrap().try_into().unwrap(),
            )
            .script_pubkey();
            let outputs: Vec<_> = ledger
                .outputs
                .iter()
                .find(|(address, _)| {
                    crate::send::account_utxo::source_script(chain, address).unwrap() == script
                })
                .map(|(_, outputs)| outputs.clone())
                .unwrap_or_default()
                .into_iter()
                .map(|(txid, vout, value)| (txid, vout, value as i64, 1))
                .collect();
            ResponseTemplate::new(200)
                .set_body_bytes(crate::api::chronik::script_utxos_answer(script, &outputs))
        }
        (EndpointApi::Blockbook, ["api", "v2", "tx", txid]) => {
            let (hex, confirmations) = &ledger.transactions[*txid];
            ResponseTemplate::new(200).set_body_json(json!({
                "txid": txid, "hex": hex, "confirmations": confirmations,
            }))
        }
        _ => ResponseTemplate::new(200).set_body_json(indexer_answer(chain, ledger, request)),
    }
}

/// The JSON answer `chain`'s default indexer gives to `request`.
fn indexer_answer(chain: Chain, ledger: &mut Ledger, request: &Request) -> Value {
    let path = request.url.path().to_string();
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    let query = request.url.query().unwrap_or_default();
    match (chain.default_api().unwrap(), parts.as_slice()) {
        (EndpointApi::Blockbook, ["api", "v2", "block-index", "0"]) => {
            json!({"blockHash": match chain.mainnet_counterpart() {
                Chain::Zcash => chain.zcash_genesis().unwrap().to_string(),
                _ => ledger
                    .claimed_identity
                    .clone()
                    .unwrap_or_else(|| chain.identity_block().unwrap().1),
            }})
        }
        (EndpointApi::Blockbook, ["api", "v2"]) => {
            let height = ledger.height;
            let branch = |height| {
                let branch = chain.zcash_consensus_branch(height).unwrap();
                format!("{:08x}", ledger.claimed_branch.unwrap_or(branch))
            };
            json!({"backend": {"blocks": height,
                "consensus": {"chaintip": branch(height), "nextblock": branch(height + 1)}}})
        }
        (EndpointApi::Blockbook, ["api", "v2", "utxo", address]) => json!(
            ledger
                .held(chain, address)
                .iter()
                .map(|(txid, vout, value)| json!({
                    "txid": txid, "vout": vout, "value": value.to_string(),
                    "confirmations": 6, "height": 1,
                }))
                .collect::<Vec<_>>()
        ),
        (EndpointApi::Blockbook, ["api", "v2", "sendtx", ..]) => {
            ledger
                .submitted
                .push(String::from_utf8(request.body.clone()).unwrap());
            json!({"result": ledger.accepted_hash})
        }
        (EndpointApi::Esplora, ["fee-estimates"]) => {
            json!({"1": 3.0, "6": 2.0, "144": 1.0})
        }
        (EndpointApi::Esplora, ["address", address, "utxo"]) => json!(
            ledger
                .held(chain, address)
                .iter()
                .map(|(txid, vout, value)| json!({
                    "txid": txid, "vout": vout, "value": value,
                    "status": {"confirmed": true, "block_height": 1},
                }))
                .collect::<Vec<_>>()
        ),
        (EndpointApi::Whatsonchain, ["address", address, "unspent"]) => json!(
            ledger
                .held(chain, address)
                .iter()
                .map(|(txid, vout, value)| json!({
                    "tx_hash": txid, "tx_pos": vout, "value": value, "height": 1,
                }))
                .collect::<Vec<_>>()
        ),
        (EndpointApi::Blockcypher, ["addrs", address]) if query.contains("unspentOnly") => {
            json!({"txrefs": ledger.held(chain, address).iter().map(|(txid, vout, value)| json!({
                "tx_hash": txid, "tx_output_n": vout, "value": value,
                "block_height": 1, "confirmations": 6,
            })).collect::<Vec<_>>()})
        }
        (EndpointApi::Insight, ["addr", address, "utxo"]) => json!(
            ledger
                .held(chain, address)
                .iter()
                .map(|(txid, vout, value)| json!({
                    "txid": txid, "vout": vout, "satoshis": value, "confirmations": 6,
                }))
                .collect::<Vec<_>>()
        ),
        (EndpointApi::KaspaRest, ["addresses", address, "utxos"]) => {
            // As api.kaspa.org answers (2026-11-26): no script version, it
            // being protobuf's default 0.
            let script = crate::send::account_utxo::source_script(chain, address).unwrap();
            json!(
                ledger
                    .held(chain, address)
                    .iter()
                    .map(|(txid, vout, value)| json!({
                        "address": address,
                        "outpoint": {"transactionId": txid, "index": vout},
                        "utxoEntry": {"amount": value.to_string(),
                            "scriptPublicKey": {"scriptPublicKey": hex::encode(&script)},
                            "blockDaaScore": "1", "isCoinbase": false},
                    }))
                    .collect::<Vec<_>>()
            )
        }
        (api, _) => panic!("unexpected {} read {path}?{query}", api.as_str()),
    }
}

/// An account wallet on `chain` with its phrase, its own address the
/// account's first, and receive 7 and change 3 handed out; and its indexer.
struct Account {
    chain: Chain,
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
                indexer_response(chain, &mut reads.lock().unwrap(), request)
            })
            .mount(&server)
            .await;
        let service = WalletService::serving(vec![ChainEndpoints {
            capabilities: EndpointCapability::ALL.to_vec(),
            chain_id: chain,
            endpoints: vec![server.uri()],
        }])
        .unwrap();
        let database = std::env::temp_dir()
            .join(format!("account-send-{}.sqlite", crate::store::new_id()))
            .to_string_lossy()
            .into_owned();
        service.opened(database).await.unwrap();
        let secrets = Arc::new(InMemorySecretStore::new());
        store_seed_phrase(&*secrets, "w", SEED, &Default::default(), None).unwrap();
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
        service.upsert_wallet(wallet).await.unwrap();
        let receive = context.derive(7).unwrap();
        let change = context.derive_on_branch(1, 3).unwrap();
        for ((address, path), branch, index) in [
            (&receive, crate::wallet_db::AddressBranch::Receive, 7),
            (&change, crate::wallet_db::AddressBranch::Change, 3),
        ] {
            service
                .register_owned_address(
                    "w".into(),
                    chain,
                    address.clone(),
                    path.clone(),
                    Some(branch),
                    Some(index),
                )
                .await
                .unwrap();
        }
        Self {
            chain,
            service,
            server,
            ledger,
            context,
            root,
            receive,
            change,
        }
    }

    /// One output of `value` at `address`, output 1 of its transaction:
    /// a real one where a send reads outputs from their transactions.
    fn fund(&self, address: &str, serial: u64, value: u64) {
        let output = if self.chain.verifies_spent_outputs() {
            let script = crate::send::account_utxo::source_script(self.chain, address).unwrap();
            self.transaction(serial, (P2PKH.to_vec(), 1), (script, value), 6)
        } else {
            (format!("{serial:064x}"), 1, value)
        };
        self.ledger
            .lock()
            .unwrap()
            .outputs
            .insert(address.into(), vec![output]);
    }

    /// A transaction of two outputs, `first` and `second` (script, value),
    /// with `confirmations`, that the indexer serves; the outpoint of its
    /// second output.
    fn transaction(
        &self,
        serial: u64,
        first: (Vec<u8>, u64),
        second: (Vec<u8>, u64),
        confirmations: u64,
    ) -> (String, u32, u64) {
        use bitcoin::hashes::Hash as _;
        let value = second.1;
        let tx = bitcoin::Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![bitcoin::TxIn {
                previous_output: bitcoin::OutPoint {
                    txid: bitcoin::Txid::from_byte_array([serial as u8; 32]),
                    vout: 0,
                },
                ..Default::default()
            }],
            output: [first, second]
                .into_iter()
                .map(|(script, value)| bitcoin::TxOut {
                    value: bitcoin::Amount::from_sat(value),
                    script_pubkey: bitcoin::ScriptBuf::from_bytes(script),
                })
                .collect(),
        };
        let txid = tx.compute_txid().to_string();
        self.ledger.lock().unwrap().transactions.insert(
            txid.clone(),
            (
                bitcoin::consensus::encode::serialize_hex(&tx),
                confirmations,
            ),
        );
        (txid, 1, value)
    }

    fn request(&self, chain: Chain, units: u64) -> SendExecutionRequest {
        SendExecutionRequest {
            token_standard: None,
            chain_id: chain,
            wallet_id: "w".into(),
            password: None,
            to_address: self.context.derive(9).unwrap().0,
            amount_str: crate::amount::decimal::from_units(
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
            memo: None,
        }
    }
}

/// Every account network but Litecoin and Peercoin, whose transfers have
/// stages of their own; with the amounts' scale (Dogecoin's static fee and
/// Qtum's fee rate are a hundred times the others', and Kaspa's storage
/// mass refuses outputs of less than about a tenth of a kaspa beside the
/// coins they spend).
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
            let scale = match chain.mainnet_counterpart() {
                // Qtum's minimum fee rate is 400 sat/vB.
                Chain::Dogecoin | Chain::Qtum => 100,
                Chain::Kaspa => 1_000,
                _ => 1,
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
                    input.utxo.value,
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

/// An amount the network would not relay is refused before it is built, on
/// every account network as on its multisig accounts. Kaspa's storage mass
/// refuses one this small beside the coin it spends as well.
#[tokio::test]
async fn an_amount_below_the_dust_threshold_is_refused() {
    for (chain, scale) in account_networks() {
        let account = Account::new(chain).await;
        account.fund(&account.root, 1, 1_000_000 * scale);
        let dust = crate::service::send_stage::utxo::change_dust(chain).unwrap();
        let refused = account
            .service
            .build_send(account.request(chain, dust - 1))
            .await
            .unwrap_err();
        assert!(
            refused.to_string().contains("dust threshold"),
            "{chain}: {refused}"
        );
        let at_dust = account
            .service
            .build_send(account.request(chain, dust))
            .await;
        if chain.mainnet_counterpart() == Chain::Kaspa {
            let refused = at_dust.unwrap_err().to_string();
            assert!(refused.contains("storage mass"), "{chain}: {refused}");
        } else {
            at_dust.unwrap();
        }
    }
}

/// Dogecoin Core charges an output under 0.01 DOGE that much again in fee
/// and relays none under 0.001 DOGE, so smaller change joins the fee.
#[tokio::test]
async fn dogecoin_change_under_its_dust_limit_joins_the_fee() {
    let chain = Chain::Dogecoin;
    let account = Account::new(chain).await;
    account.fund(&account.root, 1, 100_000_000);
    // One input and two outputs fit a kilobyte: a 0.01 DOGE fee, leaving
    // 0.005 DOGE of change.
    let built = account
        .service
        .build_send(account.request(chain, 98_500_000))
        .await
        .unwrap();
    let PreparedPayload::AccountTransfer(prepared) = account
        .service
        .load_send_artifact(built.id)
        .await
        .unwrap()
        .prepared
    else {
        panic!("not an account transfer")
    };
    assert_eq!(prepared.outputs.len(), 1);
    assert_eq!(prepared.fee, 1_500_000);
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

/// Qtum, Fractal Bitcoin, eCash and DigiByte: a send is built from the
/// account's outputs, signed, and broadcast through the network's own
/// indexer API (Blockbook, Esplora, Chronik's protobuf), which is first
/// checked to be the network, and the bytes sent are the bytes signed.
#[tokio::test]
async fn each_new_network_builds_signs_and_broadcasts_through_its_indexer() {
    for chain in [
        Chain::Qtum,
        Chain::FractalBitcoin,
        Chain::Ecash,
        Chain::Digibyte,
    ] {
        let scale = if chain == Chain::Qtum { 100 } else { 1 };
        let account = Account::new(chain).await;
        account.fund(&account.root, 1, 100_000 * scale);
        account.fund(&account.change.0, 3, 500_000 * scale);
        let built = account
            .service
            .build_send(account.request(chain, 300_000 * scale))
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
        assert_eq!(
            prepared.protocol,
            AccountProtocol::for_chain(chain).unwrap(),
            "{chain}"
        );
        // Each network's fee floor: Qtum's 400 sat/vB on one P2PKH input
        // and two outputs (226 bytes), DigiByte's 100 on one P2WPKH input
        // (141 vB), eCash's 1,000 per started kilobyte; Fractal pays the
        // indexer's 2 sat/vB.
        let expected_fee = match chain {
            Chain::Qtum => 226 * 400,
            Chain::Digibyte => 141 * 100,
            Chain::Ecash => 1_000,
            _ => 141 * 2,
        };
        assert_eq!(prepared.fee, expected_fee, "{chain}");
        let signed = account
            .service
            .sign_send(built.id, built.review_digest, None)
            .await
            .unwrap();
        let hash = signed.transaction_hash.clone().unwrap();
        account.ledger.lock().unwrap().accepted_hash = hash.clone();
        let sent = account
            .service
            .broadcast_send(signed.id.clone(), vec![account.server.uri()])
            .await
            .unwrap();
        assert_eq!(
            sent.attempts[0].outcome,
            SubmissionOutcome::Accepted,
            "{chain}"
        );
        assert_eq!(
            sent.transaction_hash.as_deref(),
            Some(hash.as_str()),
            "{chain}"
        );
        assert_eq!(
            account.ledger.lock().unwrap().submitted,
            [signed.signed_payload.unwrap()],
            "{chain}"
        );
    }
}

/// Fractal's keys and addresses are Bitcoin's, so its outputs are read
/// only from an indexer naming Fractal's block 1: one naming Bitcoin's is
/// refused before a send is built, and before a reviewed one is signed.
/// eCash's indexer must name its block 896,800 before it broadcasts.
#[tokio::test]
async fn an_indexer_on_another_network_is_refused_before_signing() {
    let bitcoin_block_one = Chain::Bitcoin.identity_block().unwrap().1;
    let account = Account::new(Chain::FractalBitcoin).await;
    account.fund(&account.root, 1, 500_000);
    account.ledger.lock().unwrap().claimed_identity = Some(bitcoin_block_one.clone());
    let refused = account
        .service
        .build_send(account.request(Chain::FractalBitcoin, 100_000))
        .await
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("is not on the Fractal Bitcoin network"),
        "{refused}"
    );
    account.ledger.lock().unwrap().claimed_identity = None;
    let built = account
        .service
        .build_send(account.request(Chain::FractalBitcoin, 100_000))
        .await
        .unwrap();
    account.ledger.lock().unwrap().claimed_identity = Some(bitcoin_block_one);
    let refused = account
        .service
        .sign_send(built.id.clone(), built.review_digest, None)
        .await
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("is not on the Fractal Bitcoin network"),
        "{refused}"
    );
    assert_eq!(
        account.service.inspect_send(built.id).await.unwrap().stage,
        SendStage::Prepared
    );

    // A Bitcoin Cash Chronik (any block there but eCash's) takes no eCash
    // broadcast.
    let account = Account::new(Chain::Ecash).await;
    account.fund(&account.root, 1, 500_000);
    let built = account
        .service
        .build_send(account.request(Chain::Ecash, 100_000))
        .await
        .unwrap();
    let signed = account
        .service
        .sign_send(built.id, built.review_digest, None)
        .await
        .unwrap();
    account.ledger.lock().unwrap().claimed_identity = Some("11".repeat(32));
    assert!(
        account
            .service
            .broadcast_send(signed.id, vec![account.server.uri()])
            .await
            .is_err()
    );
    assert!(account.ledger.lock().unwrap().submitted.is_empty());
}

/// A Qtum staking reward is spent only once it has 2,000 confirmations, and
/// a coinstake output paying the key itself (P2PK) is never spent by the
/// account's P2PKH signer; an output the indexer lists that its
/// transaction does not pay is refused.
#[tokio::test]
async fn qtum_spends_only_mature_outputs_paying_its_address() {
    let chain = Chain::Qtum;
    let account = Account::new(chain).await;
    let script = crate::send::account_utxo::source_script(chain, &account.root).unwrap();
    let p2pk = [&[0x21][..], &[0x02; 33][..], &[0xac][..]].concat();
    // A coinstake's first output is empty and worthless.
    let coinstake = (Vec::new(), 0);
    let ordinary = account.transaction(1, (P2PKH.to_vec(), 1), (script.clone(), 50_000_000), 6);
    let maturing = account.transaction(2, coinstake.clone(), (script.clone(), 200_000_000), 1_999);
    let to_the_key = account.transaction(3, coinstake.clone(), (p2pk, 300_000_000), 5_000);
    account.ledger.lock().unwrap().outputs.insert(
        account.root.clone(),
        vec![ordinary.clone(), maturing, to_the_key],
    );
    let refused = account
        .service
        .build_send(account.request(chain, 60_000_000))
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("nsufficient"), "{refused}");
    let built = account
        .service
        .build_send(account.request(chain, 20_000_000))
        .await
        .unwrap();
    let PreparedPayload::AccountTransfer(prepared) = account
        .service
        .load_send_artifact(built.id)
        .await
        .unwrap()
        .prepared
    else {
        panic!("not an account transfer")
    };
    assert_eq!(prepared.inputs.len(), 1);
    assert_eq!(prepared.inputs[0].utxo.txid.to_string(), ordinary.0);

    // Matured at 2,000 confirmations, the reward is spent too.
    let matured = account.transaction(4, coinstake, (script.clone(), 200_000_000), 2_000);
    account
        .ledger
        .lock()
        .unwrap()
        .outputs
        .insert(account.root.clone(), vec![ordinary, matured.clone()]);
    let built = account
        .service
        .build_send(account.request(chain, 150_000_000))
        .await
        .unwrap();
    let PreparedPayload::AccountTransfer(prepared) = account
        .service
        .load_send_artifact(built.id)
        .await
        .unwrap()
        .prepared
    else {
        panic!("not an account transfer")
    };
    assert_eq!(prepared.inputs[0].utxo.txid.to_string(), matured.0);

    // The indexer lists an amount its transaction does not pay.
    account
        .ledger
        .lock()
        .unwrap()
        .outputs
        .insert(account.root.clone(), vec![(matured.0, 1, 199_999_999)]);
    assert!(
        account
            .service
            .build_send(account.request(chain, 20_000_000))
            .await
            .is_err()
    );
}
