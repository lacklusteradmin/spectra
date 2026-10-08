use super::*;
use crate::send::flow;

/// dcrd's `TestCalcSignatureHash`: three inputs spending `HashH([i])`, two
/// `OP_TRUE` outputs, signed for input 0 against the script `51`.
/// https://github.com/decred/dcrd/blob/master/txscript/sighash_test.go
#[test]
fn signature_hash_matches_dcrd() {
    let inputs: Vec<DcrInputBuild> = (0u8..3)
        .map(|i| DcrInputBuild {
            outpoint_txid: blake256(&[i]).to_vec(),
            vout: u32::from(i),
            tree: TX_TREE_REGULAR,
            sequence: 0xFFFF_FFFF,
            amount: 0,
            script_pubkey: vec![0x51],
        })
        .collect();
    let outputs = vec![(vec![0x51], 0x0000_FF00_FF00_FF00); 2];
    let prefix_hash = blake256(&serialize_prefix(&inputs, &outputs, 0, 0));
    assert_eq!(
        hex::encode(signature_hash(&inputs, &prefix_hash, 0)),
        "4ce2cd042d64e35b36fdbd16aff0d38a5abebff0e5e8f6b6b31fcd4ac6957905"
    );
    assert_ne!(
        signature_hash(&inputs, &prefix_hash, 0),
        signature_hash(&inputs, &prefix_hash, 1)
    );
}

/// The recipients of dcrd's address vectors, with the scripts dcrd says they
/// pay. https://github.com/decred/dcrd/blob/master/txscript/stdaddr/address_test.go
const RECIPIENTS: [(Chain, &str, &str); 4] = [
    (
        Chain::Decred,
        "DsUZxxoHJSty8DCfwfartwTYbuhmVct7tJu",
        "76a9142789d58cfa0957d206f025c2af056fc8a77cebb088ac",
    ),
    (
        Chain::Decred,
        "DcuQKx8BES9wU7C6Q5VmLBjw436r27hayjS",
        "a914f0b4e85100aee1a996f22915eb3c3f764d53779a87",
    ),
    (
        Chain::DecredTestnet,
        "Tso2MVTUeVrjHTBFedFhiyM7yVTbieqp91h",
        "76a914f15da1cb8d1bcb162c6ab446c95757a6e791c91688ac",
    ),
    (
        Chain::DecredTestnet,
        "TccWLgcquqvwrfBocq5mcK5kBiyw8MvyvCi",
        "a91436c1ca10a8a6a4b5d4204ac970853979903aa28487",
    ),
];

struct Reader<'a> {
    raw: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> &'a [u8] {
        self.at += n;
        &self.raw[self.at - n..self.at]
    }

    fn compact(&mut self) -> usize {
        match self.take(1)[0] {
            0xfd => u16::from_le_bytes(self.take(2).try_into().unwrap()).into(),
            0xfe => u32::from_le_bytes(self.take(4).try_into().unwrap()) as usize,
            n => n.into(),
        }
    }
}

/// Read a full (serType 0) Decred transaction back, as dcrd's wire format
/// lays it out: the outputs with their script versions, and each input's
/// signature script.
fn decode(raw: &[u8]) -> (Vec<(u64, u16, Vec<u8>)>, Vec<Vec<u8>>) {
    let mut reader = Reader { raw, at: 0 };
    assert_eq!(reader.take(4), [1, 0, 0, 0], "version 1, serType full");
    let inputs = reader.compact();
    for _ in 0..inputs {
        reader.take(32 + 4 + 1 + 4);
    }
    let outputs = (0..reader.compact())
        .map(|_| {
            let value = u64::from_le_bytes(reader.take(8).try_into().unwrap());
            let version = u16::from_le_bytes(reader.take(2).try_into().unwrap());
            let length = reader.compact();
            (value, version, reader.take(length).to_vec())
        })
        .collect();
    assert_eq!(reader.take(8), [0; 8], "lock time and expiry");
    assert_eq!(reader.compact(), inputs);
    let scripts = (0..inputs)
        .map(|_| {
            reader.take(8 + 4 + 4);
            let length = reader.compact();
            reader.take(length).to_vec()
        })
        .collect();
    assert_eq!(reader.at, raw.len());
    (outputs, scripts)
}

