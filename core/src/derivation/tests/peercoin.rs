use super::*;
use crate::derivation::utxo_address::{ParsedUtxoAddress, parse_utxo_address};

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

#[test]
fn official_cointoolkit_private_keys_match_each_network() {
    // peercoin/cointoolkit test.html prefix4/prefix5; WIF payloads decoded to
    // raw private keys because Spectra's shared private-key import accepts hex.
    for (chain, key, expected) in [
        (
            Chain::Peercoin,
            "ffab51c05d7859d0f750d9f68a93b37165a4318944c8d13061026a8ab72b3cb9",
            "PVNK9afpBpfm8kajamaepRNJYi1af5aSJr",
        ),
        (
            Chain::PeercoinTestnet,
            "6d2cb60ccb94d2eac9e0fd59f99b0b728226061087c637afda09ae8bedb79cdb",
            "mfgbcjmRSMJq2akEPhUDHDapmHb9qMZrry",
        ),
    ] {
        let result =
            crate::derivation::dispatch::derive_from_private_key(chain, key.into(), true, true)
                .unwrap()
                .unwrap();
        assert_eq!(result.address.as_deref(), Some(expected));
        assert!(crate::send::flow::is_valid_send_address(
            chain,
            expected.into()
        ));
        let other = if chain.is_testnet() {
            Chain::Peercoin
        } else {
            Chain::PeercoinTestnet
        };
        assert!(!crate::send::flow::is_valid_send_address(
            other,
            expected.into()
        ));
        let mut typo = expected.as_bytes().to_vec();
        typo[5] = if typo[5] == b'a' { b'b' } else { b'a' };
        assert!(!crate::send::flow::is_valid_send_address(
            chain,
            String::from_utf8(typo).unwrap()
        ));
    }
}

#[test]
fn catalog_paths_derive_every_supported_script_on_the_selected_network() {
    for chain in [Chain::Peercoin, Chain::PeercoinTestnet] {
        for template in &chain.entry().derivation_path {
            let path = template.path.replace("{account}", "2");
            let result = crate::derivation::dispatch::derive_for_chain(
                chain, PHRASE, &path, None, None, None, true, true, true,
            )
            .unwrap();
            assert_eq!((result.account, result.branch, result.index), (2, 0, 0));
            let address = result.address.unwrap();
            assert!(crate::send::flow::is_valid_send_address(
                chain,
                address.clone()
            ));
            let parsed = parse_utxo_address(chain, &address).unwrap();
            match template.tag.as_str() {
                "legacy" => assert!(matches!(parsed, ParsedUtxoAddress::P2pkh(_))),
                "nestedSegWit" => assert!(matches!(parsed, ParsedUtxoAddress::P2sh(_))),
                "nativeSegWit" => {
                    assert!(
                        matches!(parsed, ParsedUtxoAddress::Witness { version: 0, ref program } if program.len() == 20)
                    );
                    assert!(address.starts_with(if chain.is_testnet() { "tpc1q" } else { "pc1q" }));
                    let upper = crate::validation::address::validate_address(
                        crate::validation::address::AddressValidationRequest {
                            kind: chain.address_validation_kind().into(),
                            value: address.to_ascii_uppercase(),
                        },
                    );
                    assert_eq!(upper.normalized_value.as_deref(), Some(address.as_str()));
                }
                "taproot" => {
                    assert!(
                        matches!(parsed, ParsedUtxoAddress::Witness { version: 1, ref program } if program.len() == 32)
                    );
                    assert!(address.starts_with(if chain.is_testnet() { "tpc1p" } else { "pc1p" }));
                }
                tag => panic!("unexpected Peercoin path tag {tag}"),
            }
            let public = secp256k1::PublicKey::from_slice(
                &hex::decode(result.public_key_hex.unwrap()).unwrap(),
            )
            .unwrap();
            assert_eq!(
                chain
                    .encode_discovery_address(
                        &public,
                        crate::derivation::dispatch::script_type_for_path(&path)
                    )
                    .unwrap(),
                address
            );
            let other = if chain.is_testnet() {
                Chain::Peercoin
            } else {
                Chain::PeercoinTestnet
            };
            assert!(parse_utxo_address(other, &address).is_err());
        }
    }
}

#[test]
fn peercoin_discovery_accepts_taproot_paths_on_the_concrete_network() {
    for (chain, coin) in [(Chain::Peercoin, 6), (Chain::PeercoinTestnet, 1)] {
        for branch in [0, 1] {
            let path = format!("m/86'/{coin}'/2'/{branch}/7");
            assert_eq!(
                crate::derivation::path::utxo_discovery_index(&path, chain, branch),
                Some(7)
            );
            let other = if chain.is_testnet() {
                Chain::Peercoin
            } else {
                Chain::PeercoinTestnet
            };
            assert_eq!(
                crate::derivation::path::utxo_discovery_index(&path, other, branch),
                None
            );
        }
    }
}

#[test]
fn funds_finder_scans_peercoin_catalog_formats_for_three_accounts() {
    let candidates = crate::derivation::funds_finder::generate_funds_finder_candidates(
        crate::derivation::funds_finder::FundsFinderRequest {
            seed_phrase: PHRASE.into(),
            passphrase: None,
        },
    )
    .unwrap();
    let peercoin: Vec<_> = candidates
        .iter()
        .filter(|c| c.chain_id == Chain::Peercoin)
        .collect();
    assert_eq!(peercoin.len(), 12);
    for template in &Chain::Peercoin.entry().derivation_path {
        for account in 0..3 {
            let path = template.path.replace("{account}", &account.to_string());
            let candidate = peercoin.iter().find(|c| c.derivation_path == path).unwrap();
            assert!(crate::send::flow::is_valid_send_address(
                Chain::Peercoin,
                candidate.address.clone()
            ));
        }
    }
}

#[test]
fn official_script_hash_fixture_uses_peercoin_network_versions() {
    // Official Cointoolkit test.html P2SH fixture: HASH160(935587).
    let hash: [u8; 20] = hex::decode("9c7d1d4a371634286f4437f7f8a38021ffbb7ca0")
        .unwrap()
        .try_into()
        .unwrap();
    for (chain, address) in [
        (Chain::Peercoin, "pKp1VjTuobrR6GdtCiU54WcYBraBTccebQ"),
        (
            Chain::PeercoinTestnet,
            "2N7WfHK1ftrTdhWej8rnFNR7guhvhfGWwFR",
        ),
    ] {
        assert_eq!(
            parse_utxo_address(chain, address).unwrap(),
            ParsedUtxoAddress::P2sh(hash)
        );
        let other = if chain.is_testnet() {
            Chain::Peercoin
        } else {
            Chain::PeercoinTestnet
        };
        assert!(parse_utxo_address(other, address).is_err());
    }
}
