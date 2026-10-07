//! Phrase formats against independent fixtures: monero-python and the
//! Polyseed reference implementation (`monero-phrases.json`), and ton-crypto
//! (`ton-mnemonics.json`). See the generator scripts named in each file.

use super::*;
use crate::validation::{SeedPhraseCheck, SeedPhraseProblem, check_seed_phrase};

fn monero_fixtures() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/monero-phrases.json")).unwrap()
}

fn ton_fixtures() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/ton-mnemonics.json")).unwrap()
}

fn verdict(chain: Chain, phrase: &str) -> crate::validation::SeedPhraseVerdict {
    check_seed_phrase(SeedPhraseCheck {
        words: phrase.split_whitespace().map(str::to_string).collect(),
        language: None,
        word_count: None,
        chain: Some(chain),
    })
}

fn derive(
    chain: Chain,
    phrase: &str,
    passphrase: Option<&str>,
) -> crate::derivation::types::DerivationResult {
    crate::derivation::dispatch::derive_for_chain(
        chain, phrase, "", passphrase, None, None, true, true, true,
    )
    .unwrap_or_else(|error| panic!("{chain}: {error}"))
}

fn text(value: &serde_json::Value, key: &str) -> String {
    value[key].as_str().unwrap().to_string()
}

#[test]
fn monero_seeds_in_every_wordlist_restore_monero_pythons_keys_and_addresses() {
    for vector in monero_fixtures()["electrum"].as_array().unwrap() {
        let phrase = text(vector, "phrase");
        let language = text(vector, "language");
        let judged = verdict(Chain::Monero, &phrase);
        assert!(judged.is_valid, "{language}: {:?}", judged.problem);
        assert_eq!(judged.format, Some(WalletSecretFormat::MoneroPhrase));
        assert_eq!(judged.language.map(|l| l.code), Some(language.clone()));
        let keys = derive(Chain::Monero, &phrase, None);
        let private = keys.private_key_hex.unwrap();
        assert_eq!(private[..64], text(vector, "spend_key"), "{language}");
        assert_eq!(private[64..], text(vector, "view_key"), "{language}");
        assert_eq!(keys.address.unwrap(), text(vector, "address"), "{language}");
        assert_eq!(
            derive(Chain::MoneroStagenet, &phrase, None)
                .address
                .unwrap(),
            text(vector, "stagenet_address"),
            "{language}"
        );
        assert_eq!(
            check_phrase(Chain::Monero, &phrase, None).unwrap(),
            WalletSecretFormat::MoneroPhrase
        );
    }
}

/// Spectra writes a created seed the way monero-python, like monero-wallet-cli,
/// writes the same key in English.
#[test]
fn an_english_monero_seed_is_written_as_monero_writes_it() {
    let vector = &monero_fixtures()["electrum"][0];
    assert_eq!(text(vector, "language"), "en");
    let key: [u8; 32] = hex::decode(text(vector, "spend_key"))
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(
        *crate::derivation::monero_words::encode(&key),
        text(vector, "phrase")
    );
}

/// The libmonero crate's documented vector.
#[test]
fn a_documented_monero_seed_decodes_to_its_key() {
    let phrase = "tissue raking haunted huts afraid volcano howls liar egotistic befit rounded \
                  older bluntly imbalance pivot exotic tuxedo amaze mostly lukewarm macro vocal \
                  hounded biplane rounded";
    assert_eq!(
        hex::encode(*crate::derivation::monero_words::decode(phrase).unwrap()),
        "f7b3beabc9bd6ced864096c0891a8fdf94dc714178a09828775dba01b4df9ab8"
    );
}

