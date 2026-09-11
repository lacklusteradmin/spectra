use crate::derivation::import::{WalletImportAddresses, WalletImportCommit, WalletImportRequest};
use crate::service::WalletService;
use crate::store::wallet_domain::{
    CoreSeedDerivationPaths, CoreSeedDerivationPreset, CoreWalletDerivationOverrides,
};
use std::collections::HashMap;

// Real addresses: core validates every import address now, so a
// placeholder would simply be dropped.
const BTC: &str = "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu";
const SOL: &str = "11111111111111111111111111111111";
const MNEMONIC: &str = "test test test test test test test test test test test junk";

fn commit(chains: &[&str], addresses: &[(&str, &str)]) -> WalletImportCommit {
    WalletImportCommit {
        request: WalletImportRequest {
            wallet_name: String::new(),
            default_wallet_name_start_index: 1,
            primary_selected_chain_name: chains[0].to_string(),
            selected_chain_names: chains.iter().map(|c| c.to_string()).collect(),
            planned_wallet_ids: (0..chains.len())
                .map(|i| format!("11111111-0000-0000-0000-00000000000{i}"))
                .collect(),
            is_watch_only_import: false,
            is_private_key_import: false,
            has_wallet_password: false,
            resolved_addresses: WalletImportAddresses {
                by_slot: addresses
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                bitcoin_xpub: None,
            },
            watch_only_entries: Default::default(),
        },
        holdings: Vec::new(),
        seed_derivation_preset: CoreSeedDerivationPreset::Standard,
        seed_derivation_paths: CoreSeedDerivationPaths {
            by_chain: HashMap::new(),
            is_custom_enabled: false,
        },
        derivation_overrides: CoreWalletDerivationOverrides::default(),
        network_chain_by_family: Default::default(),
        seed_phrase: None,
        private_key: None,
    }
}

#[tokio::test]
async fn imported_wallets_land_in_core_state() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    let outcome = service
        .import_wallets(commit(&["Solana"], &[("solana", SOL)]))
        .await
        .expect("import");

    assert_eq!(outcome.wallets.len(), 1);
    // The caller does not store anything — core already did.
    let stored = service.wallets_for_display().await.expect("wallets");
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].selected_chain, "Solana");
    assert_eq!(
        stored[0].addresses.get("solana").map(String::as_str),
        Some(SOL)
    );
}

/// A seed import stores an address for every network of the family, so
/// switching to a testnet is a read rather than a derivation.
///
/// The app used to re-derive the testnet address from the seed on every
/// read, which a password-sealed wallet cannot do — it fell back to the
/// mainnet address and showed that on testnet instead. The addresses
/// survive into the stored record, which is what makes the switch work
/// after a restart.
#[tokio::test]
async fn a_seed_import_stores_one_address_per_network_of_its_family() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    let mut commit = commit(&["Bitcoin"], &[]);
    commit.seed_phrase = Some(MNEMONIC.to_string());
    commit.seed_derivation_paths =
        crate::app_core::seed_derivation_paths_for_account(0).expect("default paths");
    service.import_wallets(commit).await.expect("import");

    let stored = service.wallets_for_display().await.expect("wallets");
    assert_eq!(stored.len(), 1);
    let addresses = &stored[0].addresses;
    for network in crate::registry::Chain::Bitcoin.network_choices() {
        let address = addresses
            .get(network.address_slot())
            .unwrap_or_else(|| panic!("no address for {}", network.chain_display_name()));
        let verdict = crate::validation::address::validate_address(
            crate::validation::address::AddressValidationRequest {
                kind: network.address_validation_kind().to_string(),
                value: address.clone(),
            },
        );
        assert!(
            verdict.is_valid,
            "{} stored {address}, which its own validator refuses",
            network.chain_display_name()
        );
    }
    // The networks are different keys, so the mainnet address is not the
    // testnet one.
    assert_ne!(
        addresses.get(crate::registry::Chain::Bitcoin.address_slot()),
        addresses.get(crate::registry::Chain::BitcoinTestnet4.address_slot())
    );
}

#[tokio::test]
async fn a_network_selection_applies_only_to_its_own_family() {
    let service = WalletService::new_typed(Vec::new()).expect("service");
    let mut input = commit(&["Bitcoin", "Solana"], &[("bitcoin", BTC), ("solana", SOL)]);
    input.network_chain_by_family =
        std::collections::HashMap::from([("bitcoin".to_string(), "bitcoin-testnet".to_string())]);
    let outcome = service.import_wallets(input).await.expect("import");

    let by_chain: std::collections::HashMap<_, _> = outcome
        .wallets
        .iter()
        .map(|w| (w.selected_chain.as_str(), w))
        .collect();
    assert_eq!(
        by_chain["Bitcoin"].network_chain_id.as_deref(),
        Some("bitcoin-testnet")
    );
    // Choosing Bitcoin testnet must not drag the Solana wallet with it.
    assert_eq!(by_chain["Solana"].network_chain_id, None);
}
