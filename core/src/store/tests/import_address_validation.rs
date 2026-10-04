use crate::derivation::import::WalletImportAddresses;

fn validated_addresses(addresses: &WalletImportAddresses) -> (WalletImportAddresses, Vec<String>) {
    crate::derivation::import::validated_addresses(addresses)
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
/// The fixture is the all-uppercase form: no EIP-55 checksum to verify, still
/// valid, and it still demonstrates the normalisation this test is about.
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
fn a_malformed_bitcoin_xpub_is_rejected() {
    let mut input = addresses(&[]);
    input.bitcoin_xpub = Some("zpub-whatever".to_string());
    let (kept, rejected) = validated_addresses(&input);
    assert!(kept.bitcoin_xpub.is_none());
    assert_eq!(rejected, vec!["zpub-whatever"]);
}

#[test]
fn bitcoin_account_xpubs_validate_checksums_payloads_and_public_keys() {
    use crate::derivation::bitcoin::{base58check_decode, base58check_encode};
    const XPUB: &str = "xpub6BemYiVNp19Zz9Bw6kmmfXR2LEFukA1hnhSZrXgE2AUJvNLW8a87gg72bQLi4RfGHcKcR4ojrEFgFJgNCXcjVYSH75YmvhTZ7qh9FCrxv3a";
    let original = base58check_decode(XPUB).unwrap();
    for version in [
        [0x04, 0x88, 0xb2, 0x1e],
        [0x04, 0x9d, 0x7c, 0xb2],
        [0x04, 0xb2, 0x47, 0x46],
        [0x04, 0x35, 0x87, 0xcf],
        [0x04, 0x4a, 0x52, 0x62],
        [0x04, 0x5f, 0x1c, 0xf6],
    ] {
        let mut payload = original.clone();
        payload[..4].copy_from_slice(&version);
        let valid = base58check_encode(&payload);
        let mut input = addresses(&[]);
        input.bitcoin_xpub = Some(format!("  {valid}  "));
        let (kept, rejected) = validated_addresses(&input);
        assert_eq!(kept.bitcoin_xpub.as_deref(), Some(valid.as_str()));
        assert!(rejected.is_empty());
        assert_eq!(
            crate::derivation::xpub_walker::derive_children(&valid, 0, 0, 1)
                .unwrap()
                .len(),
            1,
        );

        let mut bad_checksum = valid.clone().into_bytes();
        let last = bad_checksum.last_mut().unwrap();
        *last = if *last == b'1' { b'2' } else { b'1' };
        let mut bad_key = payload.clone();
        bad_key[45..].fill(0);
        for invalid in [
            String::from_utf8(bad_checksum).unwrap(),
            base58check_encode(&bad_key),
            base58check_encode(&payload[..77]),
        ] {
            input.bitcoin_xpub = Some(invalid.clone());
            let (kept, rejected) = validated_addresses(&input);
            assert!(kept.bitcoin_xpub.is_none(), "accepted {invalid}");
            assert_eq!(rejected, vec![invalid]);
        }
    }
}

/// A derived address is judged by the network that owns its slot.
#[test]
fn a_derived_address_is_judged_by_its_slots_network() {
    let derived = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
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
    use crate::derivation::import::WalletImportWatchOnlyEntries;
    use crate::registry::Chain;
    use std::collections::HashMap;

    fn validated_watch_only_entries(
        entries: &WalletImportWatchOnlyEntries,
    ) -> (WalletImportWatchOnlyEntries, Vec<String>) {
        crate::derivation::import::validated_watch_only_entries(entries)
    }

    fn entries(slot: Chain, addresses: &[&str]) -> WalletImportWatchOnlyEntries {
        WalletImportWatchOnlyEntries {
            by_chain_id: HashMap::from([(slot, addresses.iter().map(|a| a.to_string()).collect())]),
            bitcoin_xpub: None,
        }
    }

    #[test]
    fn a_malformed_watch_address_is_refused() {
        let (kept, rejected) = validated_watch_only_entries(&entries(Chain::Solana, &["garbage"]));
        assert!(kept.by_chain_id.is_empty(), "kept: {:?}", kept.by_chain_id);
        assert_eq!(rejected, vec!["garbage".to_string()]);
    }

    #[test]
    fn valid_watch_addresses_survive_and_are_normalised() {
        let (kept, rejected) = validated_watch_only_entries(&entries(
            crate::registry::Chain::Ethereum,
            &["0X742D35CC6634C0532925A3B844BC454E4438F44E"],
        ));
        assert!(rejected.is_empty());
        let stored = kept
            .by_chain_id
            .get(&crate::registry::Chain::Ethereum)
            .expect("kept");
        assert_eq!(stored.len(), 1);
        assert!(stored[0].starts_with("0x"));
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
            let (kept, rejected) = validated_watch_only_entries(&entries(slot, &[typed]));
            assert!(rejected.is_empty(), "{slot}: rejected {typed}");
            assert_eq!(
                kept.by_chain_id.get(&slot).map(Vec::as_slice),
                Some([expected.to_string()].as_slice()),
                "{slot} did not normalise"
            );
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
            let (kept, rejected) = validated_watch_only_entries(&entries(slot, &[typed]));
            assert!(rejected.is_empty(), "{chain_id}: rejected {typed}");
            let imported = kept
                .by_chain_id
                .get(&slot)
                .and_then(|list| list.first())
                .unwrap();
            let sent = normalized_send_address(chain_id, typed.to_string());
            assert_eq!(
                imported, &sent,
                "{chain_id}: import normalised to {imported}, send to {sent}"
            );
        }
    }

    #[test]
    fn surrounding_whitespace_is_not_the_caller_s_problem_either() {
        let (kept, rejected) = validated_watch_only_entries(&entries(
            crate::registry::Chain::Ethereum,
            &["  0x742d35cc6634c0532925a3b844bc454e4438f44e  "],
        ));
        assert!(rejected.is_empty());
        assert_eq!(
            kept.by_chain_id
                .get(&crate::registry::Chain::Ethereum)
                .map(Vec::as_slice),
            Some(["0x742d35cc6634c0532925a3b844bc454e4438f44e".to_string()].as_slice())
        );
    }

    /// A watched address belongs to its concrete network.
    #[test]
    fn a_testnet_watch_address_is_refused() {
        let typed = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";
        let (kept, rejected) = validated_watch_only_entries(&entries(Chain::Bitcoin, &[typed]));
        assert_eq!(rejected, vec![typed.to_string()]);
        assert!(kept.by_chain_id.is_empty());
        let (kept, rejected) =
            validated_watch_only_entries(&entries(Chain::BitcoinTestnet, &[typed]));
        assert!(rejected.is_empty());
        assert_eq!(
            kept.by_chain_id[&Chain::BitcoinTestnet],
            vec![typed.to_string()]
        );
    }

    #[test]
    fn one_bad_address_does_not_discard_the_good_ones() {
        let (kept, rejected) = validated_watch_only_entries(&entries(
            crate::registry::Chain::Ethereum,
            &[
                "0X742D35CC6634C0532925A3B844BC454E4438F44E",
                "0xnothex",
                "0x0000000000000000000000000000000000000001",
            ],
        ));
        assert_eq!(
            kept.by_chain_id
                .get(&crate::registry::Chain::Ethereum)
                .expect("kept")
                .len(),
            2
        );
        assert_eq!(rejected, vec!["0xnothex".to_string()]);
    }
}

#[test]
fn watch_only_chain_identity_is_not_an_evm_storage_slot() {
    use crate::derivation::import::{WalletImportWatchOnlyEntries, validated_watch_only_entries};
    let address = "0x742d35cc6634c0532925a3b844bc454e4438f44e".to_string();
    let entries = WalletImportWatchOnlyEntries {
        by_chain_id: std::collections::HashMap::from([
            (crate::registry::Chain::Arbitrum, vec![address.clone()]),
            (crate::registry::Chain::Ethereum, vec![address]),
        ]),
        bitcoin_xpub: None,
    };
    let (valid, rejected) = validated_watch_only_entries(&entries);
    assert_eq!(valid.by_chain_id.len(), 2);
    assert_eq!(
        valid.by_chain_id[&crate::registry::Chain::Arbitrum].len(),
        1
    );
    assert_eq!(
        valid.by_chain_id[&crate::registry::Chain::Ethereum].len(),
        1
    );
    assert!(rejected.is_empty());
}