#[test]
fn polyseeds_in_every_wordlist_restore_the_reference_keys_and_addresses() {
    for vector in monero_fixtures()["polyseed"].as_array().unwrap() {
        let phrase = text(vector, "phrase");
        let language = text(vector, "language");
        let judged = verdict(Chain::Monero, &phrase);
        assert!(judged.is_valid, "{language}: {:?}", judged.problem);
        assert_eq!(
            judged.format,
            Some(WalletSecretFormat::Polyseed),
            "{language}"
        );
        let seed = crate::derivation::polyseed::decode(&phrase).unwrap();
        assert_eq!(
            hex::encode(*seed.monero_key()),
            text(vector, "key"),
            "{language}"
        );
        assert_eq!(seed.birthday_unix(), vector["birthday"].as_u64().unwrap());
        assert!(seed.birthday_unix() <= vector["created"].as_u64().unwrap());
        let keys = derive(Chain::Monero, &phrase, None);
        assert_eq!(
            keys.private_key_hex.unwrap()[..64],
            text(vector, "spend_key")
        );
        assert_eq!(keys.address.unwrap(), text(vector, "address"), "{language}");
        assert_eq!(
            derive(Chain::MoneroStagenet, &phrase, None)
                .address
                .unwrap(),
            text(vector, "stagenet_address")
        );
    }
}

#[test]
fn an_encrypted_polyseed_is_named_and_refused() {
    let phrase = text(&monero_fixtures(), "encrypted_polyseed");
    let judged = verdict(Chain::Monero, &phrase);
    assert!(!judged.is_valid);
    assert_eq!(judged.problem, Some(SeedPhraseProblem::EncryptedPolyseed));
    assert!(check_phrase(Chain::Monero, &phrase, None).is_err());
    assert!(
        crate::derivation::dispatch::derive_for_chain(
            Chain::Monero,
            &phrase,
            "",
            None,
            None,
            None,
            true,
            false,
            false
        )
        .is_err()
    );
}

/// No Monero wallet restores a BIP-39 phrase, so neither does Spectra.
#[test]
fn a_bip39_phrase_is_not_a_monero_phrase() {
    let bip39 = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    // Twelve words are an unfinished Polyseed, judged at its 16.
    let judged = verdict(Chain::Monero, bip39);
    assert!(!judged.is_valid);
    assert_eq!(judged.word_count, 16);
    let refused = check_phrase(Chain::Monero, bip39, None).unwrap_err();
    assert!(refused.to_string().contains("16 or 25"), "{refused}");
    assert!(check_phrase(Chain::Monero, bip39, None).is_err());
    assert!(
        crate::derivation::dispatch::derive_for_chain(
            Chain::Monero,
            bip39,
            "",
            None,
            None,
            None,
            true,
            false,
            false
        )
        .is_err()
    );
}

