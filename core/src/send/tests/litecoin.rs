use super::*;
use bitcoin::absolute::LockTime;
use bitcoin::hashes::Hash;
use bitcoin::script::{Builder, PushBytesBuf};
use bitcoin::secp256k1::Message;
use bitcoin::secp256k1::ecdsa::Signature;
use bitcoin::{Amount, OutPoint, Transaction, TxIn, TxOut, Witness};
use sha2::{Digest, Sha256};

fn public_key(key: &[u8]) -> CompressedPublicKey {
    CompressedPublicKey(bitcoin::secp256k1::PublicKey::from_secret_key(
        &Secp256k1::new(),
        &SecretKey::from_slice(key).unwrap(),
    ))
}

fn address(chain: Chain, kind: InputKind, key: &[u8]) -> String {
    let key = public_key(key);
    let (p2pkh, p2sh) = chain.fixed_utxo_address_versions().unwrap();
    let (version, hash) = match kind {
        InputKind::P2pkh => (p2pkh, key.pubkey_hash().to_byte_array()),
        InputKind::NestedP2wpkh => (
            p2sh[0],
            ScriptBuf::new_p2wpkh(&key.wpubkey_hash())
                .script_hash()
                .to_byte_array(),
        ),
        InputKind::P2tr => unreachable!("Litecoin wallets hold no Taproot key"),
        InputKind::P2wpkh => {
            return bech32::segwit::encode(
                bech32::Hrp::parse(chain.fixed_utxo_segwit_hrp().unwrap()).unwrap(),
                bech32::segwit::VERSION_0,
                key.wpubkey_hash().as_byte_array(),
            )
            .unwrap();
        }
    };
    let mut payload = vec![version];
    payload.extend(hash);
    bs58::encode(payload).with_check().into_string()
}

fn input(index: u8, kind: InputKind, key: &[u8]) -> (String, u32, u64, Vec<u8>) {
    (
        format!("{index:02x}").repeat(32),
        index.into(),
        100_000 + u64::from(index),
        kind.script(&public_key(key)).into_bytes(),
    )
}

fn double_hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(Sha256::digest(bytes)).into()
}

fn compact(value: usize, bytes: &mut Vec<u8>) {
    match value {
        0..=0xfc => bytes.push(value as u8),
        0xfd..=0xffff => {
            bytes.push(0xfd);
            bytes.extend((value as u16).to_le_bytes());
        }
        _ => {
            bytes.push(0xfe);
            bytes.extend((value as u32).to_le_bytes());
        }
    }
}

fn write_script(script: &[u8], bytes: &mut Vec<u8>) {
    compact(script.len(), bytes);
    bytes.extend(script);
}

fn write_outpoint(input: &TxIn, bytes: &mut Vec<u8>) {
    bytes.extend(input.previous_output.txid.to_byte_array());
    bytes.extend(input.previous_output.vout.to_le_bytes());
}

fn write_outputs(tx: &Transaction) -> Vec<u8> {
    let mut bytes = Vec::new();
    for output in &tx.output {
        bytes.extend(output.value.to_sat().to_le_bytes());
        write_script(output.script_pubkey.as_bytes(), &mut bytes);
    }
    bytes
}

// Independently serialize legacy/BIP143 SIGHASH_ALL, following Litecoin Core's
// src/script/interpreter.cpp SignatureHash. No production sighash helper is used.
fn signature_hash(
    tx: &Transaction,
    index: usize,
    script_code: &[u8],
    amount: u64,
    witness: bool,
) -> [u8; 32] {
    let mut bytes = tx.version.0.to_le_bytes().to_vec();
    if witness {
        let mut outpoints = Vec::new();
        let mut sequences = Vec::new();
        for input in &tx.input {
            write_outpoint(input, &mut outpoints);
            sequences.extend(input.sequence.0.to_le_bytes());
        }
        bytes.extend(double_hash(&outpoints));
        bytes.extend(double_hash(&sequences));
        write_outpoint(&tx.input[index], &mut bytes);
        write_script(script_code, &mut bytes);
        bytes.extend(amount.to_le_bytes());
        bytes.extend(tx.input[index].sequence.0.to_le_bytes());
        bytes.extend(double_hash(&write_outputs(tx)));
    } else {
        compact(tx.input.len(), &mut bytes);
        for (i, input) in tx.input.iter().enumerate() {
            write_outpoint(input, &mut bytes);
            write_script(if i == index { script_code } else { &[] }, &mut bytes);
            bytes.extend(input.sequence.0.to_le_bytes());
        }
        compact(tx.output.len(), &mut bytes);
        bytes.extend(write_outputs(tx));
    }
    bytes.extend(tx.lock_time.to_consensus_u32().to_le_bytes());
    bytes.extend(1u32.to_le_bytes());
    double_hash(&bytes)
}

