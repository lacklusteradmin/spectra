//! Private-key encodings against each chain's own SDK
//! (`private-key-formats.json`, from scripts/generate-private-key-vectors.cjs).

use super::*;

fn fixtures() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/private-key-formats.json"
    ))
    .unwrap()
}

fn text(value: &serde_json::Value, key: &str) -> String {
    value[key].as_str().unwrap().to_string()
}

fn address(chain: Chain, hex: &str) -> String {
    crate::derivation::dispatch::derive_from_private_key(chain, hex.to_string(), true, false)
        .unwrap()
        .unwrap()
        .address
        .unwrap()
}

#[test]
fn compressed_wifs_read_as_their_keys_on_their_own_network() {
    for vector in fixtures()["wif"].as_array().unwrap() {
        let chain = Chain::from_str_id(&text(vector, "chain")).unwrap();
        let read = parse_private_key(chain, &text(vector, "compressed")).unwrap();
        assert_eq!(*read, text(vector, "key"), "{chain}");
        assert!(is_valid_private_key(chain, text(vector, "compressed")));
    }
}

/// An uncompressed key owns another address, so it is refused rather than
/// read as the compressed key's wallet.
#[test]
fn an_uncompressed_wif_is_refused() {
    for vector in fixtures()["wif"].as_array().unwrap() {
        let chain = Chain::from_str_id(&text(vector, "chain")).unwrap();
        let refused = parse_private_key(chain, &text(vector, "uncompressed")).unwrap_err();
        assert!(
            refused.to_string().contains("uncompressed"),
            "{chain}: {refused}"
        );
    }
}

#[test]
fn a_wif_for_another_network_is_refused() {
    let fixtures = fixtures();
    let wif = |chain: &str| {
        fixtures["wif"]
            .as_array()
            .unwrap()
            .iter()
            .find(|vector| vector["chain"] == chain)
            .map(|vector| text(vector, "compressed"))
            .unwrap()
    };
    for (key_chain, import_chain) in [
        ("bitcoin", Chain::BitcoinTestnet),
        ("bitcoin-testnet", Chain::Bitcoin),
        ("litecoin", Chain::Bitcoin),
        ("dogecoin", Chain::Litecoin),
        ("dash", Chain::Peercoin),
    ] {
        assert!(
            parse_private_key(import_chain, &wif(key_chain)).is_err(),
            "{key_chain} key on {import_chain}"
        );
    }
    // A WIF on a chain without WIF is no key at all.
    assert!(parse_private_key(Chain::Ethereum, &wif("bitcoin")).is_err());
}

#[test]
fn solana_keypairs_in_base58_and_json_read_as_their_seed() {
    let vector = &fixtures()["solana"];
    for encoded in [text(vector, "base58"), text(vector, "json")] {
        let read = parse_private_key(Chain::Solana, &encoded).unwrap();
        assert_eq!(*read, text(vector, "key"));
        assert_eq!(address(Chain::Solana, &read), text(vector, "address"));
    }
}

#[test]
fn ed25519_chains_read_their_own_secret_encodings() {
    let fixtures = fixtures();
    for (chain, name) in [
        (Chain::Stellar, "stellar"),
        (Chain::Sui, "sui"),
        (Chain::Aptos, "aptos"),
    ] {
        let vector = &fixtures[name];
        let read = parse_private_key(chain, &text(vector, "secret")).unwrap();
        assert_eq!(*read, text(vector, "key"), "{chain}");
        assert_eq!(address(chain, &read), text(vector, "address"), "{chain}");
    }
    let near = &fixtures["near"];
    let read = parse_private_key(Chain::Near, &text(near, "secret")).unwrap();
    assert_eq!(*read, text(near, "key"));
    assert_eq!(address(Chain::Near, &read), text(near, "implicit_account"));
}

#[test]
fn a_keypair_whose_public_half_is_another_keys_is_refused() {
    let refused =
        parse_private_key(Chain::Near, &text(&fixtures(), "near_mismatched")).unwrap_err();
    assert!(refused.to_string().contains("public key"), "{refused}");
}

