use super::*;
use crate::send::bitcoin_wire::{build_input, decode_txid_le, dsha256, varint};
use bitcoin::secp256k1::ecdsa::Signature;

fn public(key: &[u8; 32]) -> CompressedPublicKey {
    let secret = SecretKey::from_slice(key).unwrap();
    CompressedPublicKey(bitcoin::secp256k1::PublicKey::from_secret_key(
        &Secp256k1::new(),
        &secret,
    ))
}

fn source(chain: Chain, key: &[u8; 32], kind: InputKind) -> (String, Vec<u8>) {
    let public = public(key);
    let (p2pkh, p2sh) = chain.fixed_utxo_address_versions().unwrap();
    let base58 = |version, hash: &[u8]| {
        let mut raw = vec![version];
        raw.extend(hash);
        bs58::encode(raw).with_check().into_string()
    };
    let (address, script) = match kind {
        InputKind::P2pkh | InputKind::P2pk => (
            base58(p2pkh, &public.pubkey_hash().to_byte_array()),
            ScriptBuf::new_p2pkh(&public.pubkey_hash()),
        ),
        InputKind::NestedP2wpkh => {
            let redeem = ScriptBuf::new_p2wpkh(&public.wpubkey_hash());
            (
                base58(p2sh[0], &redeem.script_hash().to_byte_array()),
                redeem.to_p2sh(),
            )
        }
        InputKind::P2wpkh => {
            let hrp = bech32::Hrp::parse(chain.fixed_utxo_segwit_hrp().unwrap()).unwrap();
            let address = bech32::segwit::encode(
                hrp,
                bech32::segwit::VERSION_0,
                &public.wpubkey_hash().to_byte_array(),
            )
            .unwrap();
            (address, ScriptBuf::new_p2wpkh(&public.wpubkey_hash()))
        }
        InputKind::P2tr => {
            let script =
                ScriptBuf::new_p2tr(&Secp256k1::new(), public.0.x_only_public_key().0, None);
            let hrp = bech32::Hrp::parse(chain.fixed_utxo_segwit_hrp().unwrap()).unwrap();
            let address =
                bech32::segwit::encode(hrp, bech32::segwit::VERSION_1, &script.as_bytes()[2..])
                    .unwrap();
            (address, script)
        }
    };
    (address, script.into_bytes())
}

fn input(index: u32, value: u64, script: &[u8]) -> Input {
    (format!("{index:064x}"), index, value, script.to_vec())
}

fn encode_outputs(tx: &Transaction) -> Vec<u8> {
    let mut encoded = Vec::new();
    for output in &tx.output {
        encoded.extend(output.value.to_sat().to_le_bytes());
        encoded.extend(varint(output.script_pubkey.len()));
        encoded.extend(output.script_pubkey.as_bytes());
    }
    encoded
}

/// Independent protocol preimage. Version 3 has no timestamp after nVersion;
/// the witness branch commits to every atomic prevout value using BIP143.
fn protocol_sighash(
    tx: &Transaction,
    inputs: &[Input],
    index: usize,
    kind: InputKind,
    key: &CompressedPublicKey,
) -> [u8; 32] {
    let mut raw = 3u32.to_le_bytes().to_vec();
    if kind == InputKind::P2tr {
        // Peercoin's current key-path default signature follows BIP341.
        let mut message = vec![0, 0]; // epoch, SIGHASH_DEFAULT
        message.extend(raw);
        message.extend(0u32.to_le_bytes());
        let mut outpoints = Vec::new();
        let mut amounts = Vec::new();
        let mut scripts = Vec::new();
        let mut sequences = Vec::new();
        for (txid, vout, amount, script) in inputs {
            outpoints.extend(decode_txid_le(txid).unwrap());
            outpoints.extend(vout.to_le_bytes());
            amounts.extend(amount.to_le_bytes());
            scripts.extend(varint(script.len()));
            scripts.extend(script);
            sequences.extend(u32::MAX.to_le_bytes());
        }
        for data in [outpoints, amounts, scripts, sequences, encode_outputs(tx)] {
            message.extend(bitcoin::hashes::sha256::Hash::hash(&data).to_byte_array());
        }
        message.push(0); // key path, no annex
        message.extend((index as u32).to_le_bytes());
        let tag = bitcoin::hashes::sha256::Hash::hash(b"TapSighash").to_byte_array();
        let tagged: Vec<_> = tag.into_iter().chain(tag).chain(message).collect();
        return bitcoin::hashes::sha256::Hash::hash(&tagged).to_byte_array();
    }
    if kind.has_witness() {
        let mut prevouts = Vec::new();
        let mut sequences = Vec::new();
        for (txid, vout, _, _) in inputs {
            prevouts.extend(decode_txid_le(txid).unwrap());
            prevouts.extend(vout.to_le_bytes());
            sequences.extend(u32::MAX.to_le_bytes());
        }
        raw.extend(dsha256(&prevouts));
        raw.extend(dsha256(&sequences));
        raw.extend(decode_txid_le(&inputs[index].0).unwrap());
        raw.extend(inputs[index].1.to_le_bytes());
        let script_code = ScriptBuf::new_p2pkh(&key.pubkey_hash());
        raw.extend(varint(script_code.len()));
        raw.extend(script_code.as_bytes());
        raw.extend(inputs[index].2.to_le_bytes());
        raw.extend(u32::MAX.to_le_bytes());
        raw.extend(dsha256(&encode_outputs(tx)));
    } else {
        raw.extend(varint(inputs.len()));
        for (i, (txid, vout, _, script)) in inputs.iter().enumerate() {
            raw.extend(
                build_input(txid, *vout, if i == index { script } else { &[] }, u32::MAX).unwrap(),
            );
        }
        raw.extend(varint(tx.output.len()));
        raw.extend(encode_outputs(tx));
    }
    raw.extend(0u32.to_le_bytes());
    raw.extend(1u32.to_le_bytes());
    dsha256(&raw)
}

