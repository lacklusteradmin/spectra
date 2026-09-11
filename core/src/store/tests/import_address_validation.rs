use crate::derivation::import::WalletImportAddresses;

/// Mainnet, which is what every case here means unless it says otherwise.
fn validated_addresses(addresses: &WalletImportAddresses) -> (WalletImportAddresses, Vec<String>) {
    crate::derivation::import::validated_addresses(addresses, &Default::default())
}

fn addresses(pairs: &[(&str, &str)]) -> WalletImportAddresses {
    WalletImportAddresses {
        by_slot: pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        bitcoin_xpub: None,
    }
}

#[test]
fn a_malformed_address_is_dropped_whatever_the_chain() {
    // "solana" and "tron" were both in the lenient group.
    let (kept, rejected) = validated_addresses(&addresses(&[
        ("solana", "not-a-solana-address"),
        ("tron", "nonsense"),
        ("ethereum", "0xnothex"),
    ]));
    assert!(kept.by_slot.is_empty(), "kept: {:?}", kept.by_slot);
    assert_eq!(rejected.len(), 3);
}

/// A valid address survives import and is stored in core's normal form.
///
/// The fixture was `0x742D35CC…bC454E…` — arbitrary mixed case, which the
/// validator accepted because it lowercased before looking. It is a
/// **broken EIP-55 checksum** and is refused now, so the fixture is the
/// all-uppercase form: no checksum to verify, still valid, and it still
/// demonstrates the normalisation this test is about.
#[test]
fn a_valid_address_survives_and_is_normalised() {
    let (kept, rejected) = validated_addresses(&addresses(&[(
        "ethereum",
        "0X742D35CC6634C0532925A3B844BC454E4438F44E",
    )]));
    assert!(rejected.is_empty());
    let stored = kept.by_slot.get("ethereum").expect("kept");
    // Normalisation is core's, not the caller's transcription.
    assert!(stored.starts_with("0x"));
    assert_eq!(stored.len(), 42);
}

#[test]
fn empty_and_whitespace_entries_are_skipped_not_rejected() {
    let (kept, rejected) = validated_addresses(&addresses(&[("solana", "   ")]));
    assert!(kept.by_slot.is_empty());
    assert!(
        rejected.is_empty(),
        "an unfilled field is not a rejected address"
    );
}

#[test]
fn the_bitcoin_xpub_is_carried_through_untouched() {
    let mut input = addresses(&[]);
    input.bitcoin_xpub = Some("zpub-whatever".to_string());
    let (kept, _) = validated_addresses(&input);
    assert_eq!(kept.bitcoin_xpub.as_deref(), Some("zpub-whatever"));
}

/// A derived address is mainnet-format even on a testnet import, so the
/// slot map is judged against mainnet regardless of the selected mode.
///
/// Derivation at import runs against the mainnet chain — `chainPaths` is
/// keyed by mainnet display name — and the testnet address is re-derived
/// for display. Judging this map by the selected network mode dropped
/// every address on a testnet import and produced a wallet with none.
#[test]
fn a_derived_address_is_kept_on_a_testnet_import() {
    let derived = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
    // Even asked for testnet4, the slot map is mainnet.
    let (kept, rejected) = crate::derivation::import::validated_addresses(
        &addresses(&[("bitcoin", derived)]),
        &crate::derivation::import::ImportNetworks {
            by_family: std::collections::HashMap::from([(
                "bitcoin".to_string(),
                "bitcoin-testnet-4".to_string(),
            )]),
        },
    );
    // The helper itself honours what it is told...
    assert_eq!(rejected, vec![derived.to_string()]);
    assert!(kept.by_slot.is_empty());
    // ...so it is `import_wallets` that must pass mainnet here, which is
    // what the default does.
    let (kept, rejected) = validated_addresses(&addresses(&[("bitcoin", derived)]));
    assert!(rejected.is_empty());
    assert_eq!(
        kept.by_slot.get("bitcoin").map(String::as_str),
        Some(derived)
    );
}