fn independent_txid(tx: &Transaction) -> String {
    let mut bytes = tx.version.0.to_le_bytes().to_vec();
    compact(tx.input.len(), &mut bytes);
    for input in &tx.input {
        write_outpoint(input, &mut bytes);
        write_script(input.script_sig.as_bytes(), &mut bytes);
        bytes.extend(input.sequence.0.to_le_bytes());
    }
    compact(tx.output.len(), &mut bytes);
    bytes.extend(write_outputs(tx));
    bytes.extend(tx.lock_time.to_consensus_u32().to_le_bytes());
    let mut hash = double_hash(&bytes);
    hash.reverse();
    hex::encode(hash)
}

fn verify_input(tx: &Transaction, index: usize, kind: InputKind, key: &[u8], amount: u64) {
    let pubkey = public_key(key);
    let input = &tx.input[index];
    let (signature, encoded_key) = if kind == InputKind::P2pkh {
        assert!(input.witness.is_empty());
        let script = input.script_sig.as_bytes();
        let sig_len = script[0] as usize;
        assert_eq!(script[sig_len + 1], 33);
        (&script[1..sig_len + 1], &script[sig_len + 2..])
    } else {
        assert_eq!(input.witness.len(), 2);
        if kind == InputKind::P2wpkh {
            assert!(input.script_sig.is_empty());
        } else {
            let mut redeem_push = vec![22];
            redeem_push.extend(ScriptBuf::new_p2wpkh(&pubkey.wpubkey_hash()).as_bytes());
            assert_eq!(input.script_sig.as_bytes(), redeem_push);
        }
        (&input.witness[0], &input.witness[1])
    };
    assert_eq!(signature.last(), Some(&1));
    assert_eq!(encoded_key, pubkey.to_bytes());
    let signature = Signature::from_der(&signature[..signature.len() - 1]).unwrap();
    let script_code = ScriptBuf::new_p2pkh(&pubkey.pubkey_hash());
    let digest = signature_hash(
        tx,
        index,
        script_code.as_bytes(),
        amount,
        kind != InputKind::P2pkh,
    );
    let secp = Secp256k1::verification_only();
    secp.verify_ecdsa(&Message::from_digest(digest), &signature, &pubkey.0)
        .unwrap();
    if kind != InputKind::P2pkh {
        let wrong_digest = signature_hash(tx, index, script_code.as_bytes(), amount + 1, true);
        assert!(
            secp.verify_ecdsa(&Message::from_digest(wrong_digest), &signature, &pubkey.0)
                .is_err()
        );
    }
}

