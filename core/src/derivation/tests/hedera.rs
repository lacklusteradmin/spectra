use super::*;

fn vectors() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/hedera-vectors.json")).unwrap()
}

#[test]
fn every_profile_derives_the_key_the_hedera_sdk_derives() {
    let vectors = vectors();
    let mnemonic = vectors["mnemonic"].as_str().unwrap();
    let chain = Chain::Hedera;
    for row in vectors["keys"].as_array().unwrap() {
        let profile: DerivationProfile = serde_json::from_value(row["profile"].clone()).unwrap();
        let index = row["index"].as_u64().unwrap() as u32;
        let path = row["path"].as_str().unwrap();
        assert_eq!(
            chain.derivation_profile_path(profile, index).as_deref(),
            Some(path),
            "the catalog's {profile:?} path"
        );
        let passphrase = row["passphrase"].as_str().filter(|p| !p.is_empty());
        let derived =
            derive_from_seed_phrase(chain, mnemonic, path, passphrase, Wants::ALL).unwrap();
        assert_eq!(
            derived.address.as_deref(),
            row["public_key"].as_str(),
            "{path}"
        );
        assert_eq!(
            derived.public_key_hex.as_deref(),
            row["public_key"].as_str()
        );
        assert_eq!(
            derived.private_key_hex.as_deref().map(String::as_str),
            row["private_key"].as_str(),
            "{path}"
        );
        // The stored key derives the same public key.
        let stored =
            derive_from_private_key(row["private_key"].as_str().unwrap(), Wants::ALL).unwrap();
        assert_eq!(stored.public_key_hex, derived.public_key_hex);
    }
}

#[test]
fn a_path_no_profile_names_is_refused_rather_than_guessed() {
    let mnemonic = vectors()["mnemonic"].as_str().unwrap().to_string();
    for path in [
        "m/44'/3030'/0'",
        "m/44'/3030'/0'/0'/0",
        "m/44'/60'/0'/0'/0'",
        "m/84'/0'/0'/0/0",
    ] {
        assert!(
            derive_from_seed_phrase(Chain::Hedera, &mnemonic, path, None, Wants::ALL).is_err(),
            "{path}"
        );
    }
}

#[test]
fn a_bare_key_is_both_schemes_until_the_network_says_which() {
    let raw = &vectors()["raw_key"];
    let candidates = import_key_candidates(raw["hex"].as_str().unwrap()).unwrap();
    let publics: Vec<String> = candidates
        .iter()
        .map(|key| key.public_key().der_hex())
        .collect();
    assert_eq!(
        publics,
        [
            raw["ed25519"]["public_key"].as_str().unwrap(),
            raw["ecdsa"]["public_key"].as_str().unwrap()
        ]
    );
    let ders: Vec<String> = candidates
        .iter()
        .map(|key| key.der_hex().to_string())
        .collect();
    assert_eq!(
        ders,
        [
            raw["ed25519"]["private_key"].as_str().unwrap(),
            raw["ecdsa"]["private_key"].as_str().unwrap()
        ]
    );
    // A key in DER names its one scheme; bare hex is no stored key.
    for scheme in ["ed25519", "ecdsa"] {
        let der = raw[scheme]["private_key"].as_str().unwrap();
        assert_eq!(import_key_candidates(der).unwrap().len(), 1);
        assert_eq!(
            derive_from_private_key(der, Wants::ALL)
                .unwrap()
                .address
                .as_deref(),
            raw[scheme]["public_key"].as_str()
        );
    }
    assert!(stored_key(raw["hex"].as_str().unwrap()).is_err());
    // The order of the group is no secp256k1 key: only its Ed25519 reading.
    let order = "fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141";
    assert_eq!(import_key_candidates(order).unwrap().len(), 1);
    assert!(import_key_candidates(&"ab".repeat(31)).is_err());
}

#[test]
fn checksums_are_the_ones_the_sdk_writes_on_mainnet() {
    let ledger = Chain::Hedera.hedera_network().unwrap().ledger_id;
    for row in vectors()["checksums"].as_array().unwrap() {
        let account = row["account"].as_str().unwrap();
        let written = row["with_checksum"].as_str().unwrap();
        let id = HederaAccountId::parse(Chain::Hedera, account).unwrap();
        assert_eq!(id.to_string(), account);
        assert_eq!(format!("{id}-{}", id.checksum(ledger)), written);
        assert_eq!(HederaAccountId::parse(Chain::Hedera, written).unwrap(), id);
    }
}

#[test]
fn an_account_is_refused_unless_it_is_this_networks_and_canonical() {
    let chain = Chain::Hedera;
    // Testnet's checksum of the same account (ledger 0x01), and a typo.
    let testnet = HederaAccountId {
        shard: 0,
        realm: 0,
        num: 123,
    }
    .checksum(&[0x01]);
    assert_ne!(testnet, "vfmkw");
    for bad in [
        "0.0.123-vfmkx".to_string(),
        format!("0.0.123-{testnet}"),
        "0.0.124-vfmkw".into(),
        "0.0".into(),
        "0.0.0123".into(),
        "0.0.-1".into(),
        "0.0.1.2".into(),
        "1.0.123".into(),
        "0.1.123".into(),
        "0.0.123-".into(),
        "0.0.18446744073709551616".into(),
        "0x0000000000000000000000000000000000000001".into(),
        "302a300506032b657003210010fcec8c5db29aeab62272a08be39b1ec75eba50624351fba9948ff5d3d78ecf"
            .into(),
    ] {
        assert!(HederaAccountId::parse(chain, &bad).is_err(), "{bad}");
        assert!(
            !crate::send::flow::is_valid_send_address(chain, bad.clone()),
            "{bad}"
        );
    }
    assert!(crate::send::flow::is_valid_send_address(
        chain,
        "0.0.123-vfmkw".into()
    ));
    assert_eq!(
        crate::validation::address::validate_address(chain, "0.0.123-vfmkw")
            .normalized_value
            .as_deref(),
        Some("0.0.123")
    );
    assert!(HederaAccountId::parse(Chain::Ethereum, "0.0.123").is_err());
}

#[test]
fn signatures_verify_under_the_key_each_scheme_names() {
    let raw = &vectors()["raw_key"];
    let message = b"hedera transaction body";
    for scheme in ["ed25519", "ecdsa"] {
        let key = stored_key(raw[scheme]["private_key"].as_str().unwrap()).unwrap();
        let signature = key.sign(message);
        assert_eq!(signature.len(), 64);
        let public = key.public_key();
        match public.scheme {
            HederaKeyScheme::Ed25519 => {
                let verifying = ed25519_dalek::VerifyingKey::from_bytes(
                    &public.bytes.clone().try_into().unwrap(),
                )
                .unwrap();
                verifying
                    .verify_strict(
                        message,
                        &ed25519_dalek::Signature::from_bytes(&signature.try_into().unwrap()),
                    )
                    .unwrap();
            }
            HederaKeyScheme::EcdsaSecp256k1 => {
                use sha3::Digest as _;
                let digest: [u8; 32] = sha3::Keccak256::digest(message).into();
                let mut signature = secp256k1::ecdsa::Signature::from_compact(&signature).unwrap();
                let normalized = signature;
                signature.normalize_s();
                assert_eq!(signature, normalized, "a low s");
                Secp256k1::verification_only()
                    .verify_ecdsa(
                        &Message::from_digest(digest),
                        &signature,
                        &PublicKey::from_slice(&public.bytes).unwrap(),
                    )
                    .unwrap();
            }
        }
    }
}