#[test]
fn a_rejection_names_the_address_not_the_slot() {
    // The caller has to be able to tell the user which address was
    // refused. A slot name ("ethereum") does not identify one when the
    // import supplied several.
    let (_, rejected) = validated_addresses(&addresses(&[("solana", "not-an-address")]));
    assert_eq!(rejected, vec!["not-an-address".to_string()]);
}

/// The watch-only list is a separate input from the slot map, and it is the
/// one where the address is typed rather than derived.
mod watch_only {
    use crate::derivation::import::{ImportNetworks, WalletImportWatchOnlyEntries};
    use std::collections::HashMap;

    /// Mainnet, as above.
    fn validated_watch_only_entries(
        entries: &WalletImportWatchOnlyEntries,
    ) -> (WalletImportWatchOnlyEntries, Vec<String>) {
        validated_watch_only_entries_on(entries, Default::default())
    }

    fn validated_watch_only_entries_on(
        entries: &WalletImportWatchOnlyEntries,
        networks: ImportNetworks,
    ) -> (WalletImportWatchOnlyEntries, Vec<String>) {
        crate::derivation::import::validated_watch_only_entries(entries, &networks)
    }

    fn entries(slot: &str, addresses: &[&str]) -> WalletImportWatchOnlyEntries {
        WalletImportWatchOnlyEntries {
            by_slot: HashMap::from([(
                slot.to_string(),
                addresses.iter().map(|a| a.to_string()).collect(),
            )]),
            bitcoin_xpub: None,
        }
    }

    #[test]
    fn a_malformed_watch_address_is_refused() {
        let (kept, rejected) = validated_watch_only_entries(&entries("solana", &["garbage"]));
        assert!(kept.by_slot.is_empty(), "kept: {:?}", kept.by_slot);
        assert_eq!(rejected, vec!["garbage".to_string()]);
    }

    #[test]
    fn valid_watch_addresses_survive_and_are_normalised() {
        let (kept, rejected) = validated_watch_only_entries(&entries(
            "ethereum",
            &["0X742D35CC6634C0532925A3B844BC454E4438F44E"],
        ));
        assert!(rejected.is_empty());
        let stored = kept.by_slot.get("ethereum").expect("kept");
        assert_eq!(stored.len(), 1);
        assert!(stored[0].starts_with("0x"));
    }

    /// Core normalises on the way in, so a caller does not have to.
    #[test]
    fn every_slot_normalises_without_help_from_the_caller() {
        let padded = "0x0000000000000000000000000000000000000000000000000000000000000ABC";
        let cases: [(&str, &str, &str); 3] = [
            (
                "ethereum",
                "0x742D35CC6634C0532925A3B844BC454E4438F44E",
                "0x742d35cc6634c0532925a3b844bc454e4438f44e",
            ),
            ("sui", padded, &padded.to_lowercase()),
            ("aptos", padded, &padded.to_lowercase()),
        ];
        for (slot, typed, expected) in cases {
            let (kept, rejected) = validated_watch_only_entries(&entries(slot, &[typed]));
            assert!(rejected.is_empty(), "{slot}: rejected {typed}");
            assert_eq!(
                kept.by_slot.get(slot).map(Vec::as_slice),
                Some([expected.to_string()].as_slice()),
                "{slot} did not normalise"
            );
        }
    }