#[test]
fn independent_bip143_oracle_matches_published_native_and_nested_vectors() {
    // Public-domain BIP143 Native P2WPKH and P2SH-P2WPKH examples:
    // https://github.com/bitcoin/bips/blob/master/bip-0143.mediawiki
    // Litecoin Core uses this same witness-v0 SignatureHash algorithm.
    for (raw, index, amount, key, expected_hash, signature) in [
        (
            "0100000002fff7f7881a8099afa6940d42d1e7f6362bec38171ea3edf433541db4e4ad969f0000000000eeffffffef51e1b804cc89d182d279655c3aa89e815b1b309fe287d9b2b55d57b90ec68a0100000000ffffffff02202cb206000000001976a9148280b37df378db99f66f85c95a783a76ac7a6d5988ac9093510d000000001976a9143bde42dbee7e4dbe6a21b2d50ce2f0167faa815988ac11000000",
            1,
            600_000_000,
            "619c335025c7f4012e556c2a58b2506e30b8511b53ade95ea316fd8c3286feb9",
            "c37af31116d1b27caf68aae9e3ac82f1477929014d5b917657d0eb49478cb670",
            "304402203609e17b84f6a7d30c80bfa610b5b4542f32a8a0d5447a12fb1366d7f01cc44a0220573a954c4518331561406f90300e8f3358f51928d43c212a8caed02de67eebee",
        ),
        (
            "0100000001db6b1b20aa0fd7b23880be2ecbd4a98130974cf4748fb66092ac4d3ceb1a54770100000000feffffff02b8b4eb0b000000001976a914a457b684d7f0d539a46a45bbc043f35b59d0d96388ac0008af2f000000001976a914fd270b1ee6abcaea97fea7ad0402e8bd8ad6d77c88ac92040000",
            0,
            1_000_000_000,
            "eb696a065ef48a2192da5b28b694f87544b30fae8327c4510137a922f32c6dcf",
            "64f3b0f4dd2bb3aa1ce8566d220cc74dda9df97d8490cc81d89d735c92e59fb6",
            "3044022047ac8e878352d3ebbde1c94ce3a10d057c24175747116f8288e5d794d12d482f0220217f36a485cae903c713331d877c1f64677e3622ad4010726870540656fe9dcb",
        ),
    ] {
        let tx: Transaction = bitcoin::consensus::deserialize(&hex::decode(raw).unwrap()).unwrap();
        let key = public_key(&hex::decode(key).unwrap());
        let script_code = ScriptBuf::new_p2pkh(&key.pubkey_hash());
        let digest = signature_hash(&tx, index, script_code.as_bytes(), amount, true);
        assert_eq!(hex::encode(digest), expected_hash);
        Secp256k1::verification_only()
            .verify_ecdsa(
                &Message::from_digest(digest),
                &Signature::from_der(&hex::decode(signature).unwrap()).unwrap(),
                &key.0,
            )
            .unwrap();
    }
}

#[test]
fn legacy_native_and_nested_spends_sign_each_network_and_return_same_type_change() {
    let key = [1; 32];
    let destination = ScriptBuf::new_p2pkh(&bitcoin::PubkeyHash::from_byte_array([0x33; 20]));
    for chain in [Chain::Litecoin, Chain::LitecoinTestnet] {
        for kind in [InputKind::P2pkh, InputKind::P2wpkh, InputKind::NestedP2wpkh] {
            let sender = address(chain, kind, &key);
            validate_ltc_sender(chain, &sender, &key).unwrap();
            let utxos = vec![input(1, kind, &key), input(2, kind, &key)];
            let raw = sign_ltc_with_output_script(
                chain,
                &utxos,
                destination.as_bytes(),
                150_000,
                1_000,
                &sender,
                &key,
            )
            .unwrap();
            let tx: Transaction = bitcoin::consensus::deserialize(&raw).unwrap();
            assert_eq!(tx.output[0].script_pubkey, destination);
            assert_eq!(tx.output[1].script_pubkey, kind.script(&public_key(&key)));
            assert_eq!(tx.output[1].value.to_sat(), 49_003);
            for (index, utxo) in utxos.iter().enumerate() {
                verify_input(&tx, index, kind, &key, utxo.2);
            }
            assert_eq!(tx.compute_txid().to_string(), independent_txid(&tx));
            if kind == InputKind::P2pkh {
                assert_eq!(
                    tx.compute_txid().to_string(),
                    tx.compute_wtxid().to_string()
                );
            } else {
                assert_eq!(&raw[4..6], &[0, 1]);
                assert_ne!(
                    tx.compute_txid().to_string(),
                    tx.compute_wtxid().to_string()
                );
            }
            let estimated = estimate_ltc_vsize(
                utxos.iter().map(|utxo| utxo.3.as_slice()),
                destination.len(),
                Some(tx.output[1].script_pubkey.as_bytes()),
            )
            .unwrap();
            assert!(tx.vsize() as u64 <= estimated);
        }
    }
}