#[test]
fn version_three_multikey_inputs_verify_against_peercoin_preimages_on_both_networks() {
    for chain in [Chain::Peercoin, Chain::PeercoinTestnet] {
        let keys = [[1u8; 32], [2u8; 32], [3u8; 32], [4u8; 32], [5u8; 32]];
        let kinds = [
            InputKind::P2pkh,
            InputKind::P2pk,
            InputKind::P2wpkh,
            InputKind::NestedP2wpkh,
            InputKind::P2tr,
        ];
        let (sender, sender_script) = source(chain, &keys[0], InputKind::P2pkh);
        let inputs: Vec<_> = keys
            .iter()
            .zip(kinds)
            .enumerate()
            .map(|(index, (key, kind))| {
                let script = if kind == InputKind::P2pk {
                    ScriptBuf::new_p2pk(&bitcoin::PublicKey::new_uncompressed(public(key).0))
                        .into_bytes()
                } else {
                    source(chain, key, kind).1
                };
                input(index as u32 + 1, 1_000_000, &script)
            })
            .collect();
        let to = ScriptBuf::new_p2wsh(&bitcoin::WScriptHash::from_byte_array([8; 32]));
        let quote = quote_peercoin_fee(
            chain,
            &inputs,
            1_000_000,
            to.as_bytes(),
            &sender_script,
            None,
        )
        .unwrap();
        let signing: Vec<_> = inputs
            .iter()
            .zip(&keys)
            .map(|(utxo, private_key)| PeercoinSigningInput { utxo, private_key })
            .collect();
        let raw = sign_peercoin_inputs_with_output_script(
            chain,
            &signing,
            to.as_bytes(),
            1_000_000,
            quote.fee,
            &sender,
            &keys[0],
        )
        .unwrap();
        let tx: Transaction = bitcoin::consensus::deserialize(&raw).unwrap();
        assert_eq!(tx.version, Version(3));
        assert_eq!(
            &raw[..6],
            &[3, 0, 0, 0, 0, 1],
            "timestamp omitted before witness marker"
        );
        assert_eq!(tx.output[0].script_pubkey, to);
        assert_eq!(tx.output[1].value.to_sat(), quote.change);
        assert!(raw.len() as u64 <= quote.estimated_bytes);
        assert!(quote.fee >= (raw.len() as u64 * 10).max(1_000));
        for i in 0..inputs.len() {
            let signature = if kinds[i].has_witness() {
                tx.input[i].witness.iter().next().unwrap().to_vec()
            } else {
                let bytes = tx.input[i].script_sig.as_bytes();
                bytes[1..1 + usize::from(bytes[0])].to_vec()
            };
            let key = public(&keys[i]);
            let digest = protocol_sighash(&tx, &inputs, i, kinds[i], &key);
            if kinds[i] == InputKind::P2tr {
                assert_eq!(signature.len(), 64);
                let signature =
                    bitcoin::secp256k1::schnorr::Signature::from_slice(&signature).unwrap();
                let output_key =
                    bitcoin::secp256k1::XOnlyPublicKey::from_slice(&inputs[i].3[2..]).unwrap();
                Secp256k1::new()
                    .verify_schnorr(&signature, &Message::from_digest(digest), &output_key)
                    .unwrap();
                let mut wrong_other_prevout = inputs.clone();
                wrong_other_prevout[0].2 += 1;
                let incorrect = protocol_sighash(&tx, &wrong_other_prevout, i, kinds[i], &key);
                assert!(
                    Secp256k1::new()
                        .verify_schnorr(&signature, &Message::from_digest(incorrect), &output_key)
                        .is_err()
                );
                continue;
            }
            assert_eq!(signature.last(), Some(&1));
            let signature = Signature::from_der(&signature[..signature.len() - 1]).unwrap();
            Secp256k1::new()
                .verify_ecdsa(&Message::from_digest(digest), &signature, &key.0)
                .unwrap();
            if kinds[i].has_witness() {
                let mut wrong_amount = inputs.clone();
                wrong_amount[i].2 += 1;
                let incorrect = protocol_sighash(&tx, &wrong_amount, i, kinds[i], &key);
                assert!(
                    Secp256k1::new()
                        .verify_ecdsa(&Message::from_digest(incorrect), &signature, &key.0)
                        .is_err()
                );
            }
        }
    }
}