/// Each encoding belongs to its chain; hex keeps working everywhere it did.
#[test]
fn an_encoding_on_the_wrong_chain_is_refused_and_hex_still_reads() {
    let fixtures = fixtures();
    assert!(parse_private_key(Chain::Solana, &text(&fixtures["stellar"], "secret")).is_err());
    assert!(parse_private_key(Chain::Aptos, &text(&fixtures["sui"], "secret")).is_err());
    assert!(parse_private_key(Chain::Ethereum, &text(&fixtures["near"], "secret")).is_err());
    let hex = format!("0X{}", "AB".repeat(32));
    assert_eq!(
        *parse_private_key(Chain::Ethereum, &hex).unwrap(),
        "ab".repeat(32)
    );
    assert!(parse_private_key(Chain::Cardano, &"ab".repeat(32)).is_err());
    assert!(parse_private_key(Chain::Monero, &"ab".repeat(32)).is_err());
}

/// Every format the setup descriptor offers is one `parse_private_key` reads.
#[test]
fn the_descriptor_lists_only_formats_this_reads() {
    for chain in Chain::all() {
        for format in chain.private_key_formats() {
            assert!(
                matches!(
                    format,
                    WalletSecretFormat::HexSecret32
                        | WalletSecretFormat::CardanoExtendedKey
                        | WalletSecretFormat::Wif
                        | WalletSecretFormat::SolanaKeypair
                        | WalletSecretFormat::StellarSecretSeed
                        | WalletSecretFormat::SuiPrivateKey
                        | WalletSecretFormat::AptosPrivateKey
                        | WalletSecretFormat::NearSecretKey
                ),
                "{chain}: {format:?}"
            );
        }
        if chain.wif_version().is_some() {
            assert!(
                chain
                    .private_key_formats()
                    .contains(&WalletSecretFormat::Wif)
            );
        }
    }
}

/// Each chain's export encoding is the one its own SDK writes for the key.
#[test]
fn keys_export_as_their_chains_sdks_write_them() {
    let fixtures = fixtures();
    for vector in fixtures["wif"].as_array().unwrap() {
        let chain = Chain::from_str_id(&text(vector, "chain")).unwrap();
        assert_eq!(
            export_format(chain),
            Some(WalletSecretFormat::Wif),
            "{chain}"
        );
        let written = encode_private_key(chain, WalletSecretFormat::Wif, &text(vector, "key"));
        assert_eq!(*written.unwrap(), text(vector, "compressed"), "{chain}");
    }
    for (chain, format, field) in [
        (Chain::Solana, WalletSecretFormat::SolanaKeypair, "base58"),
        (
            Chain::Stellar,
            WalletSecretFormat::StellarSecretSeed,
            "secret",
        ),
        (Chain::Sui, WalletSecretFormat::SuiPrivateKey, "secret"),
        (Chain::Aptos, WalletSecretFormat::AptosPrivateKey, "secret"),
        (Chain::Near, WalletSecretFormat::NearSecretKey, "secret"),
    ] {
        let vector = &fixtures[chain.str_id()];
        assert_eq!(export_format(chain), Some(format), "{chain}");
        let written = encode_private_key(chain, format, &text(vector, "key")).unwrap();
        assert_eq!(*written, text(vector, field), "{chain}");
    }
}

/// On every chain that takes a key, the exported key reads back as itself
/// and derives the address the key does.
#[test]
fn every_exported_key_reads_back_as_itself() {
    for chain in Chain::all() {
        let Some(format) = export_format(chain) else {
            assert!(chain.private_key_formats().is_empty(), "{chain}");
            continue;
        };
        let key = if format == WalletSecretFormat::CardanoExtendedKey {
            let path = crate::derivation::path::default_path_from_catalog(chain).unwrap();
            crate::derivation::dispatch::derive_for_chain(
                chain,
                crate::derivation::phrase::test_phrase(chain),
                &path,
                None,
                None,
                None,
                false,
                false,
                true,
            )
            .unwrap()
            .private_key_hex
            .unwrap()
        } else {
            "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318".to_string()
        };
        let written = encode_private_key(chain, format, &key).unwrap();
        let read = parse_private_key(chain, &written).unwrap();
        assert_eq!(*read, key, "{chain} {format:?}");
        assert_eq!(address(chain, &read), address(chain, &key), "{chain}");
    }
}
