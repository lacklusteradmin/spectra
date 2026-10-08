//! The account signer against bitcoinjs-lib (`account-utxo-transactions.json`,
//! from scripts/generate-account-utxo-vectors.cjs).

use super::*;

pub(crate) type Utxo = (String, u32, u64, Vec<u8>);

pub(crate) fn fixtures() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/account-utxo-transactions.json"
    ))
    .unwrap()
}

/// A vector's inputs, their keys and its outputs.
pub(crate) fn spend(vector: &serde_json::Value) -> (Vec<Utxo>, Vec<Vec<u8>>, Vec<(Vec<u8>, u64)>) {
    let text = |value: &serde_json::Value, key: &str| value[key].as_str().unwrap().to_string();
    let inputs = vector["inputs"].as_array().unwrap();
    (
        inputs
            .iter()
            .map(|input| {
                (
                    text(input, "txid"),
                    input["vout"].as_u64().unwrap() as u32,
                    input["value"].as_u64().unwrap(),
                    hex::decode(text(input, "script")).unwrap(),
                )
            })
            .collect(),
        inputs
            .iter()
            .map(|input| hex::decode(text(input, "key")).unwrap())
            .collect(),
        vector["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|output| {
                (
                    hex::decode(text(output, "script")).unwrap(),
                    output["value"].as_u64().unwrap(),
                )
            })
            .collect(),
    )
}

fn sign(
    utxos: &[Utxo],
    keys: &[Vec<u8>],
    outputs: &[(Vec<u8>, u64)],
) -> Result<Vec<u8>, SendError> {
    sign_inputs(
        &utxos
            .iter()
            .zip(keys)
            .map(|(utxo, key)| SigningInput {
                utxo,
                private_key: key,
            })
            .collect::<Vec<_>>(),
        outputs,
        Version::TWO,
        Sequence::ENABLE_RBF_NO_LOCKTIME,
    )
}

/// ECDSA spends of three inputs on two keys are bitcoinjs-lib's bytes, and
/// the size estimate bounds each.
#[test]
fn ecdsa_account_spends_match_bitcoinjs_byte_for_byte() {
    for vector in fixtures()["bitcoin"].as_array().unwrap() {
        let name = vector["name"].as_str().unwrap();
        if name == "p2tr" {
            continue;
        }
        let (utxos, keys, outputs) = spend(vector);
        let raw = sign(&utxos, &keys, &outputs).unwrap();
        assert_eq!(hex::encode(&raw), vector["raw"].as_str().unwrap(), "{name}");
        let tx: Transaction = bitcoin::consensus::deserialize(&raw).unwrap();
        assert_eq!(
            tx.compute_txid().to_string(),
            vector["txid"].as_str().unwrap()
        );
        let estimate = estimate_vsize(
            utxos.iter().map(|utxo| utxo.3.as_slice()),
            outputs.iter().map(|output| output.0.len()),
        )
        .unwrap();
        assert!(
            estimate >= tx.vsize() as u64,
            "{name}: {estimate} < {}",
            tx.vsize()
        );
    }
}

/// A Taproot key-path spend is bitcoinjs-lib's unsigned transaction, each
/// input's Schnorr signature verifying under its own output key against
/// bitcoinjs-lib's BIP341 sighash.
#[test]
fn taproot_account_spends_sign_bitcoinjs_sighashes() {
    let fixtures = fixtures();
    let vector = fixtures["bitcoin"]
        .as_array()
        .unwrap()
        .iter()
        .find(|vector| vector["name"] == "p2tr")
        .unwrap();
    let (utxos, keys, outputs) = spend(vector);
    let raw = sign(&utxos, &keys, &outputs).unwrap();
    let mut tx: Transaction = bitcoin::consensus::deserialize(&raw).unwrap();
    let secp = Secp256k1::verification_only();
    for (index, sighash) in vector["sighashes"].as_array().unwrap().iter().enumerate() {
        let witness = &tx.input[index].witness;
        assert_eq!(witness.len(), 1);
        let signature = bitcoin::secp256k1::schnorr::Signature::from_slice(&witness[0]).unwrap();
        let key = bitcoin::secp256k1::XOnlyPublicKey::from_slice(&utxos[index].3[2..]).unwrap();
        let digest: [u8; 32] = hex::decode(sighash.as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        secp.verify_schnorr(&signature, &Message::from_digest(digest), &key)
            .unwrap();
    }
    for input in &mut tx.input {
        input.witness = Witness::new();
    }
    assert_eq!(
        bitcoin::consensus::encode::serialize_hex(&tx),
        vector["unsigned"].as_str().unwrap()
    );
    let estimate = estimate_vsize(
        utxos.iter().map(|utxo| utxo.3.as_slice()),
        outputs.iter().map(|output| output.0.len()),
    )
    .unwrap();
    let signed: Transaction = bitcoin::consensus::deserialize(&raw).unwrap();
    assert!(estimate >= signed.vsize() as u64);
}

/// A key that does not own its input's script, an outpoint twice, and a
/// script no key spends are refused before anything is signed.
#[test]
fn foreign_keys_duplicate_and_unspendable_inputs_are_refused() {
    let fixtures = fixtures();
    for vector in fixtures["bitcoin"].as_array().unwrap() {
        let (utxos, mut keys, outputs) = spend(vector);
        keys.swap(0, 1);
        assert!(sign(&utxos, &keys, &outputs).is_err(), "{}", vector["name"]);
    }
    let (mut utxos, keys, outputs) = spend(&fixtures["bitcoin"][1]);
    utxos[2].0 = utxos[0].0.clone();
    utxos[2].1 = utxos[0].1;
    assert!(sign(&utxos, &keys, &outputs).is_err());
    let (mut utxos, keys, outputs) = spend(&fixtures["bitcoin"][1]);
    utxos[0].3 = ScriptBuf::new_op_return([1u8; 4]).into_bytes();
    assert!(sign(&utxos, &keys, &outputs).is_err());
    assert!(sign(&[], &[], &outputs).is_err());
}
