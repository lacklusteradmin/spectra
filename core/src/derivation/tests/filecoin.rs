use super::*;

fn vectors() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/filecoin-vectors.json"
    ))
    .unwrap()
}

#[test]
fn a_phrase_derives_the_key_and_address_filecoin_signing_tools_derives() {
    let vectors = vectors();
    let mnemonic = vectors["mnemonic"].as_str().unwrap();
    let profile = Chain::Filecoin.derivation_profiles()[0];
    for row in vectors["accounts"].as_array().unwrap() {
        let path = row["path"].as_str().unwrap();
        let account = row["account"].as_u64().unwrap() as u32;
        assert_eq!(
            Chain::Filecoin
                .derivation_profile_path(profile, account)
                .as_deref(),
            Some(path)
        );
        let derived =
            derive_from_seed_phrase(Chain::Filecoin, mnemonic, path, None, Wants::ALL).unwrap();
        assert_eq!(derived.address.as_deref(), row["address"].as_str());
        assert_eq!(
            derived.public_key_hex.as_deref(),
            row["public_key"].as_str()
        );
        assert_eq!(
            derived.private_key_hex.as_deref().map(String::as_str),
            row["private_key"].as_str()
        );
        // A raw key holds the account its phrase holds.
        let raw = crate::derivation::dispatch::derive_from_private_key(
            Chain::Filecoin,
            row["private_key"].as_str().unwrap().into(),
            true,
            true,
        )
        .unwrap()
        .unwrap();
        assert_eq!(raw.address.as_deref(), row["address"].as_str());
    }
}

#[test]
fn every_protocol_reads_to_the_bytes_glif_reads() {
    for row in vectors()["recipients"].as_array().unwrap() {
        let text = row["address"].as_str().unwrap();
        let address = decode_address(Chain::Filecoin, text).unwrap();
        assert_eq!(
            hex::encode(address.bytes()),
            row["bytes"].as_str().unwrap(),
            "{text}"
        );
        let protocol = match row["protocol"].as_str().unwrap() {
            "id" => FilecoinProtocol::Id,
            "secp256k1" => FilecoinProtocol::Secp256k1,
            "actor" => FilecoinProtocol::Actor,
            "bls" => FilecoinProtocol::Bls,
            _ => FilecoinProtocol::Delegated,
        };
        assert_eq!(address.protocol(), protocol, "{text}");
        assert!(crate::send::flow::is_valid_send_address(
            Chain::Filecoin,
            text.into()
        ));
    }
}

/// `text` with its last character replaced by another of the alphabet.
fn retyped(text: &str) -> String {
    let last = text.chars().last().unwrap();
    format!(
        "{}{}",
        &text[..text.len() - 1],
        if last == 'a' { 'b' } else { 'a' }
    )
}

#[test]
fn a_mistyped_or_foreign_address_is_refused() {
    let vectors = vectors();
    let f1 = vectors["accounts"][0]["address"].as_str().unwrap();
    let f3 = vectors["recipients"][4]["address"].as_str().unwrap();
    let f410 = vectors["recipients"][5]["address"].as_str().unwrap();
    for text in [
        // A typo breaks the checksum.
        retyped(f1),
        retyped(f3),
        retyped(f410),
        // Another network, mixed case, another protocol; a dropped character.
        f1.replacen('f', "t", 1),
        f1.replacen("qode", "QODE", 1),
        f1.replacen("f1", "f5", 1),
        f1[..f1.len() - 1].to_string(),
        // IDs as go-address writes them only: no leading zero, nothing past
        // a signed 64-bit number, digits alone.
        "f00123".into(),
        "f09223372036854775808".into(),
        "f0".into(),
        "f0-1".into(),
        // A delegated address without its namespace's `f`.
        "f410".into(),
        // An Ethereum address is written as its f410f address.
        "0x8ba1f109551bd432803012645ac136ddd64dba72".into(),
    ] {
        assert!(decode_address(Chain::Filecoin, &text).is_err(), "{text}");
        assert!(
            !crate::send::flow::is_valid_send_address(Chain::Filecoin, text.clone()),
            "{text}"
        );
    }
    assert!(decode_address(Chain::Filecoin, "f00").is_ok());
    // An address is lowercase; a QR code's all-uppercase spelling is the
    // same address, which a destination folds to before it is read.
    let upper = f1.to_ascii_uppercase();
    assert!(decode_address(Chain::Filecoin, &upper).is_err());
    assert_eq!(
        crate::send::flow::normalize_address(Chain::Filecoin, &upper),
        f1
    );
}

/// A well-formed address of `protocol` with `payload`, checksummed.
fn address(protocol: u8, namespace: Option<u64>, payload: &[u8]) -> String {
    let mut bytes = vec![protocol];
    if let Some(namespace) = namespace {
        crate::send::leb128::write(namespace, &mut bytes);
    }
    bytes.extend_from_slice(payload);
    let mut body = payload.to_vec();
    body.extend(checksum(&bytes));
    match namespace {
        Some(namespace) => format!("f{protocol}{namespace}f{}", BASE32.encode(&body)),
        None => format!("f{protocol}{}", BASE32.encode(&body)),
    }
}

#[test]
fn an_address_no_key_can_hold_is_refused_though_well_formed() {
    // The helper writes what go-address writes.
    let f3 = vectors()["recipients"][4]["address"]
        .as_str()
        .unwrap()
        .to_string();
    let key = decode_address(Chain::Filecoin, &f3).unwrap();
    assert_eq!(address(3, None, &key.bytes()[1..]), f3);
    let ethereum = [0x8b; 20];
    assert!(decode_address(Chain::Filecoin, &address(4, Some(10), &ethereum)).is_ok());

    // A BLS payload off the curve, and the point at infinity.
    let mut off_curve = [0u8; 48];
    off_curve[0] = 0x80;
    off_curve[47] = 1;
    let mut infinity = [0u8; 48];
    infinity[0] = 0xc0;
    let mut masked_id = [0u8; 20];
    masked_id[0] = 0xff;
    masked_id[19] = 7;
    for text in [
        address(3, None, &off_curve),
        address(3, None, &infinity),
        // Another namespace than the Ethereum Address Manager's, another
        // length than an Ethereum address, an ID in Ethereum's spelling.
        address(4, Some(32), &ethereum),
        address(4, Some(10), &[0x8b; 32]),
        address(4, Some(10), &masked_id),
    ] {
        assert!(decode_address(Chain::Filecoin, &text).is_err(), "{text}");
    }
}