#[test]
fn restored_addresses_spend_different_keys_and_scripts_into_fresh_change() {
    let keys = [[1; 32], [2; 32], [3; 32]];
    let kinds = [InputKind::P2pkh, InputKind::P2wpkh, InputKind::NestedP2wpkh];
    let utxos: Vec<_> = keys
        .iter()
        .zip(kinds)
        .enumerate()
        .map(|(index, (key, kind))| input((index + 1) as u8, kind, key))
        .collect();
    let inputs: Vec<_> = utxos
        .iter()
        .zip(&keys)
        .map(|(utxo, key)| LtcSigningInput {
            utxo,
            private_key: key,
        })
        .collect();
    let change_key = [4; 32];
    let change = address(Chain::Litecoin, InputKind::P2wpkh, &change_key);
    // A Taproot recipient output is independent from the wallet's supported
    // input scripts and its native SegWit change output.
    let mut recipient = vec![0x51, 0x20];
    recipient.extend([0x44; 32]);
    let raw = sign_ltc_inputs_with_output_script(
        Chain::Litecoin,
        &inputs,
        &recipient,
        200_000,
        1_000,
        &change,
        &change_key,
    )
    .unwrap();
    let tx: Transaction = bitcoin::consensus::deserialize(&raw).unwrap();
    assert_eq!(tx.output[0].script_pubkey.as_bytes(), recipient);
    assert_eq!(
        tx.output[1].script_pubkey,
        InputKind::P2wpkh.script(&public_key(&change_key))
    );
    for (index, ((kind, key), utxo)) in kinds.iter().zip(&keys).zip(&utxos).enumerate() {
        verify_input(&tx, index, *kind, key, utxo.2);
    }
    assert_eq!(tx.compute_txid().to_string(), independent_txid(&tx));
    let estimate = estimate_ltc_vsize(
        utxos.iter().map(|utxo| utxo.3.as_slice()),
        recipient.len(),
        Some(tx.output[1].script_pubkey.as_bytes()),
    )
    .unwrap();
    assert!(tx.vsize() as u64 <= estimate);
}

#[test]
fn sender_and_input_validation_refuse_unowned_and_unsupported_scripts() {
    let key = [1; 32];
    let wrong_key = [2; 32];
    let sender = address(Chain::Litecoin, InputKind::P2wpkh, &key);
    assert!(validate_ltc_sender(Chain::Litecoin, &sender, &wrong_key).is_err());
    assert!(validate_ltc_sender(Chain::LitecoinTestnet, &sender, &key).is_err());
    assert!(validate_ltc_sender(Chain::Bitcoin, &sender, &key).is_err());
    let hrp = bech32::Hrp::parse("ltc").unwrap();
    for version in [bech32::segwit::VERSION_0, bech32::segwit::VERSION_1] {
        let unsupported = bech32::segwit::encode(hrp, version, &[0x55; 32]).unwrap();
        assert!(validate_ltc_sender(Chain::Litecoin, &unsupported, &key).is_err());
    }
    let utxo = input(1, InputKind::P2wpkh, &key);
    let destination = InputKind::P2pkh.script(&public_key(&key));
    let sign = |utxos: &[(String, u32, u64, Vec<u8>)], change: &str, key: &[u8]| {
        sign_ltc_with_output_script(
            Chain::Litecoin,
            utxos,
            destination.as_bytes(),
            50_000,
            1_000,
            change,
            key,
        )
    };
    let wrong_sender = address(Chain::Litecoin, InputKind::P2wpkh, &wrong_key);
    assert!(sign(std::slice::from_ref(&utxo), &sender, &wrong_key).is_err());
    assert!(sign(std::slice::from_ref(&utxo), &wrong_sender, &key).is_err());
    assert!(sign(&[utxo.clone(), utxo.clone()], &sender, &key).is_err());
    assert!(sign(&[], &sender, &key).is_err());
    for script in [
        vec![],
        vec![0x58, 0x20].into_iter().chain([0x33; 32]).collect(),
    ] {
        assert!(
            sign_ltc_with_output_script(
                Chain::Litecoin,
                std::slice::from_ref(&utxo),
                &script,
                50_000,
                1_000,
                &sender,
                &key,
            )
            .is_err()
        );
    }
    for invalid in [
        ("bad".into(), utxo.1, utxo.2, utxo.3.clone()),
        (utxo.0.clone(), utxo.1, 0, utxo.3.clone()),
        ("00".repeat(32), u32::MAX, utxo.2, utxo.3.clone()),
        (utxo.0.clone(), utxo.1, utxo.2, vec![0x00, 0x20, 0x55]),
        (
            utxo.0.clone(),
            utxo.1,
            utxo.2,
            InputKind::P2wpkh
                .script(&public_key(&wrong_key))
                .into_bytes(),
        ),
    ] {
        assert!(sign(&[invalid], &sender, &key).is_err());
    }
}

