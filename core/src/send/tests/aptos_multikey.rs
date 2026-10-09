//! Aptos MultiKey accounts against the Aptos TS SDK's (`aptos-multikey.json`,
//! from scripts/generate-aptos-multikey-vectors.cjs).

use super::*;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/aptos-multikey.json")).unwrap()
}

fn policy(fixture: &serde_json::Value) -> AptosMultiKey {
    let keys: Vec<String> = fixture["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|key| {
            format!(
                "{}-pub-0x{}",
                key["scheme"].as_str().unwrap(),
                key["public_key"].as_str().unwrap()
            )
        })
        .collect();
    AptosMultiKey::parse(
        &serde_json::json!({"signaturesRequired": 2, "publicKeys": keys}).to_string(),
    )
    .unwrap()
}

/// The policy's MultiKey, authentication key and members' own addresses are
/// the SDK's; the transfer decodes as the SDK built it and its signing
/// message is the SDK's.
#[test]
fn a_policy_and_transfer_are_the_sdks() {
    let fixture = fixture();
    let policy = policy(&fixture);
    assert_eq!(
        hex::encode(policy.bcs()),
        fixture["multi_key"]["bcs"].as_str().unwrap()
    );
    assert_eq!(
        policy.authentication_key(),
        fixture["multi_key"]["authentication_key"].as_str().unwrap()
    );
    assert_eq!(AptosMultiKey::parse(&policy.canonical()).unwrap(), policy);
    // An Ed25519 member's own account is the legacy one a Spectra Aptos
    // wallet holds; a secp256k1 member's is the SDK's SingleKey account.
    for (key, vector) in policy.keys.iter().zip(fixture["keys"].as_array().unwrap()) {
        assert_eq!(
            hex::encode(key.bcs()),
            vector["any_public_key"].as_str().unwrap()
        );
        match key {
            AptosKey::Ed25519(public) => assert_eq!(
                key.single_address(),
                crate::derivation::aptos::address_from_public_key(public)
            ),
            AptosKey::Secp256k1(_) => {
                assert_eq!(
                    key.single_address(),
                    vector["single_key_address"].as_str().unwrap()
                )
            }
        }
    }
    let raw = hex::decode(fixture["transaction"]["raw"].as_str().unwrap()).unwrap();
    let transfer = decode(&raw).unwrap();
    assert_eq!(transfer.amount, 1_000_000);
    assert_eq!(transfer.sequence, 5);
    assert_eq!(transfer.chain_id, 2);
    assert_eq!(
        hex::encode(transfer.message().unwrap()),
        fixture["transaction"]["signing_message"].as_str().unwrap()
    );
}

/// Each member's signature, the secp256k1 one over the SHA3 of the message
/// included, verifies; an Ed25519 signature made here is the SDK's; and the
/// signed transactions and their hashes are the SDK's.
#[test]
fn signatures_verify_and_assemble_as_the_sdk_does() {
    let fixture = fixture();
    let policy = policy(&fixture);
    let raw = hex::decode(fixture["transaction"]["raw"].as_str().unwrap()).unwrap();
    let transfer = decode(&raw).unwrap();
    let signatures: Vec<MemberSignature> = fixture["signatures"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(member, signature)| {
            member_signature(
                &policy,
                &transfer,
                member,
                &hex::decode(signature["signature"].as_str().unwrap()).unwrap(),
            )
            .unwrap()
        })
        .collect();
    let seed = crate::send::keys::Ed25519Seed::from_hex(
        fixture["keys"][0]["private_key"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(
        sign_ed25519(&transfer, &seed).unwrap(),
        signatures[0].signature
    );
    for set in fixture["signers"].as_array().unwrap() {
        let chosen: Vec<MemberSignature> = set["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|member| signatures[member.as_u64().unwrap() as usize].clone())
            .collect();
        let signed = signed_transaction(&policy, &transfer, &chosen).unwrap();
        assert_eq!(
            hex::encode(&signed),
            set["signed_transaction"].as_str().unwrap(),
            "{}",
            set["name"]
        );
        assert_eq!(
            transaction_hash(&signed),
            set["transaction_hash"].as_str().unwrap()
        );
    }
}

/// Refused: a signature under another member's place or over another
/// transfer, a member twice, fewer signatures than required, a transaction
/// another payload would make.
#[test]
fn foreign_signatures_and_unread_transactions_are_refused() {
    let fixture = fixture();
    let policy = policy(&fixture);
    let raw = hex::decode(fixture["transaction"]["raw"].as_str().unwrap()).unwrap();
    let transfer = decode(&raw).unwrap();
    let signature = hex::decode(fixture["signatures"][0]["signature"].as_str().unwrap()).unwrap();
    assert!(member_signature(&policy, &transfer, 1, &signature).is_err());
    let mut other = transfer.clone();
    other.amount += 1;
    assert!(member_signature(&policy, &other, 0, &signature).is_err());
    let first = member_signature(&policy, &transfer, 0, &signature).unwrap();
    assert!(signed_transaction(&policy, &transfer, &[first.clone(), first.clone()]).is_err());
    assert!(signed_transaction(&policy, &transfer, &[first]).is_err());
    let mut fungible = raw.clone();
    fungible[73] ^= 1;
    assert!(decode(&fungible).is_err());
    assert!(decode(&raw[..raw.len() - 1]).is_err());
}
