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
    let total: u64 = utxos.iter().map(|utxo| utxo.value).sum();
    let paid: u64 = outputs.iter().map(|output| output.1).sum();
    (
        PreparedAccountTransfer {
            inputs: utxos
                .into_iter()
                .map(|utxo| UtxoPreparedInput {
                    source: UtxoSendSource {
                        address: format!("source of {}:{}", utxo.txid, utxo.vout),
                        derivation_path: None,
                        script_pubkey: utxo.script_pubkey.clone(),
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
    assert_eq!(
        signed.payload.raw_hex().unwrap(),
        bitcoin["raw"].as_str().unwrap()
    );
    assert_eq!(signed.transaction_hash.as_deref(), bitcoin["txid"].as_str());

    for vector in fixtures["legacy"].as_array().unwrap() {
        let fork_id = vector["fork_id"].as_u64().map(|id| id as u32);
        let (prepared, keys) = transfer(vector, AccountProtocol::LegacyP2pkh { fork_id });
        let signed = prepared.sign(&keys).unwrap();
        assert_eq!(
            signed.payload.raw_hex().unwrap(),
            vector["raw"].as_str().unwrap()
        );
        assert_eq!(signed.transaction_hash.as_deref(), vector["txid"].as_str());
    }

    let (prepared, keys) = transfer(&fixtures["kaspa"], AccountProtocol::Kaspa);
    let crate::send::payload::SignedPayload::Kaspa(transaction) =
        prepared.sign(&keys).unwrap().payload
    else {
        panic!("Kaspa submits a typed transaction")
    };
    assert_eq!(transaction.inputs.len(), 3);

    // Fewer keys than inputs, a fee that does not balance, an input whose
    // script is not its source's: refused before signing.
    let (prepared, keys) = transfer(bitcoin, AccountProtocol::Bitcoin);
    assert!(prepared.sign(&keys[..2]).is_err());
    let mut unbalanced = prepared.clone();
    unbalanced.fee += 1;
    assert!(unbalanced.sign(&keys).is_err());
    let mut foreign = prepared.clone();
    foreign.inputs[0].source.script_pubkey = foreign.inputs[1].utxo.script_pubkey.clone();
    foreign.inputs[0].source.script_pubkey[5] ^= 1;
    assert!(foreign.sign(&keys).is_err());
}

/// Qtum, Fractal Bitcoin, DigiByte and eCash: a spend between the network's
/// own addresses, signed by an independent library with its parameters (and
/// eCash's through ecash-lib's own BIP143 preimage), is the transfer core
/// builds from those addresses under the network's protocol, byte for byte.
#[test]
fn each_network_signs_its_own_addresses_vector() {
    let fixtures = fixtures();
    let vectors = fixtures["networks"].as_array().unwrap();
    let mut chains = std::collections::BTreeSet::new();
    for vector in vectors {
        let chain = Chain::parse(vector["chain"].as_str().unwrap()).unwrap();
        chains.insert(chain);
        let protocol = AccountProtocol::for_chain(chain).unwrap();
        let (mut prepared, keys) = transfer(vector, protocol);
        // The scripts come from the network's addresses, as a send derives
        // them: each source's own, and the recipient's and change's.
        for (input, fixture) in prepared
            .inputs
            .iter_mut()
            .zip(vector["inputs"].as_array().unwrap())
        {
            let address = fixture["address"].as_str().unwrap();
            let script = source_script(chain, address).unwrap();
            assert_eq!(script, input.utxo.script_pubkey, "{chain} {address}");
            input.source.address = address.into();
            input.source.script_pubkey = script;
        }
        for (output, fixture) in prepared
            .outputs
            .iter_mut()
            .zip(vector["outputs"].as_array().unwrap())
        {
            output.0 = recipient_script(chain, fixture["address"].as_str().unwrap()).unwrap();
        }
        let signed = prepared.sign(&keys).unwrap();
        assert_eq!(
            signed.payload.raw_hex().unwrap(),
            vector["raw"].as_str().unwrap(),
            "{}",
            vector["name"]
        );
        assert_eq!(signed.transaction_hash.as_deref(), vector["txid"].as_str());
    }
    assert_eq!(
        chains.into_iter().collect::<Vec<_>>(),
        [
            Chain::Qtum,
            Chain::FractalBitcoin,
            Chain::Ecash,
            Chain::Digibyte
        ]
    );
}

/// What an account's own address may pay, per network: SegWit only where the
/// network has it, Taproot only where its signer spends it, Decred and Zcash
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
        Chain::Qtum,
        Chain::FractalBitcoin,
        Chain::Digibyte,
    ] {
        for script in [P2pkh, P2shP2wpkh, P2wpkh] {
            assert!(
                source_script(chain, &address(chain, script)).is_ok(),
                "{chain} {script:?}"
            );
        }
    }
    for chain in [Chain::Bitcoin, Chain::FractalBitcoin, Chain::Digibyte] {
        assert_eq!(
            source_script(chain, &address(chain, P2tr)).unwrap().len(),
            34,
            "{chain}"
        );
    }
    // Qtum's Taproot is not one Spectra signs, nor are its addresses
    // derived; a Qtum account spends P2PKH and P2WPKH.
    assert!(Chain::Qtum.encode_discovery_address(&key, P2tr).is_ok());
    assert!(source_script(Chain::Qtum, &address(Chain::Qtum, P2tr)).is_err());
    for chain in [
        Chain::BitcoinCash,
        Chain::BitcoinSV,
        Chain::Dogecoin,
        Chain::Dash,
        Chain::BitcoinGold,
        Chain::Zcash,
        Chain::Decred,
        Chain::Ecash,
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
    // Fractal's addresses are Bitcoin's, letter for letter: no address can
    // tell the two networks apart. DigiByte's `D…` is Dogecoin's P2PKH
    // version too; its `S…` P2SH and `dgb1` are its own.
    for script in [P2pkh, P2shP2wpkh, P2wpkh, P2tr] {
        assert_eq!(
            address(Chain::FractalBitcoin, script),
            address(Chain::Bitcoin, script)
        );
    }
    assert_eq!(
        address(Chain::Digibyte, P2pkh),
        address(Chain::Dogecoin, P2pkh)
    );
    assert!(recipient_script(Chain::Dogecoin, &address(Chain::Digibyte, P2shP2wpkh)).is_err());
    assert!(recipient_script(Chain::Litecoin, &address(Chain::Digibyte, P2wpkh)).is_err());
    // eCash writes CashAddr alone: Bitcoin Cash's prefix, and the legacy
    // Base58 spelling of the same hash, are refused.
    let ecash = address(Chain::Ecash, P2pkh);
    assert!(ecash.starts_with("ecash:q"), "{ecash}");
    assert!(recipient_script(Chain::Ecash, ecash.trim_start_matches("ecash:")).is_ok());
    assert!(recipient_script(Chain::Ecash, &address(Chain::BitcoinCash, P2pkh)).is_err());
    assert!(recipient_script(Chain::BitcoinCash, &ecash).is_err());
}
