use super::*;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/vechain-vectors.json")).unwrap()
}

fn text<'a>(vector: &'a serde_json::Value, field: &str) -> &'a str {
    vector[field].as_str().unwrap()
}

fn bytes8(text: &str) -> [u8; 8] {
    hex::decode(text.trim_start_matches("0x"))
        .unwrap()
        .try_into()
        .map(|short: [u8; 8]| short)
        .unwrap_or_else(|long: Vec<u8>| panic!("{long:?} is not 8 bytes"))
}

/// A vector's transfer, prepared from its terms: VET to its recipient, or
/// its token's `transfer`.
fn prepared(vector: &serde_json::Value, sender: &str) -> PreparedVechainTransfer {
    let nonce: u64 = text(vector, "nonce").parse().unwrap();
    // A block reference shorter than 8 bytes is a small number, which RLP
    // writes without its leading zero bytes.
    let block_ref = format!(
        "{:0>16}",
        text(vector, "block_ref").trim_start_matches("0x")
    );
    prepare_transfer(
        Chain::Vechain,
        VechainTransferTerms {
            sender,
            recipient: text(vector, "recipient"),
            token: vector["token"].as_str(),
            amount: text(vector, "amount").parse().unwrap(),
            block_ref: bytes8(&block_ref),
            expiration: vector["expiration"].as_u64().unwrap() as u32,
            max_priority_fee_per_gas: text(vector, "max_priority_fee_per_gas").parse().unwrap(),
            max_fee_per_gas: text(vector, "max_fee_per_gas").parse().unwrap(),
            gas: vector["gas"].as_u64().unwrap(),
            nonce: nonce.to_be_bytes(),
        },
    )
    .unwrap()
}

fn key(vector: &serde_json::Value) -> SecretKey {
    SecretKey::from_slice(&hex::decode(text(vector, "private_key")).unwrap()).unwrap()
}

#[test]
fn transfers_sign_to_the_bytes_and_id_the_vechain_sdk_signs() {
    let fixture = fixture();
    assert_eq!(
        fixture["chain_tag"].as_u64(),
        Some(u64::from(Chain::Vechain.thor().unwrap().chain_tag()))
    );
    for vector in fixture["transfers"].as_array().unwrap() {
        let name = text(vector, "name");
        let transfer = prepared(vector, text(vector, "sender"));
        let (to, value, data) = transfer.clause().unwrap();
        assert_eq!(to, text(vector, "to"), "{name}");
        assert_eq!(value.to_string(), text(vector, "value"), "{name}");
        assert_eq!(
            format!("0x{}", hex::encode(&data)),
            text(vector, "data"),
            "{name}"
        );
        assert_eq!(
            intrinsic_gas(&data).to_string(),
            text(vector, "intrinsic_gas"),
            "{name}"
        );
        assert_eq!(
            hex::encode(transfer.unsigned().unwrap()),
            text(vector, "unsigned"),
            "{name}"
        );
        assert_eq!(
            hex::encode(transfer.signing_hash().unwrap()),
            text(vector, "signing_hash"),
            "{name}"
        );
        let (raw, id) = transfer.sign(Chain::Vechain, &key(vector)).unwrap();
        assert_eq!(hex::encode(&raw), text(vector, "raw"), "{name}");
        assert!(
            raw.ends_with(&hex::decode(text(vector, "signature")).unwrap()),
            "{name}"
        );
        assert_eq!(id, text(vector, "id"), "{name}");
    }
}