/// The W5 accounts of the `ton-mnemonics.json` wallets, by mnemonic.
fn w5_addresses() -> serde_json::Value {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/ton-w5.json")).unwrap();
    fixture["addresses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| (text(entry, "mnemonic"), entry.clone()))
        .collect::<serde_json::Map<_, _>>()
        .into()
}

/// ton-crypto's keys, the v4R2 account `ton-mnemonics.json` records, and the
/// W5 accounts on both networks that `ton-w5.json` does — W5 being the
/// default a mnemonic derives.
#[test]
fn ton_mnemonics_restore_ton_cryptos_keys_and_every_wallet_version() {
    use crate::derivation::ton::TonWalletVersion;
    let fixtures = ton_fixtures();
    let w5 = w5_addresses();
    for vector in fixtures["mnemonics"].as_array().unwrap() {
        let mnemonic = text(vector, "mnemonic");
        let judged = verdict(Chain::Ton, &mnemonic);
        assert!(judged.is_valid, "{:?}", judged.problem);
        assert_eq!(judged.format, Some(WalletSecretFormat::TonMnemonic));
        let keys = derive(Chain::Ton, &mnemonic, None);
        assert_eq!(
            keys.public_key_hex.as_deref(),
            Some(&*text(vector, "public_key"))
        );
        assert_eq!(keys.private_key_hex.unwrap(), text(vector, "private_key"));
        assert_eq!(keys.address.unwrap(), text(&w5[&mnemonic], "mainnet"));
        let testnet = derive(Chain::TonTestnet, &mnemonic, None);
        assert_eq!(testnet.address.unwrap(), text(&w5[&mnemonic], "testnet"));
        let public: [u8; 32] = hex::decode(keys.public_key_hex.unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(
            TonWalletVersion::V4R2.address(&public, Chain::Ton).unwrap(),
            text(vector, "address")
        );
    }
}

#[test]
fn a_password_protected_ton_mnemonic_needs_its_password() {
    let vector = &ton_fixtures()["password_protected"];
    let mnemonic = text(vector, "mnemonic");
    let password = text(vector, "password");
    assert!(verdict(Chain::Ton, &mnemonic).is_valid);
    let keys = derive(Chain::Ton, &mnemonic, Some(&password));
    assert_eq!(keys.public_key_hex.unwrap(), text(vector, "public_key"));
    assert_eq!(
        keys.address.unwrap(),
        text(&w5_addresses()[&mnemonic], "mainnet")
    );
    assert!(check_phrase(Chain::Ton, &mnemonic, None).is_err());
    assert!(check_phrase(Chain::Ton, &mnemonic, Some("wrong")).is_err());
    assert!(check_phrase(Chain::Ton, &mnemonic, Some(&password)).is_ok());
}

/// ton-crypto refuses this 24-word BIP-39 phrase as a TON mnemonic, and so
/// does Spectra, rather than deriving a wallet no TON app would restore.
#[test]
fn a_bip39_phrase_is_not_a_ton_mnemonic() {
    let fixtures = ton_fixtures();
    assert!(!fixtures["bip39_phrase_is_ton_mnemonic"].as_bool().unwrap());
    let bip39 = text(&fixtures, "bip39_phrase");
    let judged = verdict(Chain::Ton, &bip39);
    assert!(!judged.is_valid);
    assert_eq!(judged.problem, Some(SeedPhraseProblem::InvalidChecksum));
    assert!(check_phrase(Chain::Ton, &bip39, None).is_err());
    assert!(
        crate::derivation::dispatch::derive_for_chain(
            Chain::Ton,
            &bip39,
            "",
            None,
            None,
            None,
            true,
            false,
            false
        )
        .is_err()
    );
}

/// A created phrase is in the format the chain's own wallets restore, and
/// reads back as one.
#[test]
fn created_phrases_are_in_each_chains_own_format() {
    for (chain, words, format) in [
        (Chain::Monero, 25, WalletSecretFormat::MoneroPhrase),
        (Chain::MoneroStagenet, 25, WalletSecretFormat::MoneroPhrase),
        (Chain::Ton, 24, WalletSecretFormat::TonMnemonic),
        (Chain::Bitcoin, 12, WalletSecretFormat::Bip39Phrase),
        (Chain::Ethereum, 24, WalletSecretFormat::Bip39Phrase),
    ] {
        let phrase = generate_phrase(chain, words).unwrap();
        assert_eq!(phrase.split(' ').count(), words as usize);
        let judged = verdict(chain, &phrase);
        assert!(judged.is_valid, "{chain}: {:?}", judged.problem);
        assert_eq!(judged.format, Some(format), "{chain}");
        assert_eq!(check_phrase(chain, &phrase, None).unwrap(), format);
    }
    assert!(generate_phrase(Chain::Monero, 12).is_err());
    assert!(generate_phrase(Chain::Ton, 12).is_err());
    assert!(generate_phrase(Chain::Bitcoin, 25).is_err());
}

#[test]
fn each_chain_offers_the_lengths_of_its_own_formats() {
    use crate::validation::seed_phrase_lengths;
    let lengths = |chain| {
        seed_phrase_lengths(chain)
            .into_iter()
            .map(|length| (length.word_count, length.format))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        lengths(Some(Chain::Monero)),
        [
            (16, WalletSecretFormat::Polyseed),
            (25, WalletSecretFormat::MoneroPhrase)
        ]
    );
    assert_eq!(
        lengths(Some(Chain::TonTestnet)),
        [(24, WalletSecretFormat::TonMnemonic)]
    );
    assert_eq!(lengths(Some(Chain::Bitcoin)), lengths(None));
    assert_eq!(lengths(None).len(), 5);
    assert!(crate::validation::seed_phrase_languages(Some(Chain::Monero)).is_empty());
    assert_eq!(
        crate::validation::seed_phrase_languages(Some(Chain::Ethereum)).len(),
        10
    );
}
