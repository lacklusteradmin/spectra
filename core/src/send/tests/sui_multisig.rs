//! Sui multisig accounts against @mysten/sui's (`sui-multisig.json`, from
//! scripts/generate-sui-multisig-vectors.cjs).

use super::*;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/sui-multisig.json")).unwrap()
}

fn policy(fixture: &serde_json::Value, weights: &[u8], threshold: u16) -> SuiMultisig {
    let keys: Vec<serde_json::Value> = fixture["keys"]
        .as_array()
        .unwrap()
        .iter()
        .zip(weights)
        .map(|(key, weight)| serde_json::json!({"publicKey": key["sui_public_key"], "weight": weight}))
        .collect();
    SuiMultisig::parse(&serde_json::json!({"threshold": threshold, "publicKeys": keys}).to_string())
        .unwrap()
}

/// The policy derives the SDK's address and multisig key, reads back from
/// its canonical form, and its member keys hold the SDK's own addresses.
#[test]
fn a_policy_derives_the_sdks_address_and_key() {
    let fixture = fixture();
    let policy = policy(&fixture, &[1, 1, 2], 2);
    assert_eq!(
        policy.address(),
        fixture["multisig"]["address"].as_str().unwrap()
    );
    let mut key = vec![MULTISIG_FLAG];
    key.extend(policy.bcs());
    assert_eq!(
        hex::encode(policy.bcs()),
        fixture["multisig"]["public_key"].as_str().unwrap()
    );
    assert_eq!(SuiMultisig::parse(&policy.canonical()).unwrap(), policy);
    for (member, key) in policy
        .members
        .iter()
        .zip(fixture["keys"].as_array().unwrap())
    {
        assert_eq!(member.address(), key["address"].as_str().unwrap());
    }
    let unreachable = self::policy(&fixture, &[1, 1, 1], 3);
    assert_eq!(
        unreachable.address(),
        fixture["unreachable_policy"]["address"].as_str().unwrap()
    );
    for text in [
        r#"{"threshold": 3, "publicKeys": []}"#,
        r#"{"threshold": 0, "publicKeys": [{"publicKey": "AJALTYHuzqPfL3SxQgDE9M8/Sa+sp6Y0/9LPb/gr2uzy", "weight": 1}]}"#,
        r#"{"threshold": 2, "publicKeys": [{"publicKey": "AJALTYHuzqPfL3SxQgDE9M8/Sa+sp6Y0/9LPb/gr2uzy", "weight": 1}]}"#,
        r#"{"threshold": 1, "publicKeys": [{"publicKey": "AJALTYHuzqPfL3SxQgDE9M8/Sa+sp6Y0/9LPb/gr2uzy", "weight": 0}]}"#,
        r#"{"threshold": 1, "publicKeys": [{"publicKey": "BJALTYHuzqPfL3SxQgDE9M8/Sa+sp6Y0/9LPb/gr2uzy", "weight": 1}]}"#,
    ] {
        assert!(SuiMultisig::parse(text).is_err(), "{text}");
    }
}

/// The transfer decodes as the SDK built it; each member's signature, the
/// secp256k1 and secp256r1 ones included, verifies; an Ed25519 signature
/// made here is the SDK's; and the combined signatures are the SDK's.
#[test]
fn signatures_verify_and_combine_as_the_sdk_does() {
    let fixture = fixture();
    let policy = policy(&fixture, &[1, 1, 2], 2);
    let bytes = hex::decode(fixture["transaction"]["raw"].as_str().unwrap()).unwrap();
    let transfer = decode(&bytes).unwrap();
    assert_eq!(transfer.amount, 1_000_000);
    assert_eq!(
        format!("0x{}", hex::encode(transfer.sender)),
        policy.address()
    );
    assert_eq!(
        hex::encode(intent_digest(&bytes)),
        fixture["transaction"]["intent_message_digest"]
            .as_str()
            .unwrap()
    );
    assert_eq!(
        transaction_digest(&bytes),
        fixture["transaction"]["digest"].as_str().unwrap()
    );
    let signatures: Vec<MemberSignature> = fixture["partial_signatures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|signature| {
            member_signature(&policy, &bytes, signature["signature"].as_str().unwrap()).unwrap()
        })
        .collect();
    assert_eq!(
        signatures.iter().map(|s| s.member).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    let seed = crate::send::keys::Ed25519Seed::from_hex(
        fixture["keys"][0]["secret_key"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(
        sign_ed25519(&bytes, &seed),
        fixture["partial_signatures"][0]["signature"]
            .as_str()
            .unwrap()
    );
    for combined in fixture["combined_signatures"].as_array().unwrap() {
        let chosen: Vec<MemberSignature> = combined["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|member| signatures[member.as_u64().unwrap() as usize].clone())
            .collect();
        assert_eq!(
            hex::encode(combine(&policy, &chosen).unwrap()),
            combined["signature_hex"].as_str().unwrap(),
            "{}",
            combined["name"]
        );
    }
}

/// Refused: a signature over another transaction, a key that is no member,
/// a member twice, a combination short of the threshold, a transaction with
/// anything a transfer does not hold.
#[test]
fn foreign_signatures_and_unread_transactions_are_refused() {
    let fixture = fixture();
    let policy = policy(&fixture, &[1, 1, 2], 2);
    let bytes = hex::decode(fixture["transaction"]["raw"].as_str().unwrap()).unwrap();
    let signature = fixture["partial_signatures"][0]["signature"]
        .as_str()
        .unwrap();
    let mut other = decode(&bytes).unwrap();
    other.amount += 1;
    assert!(member_signature(&policy, &other.bytes(), signature).is_err());
    let stranger = crate::send::keys::Ed25519Seed::from_hex(&"07".repeat(32)).unwrap();
    assert!(member_signature(&policy, &bytes, &sign_ed25519(&bytes, &stranger)).is_err());
    let first = member_signature(&policy, &bytes, signature).unwrap();
    assert!(combine(&policy, &[first.clone(), first.clone()]).is_err());
    assert!(combine(&policy, &[first]).is_err());
    let mut expiring = bytes.clone();
    *expiring.last_mut().unwrap() = 1;
    assert!(decode(&expiring).is_err());
    assert!(decode(&bytes[..bytes.len() - 1]).is_err());
}
