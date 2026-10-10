//! The coins behind an account wallet's balance, as its indexer reports them.
use super::*;
use crate::service::address_discovery::UtxoDerivation;
use crate::store::state::WalletState;
use bitcoin::{Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

/// A wallet on `chain` at `path`'s account, its own address the path's,
/// reading from `server`.
async fn account(chain: Chain, server: &MockServer, path: &str) -> Arc<WalletService> {
    let service = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: chain,
        endpoints: vec![server.uri()],
    }])
    .unwrap();
    let database = std::env::temp_dir()
        .join(format!(
            "wallet-coins-{}.sqlite",
            crate::store::new_event_id()
        ))
        .to_string_lossy()
        .into_owned();
    service.open_state(database).await.unwrap();
    let context = UtxoDerivation::new(chain, SEED, path.into()).unwrap();
    let mut wallet = WalletState::single_address(
        "w",
        "Account",
        chain,
        context.derive(0).unwrap().0,
        Some(path.into()),
        false,
    );
    wallet.xpub =
        Some(UtxoDerivation::account_xpub(chain, SEED, path, &Default::default()).unwrap());
    service
        .apply_state_command(StateCommand::UpsertWallet { wallet })
        .await
        .unwrap();
    service
}

/// Each address in its place on the account with what it holds, receive
/// before change; every output, largest first, with its confirmations at
/// the indexer's tip; and as the next receive address the first past every
/// one handed out, since the reserved one has received.
#[tokio::test]
async fn an_account_lists_its_addresses_in_place_its_coins_and_its_next_receive_address() {
    let chain = Chain::Litecoin;
    let path = "m/84'/2'/0'/0/0";
    let context = UtxoDerivation::new(chain, SEED, path.into()).unwrap();
    let receive = |index| context.derive(index).unwrap().0;
    let change = context.derive_on_branch(1, 3).unwrap().0;
    // (address, [(value, mined height or unconfirmed)], transactions)
    let held: Vec<(String, Vec<(u64, Option<u64>)>, u64)> = vec![
        (receive(0), vec![(100_000, Some(1))], 1),
        (receive(1), vec![(200_000, Some(1))], 1),
        (receive(2), vec![], 0),
        (receive(3), vec![], 2),
        (receive(7), vec![(300_000, Some(1)), (50_000, None)], 2),
        (change.clone(), vec![(500_000, Some(91))], 1),
    ];
    let fixture = held.clone();
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let path = request.url.path();
            let find = |prefix: &str| {
                let address = path.strip_prefix(prefix)?;
                fixture.iter().find(|(held, _, _)| held == address)
            };
            let body = if path == "/api/v2" {
                json!({"blockbook": {"bestHeight": 100}, "backend": {"blocks": 100}})
            } else if let Some((address, outputs, _)) = find("/api/v2/utxo/") {
                json!(outputs.iter().enumerate().map(|(vout, (value, height))| json!({
                    "txid": format!("{:064x}", value), "vout": vout, "value": value.to_string(),
                    "confirmations": height.map_or(0, |height| 101 - height),
                    "height": height.unwrap_or(0), "address": address,
                })).collect::<Vec<_>>())
            } else if let Some((_, _, transactions)) = find("/api/v2/address/") {
                json!({"balance": "0", "unconfirmedBalance": "0", "txs": transactions, "unconfirmedTxs": 0})
            } else {
                panic!("unexpected indexer read {path}")
            };
            ResponseTemplate::new(200).set_body_json(body)
        })
        .mount(&server)
        .await;
    let service = account(chain, &server, path).await;
    // Handed out, then received at.
    assert_eq!(
        service
            .receive_address("w".into(), chain, true)
            .await
            .unwrap(),
        Some(receive(1))
    );
    for (address, branch, index) in [
        (receive(2), "external", 2),
        (receive(3), "external", 3),
        (receive(7), "external", 7),
        (change.clone(), "change", 3),
    ] {
        service
            .register_owned_address(
                "w".into(),
                chain,
                address,
                None,
                Some(branch.into()),
                Some(index),
            )
            .await
            .unwrap();
    }

    let coins = service.wallet_coins("w".into()).await.unwrap();
    let places: Vec<_> = coins
        .addresses
        .iter()
        .map(|entry| {
            (
                entry.address.clone(),
                entry.branch,
                entry.index,
                entry.used,
                entry.balance.as_str(),
            )
        })
        .collect();
    use super::AddressBranch::{Change, Receive};
    assert_eq!(
        places,
        [
            (receive(0), Some(Receive), Some(0), true, "0.001"),
            (receive(1), Some(Receive), Some(1), true, "0.002"),
            (receive(2), Some(Receive), Some(2), false, "0"),
            (receive(3), Some(Receive), Some(3), true, "0"),
            (receive(7), Some(Receive), Some(7), true, "0.0035"),
            (change, Some(Change), Some(3), true, "0.005"),
        ]
    );
    assert_eq!(coins.next_receive_address, Some(receive(8)));
    let outputs: Vec<_> = coins
        .outputs
        .iter()
        .map(|output| {
            (
                output.amount.as_str(),
                output.confirmations,
                output.spendable,
            )
        })
        .collect();
    assert_eq!(
        outputs,
        [
            ("0.005", 10, true),
            ("0.003", 100, true),
            ("0.002", 100, true),
            ("0.001", 100, true),
            ("0.0005", 0, true),
        ]
    );
    assert_eq!((coins.maturing.as_str(), coins.tip_height), ("0", 100));
}