#[test]
fn dust_policy_and_recipient_boundary_match_litecoin_core() {
    let key = [1; 32];
    let pubkey = public_key(&key);
    let mut taproot = vec![0x51, 0x20];
    taproot.extend(pubkey.0.x_only_public_key().0.serialize());
    let scripts = [
        (ScriptBuf::new_p2pkh(&pubkey.pubkey_hash()), 5_460),
        (
            ScriptBuf::new_p2wpkh(&pubkey.wpubkey_hash()).to_p2sh(),
            5_400,
        ),
        (ScriptBuf::new_p2wpkh(&pubkey.wpubkey_hash()), 2_940),
        (
            ScriptBuf::new_p2wsh(&bitcoin::WScriptHash::from_byte_array([0x44; 32])),
            3_300,
        ),
        (ScriptBuf::from_bytes(taproot), 3_300),
    ];
    for chain in [Chain::Litecoin, Chain::LitecoinTestnet] {
        let sender = address(chain, InputKind::P2wpkh, &key);
        let utxo = input(1, InputKind::P2wpkh, &key);
        for (script, expected) in &scripts {
            assert_eq!(
                litecoin_dust_threshold(chain, script.as_bytes()).unwrap(),
                *expected
            );
            for amount in [expected - 1, *expected, expected + 1] {
                let result = sign_ltc_with_output_script(
                    chain,
                    std::slice::from_ref(&utxo),
                    script.as_bytes(),
                    amount,
                    1_000,
                    &sender,
                    &key,
                );
                assert_eq!(result.is_ok(), amount >= *expected);
            }
        }
        assert!(litecoin_dust_threshold(chain, &[]).is_err());
    }
    assert!(litecoin_dust_threshold(Chain::Bitcoin, scripts[0].0.as_bytes()).is_err());
}

#[test]
fn dust_boundary_and_checked_accounting_return_only_spendable_change() {
    let key = [1; 32];
    let sender = address(Chain::Litecoin, InputKind::P2wpkh, &key);
    let utxo = input(1, InputKind::P2wpkh, &key);
    let destination = InputKind::P2pkh.script(&public_key(&key));
    for (change, output_count) in [(2_939, 1), (2_940, 2), (0, 1)] {
        let raw = sign_ltc_with_output_script(
            Chain::Litecoin,
            std::slice::from_ref(&utxo),
            destination.as_bytes(),
            utxo.2 - 1_000 - change,
            1_000,
            &sender,
            &key,
        )
        .unwrap();
        let tx: Transaction = bitcoin::consensus::deserialize(&raw).unwrap();
        assert_eq!(tx.output.len(), output_count);
        if output_count == 2 {
            assert_eq!(tx.output[1].value.to_sat(), change);
        }
    }
    assert!(
        sign_ltc_with_output_script(
            Chain::Litecoin,
            &[utxo],
            destination.as_bytes(),
            u64::MAX,
            1,
            &sender,
            &key,
        )
        .is_err()
    );
}

