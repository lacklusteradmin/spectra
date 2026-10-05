use super::*;
use crate::registry::Chain;
use bitcoin::hashes::{Hash, sha256d};
use bitcoin::{Amount, ScriptBuf, Transaction, TxIn, TxOut, Txid, Witness, absolute, transaction};
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

fn transaction(kind: &str, witness: bool) -> Transaction {
    let input = TxIn {
        previous_output: if kind == "coinbase" {
            bitcoin::OutPoint::null()
        } else {
            bitcoin::OutPoint {
                txid: Txid::from_byte_array([7; 32]),
                vout: 1,
            }
        },
        script_sig: ScriptBuf::from_bytes(vec![1, 1]),
        sequence: bitcoin::Sequence::MAX,
        witness: if witness {
            Witness::from_slice(&[vec![1, 2, 3]])
        } else {
            Witness::new()
        },
    };
    let mut outputs = vec![TxOut {
        value: Amount::from_sat(50_000),
        script_pubkey: ScriptBuf::from_bytes(
            [vec![0x76, 0xa9, 0x14], vec![2; 20], vec![0x88, 0xac]].concat(),
        ),
    }];
    if kind == "coinstake" {
        outputs.insert(
            0,
            TxOut {
                value: Amount::ZERO,
                script_pubkey: ScriptBuf::new(),
            },
        );
    }
    Transaction {
        version: transaction::Version(3),
        lock_time: absolute::LockTime::ZERO,
        input: vec![input],
        output: outputs,
    }
}

fn status(chain: Chain) -> Value {
    json!({"blockbook":{"coin":if chain.is_testnet() {"Peercoin Testnet"} else {"Peercoin"},"decimals":6},
        "backend":{"chain":if chain.is_testnet() {"testnet"} else {"livenet"},"blocks":900_000}})
}

#[test]
fn future_dated_legacy_inputs_are_refused_before_spending() {
    let mut tx = transaction("ordinary", false);
    tx.version = transaction::Version(2);
    let mut raw = bitcoin::consensus::serialize(&tx);
    raw.splice(4..4, u32::MAX.to_le_bytes());
    let id = Txid::from_raw_hash(sha256d::Hash::hash(&raw)).to_string();
    assert!(decode_peercoin_transaction(&hex::encode(raw), &id).is_err());
}

async fn client_for(
    chain: Chain,
    tx: &Transaction,
    index: u32,
    confirmations: u64,
    modify: impl FnOnce(&mut Value, &mut Value, &mut Value),
) -> (MockServer, BlockbookClient) {
    let server = MockServer::start().await;
    let id = tx.compute_txid().to_string();
    let mut identity = status(chain);
    // The UTXO's confirmation count is deliberately inflated: maturity is
    // judged from the transaction response, never trusted from this list.
    let mut outputs =
        json!([{"txid":id,"vout":index,"value":"50000","confirmations":999_999,"height":100}]);
    let mut response = json!({"txid":id,"hex":hex::encode(bitcoin::consensus::serialize(tx)),"confirmations":confirmations});
    modify(&mut identity, &mut outputs, &mut response);
    Mock::given(any())
        .respond_with(move |request: &Request| {
            ResponseTemplate::new(200).set_body_json(if request.url.path() == "/api/v2" {
                identity.clone()
            } else if request.url.path().contains("/utxo/") {
                outputs.clone()
            } else {
                response.clone()
            })
        })
        .mount(&server)
        .await;
    let client = BlockbookClient::new(Arc::new(vec![server.uri()]), chain);
    (server, client)
}

#[tokio::test]
async fn generated_outputs_mature_at_each_networks_boundary() {
    for (chain, maturity) in [(Chain::Peercoin, 500), (Chain::PeercoinTestnet, 60)] {
        for kind in ["coinbase", "coinstake"] {
            let tx = transaction(kind, false);
            let index = u32::from(kind == "coinstake");
            for (confirmations, count) in [(maturity - 1, 0), (maturity, 1)] {
                let (_server, client) =
                    client_for(chain, &tx, index, confirmations, |_, _, _| {}).await;
                let inputs = client.fetch_peercoin_inputs("holder").await.unwrap();
                assert_eq!(inputs.len(), count, "{chain} {kind} at {confirmations}");
                if count == 1 {
                    assert_eq!(inputs[0].0, tx.compute_txid().to_string());
                    assert_eq!(inputs[0].1, index);
                    assert_eq!(inputs[0].2, 50_000);
                    assert_eq!(
                        inputs[0].3,
                        tx.output[index as usize].script_pubkey.to_bytes()
                    );
                }
            }
        }
        let tx = transaction("ordinary", false);
        let (_server, client) = client_for(chain, &tx, 0, 0, |_, _, _| {}).await;
        assert_eq!(
            client.fetch_peercoin_inputs("holder").await.unwrap().len(),
            1
        );
    }
}