/// Two transactions as a mainnet node served them, rebuilt from their
/// fields and their own signatures: the same bytes, a signature that
/// recovers their origin over the same signing hash, and the same id.
#[test]
fn mainnet_transactions_are_rebuilt_byte_for_byte() {
    use secp256k1::ecdsa::{RecoverableSignature, RecoveryId};
    for vector in fixture()["mainnet"].as_array().unwrap() {
        let origin = text(vector, "origin");
        let transfer = prepared(vector, origin);
        assert_eq!(
            u64::from(transfer.chain_tag),
            vector["chain_tag"].as_u64().unwrap()
        );
        let signature: [u8; 65] = hex::decode(text(vector, "signature"))
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(
            hex::encode(transfer.encode(Some(&signature)).unwrap()),
            text(vector, "raw"),
            "{origin}"
        );
        let hash = transfer.signing_hash().unwrap();
        assert_eq!(hex::encode(hash), text(vector, "signing_hash"));
        let recovered = Secp256k1::new()
            .recover_ecdsa(
                &Message::from_digest(hash),
                &RecoverableSignature::from_compact(
                    &signature[..64],
                    RecoveryId::from_i32(i32::from(signature[64])).unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
        let address = crate::derivation::evm::keccak256(&recovered.serialize_uncompressed()[1..]);
        assert_eq!(format!("0x{}", hex::encode(&address[12..])), origin);
        let id = crate::derivation::primitives::blake2b::<32>(&[&hash, &address[12..]]);
        assert_eq!(format!("0x{}", hex::encode(id)), text(vector, "id"));
    }
}

#[test]
fn a_key_for_another_account_or_network_is_refused() {
    let fixture = fixture();
    let vector = &fixture["transfers"][0];
    let transfer = prepared(vector, text(vector, "sender"));
    let other = SecretKey::from_slice(&[7; 32]).unwrap();
    assert!(transfer.sign(Chain::Vechain, &other).is_err());
    assert!(transfer.sign(Chain::Ethereum, &key(vector)).is_err());
    let mut moved = transfer.clone();
    moved.chain_tag = 0x27;
    assert!(moved.sign(Chain::Vechain, &key(vector)).is_err());
}

#[test]
fn a_transfer_the_network_would_refuse_is_not_built() {
    let fixture = fixture();
    let vector = &fixture["transfers"][0];
    let sender = text(vector, "sender");
    let recipient = text(vector, "recipient");
    let terms = |recipient, token, amount, gas, max_fee, expiration| VechainTransferTerms {
        sender,
        recipient,
        token,
        amount,
        block_ref: [1; 8],
        expiration,
        max_priority_fee_per_gas: 10,
        max_fee_per_gas: max_fee,
        gas,
        nonce: [0; 8],
    };
    // Addresses with one checksum letter's case flipped.
    let mistyped = text(&fixture["accounts"][1], "address").replacen('D', "d", 1);
    let mistyped_token = text(&fixture["accounts"][2], "address").replacen('E', "e", 1);
    let energy = Chain::Vechain.thor().unwrap().energy;
    for (recipient, token, amount, gas, max_fee, expiration) in [
        (mistyped.as_str(), None, 1, 21_000, 10, 60),
        ("0x1234", None, 1, 21_000, 10, 60),
        (recipient, Some(mistyped_token.as_str()), 1, 60_000, 10, 60),
        (recipient, None, 0, 21_000, 10, 60),
        (recipient, None, 1, 20_999, 10, 60),
        (recipient, Some(energy), 1, 21_000, 10, 60),
        (recipient, None, 1, MAX_TX_GAS + 1, 10, 60),
        (recipient, None, 1, 21_000, 9, 60),
        (recipient, None, 1, 21_000, 10, 0),
    ] {
        assert!(
            prepare_transfer(
                Chain::Vechain,
                terms(recipient, token, amount, gas, max_fee, expiration)
            )
            .is_err(),
            "{recipient} {token:?} {amount} {gas} {max_fee} {expiration}"
        );
    }
    // The same recipient in one case, and VTHO at its checksummed spelling.
    let upper = format!("0x{}", recipient[2..].to_ascii_uppercase());
    let built = prepare_transfer(Chain::Vechain, terms(&upper, None, 1, 21_000, 10, 60)).unwrap();
    assert_eq!(built.recipient, recipient.to_ascii_lowercase());
    let token = prepare_transfer(
        Chain::Vechain,
        terms(
            recipient,
            Some("0x0000000000000000000000000000456E65726779"),
            1,
            60_000,
            10,
            60,
        ),
    )
    .unwrap();
    assert_eq!(token.token.as_deref(), Some(energy));
    assert!(prepare_transfer(Chain::Ethereum, terms(recipient, None, 1, 21_000, 10, 60)).is_err());
}

#[test]
fn a_transfer_lives_until_its_reference_block_plus_its_expiration() {
    let fixture = fixture();
    let vector = &fixture["mainnet"][0];
    let transfer = prepared(vector, text(vector, "origin"));
    // `0x019457ba…` is block 26,499,002; 32 blocks after it.
    assert_eq!(transfer.last_block(), 26_499_002 + 32);
}