#[test]
fn money_range_rejects_invalid_provider_values_before_signing() {
    let key = [1; 32];
    for chain in [Chain::Litecoin, Chain::LitecoinTestnet] {
        let maximum = chain.litecoin_max_money().unwrap();
        assert_eq!(maximum, 84_000_000 * 100_000_000);
        assert_eq!(validate_ltc_values(chain, [maximum]).unwrap(), maximum);
        assert_eq!(
            validate_ltc_values(chain, [maximum - 1, 1]).unwrap(),
            maximum
        );
        for values in [
            vec![0],
            vec![maximum + 1],
            vec![maximum, 1],
            vec![u64::MAX, 1],
        ] {
            assert!(validate_ltc_values(chain, values).is_err());
        }
        let mut utxo = input(1, InputKind::P2wpkh, &key);
        utxo.2 = maximum;
        let sender = address(chain, InputKind::P2wpkh, &key);
        let script = InputKind::P2pkh.script(&public_key(&key));
        let sign = |utxo: &(String, u32, u64, Vec<u8>), amount, fee| {
            sign_ltc_with_output_script(
                chain,
                std::slice::from_ref(utxo),
                script.as_bytes(),
                amount,
                fee,
                &sender,
                &key,
            )
        };
        let raw = sign(&utxo, maximum - 1_000, 1_000).unwrap();
        let tx: Transaction = bitcoin::consensus::deserialize(&raw).unwrap();
        assert_eq!(tx.output.len(), 1);
        verify_input(&tx, 0, InputKind::P2wpkh, &key, maximum);
        assert!(sign(&utxo, 0, 1_000).is_err());
        assert!(sign(&utxo, maximum + 1, 0).is_err());
        assert!(sign(&utxo, maximum, 1).is_err());
        assert!(sign(&utxo, 1, u64::MAX).is_err());
        utxo.2 = maximum + 1;
        assert!(sign(&utxo, 1, 1_000).is_err());
    }
    assert!(validate_ltc_values(Chain::Bitcoin, [1]).is_err());
}

#[test]
fn virtual_size_upper_bound_covers_mixed_inputs_and_compact_size_boundary() {
    let key = public_key(&[1; 32]);
    for count in [1, 2, 252, 253] {
        for kind in [InputKind::P2pkh, InputKind::P2wpkh, InputKind::NestedP2wpkh] {
            let script = kind.script(&key);
            let estimate = estimate_ltc_vsize(
                std::iter::repeat_n(script.as_bytes(), count),
                25,
                Some(script.as_bytes()),
            )
            .unwrap();
            let signature = [0x30; 72];
            let mut tx_input = TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            };
            if kind == InputKind::P2pkh {
                tx_input.script_sig = Builder::new()
                    .push_slice(signature)
                    .push_slice(key.to_bytes())
                    .into_script();
            } else {
                if kind == InputKind::NestedP2wpkh {
                    let redeem =
                        PushBytesBuf::try_from(InputKind::P2wpkh.script(&key).into_bytes())
                            .unwrap();
                    tx_input.script_sig = Builder::new().push_slice(redeem).into_script();
                }
                tx_input.witness =
                    Witness::from_slice(&[signature.to_vec(), key.to_bytes().to_vec()]);
            }
            let tx = Transaction {
                version: Version::ONE,
                lock_time: LockTime::ZERO,
                input: vec![tx_input; count],
                output: vec![
                    TxOut {
                        value: Amount::from_sat(1),
                        script_pubkey: InputKind::P2pkh.script(&key),
                    },
                    TxOut {
                        value: Amount::from_sat(1),
                        script_pubkey: script,
                    },
                ],
            };
            assert_eq!(estimate, tx.vsize() as u64, "{kind:?}: {count}");
        }
    }
    let scripts = [
        InputKind::P2pkh.script(&key),
        InputKind::P2wpkh.script(&key),
    ];
    assert_eq!(
        estimate_ltc_vsize(scripts.iter().map(|s| s.as_bytes()), 25, None).unwrap(),
        261
    );
    assert!(estimate_ltc_vsize([], 25, None).is_err());
    assert!(estimate_ltc_vsize([scripts[0].as_bytes()], usize::MAX, None).is_err());
}
