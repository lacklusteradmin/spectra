use super::*;

#[test]
fn signed_payment_matches_official_xrpl_codec_and_signature() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/xrp-payment.json")).unwrap();
    let tx = &fixture["transaction"];
    let mut key = [0; 32];
    key[31] = 1;
    let signed = build_signed_payment(
        tx["Account"].as_str().unwrap(),
        tx["Destination"].as_str().unwrap(),
        &PaymentAmount::Drops(tx["Amount"].as_str().unwrap().parse().unwrap()),
        None,
        tx["Fee"].as_str().unwrap().parse().unwrap(),
        tx["Sequence"].as_u64().unwrap().try_into().unwrap(),
        &key,
        tx["SigningPubKey"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(signed, fixture["signed_hex"]);
}

#[test]
fn refuses_native_amounts_that_would_corrupt_asset_flags() {
    for drops in [0, 100_000_000_000_000_001, u128::from(u64::MAX), u128::MAX] {
        assert!(validate_drops(drops).is_err(), "{drops}");
    }
    assert!(validate_drops(1).is_ok());
    assert!(validate_drops(100_000_000_000_000_000).is_ok());
}

#[test]
fn signed_account_delete_matches_official_xrpl_codec_and_signature() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/account-closing.json")).unwrap();
    let vector = &fixture["xrp_account_delete"];
    let tx = &vector["transaction"];
    let key = hex::decode(vector["key"].as_str().unwrap()).unwrap();
    let signed = build_signed_account_delete(
        tx["Account"].as_str().unwrap(),
        tx["Destination"].as_str().unwrap(),
        tx["Fee"].as_str().unwrap().parse().unwrap(),
        tx["Sequence"].as_u64().unwrap().try_into().unwrap(),
        &key,
        tx["SigningPubKey"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(signed, vector["signed_hex"]);
    // Into itself is no deletion.
    assert!(
        build_signed_account_delete(
            tx["Account"].as_str().unwrap(),
            tx["Account"].as_str().unwrap(),
            200_000,
            1,
            &key,
            tx["SigningPubKey"].as_str().unwrap(),
        )
        .is_err()
    );
}

/// Issued-currency payments and trust lines as ripple-binary-codec encodes
/// them and ripple-keypairs signs them.
#[test]
fn issued_currency_transactions_match_the_official_codec() {
    use crate::api::xrpl_amount::{IouValue, XrplIssue};
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/issued-assets.json")).unwrap();
    let vectors = &fixture["xrpl"];
    let key = hex::decode(vectors["key"].as_str().unwrap()).unwrap();
    let issued = |amount: &serde_json::Value| IssuedAmount {
        issue: XrplIssue::parse(&format!(
            "{}.{}",
            amount["currency"].as_str().unwrap(),
            amount["issuer"].as_str().unwrap()
        ))
        .unwrap(),
        value: IouValue::parse(amount["value"].as_str().unwrap()).unwrap(),
    };
    for vector in vectors["payments"].as_array().unwrap() {
        let tx = &vector["transaction"];
        let send_max = (!tx["SendMax"].is_null()).then(|| issued(&tx["SendMax"]));
        let signed = build_signed_payment(
            tx["Account"].as_str().unwrap(),
            tx["Destination"].as_str().unwrap(),
            &PaymentAmount::Issued(issued(&tx["Amount"])),
            send_max.as_ref(),
            tx["Fee"].as_str().unwrap().parse().unwrap(),
            tx["Sequence"].as_u64().unwrap().try_into().unwrap(),
            &key,
            tx["SigningPubKey"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(signed, vector["signed_hex"], "{tx}");
    }
    for vector in vectors["trust_sets"].as_array().unwrap() {
        let tx = &vector["transaction"];
        let signed = build_signed_trust_set(
            tx["Account"].as_str().unwrap(),
            &issued(&tx["LimitAmount"]),
            tx["Fee"].as_str().unwrap().parse().unwrap(),
            tx["Sequence"].as_u64().unwrap().try_into().unwrap(),
            &key,
            tx["SigningPubKey"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(signed, vector["signed_hex"], "{tx}");
    }
    // A SendMax below what is delivered, or of another currency, is refused.
    let tx = &vectors["payments"][0]["transaction"];
    let amount = issued(&tx["Amount"]);
    let mut short = issued(&tx["SendMax"]);
    short.value = IouValue::parse("1").unwrap();
    assert!(
        build_signed_payment(
            tx["Account"].as_str().unwrap(),
            tx["Destination"].as_str().unwrap(),
            &PaymentAmount::Issued(amount.clone()),
            Some(&short),
            12,
            7,
            &key,
            tx["SigningPubKey"].as_str().unwrap(),
        )
        .is_err()
    );
    // A trust line to oneself is no trust line.
    let mut own = amount;
    own.issue.issuer = tx["Account"].as_str().unwrap().into();
    assert!(
        build_signed_trust_set(
            tx["Account"].as_str().unwrap(),
            &own,
            12,
            9,
            &key,
            tx["SigningPubKey"].as_str().unwrap()
        )
        .is_err()
    );
}