    /// The import path and the send path must agree on what an address
    /// looks like once normalised.
    ///
    /// They are two separate tables today — `validate_address` matches on
    /// the validation kind, `normalize_address` on the chain display name.
    /// iOS called the second (as `normalizedSendAddress`) before handing
    /// addresses to the first. Deleting those calls is only safe while the
    /// two agree, so this fails if they ever drift apart.
    #[test]
    fn the_send_normaliser_and_the_import_normaliser_agree() {
        use crate::send::flow::normalized_send_address;
        // Internet Computer is absent on purpose: its account identifier
        // carries a CRC32 prefix, so there is no fixture to write here
        // without computing a real one, and a fixture the validator
        // rejects would test nothing.
        let cases: [(&str, &str, &str); 5] = [
            (
                "Ethereum",
                "ethereum",
                "0x742D35CC6634C0532925A3B844BC454E4438F44E",
            ),
            (
                "Sui",
                "sui",
                "0x0000000000000000000000000000000000000000000000000000000000000ABC",
            ),
            (
                "Aptos",
                "aptos",
                "0x0000000000000000000000000000000000000000000000000000000000000ABC",
            ),
            ("NEAR", "near", "Example.NEAR"),
            (
                "Solana",
                "solana",
                "BLeUXTx9thHGT7VJUtF9vHEmfMDgW1nnKZ9UVer2CoLX",
            ),
        ];
        for (chain_name, slot, typed) in cases {
            let (kept, rejected) = validated_watch_only_entries(&entries(slot, &[typed]));
            assert!(rejected.is_empty(), "{chain_name}: rejected {typed}");
            let imported = kept
                .by_slot
                .get(slot)
                .and_then(|list| list.first())
                .unwrap();
            let sent = normalized_send_address(chain_name.to_string(), typed.to_string());
            assert_eq!(
                imported, &sent,
                "{chain_name}: import normalised to {imported}, send to {sent}"
            );
        }
    }

    #[test]
    fn surrounding_whitespace_is_not_the_caller_s_problem_either() {
        let (kept, rejected) = validated_watch_only_entries(&entries(
            "ethereum",
            &["  0x742d35cc6634c0532925a3b844bc454e4438f44e  "],
        ));
        assert!(rejected.is_empty());
        assert_eq!(
            kept.by_slot.get("ethereum").map(Vec::as_slice),
            Some(["0x742d35cc6634c0532925a3b844bc454e4438f44e".to_string()].as_slice())
        );
    }

    /// A testnet address arrives in its mainnet's slot, so validation has
    /// to be told which network the import is for.
    ///
    /// `ImportDraft.watchOnlyInputsByChainName` is keyed by mainnet display
    /// name — there is no "Bitcoin Testnet" row — so a testnet watch import
    /// puts a testnet address in the `bitcoin` slot. Validating that slot as
    /// mainnet refuses a wallet the app has always allowed.
    #[test]
    fn a_testnet_watch_address_survives_when_the_import_is_for_testnet() {
        // tb1 prefix — valid Bitcoin testnet, invalid on mainnet.
        let typed = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";
        let (kept, rejected) = validated_watch_only_entries_on(
            &entries("bitcoin", &[typed]),
            ImportNetworks {
                by_family: std::collections::HashMap::from([(
                    "bitcoin".to_string(),
                    "bitcoin-testnet".to_string(),
                )]),
            },
        );
        assert!(rejected.is_empty(), "testnet address refused: {rejected:?}");
        assert_eq!(kept.by_slot.get("bitcoin").map(Vec::len), Some(1));
    }

    #[test]
    fn a_testnet_watch_address_is_still_refused_on_mainnet() {
        let typed = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";
        let (kept, rejected) = validated_watch_only_entries(&entries("bitcoin", &[typed]));
        assert_eq!(rejected, vec![typed.to_string()]);
        assert!(kept.by_slot.is_empty());
    }

    #[test]
    fn one_bad_address_does_not_discard_the_good_ones() {
        let (kept, rejected) = validated_watch_only_entries(&entries(
            "ethereum",
            &[
                "0X742D35CC6634C0532925A3B844BC454E4438F44E",
                "0xnothex",
                "0x0000000000000000000000000000000000000001",
            ],
        ));
        assert_eq!(kept.by_slot.get("ethereum").expect("kept").len(), 2);
        assert_eq!(rejected, vec!["0xnothex".to_string()]);
    }
}