#[tokio::test]
async fn verified_zero_valued_outputs_do_not_block_spendable_inputs() {
    for chain in [Chain::Peercoin, Chain::PeercoinTestnet] {
        let mut tx = transaction("ordinary", false);
        let spendable = tx.output[0].clone();
        tx.output[0].value = Amount::ZERO;
        tx.output.push(spendable);
        let id = tx.compute_txid().to_string();
        let (_server, client) = client_for(chain, &tx, 1, 1000, |_, outputs, _| {
            outputs.as_array_mut().unwrap().insert(
                0,
                json!({"txid":id,"vout":0,"value":"0","confirmations":1000}),
            );
        })
        .await;
        let inputs = client.fetch_peercoin_inputs("holder").await.unwrap();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].1, 1);
        assert_eq!(inputs[0].2, 50_000);
        assert_eq!(inputs[0].3, tx.output[1].script_pubkey.to_bytes());

        let (_server, client) = client_for(chain, &tx, 0, 1000, |_, outputs, _| {
            outputs[0]["value"] = json!("0");
        })
        .await;
        assert!(
            client
                .fetch_peercoin_inputs("holder")
                .await
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn transaction_status_must_match_the_requested_identity() {
    let tx = transaction("ordinary", false);
    let id = tx.compute_txid().to_string();
    for chain in [Chain::Peercoin, Chain::PeercoinTestnet] {
        let (_server, client) = client_for(chain, &tx, 0, 1000, |_, _, response| {
            response["blockHeight"] = json!(899_001);
        })
        .await;
        let confirmed = client.fetch_tx_status(&id).await.unwrap();
        assert!(confirmed.confirmed);
        assert_eq!(confirmed.txid, id);

        let (_server, client) = client_for(chain, &tx, 0, 1000, |_, _, response| {
            response["txid"] = json!("ff".repeat(32));
            response["blockHeight"] = json!(899_001);
        })
        .await;
        assert!(client.fetch_tx_status(&id).await.is_err());
    }
}

#[tokio::test]
async fn untrusted_input_metadata_is_refused() {
    let tx = transaction("ordinary", false);
    for case in [
        "no raw",
        "bad raw",
        "wrong hash",
        "wrong response id",
        "value",
        "false zero value",
        "index",
        "duplicate",
        "no confirmations",
    ] {
        let (_server, client) = client_for(
            Chain::Peercoin,
            &tx,
            0,
            1000,
            |_, outputs, response| match case {
                "no raw" => {
                    response.as_object_mut().unwrap().remove("hex");
                }
                "bad raw" => response["hex"] = json!("00"),
                "wrong hash" => {
                    let other = transaction("coinbase", false);
                    response["hex"] = json!(hex::encode(bitcoin::consensus::serialize(&other)));
                }
                "wrong response id" => response["txid"] = json!("ff".repeat(32)),
                "value" => outputs[0]["value"] = json!("50001"),
                "false zero value" => outputs[0]["value"] = json!("0"),
                "index" => outputs[0]["vout"] = json!(9),
                "duplicate" => {
                    let duplicate = outputs[0].clone();
                    outputs.as_array_mut().unwrap().push(duplicate);
                }
                "no confirmations" => {
                    response.as_object_mut().unwrap().remove("confirmations");
                }
                _ => unreachable!(),
            },
        )
        .await;
        assert!(
            client.fetch_peercoin_inputs("holder").await.is_err(),
            "{case}"
        );
    }
}

#[tokio::test]
async fn every_peercoin_request_refuses_wrong_network_or_precision() {
    for chain in [Chain::Peercoin, Chain::PeercoinTestnet] {
        for case in ["coin", "network", "precision", "missing identity"] {
            let server = MockServer::start().await;
            let mut identity = status(chain);
            match case {
                "coin" => identity["blockbook"]["coin"] = json!("Bitcoin"),
                "network" => {
                    identity["backend"]["chain"] = json!(if chain.is_testnet() {
                        "livenet"
                    } else {
                        "testnet"
                    })
                }
                "precision" => identity["blockbook"]["decimals"] = json!(8),
                "missing identity" => {
                    identity.as_object_mut().unwrap().remove("blockbook");
                }
                _ => unreachable!(),
            }
            Mock::given(any())
                .respond_with(ResponseTemplate::new(200).set_body_json(identity))
                .mount(&server)
                .await;
            let client = BlockbookClient::new(Arc::new(vec![server.uri()]), chain);
            assert!(
                client.fetch_balance("holder").await.is_err(),
                "{chain} {case}"
            );
            assert!(
                client.fetch_peercoin_inputs("holder").await.is_err(),
                "{chain} {case}"
            );
            assert!(client.fetch_fee_rate(6).await.is_err(), "{chain} {case}");
            assert!(
                client.broadcast_raw_tx("00").await.is_err(),
                "{chain} {case}"
            );
            assert!(
                server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .all(|r| r.url.path() == "/api/v2")
            );
        }
    }
}

#[tokio::test]
async fn fee_rates_convert_each_networks_native_precision() {
    for (chain, expected) in [
        (Chain::Bitcoin, 1000.0),
        (Chain::Peercoin, 10.0),
        (Chain::PeercoinTestnet, 10.0),
    ] {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(move |request: &Request| {
                ResponseTemplate::new(200).set_body_json(if request.url.path() == "/api/v2" {
                    status(chain)
                } else {
                    json!({"result":"0.01"})
                })
            })
            .mount(&server)
            .await;
        let client = BlockbookClient::new(Arc::new(vec![server.uri()]), chain);
        assert_eq!(
            client.fetch_fee_rate(6).await.unwrap().sats_per_vbyte,
            expected
        );
    }
}

#[test]
fn old_timestamps_and_witness_transaction_ids_are_hash_verified() {
    for version in [1, 2, 3] {
        for witness in [false, true] {
            let mut tx = transaction("ordinary", witness);
            tx.version = transaction::Version(version);
            let mut raw = bitcoin::consensus::serialize(&tx);
            let mut stripped = tx.clone();
            stripped.input[0].witness = Witness::new();
            let mut txid_bytes = bitcoin::consensus::serialize(&stripped);
            if version < 3 {
                // Wire v1/v2 keep timestamp in the txid preimage; witnesses do
                // not. This fixture does not use the decoder to derive its ID.
                let timestamp = 1_700_000_000u32.to_le_bytes();
                raw.splice(4..4, timestamp);
                txid_bytes.splice(4..4, timestamp);
            }
            let id = Txid::from_raw_hash(sha256d::Hash::hash(&txid_bytes)).to_string();
            assert_eq!(
                decode_peercoin_transaction(&hex::encode(&raw), &id).unwrap(),
                tx
            );
            assert!(decode_peercoin_transaction(&hex::encode(&raw), &"ff".repeat(32)).is_err());
            if version < 3 {
                assert_ne!(id, tx.compute_txid().to_string());
            }
            if witness {
                assert_ne!(
                    id,
                    Txid::from_raw_hash(sha256d::Hash::hash(&raw)).to_string()
                );
            }
        }
    }
}

#[test]
fn live_coinstake_fixture_preserves_its_actual_p2pk_script() {
    // Block 894599 on the public mainnet Blockbook, checked 2026-10-04.
    let id = "6ee6e0213f4cd70e020e208d62fe9c6aeeca6db182516175bf4cef16535b8c8d";
    let raw = "03000000017246d049372c7130ec4195cb93648203d396831bfb45942233c7bd4be9dd9829010000004847304402206a87938b258c1aed15edb2c9c3b6696696cd250f6015f8100e1c963cfe017650022041be142cb2ece3d0835f6d8d6de9eaece74be0e584eddf75038ea6e3f93eaff401ffffffff020000000000000000000e44dfdd0100000023210213d8571ead2765e5b98cd0733d8b36ff1bc378a2e5378dc6e33ea709223f7910ac00000000";
    let tx = decode_peercoin_transaction(raw, id).unwrap();
    assert!(!tx.is_coinbase());
    assert_eq!(tx.output[0].value.to_sat(), 0);
    assert!(tx.output[0].script_pubkey.is_empty());
    assert_eq!(tx.output[1].value.to_sat(), 8_017_363_982);
    assert_eq!(
        hex::encode(tx.output[1].script_pubkey.as_bytes()),
        "210213d8571ead2765e5b98cd0733d8b36ff1bc378a2e5378dc6e33ea709223f7910ac"
    );
    assert!(tx.output[1].script_pubkey.is_p2pk());
}