#[test]
fn signed_transaction_pays_the_script_each_address_names() {
    let secp = secp256k1::Secp256k1::new();
    let key = [7u8; 32];
    let secret = secp256k1::SecretKey::from_slice(&key).unwrap();
    let public = secp256k1::PublicKey::from_secret_key(&secp, &secret);
    let hash = crate::derivation::decred::dcr_hash160(&public.serialize());
    for (chain, recipient, script) in RECIPIENTS {
        assert!(flow::is_valid_send_address(chain, recipient.to_string()));
        let sender = crate::derivation::decred::encode_decred_p2pkh(chain, &hash).unwrap();
        let from_script = parse_decred_address(chain, &sender)
            .unwrap()
            .script_pubkey();
        let inputs = vec![DcrInputBuild {
            outpoint_txid: vec![0x11; 32],
            vout: 0,
            tree: TX_TREE_REGULAR,
            sequence: 0xFFFF_FFFF,
            amount: 100_000,
            script_pubkey: from_script.clone(),
        }];
        let outputs = vec![
            (
                parse_decred_address(chain, recipient)
                    .unwrap()
                    .script_pubkey(),
                50_000,
            ),
            (from_script.clone(), 40_000),
        ];
        let raw = sign_dcr_tx(&inputs, &outputs, &[&key]).unwrap();
        let (decoded, scripts) = decode(&raw);
        assert_eq!(
            decoded,
            vec![
                (50_000, 0, hex::decode(script).unwrap()),
                (40_000, 0, from_script.clone()),
            ],
            "{chain}: {recipient}"
        );

        // The one signature script is `<DER signature + SIGHASH_ALL> <pubkey>`
        // over the transaction's own signature hash.
        let sig_script = &scripts[0];
        let der_length = usize::from(sig_script[0]);
        let der = &sig_script[1..der_length];
        assert_eq!(sig_script[der_length], SIGHASH_ALL as u8);
        assert_eq!(&sig_script[der_length + 2..], public.serialize());
        let prefix_hash = blake256(&serialize_prefix(&inputs, &outputs, 0, 0));
        let message = secp256k1::Message::from_digest(signature_hash(&inputs, &prefix_hash, 0));
        secp.verify_ecdsa(
            &message,
            &secp256k1::ecdsa::Signature::from_der(der).unwrap(),
            &public,
        )
        .unwrap();
    }
}

/// A recipient on the other network, or of a form these sends cannot pay, is
/// refused from its address alone, and so is a script-hash sender.
#[test]
fn refuses_a_recipient_before_reading_inputs() {
    for (chain, recipient) in [
        (Chain::Decred, "TccWLgcquqvwrfBocq5mcK5kBiyw8MvyvCi"),
        (Chain::DecredTestnet, "DcuQKx8BES9wU7C6Q5VmLBjw436r27hayjS"),
        // dcrd's Schnorr pubkey-hash vector: a valid address on the network,
        // whose script this send does not build.
        (Chain::Decred, "DSXcZv4oSRiEoWL2a9aD8sgfptRo1YEXNKj"),
    ] {
        assert!(!flow::is_valid_send_address(chain, recipient.to_string()));
        let error = recipient_script(chain, recipient).unwrap_err();
        assert!(matches!(error, SendError::Derivation(_)), "{error:?}");
    }
    // A script hash cannot be the sender: the wallet's key signs P2PKH.
    let error = sender_script(Chain::Decred, "DcuQKx8BES9wU7C6Q5VmLBjw436r27hayjS").unwrap_err();
    assert!(matches!(error, SendError::Derivation(_)), "{error:?}");
    assert!(sender_script(Chain::Decred, "DsUZxxoHJSty8DCfwfartwTYbuhmVct7tJu").is_ok());
}

/// Three inputs on two keys: each signature script carries its own input's
/// key and signs its own input's dcrd signature hash; a key that does not
/// own its input signs nothing.
#[test]
fn each_input_signs_with_the_key_of_the_address_it_pays() {
    let secp = secp256k1::Secp256k1::new();
    let keys = [[5u8; 32], [6u8; 32]];
    let public: Vec<_> = keys
        .iter()
        .map(|key| {
            secp256k1::PublicKey::from_secret_key(
                &secp,
                &secp256k1::SecretKey::from_slice(key).unwrap(),
            )
        })
        .collect();
    let script = |owner: usize| {
        ParsedUtxoAddress::P2pkh(crate::derivation::decred::dcr_hash160(
            &public[owner].serialize(),
        ))
        .script_pubkey()
    };
    let owners = [0usize, 1, 0];
    let utxos: Vec<(String, u32, u64, Vec<u8>)> = owners
        .iter()
        .enumerate()
        .map(|(index, owner)| {
            (
                format!("{index:02x}").repeat(32),
                index as u32,
                100_000,
                script(*owner),
            )
        })
        .collect();
    let outputs = vec![(script(1), 150_000), (script(0), 140_000)];
    let signing: Vec<_> = utxos
        .iter()
        .zip(owners)
        .map(|(utxo, owner)| DecredSigningInput {
            utxo,
            private_key: &keys[owner],
        })
        .collect();
    let raw = sign(&signing, &outputs).unwrap();
    let (_, scripts) = decode(&raw);
    let inputs: Vec<DcrInputBuild> = utxos
        .iter()
        .map(|utxo| DcrInputBuild {
            outpoint_txid: decode_txid_le(&utxo.0).unwrap(),
            vout: utxo.1,
            tree: TX_TREE_REGULAR,
            sequence: 0xFFFF_FFFF,
            amount: utxo.2,
            script_pubkey: utxo.3.clone(),
        })
        .collect();
    let prefix_hash = blake256(&serialize_prefix(&inputs, &outputs, 0, 0));
    for (index, owner) in owners.iter().enumerate() {
        let sig_script = &scripts[index];
        let der_length = usize::from(sig_script[0]);
        assert_eq!(&sig_script[der_length + 2..], public[*owner].serialize());
        secp.verify_ecdsa(
            &secp256k1::Message::from_digest(signature_hash(&inputs, &prefix_hash, index)),
            &secp256k1::ecdsa::Signature::from_der(&sig_script[1..der_length]).unwrap(),
            &public[*owner],
        )
        .unwrap();
    }
    let swapped: Vec<_> = utxos
        .iter()
        .zip(owners)
        .map(|(utxo, owner)| DecredSigningInput {
            utxo,
            private_key: &keys[1 - owner],
        })
        .collect();
    assert!(sign(&swapped, &outputs).is_err());
}
