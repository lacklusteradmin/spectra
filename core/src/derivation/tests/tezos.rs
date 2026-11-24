use super::*;
use crate::registry::Chain;

fn vectors() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/tezos-vectors.json")).unwrap()
}

fn text<'a>(value: &'a serde_json::Value, key: &str) -> &'a str {
    value[key].as_str().unwrap()
}

#[test]
fn a_phrase_derives_the_accounts_taquito_derives() {
    let vectors = vectors();
    let phrase = text(&vectors, "mnemonic");
    let profile = Chain::Tezos.derivation_profiles()[0];
    for account in vectors["accounts"].as_array().unwrap() {
        let index = account["account"].as_u64().unwrap() as u32;
        let path = text(account, "path");
        assert_eq!(
            Chain::Tezos
                .derivation_profile_path(profile, index)
                .as_deref(),
            Some(path),
            "the catalog's path"
        );
        let derived = derive_from_seed_phrase(phrase, path, None, Wants::ALL).unwrap();
        assert_eq!(derived.address.as_deref(), Some(text(account, "address")));
        assert_eq!(
            derived.public_key_hex.as_deref(),
            Some(text(account, "public_key_hex"))
        );
        assert_eq!(
            derived.private_key_hex.as_ref().map(|key| key.as_str()),
            Some(text(account, "private_key"))
        );
        let public: [u8; 32] = hex::decode(text(account, "public_key_hex"))
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(public_key_string(&public), text(account, "public_key"));
        assert!(crate::send::flow::is_valid_send_address(
            Chain::Tezos,
            text(account, "address").into()
        ));
    }
}

#[test]
fn an_edsk_reads_as_its_seed_in_both_forms_wallets_export() {
    let vectors = vectors();
    for account in vectors["accounts"].as_array().unwrap() {
        let seed = text(account, "private_key");
        for key in [text(account, "edsk_seed"), text(account, "edsk_secret_key")] {
            let read = read_secret_key(key).unwrap().unwrap();
            assert_eq!(hex::encode(*read), seed, "{key}");
        }
        let seed: [u8; 32] = hex::decode(seed).unwrap().try_into().unwrap();
        assert_eq!(*secret_key_string(&seed), text(account, "edsk_seed"));
    }
    // A 64-byte key whose public half is another key's.
    let first = &vectors["accounts"][0];
    let second = &vectors["accounts"][1];
    let mut pair = hex::decode(text(first, "private_key")).unwrap();
    pair.extend(hex::decode(text(second, "public_key_hex")).unwrap());
    let mismatched = base58check(&prefix::EDSK_SECRET, &pair);
    assert!(read_secret_key(&mismatched).unwrap().is_err());
    // Not an edsk at all, or one with a broken checksum.
    assert!(read_secret_key(text(first, "address")).is_none());
    let mut typo = text(first, "edsk_seed").to_string();
    typo.replace_range(10..11, if &typo[10..11] == "a" { "b" } else { "a" });
    assert!(read_secret_key(&typo).is_none());
}

#[test]
fn every_kind_of_account_a_transfer_pays_decodes_and_spells_back() {
    let vectors = vectors();
    let recipients = &vectors["recipients"];
    let mut addresses: Vec<&str> = ["tz2", "tz3", "tz4", "kt1"]
        .into_iter()
        .map(|kind| text(recipients, kind))
        .collect();
    addresses.push(text(&vectors["accounts"][0], "address"));
    for address in addresses {
        let decoded = decode_address(address).unwrap();
        assert_eq!(decoded.encode(), address);
        assert_eq!(decoded.is_implicit(), !address.starts_with("KT1"));
        assert!(crate::send::flow::is_valid_send_address(
            Chain::Tezos,
            address.into()
        ));
    }
    assert!(matches!(
        decode_address(text(recipients, "kt1")).unwrap(),
        TezosAddress::Originated(_)
    ));
}

#[test]
fn an_address_is_refused_unless_it_is_an_enabled_tezos_account() {
    let address = text(&vectors()["accounts"][0], "address").to_string();
    let hash: [u8; 20] = match decode_address(&address).unwrap() {
        TezosAddress::Implicit { hash, .. } => hash,
        TezosAddress::Originated(_) => unreachable!(),
    };
    // A typo breaks the checksum; base58 is case sensitive.
    let mut typo = address.clone();
    typo.replace_range(20..21, if &typo[20..21] == "a" { "b" } else { "a" });
    // tz5 (ML-DSA-44) is not enabled on mainnet; a public key is no address.
    let tz5 = base58check(&[6, 161, 169], &hash);
    let edpk = text(&vectors()["accounts"][0], "public_key").to_string();
    for invalid in [
        typo,
        address.to_uppercase(),
        tz5,
        edpk,
        format!("{address} "),
        String::new(),
        "0x0000000000000000000000000000000000000000".into(),
    ] {
        assert!(decode_address(&invalid).is_err(), "{invalid}");
        assert!(
            !crate::validation::address::validate_address(Chain::Tezos, &invalid).is_valid
                || invalid.trim() == address,
            "{invalid}"
        );
    }
}

#[test]
fn a_raw_key_holds_the_account_its_phrase_holds() {
    for account in vectors()["accounts"].as_array().unwrap() {
        let derived = crate::derivation::dispatch::derive_from_private_key(
            Chain::Tezos,
            text(account, "private_key").into(),
            true,
            true,
        )
        .unwrap()
        .unwrap();
        assert_eq!(derived.address.as_deref(), Some(text(account, "address")));
        assert_eq!(
            derived.public_key_hex.as_deref(),
            Some(text(account, "public_key_hex"))
        );
    }
}