/// A Peercoin parent paying `value` to `script`: a coinbase, a coinstake
/// (whose first output is empty), or an ordinary transfer.
fn peercoin_parent(script: &ScriptBuf, value: u64, kind: &str, nonce: u32) -> Transaction {
    let mut output = Vec::new();
    if kind == "coinstake" {
        output.push(TxOut {
            value: Amount::ZERO,
            script_pubkey: ScriptBuf::new(),
        });
    }
    output.push(TxOut {
        value: Amount::from_sat(value),
        script_pubkey: script.clone(),
    });
    Transaction {
        version: bitcoin::transaction::Version(3),
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: if kind == "coinbase" {
                OutPoint::null()
            } else {
                OutPoint {
                    txid: "11".repeat(32).parse().unwrap(),
                    vout: nonce,
                }
            },
            script_sig: ScriptBuf::from_bytes(vec![1, nonce as u8]),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output,
    }
}

/// A minting reward is in the wallet's total from the block it is minted
/// in, and spendable only once it has matured: Peercoin's coins name what
/// is still maturing.
#[tokio::test]
async fn a_peercoin_reward_still_maturing_is_held_and_not_spendable() {
    let chain = Chain::Peercoin;
    let path = "m/44'/6'/0'/0/0";
    let address = UtxoDerivation::new(chain, SEED, path.into())
        .unwrap()
        .derive(0)
        .unwrap()
        .0;
    let script = ScriptBuf::from_bytes(
        crate::derivation::utxo_address::parse_utxo_address(chain, &address)
            .unwrap()
            .script_pubkey(),
    );
    let maturity = u64::from(chain.peercoin_generated_output_maturity().unwrap());
    // (parent, confirmations)
    let parents = vec![
        (
            peercoin_parent(&script, 1_000_000, "coinstake", 1),
            maturity,
        ),
        (
            peercoin_parent(&script, 2_000_000, "coinstake", 2),
            maturity - 1,
        ),
        (
            peercoin_parent(&script, 3_000_000, "coinbase", 3),
            maturity - 1,
        ),
    ];
    let fixture = parents.clone();
    let owner = address.clone();
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let path = request.url.path();
            let body = if path == "/api/v2" {
                json!({"blockbook": {"coin": "Peercoin", "decimals": 6, "bestHeight": 900_000},
                    "backend": {"chain": "livenet", "blocks": 900_000}})
            } else if path == format!("/api/v2/utxo/{owner}") {
                json!(
                    fixture
                        .iter()
                        .map(|(tx, confirmations)| json!({
                            "txid": tx.compute_txid().to_string(), "vout": tx.output.len() - 1,
                            "value": tx.output.last().unwrap().value.to_sat().to_string(),
                            "confirmations": confirmations, "height": 1,
                        }))
                        .collect::<Vec<_>>()
                )
            } else if let Some(id) = path.strip_prefix("/api/v2/tx/") {
                let (tx, confirmations) = fixture
                    .iter()
                    .find(|(tx, _)| tx.compute_txid().to_string() == id)
                    .unwrap();
                json!({"txid": id, "hex": hex::encode(bitcoin::consensus::serialize(tx)),
                    "confirmations": confirmations, "blockHeight": 1})
            } else {
                panic!("unexpected indexer read {path}")
            };
            ResponseTemplate::new(200).set_body_json(body)
        })
        .mount(&server)
        .await;
    let service = account(chain, &server, path).await;
    let coins = service.wallet_coins("w".into()).await.unwrap();
    let txid = |index: usize| parents[index].0.compute_txid().to_string();
    let outputs: Vec<_> = coins
        .outputs
        .iter()
        .map(|output| {
            (
                output.txid.clone(),
                output.amount.as_str(),
                output.spendable,
            )
        })
        .collect();
    assert_eq!(
        outputs,
        [
            (txid(2), "3", false),
            (txid(1), "2", false),
            (txid(0), "1", true)
        ]
    );
    assert_eq!(coins.maturing, "5");
    assert_eq!(coins.tip_height, 900_000);
    assert_eq!(
        (
            coins.addresses[0].address.as_str(),
            coins.addresses[0].balance.as_str()
        ),
        (address.as_str(), "6")
    );
}
