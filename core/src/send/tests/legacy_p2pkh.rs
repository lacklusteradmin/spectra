//! Legacy and SIGHASH_FORKID account spends against bitcoinjs-lib
//! (`account-utxo-transactions.json`).

use super::*;
use crate::send::bitcoin::tests::{fixtures, spend};

fn sign_vector(vector: &serde_json::Value, swap: bool) -> Result<Vec<u8>, SendError> {
    let (utxos, mut keys, outputs) = spend(vector);
    if swap {
        keys.swap(0, 1);
    }
    let fork_id = vector["fork_id"].as_u64().map(|id| id as u32);
    sign(
        &utxos
            .iter()
            .zip(&keys)
            .map(|(utxo, key)| LegacyInput {
                utxo,
                private_key: key,
            })
            .collect::<Vec<_>>(),
        &outputs,
        fork_id,
    )
}

/// Dogecoin and Dash's legacy hash, Bitcoin Cash and SV's fork id 0 and
/// Bitcoin Gold's 79: bitcoinjs-lib's signed bytes, three inputs on two keys.
#[test]
fn legacy_and_forkid_spends_match_bitcoinjs_byte_for_byte() {
    for vector in fixtures()["legacy"].as_array().unwrap() {
        let raw = sign_vector(vector, false).unwrap();
        assert_eq!(
            hex::encode(&raw),
            vector["raw"].as_str().unwrap(),
            "{}",
            vector["name"]
        );
        assert_eq!(
            crate::send::payload::bitcoin_transaction_id(&hex::encode(&raw)).as_deref(),
            vector["txid"].as_str()
        );
        assert!(sign_vector(vector, true).is_err(), "{}", vector["name"]);
    }
}
