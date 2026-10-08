fn normalized(chain: crate::registry::Chain, address: &str) -> Option<String> {
    crate::derivation::import::normalized_import_address(chain, address)
}

#[test]
fn a_malformed_address_is_refused_whatever_the_chain() {
    // Solana and Tron were both in the lenient group.
    use crate::registry::Chain;
    for (chain, address) in [
        (Chain::Solana, "not-a-solana-address"),
        (Chain::Tron, "nonsense"),
        (Chain::Ethereum, "0xnothex"),
    ] {
        assert_eq!(normalized(chain, address), None, "{chain} kept {address}");
    }
}

/// A valid address survives import and is stored in core's normal form.
///
/// The fixture is the all-uppercase form: no EIP-55 checksum to verify, still
/// valid, and it still demonstrates the normalisation this test is about.
#[test]
fn a_valid_address_survives_and_is_normalised() {
    let stored = normalized(
        crate::registry::Chain::Ethereum,
        "0X742D35CC6634C0532925A3B844BC454E4438F44E",
    )
    .expect("kept");
    // Normalisation is core's, not the caller's transcription.
    assert!(stored.starts_with("0x"));
    assert_eq!(stored.len(), 42);
}

/// A derived address is judged by the network it was derived for.
#[test]
fn a_derived_address_is_judged_by_its_network() {
    let derived = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
    assert_eq!(
        normalized(crate::registry::Chain::Bitcoin, derived).as_deref(),
        Some(derived)
    );
    assert_eq!(
        normalized(crate::registry::Chain::BitcoinTestnet, derived),
        None
    );
}

/// Watched addresses are the input where the address is typed rather than
/// derived.
mod watch_only {
    use crate::registry::Chain;

    /// Kept addresses and refused ones for typed lines on `chain`.
    fn validated(chain: Chain, addresses: &[&str]) -> (Vec<String>, Vec<String>) {
        let typed: Vec<String> = addresses.iter().map(|a| a.to_string()).collect();
        crate::derivation::import::validated_watch_addresses(chain, &typed)
    }

    #[test]
    fn a_malformed_watch_address_is_refused() {
        let (kept, rejected) = validated(Chain::Solana, &["garbage"]);
        assert!(kept.is_empty(), "kept: {kept:?}");
        assert_eq!(rejected, vec!["garbage".to_string()]);
    }

    #[test]
    fn valid_watch_addresses_survive_and_are_normalised() {
        let (kept, rejected) = validated(
            Chain::Ethereum,
            &["0X742D35CC6634C0532925A3B844BC454E4438F44E"],
        );
        assert!(rejected.is_empty());
        assert_eq!(kept.len(), 1);
        assert!(kept[0].starts_with("0x"));
    }

    /// Core normalises on the way in, so a caller does not have to.
    #[test]
    fn every_slot_normalises_without_help_from_the_caller() {
        let padded = "0x0000000000000000000000000000000000000000000000000000000000000ABC";
        let cases: [(Chain, &str, &str); 3] = [
            (
                Chain::Ethereum,
                "0x742D35CC6634C0532925A3B844BC454E4438F44E",
                "0x742d35cc6634c0532925a3b844bc454e4438f44e",
            ),
            (Chain::Sui, padded, &padded.to_lowercase()),
            (Chain::Aptos, padded, &padded.to_lowercase()),
        ];
        for (slot, typed, expected) in cases {
            let (kept, rejected) = validated(slot, &[typed]);
            assert!(rejected.is_empty(), "{slot}: rejected {typed}");
            assert_eq!(kept, vec![expected.to_string()], "{slot} did not normalise");
        }
    }

    /// The import path and the send path must agree on what an address
    /// looks like once normalised.
    ///
    /// They are two separate tables — `validate_address` matches on the
    /// validation kind, `normalize_address` on the chain display name — so
    /// this fails if they ever drift apart.
    #[test]
    fn the_send_normaliser_and_the_import_normaliser_agree() {
        use crate::send::flow::normalized_send_address;
        // Internet Computer is absent on purpose: its account identifier
        // carries a CRC32 prefix, so there is no fixture to write here
        // without computing a real one, and a fixture the validator
        // rejects would test nothing.
        let cases: [(Chain, Chain, &str); 5] = [
            (
                Chain::Ethereum,
                Chain::Ethereum,
                "0x742D35CC6634C0532925A3B844BC454E4438F44E",
            ),
            (
                Chain::Sui,
                Chain::Sui,
                "0x0000000000000000000000000000000000000000000000000000000000000ABC",
            ),
            (
                Chain::Aptos,
                Chain::Aptos,
                "0x0000000000000000000000000000000000000000000000000000000000000ABC",
            ),
            (Chain::Near, Chain::Near, "Example.NEAR"),
            (
                Chain::Solana,
                Chain::Solana,
                "BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX",
            ),
        ];
        for (chain_id, slot, typed) in cases {
            let (kept, rejected) = validated(slot, &[typed]);
            assert!(rejected.is_empty(), "{chain_id}: rejected {typed}");
            let imported = kept.first().unwrap();
            let sent = normalized_send_address(chain_id, typed.to_string());
            assert_eq!(
                imported, &sent,
                "{chain_id}: import normalised to {imported}, send to {sent}"
            );
        }
    }

    #[test]
    fn surrounding_whitespace_is_not_the_caller_s_problem_either() {
        let (kept, rejected) = validated(
            Chain::Ethereum,
            &["  0x742d35cc6634c0532925a3b844bc454e4438f44e  "],
        );
        assert!(rejected.is_empty());
        assert_eq!(
            kept,
            vec!["0x742d35cc6634c0532925a3b844bc454e4438f44e".to_string()]
        );
    }

    /// A watched address belongs to its concrete network.
    #[test]
    fn a_testnet_watch_address_is_refused() {
        let typed = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";
        let (kept, rejected) = validated(Chain::Bitcoin, &[typed]);
        assert_eq!(rejected, vec![typed.to_string()]);
        assert!(kept.is_empty());
        let (kept, rejected) = validated(Chain::BitcoinTestnet, &[typed]);
        assert!(rejected.is_empty());
        assert_eq!(kept, vec![typed.to_string()]);
    }

    #[test]
    fn one_bad_address_does_not_discard_the_good_ones() {
        let (kept, rejected) = validated(
            Chain::Ethereum,
            &[
                "0X742D35CC6634C0532925A3B844BC454E4438F44E",
                "0xnothex",
                "   ",
                "0x0000000000000000000000000000000000000001",
            ],
        );
        assert_eq!(kept.len(), 2);
        // A blank line is neither kept nor refused.
        assert_eq!(rejected, vec!["0xnothex".to_string()]);
    }
}

/// An MWEB address is a Litecoin address a payment can reach, and nothing a
/// watched wallet could read: no wallet is one.
#[test]
fn an_mweb_address_is_no_wallet() {
    use crate::registry::Chain;
    let key = secp256k1::PublicKey::from_secret_key(
        &secp256k1::Secp256k1::new(),
        &secp256k1::SecretKey::from_slice(&[2; 32]).unwrap(),
    );
    let address = crate::send::litecoin_mweb::keys::StealthAddress {
        scan: key,
        spend: key,
    }
    .encode(Chain::Litecoin)
    .unwrap();
    assert!(crate::send::flow::is_valid_send_address(
        Chain::Litecoin,
        address.clone()
    ));
    assert_eq!(normalized(Chain::Litecoin, &address), None);
    assert!(!crate::derivation::import::is_valid_watch_only_address(
        Chain::Litecoin,
        address
    ));
}