#[test]
fn quotes_keep_minimum_change_absorb_smaller_residuals_and_allow_maximum_spend() {
    let chain = Chain::Peercoin;
    let (sender, script) = source(chain, &[1; 32], InputKind::P2pkh);
    let inputs = vec![input(1, 1_000_000, &script)];
    assert_eq!(peercoin_minimum_fee(chain, 99).unwrap(), 1_000);
    assert_eq!(peercoin_minimum_fee(chain, 100).unwrap(), 1_000);
    assert_eq!(peercoin_minimum_fee(chain, 226).unwrap(), 2_260);
    let amount = 1_000_000 - 2_260 - 10_000;
    let quote = quote_peercoin_fee(chain, &inputs, amount, &script, &script, None).unwrap();
    assert_eq!(
        (
            quote.fee,
            quote.change,
            quote.estimated_bytes,
            quote.max_sendable
        ),
        (2_260, 10_000, 226, 998_080)
    );
    let absorbed = quote_peercoin_fee(chain, &inputs, amount + 1, &script, &script, None).unwrap();
    assert_eq!(
        (absorbed.fee, absorbed.change, absorbed.estimated_bytes),
        (12_259, 0, 192)
    );
    let maximum =
        quote_peercoin_fee(chain, &inputs, quote.max_sendable, &script, &script, None).unwrap();
    assert_eq!((maximum.fee, maximum.change), (1_920, 0));
    for (amount, quote) in [(amount, quote), (amount + 1, absorbed), (998_080, maximum)] {
        let raw = sign_peercoin_tx(
            chain, &inputs, &sender, amount, quote.fee, &sender, &[1; 32],
        )
        .unwrap();
        let tx: Transaction = bitcoin::consensus::deserialize(&raw).unwrap();
        assert_eq!(
            tx.output.iter().map(|o| o.value.to_sat()).sum::<u64>() + quote.fee,
            1_000_000
        );
    }
    assert!(quote_peercoin_fee(chain, &inputs, 9_999, &script, &script, None).is_err());
    assert!(quote_peercoin_fee(chain, &inputs, 500_000, &script, &script, Some(2_259)).is_err());
    assert!(quote_peercoin_fee(chain, &inputs, 998_081, &script, &script, None).is_err());
}

#[test]
fn witness_fees_charge_raw_bytes_and_compactsize_boundaries_are_counted() {
    let chain = Chain::Peercoin;
    let (_, p2pkh) = source(chain, &[1; 32], InputKind::P2pkh);
    for (kind, expected) in [
        (InputKind::P2wpkh, 229),
        (InputKind::NestedP2wpkh, 252),
        (InputKind::P2tr, 187),
    ] {
        let (_, script) = source(chain, &[1; 32], kind);
        let inputs = vec![input(1, 1_000_000, &script)];
        assert_eq!(
            estimate_bytes(&inputs, &p2pkh, Some(&p2pkh)).unwrap(),
            expected
        );
        let quote = quote_peercoin_fee(chain, &inputs, 500_000, &p2pkh, &p2pkh, None).unwrap();
        assert_eq!(
            quote.fee,
            expected * 10,
            "full witness bytes carry the consensus fee"
        );
    }
    let inputs: Vec<_> = (1..=253).map(|i| input(i, 1_000_000, &p2pkh)).collect();
    assert_eq!(
        estimate_bytes(&inputs[..252], &p2pkh, None).unwrap(),
        10 + 252 * 148 + 34
    );
    assert_eq!(
        estimate_bytes(&inputs, &p2pkh, None).unwrap(),
        12 + 253 * 148 + 34
    );
}

