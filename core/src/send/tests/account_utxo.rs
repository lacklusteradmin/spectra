//! Reviewed account transfers: their own arithmetic, each protocol's signer
//! behind one call, and the scripts an account's addresses may pay.

use super::*;
use crate::send::bitcoin::tests::{fixtures, spend};
use crate::send::stages::UtxoSendSource;

/// A vector's spend as a reviewed transfer under `protocol`.
fn transfer(
    vector: &serde_json::Value,
    protocol: AccountProtocol,
) -> (PreparedAccountTransfer, Vec<Zeroizing<Vec<u8>>>) {
    let (utxos, keys, outputs) = spend(vector);
    let total: u64 = utxos.iter().map(|utxo| utxo.2).sum();
    let paid: u64 = outputs.iter().map(|output| output.1).sum();
    (
        PreparedAccountTransfer {
            inputs: utxos
                .into_iter()
                .map(|utxo| UtxoPreparedInput {
                    source: UtxoSendSource {
                        address: format!("source of {}:{}", utxo.0, utxo.1),
                        derivation_path: None,
                        script_pubkey: utxo.3.clone(),
                    },
                    utxo,
                })
                .collect(),
            outputs,
            fee: total - paid,
            protocol,
        },
        keys.into_iter().map(Zeroizing::new).collect(),
    )
}

/// One call signs every protocol's vector to its library's bytes, and the
/// transfer's arithmetic is checked first.
#[test]
fn each_protocol_signs_its_vector_through_the_transfer() {
    let fixtures = fixtures();
    let bitcoin = &fixtures["bitcoin"][1];
    let (prepared, keys) = transfer(bitcoin, AccountProtocol::Bitcoin);
    let signed = prepared.sign(&keys).unwrap();
    assert_eq!(signed.payload, bitcoin["raw"].as_str().unwrap());
    assert_eq!(signed.transaction_hash.as_deref(), bitcoin["txid"].as_str());

    for vector in fixtures["legacy"].as_array().unwrap() {
        let fork_id = vector["fork_id"].as_u64().map(|id| id as u32);
        let (prepared, keys) = transfer(vector, AccountProtocol::LegacyP2pkh { fork_id });
        let signed = prepared.sign(&keys).unwrap();
        assert_eq!(signed.payload, vector["raw"].as_str().unwrap());
        assert_eq!(signed.transaction_hash.as_deref(), vector["txid"].as_str());
    }

    let (prepared, keys) = transfer(&fixtures["kaspa"], AccountProtocol::Kaspa);
    let body: serde_json::Value =
        serde_json::from_str(&prepared.sign(&keys).unwrap().payload).unwrap();
    assert_eq!(body["transaction"]["inputs"].as_array().unwrap().len(), 3);

    // Fewer keys than inputs, a fee that does not balance, an input whose
    // script is not its source's: refused before signing.
    let (prepared, keys) = transfer(bitcoin, AccountProtocol::Bitcoin);
    assert!(prepared.sign(&keys[..2]).is_err());
    let mut unbalanced = prepared.clone();
    unbalanced.fee += 1;
    assert!(unbalanced.sign(&keys).is_err());
    let mut foreign = prepared.clone();
    foreign.inputs[0].source.script_pubkey = foreign.inputs[1].utxo.3.clone();
    foreign.inputs[0].source.script_pubkey[5] ^= 1;
    assert!(foreign.sign(&keys).is_err());
}

/// What an account's own address may pay, per network: SegWit only where the
/// network has it, Taproot only on Bitcoin and Peercoin, Decred and Zcash
/// P2PKH, Kaspa's Schnorr key; and recipients refused on another network.
#[test]
fn account_scripts_follow_each_networks_signer() {
    use crate::derivation::types::BitcoinScriptType::*;
    let key = secp256k1::PublicKey::from_secret_key(
        &secp256k1::Secp256k1::new(),
        &secp256k1::SecretKey::from_slice(&[9; 32]).unwrap(),
    );
    let address = |chain: Chain, script| chain.encode_discovery_address(&key, script).unwrap();
    for chain in [
        Chain::Bitcoin,
        Chain::BitcoinTestnet4,
        Chain::Litecoin,
        Chain::Peercoin,
    ] {
        for script in [P2pkh, P2shP2wpkh, P2wpkh] {
            assert!(
                source_script(chain, &address(chain, script)).is_ok(),
                "{chain} {script:?}"
            );
        }
    }
    assert_eq!(
        source_script(Chain::Bitcoin, &address(Chain::Bitcoin, P2tr))
            .unwrap()
            .len(),
        34
    );
    for chain in [
        Chain::BitcoinCash,
        Chain::BitcoinSV,
        Chain::Dogecoin,
        Chain::Dash,
        Chain::BitcoinGold,
        Chain::Zcash,
        Chain::Decred,
    ] {
        let script = source_script(chain, &address(chain, P2pkh)).unwrap();
        assert_eq!(
            (script.len(), script[0], script[24]),
            (25, 0x76, 0xac),
            "{chain}"
        );
    }
    let kaspa = source_script(Chain::Kaspa, &address(Chain::Kaspa, P2pkh)).unwrap();
    assert_eq!((kaspa.len(), kaspa[0], kaspa[33]), (34, 0x20, 0xac));
    // A legacy Bitcoin Cash address is spelled as Bitcoin's; a mainnet
    // address is no test network's.
    assert!(recipient_script(Chain::Bitcoin, &address(Chain::BitcoinCash, P2pkh)).is_ok());
    assert!(recipient_script(Chain::BitcoinTestnet, &address(Chain::Bitcoin, P2wpkh)).is_err());
    assert!(recipient_script(Chain::KaspaTestnet, &address(Chain::Kaspa, P2pkh)).is_err());
    assert!(recipient_script(Chain::ZcashTestnet, &address(Chain::Zcash, P2pkh)).is_err());
    assert!(recipient_script(Chain::DecredTestnet, &address(Chain::Decred, P2pkh)).is_err());
}
