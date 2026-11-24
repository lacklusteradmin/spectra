use super::*;
use crate::derivation::setup::WalletSecretFormat;

fn vectors() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/algorand-vectors.json"
    ))
    .unwrap()
}

fn rows<'a>(vectors: &'a serde_json::Value, key: &str) -> &'a Vec<serde_json::Value> {
    vectors[key].as_array().unwrap()
}

#[test]
fn mnemonics_hold_the_keys_and_addresses_algosdk_reads() {
    let vectors = vectors();
    for row in rows(&vectors, "mnemonics") {
        let mnemonic = row["mnemonic"].as_str().unwrap();
        let seed: [u8; 32] = hex::decode(row["seed"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(mnemonic_from_seed(&seed).as_str(), mnemonic);
        assert_eq!(*seed_from_mnemonic(mnemonic).unwrap(), seed);
        let derived = derive_from_seed_phrase(mnemonic, "", None, Wants::ALL).unwrap();
        assert_eq!(derived.address.as_deref(), row["address"].as_str());
        assert_eq!(
            derived.public_key_hex.as_deref(),
            row["public_key"].as_str()
        );
        assert_eq!(
            derived.private_key_hex.as_deref().map(String::as_str),
            row["seed"].as_str()
        );
        assert_eq!(
            crate::derivation::phrase::check_phrase(Chain::Algorand, mnemonic, None).unwrap(),
            WalletSecretFormat::AlgorandMnemonic
        );
        assert!(crate::derivation::phrase::is_single_key(
            Chain::Algorand,
            mnemonic
        ));
        // A raw key is the same account as its mnemonic.
        let raw = crate::derivation::dispatch::derive_from_private_key(
            Chain::Algorand,
            row["seed"].as_str().unwrap().into(),
            true,
            true,
        )
        .unwrap()
        .unwrap();
        assert_eq!(raw.address.as_deref(), row["address"].as_str());
    }
}

#[test]
fn a_mnemonic_algosdk_refuses_is_refused() {
    let vectors = vectors();
    for row in rows(&vectors, "refused_mnemonics") {
        let mnemonic = row["mnemonic"].as_str().unwrap();
        let why = row["why"].as_str().unwrap();
        assert!(seed_from_mnemonic(mnemonic).is_err(), "{why}");
        assert!(
            crate::derivation::phrase::check_phrase(Chain::Algorand, mnemonic, None).is_err(),
            "{why}"
        );
    }
    // Not words of the list at all.
    let unlisted = ["zzzz"; 25].join(" ");
    assert!(seed_from_mnemonic(&unlisted).is_err());
}

#[test]
fn a_mnemonic_is_one_key_that_takes_no_path_and_no_passphrase() {
    let vectors = vectors();
    let mnemonic = rows(&vectors, "mnemonics")[2]["mnemonic"].as_str().unwrap();
    assert!(derive_from_seed_phrase(mnemonic, "m/44'/283'/0'/0/0", None, Wants::ADDRESS).is_err());
    assert!(derive_from_seed_phrase(mnemonic, "", Some("TREZOR"), Wants::ADDRESS).is_err());
    assert!(derive_from_seed_phrase(mnemonic, "", Some(""), Wants::ADDRESS).is_ok());
    assert!(
        crate::derivation::phrase::check_phrase(Chain::Algorand, mnemonic, Some("TREZOR")).is_err()
    );
}

#[test]
fn bip39_accounts_derive_along_arc52_as_xhd_wallet_api_derives_them() {
    let vectors = vectors();
    for row in rows(&vectors, "bip39_accounts") {
        let phrase = row["phrase"].as_str().unwrap();
        let path = row["path"].as_str().unwrap();
        let passphrase = row["passphrase"].as_str().filter(|p| !p.is_empty());
        let derived = derive_from_seed_phrase(phrase, path, passphrase, Wants::ALL).unwrap();
        assert_eq!(
            derived.address.as_deref(),
            row["address"].as_str(),
            "{path}"
        );
        assert_eq!(
            derived.public_key_hex.as_deref(),
            row["public_key"].as_str(),
            "{path}"
        );
        assert_eq!(
            derived.private_key_hex.as_deref().map(String::as_str),
            row["extended_key"].as_str(),
            "{path}"
        );
        assert_eq!(
            Chain::Algorand
                .derivation_profile_path(
                    crate::catalog::chains::DerivationProfile::Standard,
                    row["account"].as_u64().unwrap() as u32
                )
                .as_deref(),
            Some(path),
            "the catalog's path"
        );
        assert!(!crate::derivation::phrase::is_single_key(
            Chain::Algorand,
            phrase
        ));
        assert_eq!(
            crate::derivation::phrase::check_phrase(Chain::Algorand, phrase, passphrase).unwrap(),
            WalletSecretFormat::Bip39Phrase
        );
    }
}

#[test]
fn a_bip39_account_walks_a_path_of_one_to_eight_levels() {
    let phrase = crate::derivation::phrase::test_phrase(Chain::Algorand);
    assert!(derive_from_seed_phrase(phrase, "", None, Wants::ADDRESS).is_err());
    assert!(derive_from_seed_phrase(phrase, "m", None, Wants::ADDRESS).is_err());
    assert!(derive_from_seed_phrase(phrase, "m/0/1/2/3/4/5/6/7/8", None, Wants::ADDRESS).is_err());
    assert!(derive_from_seed_phrase(phrase, "m/0/1/2/3/4/5/6/7", None, Wants::ADDRESS).is_ok());
}

#[test]
fn an_address_is_refused_unless_its_checksum_is_its_keys() {
    let vectors = vectors();
    let address = rows(&vectors, "mnemonics")[2]["address"].as_str().unwrap();
    let key = decode_address(address).unwrap();
    assert_eq!(address_from_public_key(&key), address);
    // base32 is one alphabet in either case.
    assert_eq!(decode_address(&address.to_ascii_lowercase()).unwrap(), key);
    let validated = crate::validation::address::validate_address(
        Chain::Algorand,
        &address.to_ascii_lowercase(),
    );
    assert_eq!(validated.normalized_value.as_deref(), Some(address));
    // A changed letter breaks the checksum; a short or long one is no
    // address; the last letter carries two bits that must be zero.
    let mut changed = address.to_string();
    changed.replace_range(10..11, if &address[10..11] == "A" { "B" } else { "A" });
    const ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let last = ALPHABET.find(&address[57..]).unwrap();
    assert_eq!(last & 3, 0);
    let trailing = format!("{}{}", &address[..57], &ALPHABET[last | 1..=last | 1]);
    for bad in [
        changed.as_str(),
        &address[..57],
        &format!("{address}A"),
        &trailing,
        "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
        "",
    ] {
        assert!(decode_address(bad).is_err(), "{bad}");
    }
}

#[test]
fn a_created_phrase_is_a_mnemonic_every_algorand_wallet_restores() {
    let phrase = crate::derivation::phrase::generate_phrase(Chain::Algorand, 25).unwrap();
    assert_eq!(
        crate::derivation::phrase::check_phrase(Chain::Algorand, &phrase, None).unwrap(),
        WalletSecretFormat::AlgorandMnemonic
    );
    assert!(crate::derivation::phrase::generate_phrase(Chain::Algorand, 24).is_err());
    assert_eq!(
        crate::validation::created_phrase_lengths(Chain::Algorand)
            .iter()
            .map(|length| length.word_count)
            .collect::<Vec<_>>(),
        [25]
    );
    let lengths: Vec<_> = crate::validation::seed_phrase_lengths(Some(Chain::Algorand))
        .into_iter()
        .map(|length| (length.word_count, length.format))
        .collect();
    assert_eq!(
        lengths.last(),
        Some(&(25, WalletSecretFormat::AlgorandMnemonic))
    );
    assert!(lengths.contains(&(24, WalletSecretFormat::Bip39Phrase)));
}