#[test]
fn selection_uses_only_needed_largest_inputs_and_accepts_large_wallet_totals() {
    let chain = Chain::Peercoin;
    let (_, script) = source(chain, &[1; 32], InputKind::P2pkh);
    let inputs = vec![
        input(3, 1_000_000, &script),
        input(2, 3_000_000, &script),
        input(1, 2_000_000, &script),
    ];
    let small = select_peercoin_inputs(chain, &inputs, 500_000, &script, &script, None).unwrap();
    assert_eq!(small.indices, vec![1]);
    assert_eq!(small.spendable_balance, 6_000_000);
    assert_eq!(small.quote.max_sendable, 5_995_120);
    let larger = select_peercoin_inputs(chain, &inputs, 4_000_000, &script, &script, None).unwrap();
    assert_eq!(larger.indices, vec![1, 2]);
    let mut reordered = inputs.clone();
    reordered.reverse();
    let stable =
        select_peercoin_inputs(chain, &reordered, 4_000_000, &script, &script, None).unwrap();
    assert_eq!(
        larger
            .indices
            .iter()
            .map(|i| &inputs[*i])
            .collect::<Vec<_>>(),
        stable
            .indices
            .iter()
            .map(|i| &reordered[*i])
            .collect::<Vec<_>>()
    );

    let maximum = chain.peercoin_max_money().unwrap();
    let rich = vec![input(1, maximum, &script), input(2, 1_000_000, &script)];
    let selected = select_peercoin_inputs(chain, &rich, 500_000, &script, &script, None).unwrap();
    assert_eq!(selected.indices, vec![0]);
    assert_eq!(selected.spendable_balance, maximum + 1_000_000);
    assert_eq!(selected.quote.max_sendable, maximum - 1_920);
}

#[test]
fn selection_capacity_respects_transaction_size_and_amortizes_small_inputs() {
    let chain = Chain::Peercoin;
    let (_, script) = source(chain, &[1; 32], InputKind::P2pkh);
    let inputs: Vec<_> = (1..=800).map(|i| input(i, 1_000_000, &script)).collect();
    let selected = select_peercoin_inputs(chain, &inputs, 500_000, &script, &script, None).unwrap();
    assert_eq!(selected.indices.len(), 1);
    let maximum = select_peercoin_inputs(
        chain,
        &inputs,
        selected.quote.max_sendable,
        &script,
        &script,
        None,
    )
    .unwrap();
    assert!(maximum.indices.len() < inputs.len());
    assert!(maximum.quote.estimated_bytes <= 100_000);
    assert_eq!(maximum.quote.change, 0);

    // A single small output cannot cover the fee floor. Together these
    // outputs pay their incremental serialized bytes and a valid recipient.
    let small: Vec<_> = (1..=600).map(|i| input(i, 1_500, &script)).collect();
    let selected = select_peercoin_inputs(chain, &small, 10_000, &script, &script, None).unwrap();
    assert!(selected.indices.len() > 500);
    assert_eq!(selected.spendable_balance, 900_000);
    assert!(selected.quote.max_sendable >= 10_000);
}

#[test]
fn refuses_wrong_network_wrong_keys_bad_inputs_and_out_of_range_amounts() {
    let chain = Chain::Peercoin;
    let (sender, script) = source(chain, &[1; 32], InputKind::P2pkh);
    let (other_network, _) = source(Chain::PeercoinTestnet, &[1; 32], InputKind::P2pkh);
    let inputs = vec![input(1, 1_000_000, &script)];
    assert!(sign_peercoin_tx(chain, &inputs, &sender, 500_000, 2_260, &sender, &[2; 32]).is_err());
    assert!(
        sign_peercoin_tx(
            chain,
            &inputs,
            &other_network,
            500_000,
            2_260,
            &sender,
            &[1; 32]
        )
        .is_err()
    );
    assert!(
        sign_peercoin_tx(
            chain,
            &inputs,
            &sender,
            500_000,
            2_260,
            &other_network,
            &[1; 32]
        )
        .is_err()
    );
    for bad in [
        vec![],
        vec![inputs[0].clone(), inputs[0].clone()],
        vec![("not hex".into(), 0, 1_000_000, script.clone())],
        vec![("0".repeat(64), u32::MAX, 1_000_000, script.clone())],
        vec![input(1, 0, &script)],
        vec![input(1, u64::MAX, &script)],
        vec![
            input(1, chain.peercoin_max_money().unwrap(), &script),
            input(2, 1, &script),
        ],
    ] {
        assert!(quote_peercoin_fee(chain, &bad, 500_000, &script, &script, None).is_err());
    }
    let p2pk = ScriptBuf::new_p2pk(&bitcoin::PublicKey::new(public(&[1; 32]).0));
    assert!(peercoin_input_matches_sender(p2pk.as_bytes(), &script));
    assert!(!peercoin_input_matches_sender(
        p2pk.as_bytes(),
        &source(chain, &[2; 32], InputKind::P2pkh).1
    ));
    let (_, native) = source(chain, &[1; 32], InputKind::P2wpkh);
    assert!(peercoin_input_matches_sender(&native, &native));
}
