use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::addressing::{validate_address, AddressValidationRequest};
use crate::registry::Chain;

/// Addresses supplied by a wallet import, keyed by [`Chain::address_slot`].
///
/// Keyed rather than one field per chain: the slot set is derived from
/// `registry::Chain`, so adding a chain is a registry edit and nothing here
/// changes. EVM chains share the `"ethereum"` slot — see `address_slot`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletImportAddresses {
    /// `Chain::address_slot()` → address. Absent slot means "not supplied".
    pub by_slot: HashMap<String, String>,
    /// Bitcoin account xpub/ypub/zpub. Not an address, so it gets its own
    /// field rather than a slot.
    pub bitcoin_xpub: Option<String>,
}

impl WalletImportAddresses {
    fn empty() -> Self {
        Self::default()
    }

    /// One address in one chain's slot.
    fn single(chain: Chain, address: impl Into<String>) -> Self {
        Self {
            by_slot: HashMap::from([(chain.address_slot().to_string(), address.into())]),
            bitcoin_xpub: None,
        }
    }

    /// The address stored for `chain`, if the import supplied one.
    pub fn address_for(&self, chain: Chain) -> Option<&str> {
        self.by_slot.get(chain.address_slot()).map(String::as_str)
    }
}

/// Watch-only address lists, keyed by [`Chain::address_slot`]. A watch-only
/// import can supply several addresses per chain; each becomes one wallet.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletImportWatchOnlyEntries {
    /// `Chain::address_slot()` → addresses, in the order the user entered them.
    pub by_slot: HashMap<String, Vec<String>>,
    pub bitcoin_xpub: Option<String>,
}

impl WalletImportWatchOnlyEntries {
    /// Addresses entered for `chain`, or an empty slice when none were.
    pub fn addresses_for(&self, chain: Chain) -> &[String] {
        self.by_slot
            .get(chain.address_slot())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletImportRequest {
    pub wallet_name: String,
    pub default_wallet_name_start_index: u64,
    pub primary_selected_chain_name: String,
    pub selected_chain_names: Vec<String>,
    pub planned_wallet_ids: Vec<String>,
    pub is_watch_only_import: bool,
    pub is_private_key_import: bool,
    pub has_wallet_password: bool,
    pub resolved_addresses: WalletImportAddresses,
    pub watch_only_entries: WalletImportWatchOnlyEntries,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletSecretInstruction {
    pub wallet_id: String,
    pub secret_kind: String,
    pub should_store_seed_phrase: bool,
    pub should_store_private_key: bool,
    pub should_store_password_verifier: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PlannedWallet {
    pub wallet_id: String,
    pub name: String,
    pub chain_name: String,
    pub addresses: WalletImportAddresses,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletImportPlan {
    pub secret_kind: String,
    pub wallets: Vec<PlannedWallet>,
    pub secret_instructions: Vec<WalletSecretInstruction>,
}

/// Everything core needs to turn an import form into stored wallets.
///
/// The draft fields stay in Swift — they are an in-progress form. What crosses
/// is the resolved outcome: which chains, which addresses, and the derivation
/// settings the wallets are created with.
#[derive(Debug, Clone, uniffi::Record)]
pub struct WalletImportCommit {
    pub request: WalletImportRequest,
    /// Holdings each created wallet starts with, from the chain picker.
    pub holdings: Vec<crate::store::wallet_domain::CoreCoin>,
    pub seed_derivation_preset: crate::store::wallet_domain::CoreSeedDerivationPreset,
    pub seed_derivation_paths: crate::store::wallet_domain::CoreSeedDerivationPaths,
    pub derivation_overrides: crate::store::wallet_domain::CoreWalletDerivationOverrides,
    pub bitcoin_network_mode: crate::store::wallet_domain::CoreBitcoinNetworkMode,
    pub dogecoin_network_mode: crate::store::wallet_domain::CoreDogecoinNetworkMode,
}

/// What an import produced. `secret_instructions` is the only part the caller
/// must still act on — Keychain is platform, so writing the seed phrase stays
/// on the platform side.
#[derive(Debug, Clone, uniffi::Record)]
pub struct WalletImportOutcome {
    pub secret_kind: String,
    pub secret_instructions: Vec<WalletSecretInstruction>,
    pub wallets: Vec<crate::store::wallet_domain::CoreImportedWallet>,
    /// Addresses the import refused, in the form they were supplied.
    ///
    /// Refusals are reported rather than silent. Dropping them quietly means a
    /// watch-only import of one bad address succeeds and stores a wallet with
    /// no address at all, which reads to the user as "imported" — the same
    /// mistake the address book already fixed with `addressBookRejected`.
    pub rejected_addresses: Vec<String>,
}

/// Which network the import's addresses should be validated against.
///
/// Only two chains have a user-selectable network mode, and both put their
/// testnet addresses in the *mainnet* slot: `ImportDraft` is keyed by mainnet
/// display name, so there is no "Bitcoin Testnet" row to carry them. Without
/// this, validating the `bitcoin` slot as mainnet refuses a testnet wallet the
/// app has always allowed.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ImportNetworks {
    pub bitcoin: crate::store::wallet_domain::CoreBitcoinNetworkMode,
    pub dogecoin: crate::store::wallet_domain::CoreDogecoinNetworkMode,
}

/// Keep a Bitcoin account xpub only if it carries a serialization prefix this
/// network uses. `None` in, `None` out.
fn validated_bitcoin_xpub(
    xpub: Option<&String>,
    networks: ImportNetworks,
) -> (Option<String>, Option<String>) {
    let Some(trimmed) = xpub.map(|value| value.trim()).filter(|v| !v.is_empty()) else {
        return (None, None);
    };
    if networks
        .bitcoin_xpub_prefixes()
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
    {
        (Some(trimmed.to_string()), None)
    } else {
        (None, Some(trimmed.to_string()))
    }
}

impl ImportNetworks {
    /// Serialization prefixes a Bitcoin account xpub may carry on this network.
    ///
    /// One per BIP: 44 → `xpub`, 49 → `ypub`, 84 → `zpub`, with the testnet
    /// counterparts. An xpub is not an address, so `validate_address` has
    /// nothing to say about it — but storing an arbitrary string as one means
    /// a watch wallet that derives nothing and shows no address.
    fn bitcoin_xpub_prefixes(self) -> &'static [&'static str] {
        use crate::store::wallet_domain::CoreBitcoinNetworkMode as Btc;
        match self.bitcoin {
            Btc::Mainnet => &["xpub", "ypub", "zpub"],
            Btc::Testnet | Btc::Testnet4 | Btc::Signet => &["tpub", "upub", "vpub"],
        }
    }

    /// The chain whose format answers for `slot` under these network modes.
    fn chain_for(self, slot: &str) -> Option<Chain> {
        use crate::store::wallet_domain::{CoreBitcoinNetworkMode as Btc, CoreDogecoinNetworkMode as Doge};
        if slot == Chain::Bitcoin.address_slot() {
            return Some(match self.bitcoin {
                Btc::Mainnet => Chain::Bitcoin,
                Btc::Testnet => Chain::BitcoinTestnet,
                Btc::Testnet4 => Chain::BitcoinTestnet4,
                Btc::Signet => Chain::BitcoinSignet,
            });
        }
        if slot == Chain::Dogecoin.address_slot() {
            return Some(match self.dogecoin {
                Doge::Mainnet => Chain::Dogecoin,
                Doge::Testnet => Chain::DogecoinTestnet,
            });
        }
        // Every other slot is shared by a chain family (all EVM mainnets use
        // one), so any chain in the family answers for the format.
        Chain::all().find(|chain| chain.address_slot() == slot)
    }
}

/// Validate one address against the chain that owns `slot` on this network.
///
/// `Ok` carries the normalized form to store; `Err` means the address does not
/// parse for that chain.
fn validated_address_in_slot(
    slot: &str,
    address: &str,
    networks: ImportNetworks,
) -> Result<String, ()> {
    let chain = networks.chain_for(slot).ok_or(())?;
    let result = validate_address(AddressValidationRequest {
        kind: chain.address_validation_kind().to_string(),
        value: address.to_string(),
        network_mode: None,
    });
    if result.is_valid {
        Ok(result
            .normalized_value
            .unwrap_or_else(|| address.to_string()))
    } else {
        Err(())
    }
}

/// Drop any address that does not validate for its chain, reporting what was
/// dropped.
///
/// One rule for every chain. The iOS import path applied validation to some
/// chains and not others — twenty-one kept whatever was typed, three required
/// it to parse — which meant an unparseable address could be stored for a
/// wallet depending only on which chain it was. Storing a malformed address is
/// worse than storing none: it renders as the wallet's receive address.
pub(crate) fn validated_addresses(
    addresses: &WalletImportAddresses,
    networks: ImportNetworks,
) -> (WalletImportAddresses, Vec<String>) {
    let mut kept = HashMap::new();
    let mut rejected = Vec::new();
    for (slot, address) in &addresses.by_slot {
        let trimmed = address.trim();
        if trimmed.is_empty() {
            continue;
        }
        match validated_address_in_slot(slot, trimmed, networks) {
            Ok(normalized) => {
                kept.insert(slot.clone(), normalized);
            }
            Err(()) => rejected.push(trimmed.to_string()),
        }
    }
    let (bitcoin_xpub, refused_xpub) =
        validated_bitcoin_xpub(addresses.bitcoin_xpub.as_ref(), networks);
    rejected.extend(refused_xpub);
    (
        WalletImportAddresses {
            by_slot: kept,
            bitcoin_xpub,
        },
        rejected,
    )
}

/// The same rule over the watch-only lists, which are a separate input.
///
/// This is the path that actually needs it. A signing import's address is
/// derived by core and valid by construction; a watch-only import's is typed
/// by the user, and it is the only address the wallet will ever have. Missing
/// it here meant the "every chain is validated" rule covered the path that
/// could not fail and skipped the one that could.
pub(crate) fn validated_watch_only_entries(
    entries: &WalletImportWatchOnlyEntries,
    networks: ImportNetworks,
) -> (WalletImportWatchOnlyEntries, Vec<String>) {
    let mut kept: HashMap<String, Vec<String>> = HashMap::new();
    let mut rejected = Vec::new();
    for (slot, addresses) in &entries.by_slot {
        for address in addresses {
            let trimmed = address.trim();
            if trimmed.is_empty() {
                continue;
            }
            match validated_address_in_slot(slot, trimmed, networks) {
                Ok(normalized) => kept.entry(slot.clone()).or_default().push(normalized),
                Err(()) => rejected.push(trimmed.to_string()),
            }
        }
    }
    let (bitcoin_xpub, refused_xpub) =
        validated_bitcoin_xpub(entries.bitcoin_xpub.as_ref(), networks);
    rejected.extend(refused_xpub);
    (
        WalletImportWatchOnlyEntries {
            by_slot: kept,
            bitcoin_xpub,
        },
        rejected,
    )
}

/// Build the wallets an import plan calls for, without storing them.
///
/// Mirrors what the iOS app used to do by hand after reading the plan.
pub(crate) fn wallets_for_import(
    commit: &WalletImportCommit,
    plan: &WalletImportPlan,
) -> Vec<crate::store::wallet_domain::CoreImportedWallet> {
    use crate::store::wallet_domain::{CoreBitcoinNetworkMode, CoreDogecoinNetworkMode};
    plan.wallets
        .iter()
        .map(|planned| crate::store::wallet_domain::CoreImportedWallet {
            id: planned.wallet_id.clone(),
            name: planned.name.clone(),
            // Network mode applies only to the chain it belongs to; every other
            // wallet is mainnet regardless of what the importer had selected.
            bitcoin_network_mode: if planned.chain_name == "Bitcoin" {
                commit.bitcoin_network_mode
            } else {
                CoreBitcoinNetworkMode::Mainnet
            },
            dogecoin_network_mode: if planned.chain_name == "Dogecoin" {
                commit.dogecoin_network_mode
            } else {
                CoreDogecoinNetworkMode::Mainnet
            },
            addresses: planned.addresses.by_slot.clone(),
            bitcoin_xpub: if planned.chain_name == "Bitcoin" {
                planned.addresses.bitcoin_xpub.clone()
            } else {
                None
            },
            seed_derivation_preset: commit.seed_derivation_preset,
            seed_derivation_paths: commit.seed_derivation_paths.clone(),
            derivation_overrides: commit.derivation_overrides.clone(),
            selected_chain: planned.chain_name.clone(),
            holdings: commit.holdings.clone(),
            include_in_portfolio_total: true,
        })
        .collect()
}

#[uniffi::export]
pub fn core_validate_wallet_import_draft(request: WalletImportDraftValidationRequest) -> bool {
    validate_wallet_import_draft(request)
}

/// Key under which a chain's address belongs in [`WalletImportAddresses`] and
/// [`WalletImportWatchOnlyEntries`].
///
/// Exported so the UI never hardcodes slot keys or has to know that EVM chains
/// share one. Pass a chain display name; unknown names return an empty string.
#[uniffi::export]
pub fn core_address_slot(chain_name: String) -> String {
    Chain::from_display_name(&chain_name)
        .map(|chain| chain.address_slot().to_string())
        .unwrap_or_default()
}

/// `true` when a chain can be imported watch-only from an address alone.
/// Monero cannot — watching it needs the private view key.
#[uniffi::export]
pub fn core_supports_watch_only_import(chain_name: String) -> bool {
    Chain::from_display_name(&chain_name)
        .map(Chain::supports_watch_only_import)
        .unwrap_or(false)
}

pub fn plan_wallet_import(request: WalletImportRequest) -> Result<WalletImportPlan, String> {
    if request.is_watch_only_import {
        plan_watch_only_import(request)
    } else {
        plan_signing_import(request)
    }
}

fn plan_signing_import(request: WalletImportRequest) -> Result<WalletImportPlan, String> {
    if request.selected_chain_names.is_empty() {
        return Err("Select a chain first.".to_string());
    }
    if request.selected_chain_names.len() != request.planned_wallet_ids.len() {
        return Err("Wallet ID plan did not match selected chains.".to_string());
    }

    let selected_chain_count = request.selected_chain_names.len();
    let mut wallets = Vec::with_capacity(selected_chain_count);
    let mut secret_instructions = Vec::with_capacity(selected_chain_count);
    let secret_kind = if request.is_private_key_import {
        "privateKey"
    } else {
        "seedPhrase"
    };

    for (index, (chain_name, wallet_id)) in request
        .selected_chain_names
        .iter()
        .zip(request.planned_wallet_ids.iter())
        .enumerate()
    {
        wallets.push(PlannedWallet {
            wallet_id: wallet_id.clone(),
            name: wallet_display_name(
                &request.wallet_name,
                index + 1,
                request.default_wallet_name_start_index as usize + index,
                selected_chain_count,
            ),
            chain_name: chain_name.clone(),
            addresses: addresses_for_chain(chain_name, &request.resolved_addresses),
        });
        secret_instructions.push(WalletSecretInstruction {
            wallet_id: wallet_id.clone(),
            secret_kind: secret_kind.to_string(),
            should_store_seed_phrase: !request.is_private_key_import,
            should_store_private_key: request.is_private_key_import,
            should_store_password_verifier: !request.is_private_key_import
                && request.has_wallet_password,
        });
    }

    Ok(WalletImportPlan {
        secret_kind: secret_kind.to_string(),
        wallets,
        secret_instructions,
    })
}

fn plan_watch_only_import(request: WalletImportRequest) -> Result<WalletImportPlan, String> {
    let watch_entries = watch_only_addresses_for_chain(
        &request.primary_selected_chain_name,
        &request.watch_only_entries,
    )?;
    if watch_entries.is_empty() {
        return Err("Enter at least one valid address to import.".to_string());
    }
    if request.planned_wallet_ids.len() != watch_entries.len() {
        return Err("Watch-only wallet ID plan did not match expanded requests.".to_string());
    }

    let selected_chain_count = watch_entries.len();
    let wallets = watch_entries
        .into_iter()
        .zip(request.planned_wallet_ids.iter())
        .enumerate()
        .map(
            |(index, ((chain_name, addresses), wallet_id))| PlannedWallet {
                wallet_id: wallet_id.clone(),
                name: wallet_display_name(
                    &request.wallet_name,
                    index + 1,
                    (request.default_wallet_name_start_index as usize) + index,
                    selected_chain_count,
                ),
                chain_name,
                addresses,
            },
        )
        .collect::<Vec<_>>();
    let secret_instructions = request
        .planned_wallet_ids
        .into_iter()
        .map(|wallet_id| WalletSecretInstruction {
            wallet_id,
            secret_kind: "watchOnly".to_string(),
            should_store_seed_phrase: false,
            should_store_private_key: false,
            should_store_password_verifier: false,
        })
        .collect::<Vec<_>>();

    Ok(WalletImportPlan {
        secret_kind: "watchOnly".to_string(),
        wallets,
        secret_instructions,
    })
}

fn watch_only_addresses_for_chain(
    primary_chain_name: &str,
    entries: &WalletImportWatchOnlyEntries,
) -> Result<Vec<(String, WalletImportAddresses)>, String> {
    let unsupported =
        || format!("Watch-only planning is not available for chain: {primary_chain_name}");
    let chain = Chain::from_display_name(primary_chain_name).ok_or_else(unsupported)?;
    if !chain.supports_watch_only_import() {
        return Err(unsupported());
    }

    // Bitcoin has a second form: one xpub stands in for the whole account, so
    // it plans a single wallet instead of one per address.
    if chain == Chain::Bitcoin {
        if let Some(xpub) = trim_optional(entries.bitcoin_xpub.as_deref()) {
            return Ok(vec![(
                primary_chain_name.to_string(),
                WalletImportAddresses {
                    by_slot: HashMap::new(),
                    bitcoin_xpub: Some(xpub.to_string()),
                },
            )]);
        }
    }

    Ok(entries
        .addresses_for(chain)
        .iter()
        .map(|address| {
            (
                primary_chain_name.to_string(),
                WalletImportAddresses::single(chain, address.clone()),
            )
        })
        .collect())
}

/// The address slots a wallet on `chain_name` should carry.
///
/// A wallet is per-chain, so it takes only the slots its own chain reads.
/// Bitcoin additionally carries the account xpub when one was supplied.
fn addresses_for_chain(
    chain_name: &str,
    addresses: &WalletImportAddresses,
) -> WalletImportAddresses {
    let Some(chain) = Chain::from_display_name(chain_name) else {
        return WalletImportAddresses::empty();
    };

    let mut by_slot = HashMap::new();
    if let Some(address) = addresses.address_for(chain) {
        by_slot.insert(chain.address_slot().to_string(), address.to_string());
    }
    // Ethereum Classic is EVM-shaped but has its own slot, and downstream code
    // reads the generic `"ethereum"` slot for any EVM wallet. Fill both so an
    // ETC wallet resolves either way.
    if chain == Chain::EthereumClassic {
        if let Some(address) = addresses.address_for(Chain::EthereumClassic) {
            by_slot.insert(Chain::Ethereum.address_slot().to_string(), address.to_string());
        }
    }

    WalletImportAddresses {
        by_slot,
        bitcoin_xpub: if chain == Chain::Bitcoin {
            addresses.bitcoin_xpub.clone()
        } else {
            None
        },
    }
}


fn wallet_display_name(
    base_name: &str,
    batch_position: usize,
    default_wallet_index: usize,
    selected_chain_count: usize,
) -> String {
    let trimmed = base_name.trim();
    if trimmed.is_empty() {
        return format!("Wallet {}", default_wallet_index);
    }
    if selected_chain_count > 1 {
        format!("{trimmed} {batch_position}")
    } else {
        trimmed.to_string()
    }
}

fn trim_optional(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

// ── Import draft validation (replaces Swift-side canImportWallet) ──

#[derive(Debug, Clone, uniffi::Record)]
pub struct WalletImportDraftValidationRequest {
    pub selected_chain_names: Vec<String>,
    pub is_watch_only: bool,
    pub is_private_key_import: bool,
    pub is_editing: bool,
    pub is_create_mode: bool,
    pub has_valid_wallet_name: bool,
    pub has_valid_seed_phrase: bool,
    pub has_valid_private_key_hex: bool,
    pub is_backup_verification_complete: bool,
    pub requires_backup_verification: bool,
    pub watch_only_entries: WalletImportWatchOnlyEntries,
}

pub fn validate_wallet_import_draft(request: WalletImportDraftValidationRequest) -> bool {
    if request.is_editing {
        return request.has_valid_wallet_name;
    }

    let has_chains = !request.selected_chain_names.is_empty();

    if request.is_create_mode {
        return has_chains
            && request.has_valid_seed_phrase
            && request.is_backup_verification_complete;
    }

    // Mode compatibility
    if request.is_watch_only && request.selected_chain_names.iter().any(|n| n == "Monero") {
        return false;
    }
    if request.is_private_key_import {
        if request.selected_chain_names.len() != 1 {
            return false;
        }
        if request
            .selected_chain_names
            .iter()
            .any(|n| !is_private_key_chain_supported(n))
        {
            return false;
        }
    }
    if request.is_watch_only && request.selected_chain_names.len() != 1 {
        return false;
    }

    // Watch-only address validation
    if request.is_watch_only
        && !validate_watch_only_draft_addresses(
            &request.selected_chain_names,
            &request.watch_only_entries,
        )
    {
        return false;
    }

    // Secret validation
    if !request.is_watch_only && !request.is_private_key_import && !request.has_valid_seed_phrase {
        return false;
    }
    if request.is_private_key_import && !request.has_valid_private_key_hex {
        return false;
    }

    let is_backup_verified = request.is_watch_only
        || !request.requires_backup_verification
        || request.is_backup_verification_complete;

    has_chains && is_backup_verified
}

const PRIVATE_KEY_SUPPORTED_CHAINS: &[&str] = &[
    "Bitcoin",
    "Bitcoin Cash",
    "Bitcoin SV",
    "Litecoin",
    "Dogecoin",
    "Ethereum",
    "Ethereum Classic",
    "Arbitrum",
    "Optimism",
    "BNB Chain",
    "Avalanche",
    "Hyperliquid",
    "Tron",
    "Solana",
    "Cardano",
    "Stellar",
    "XRP Ledger",
    "Sui",
    "Aptos",
    "TON",
    "Internet Computer",
    "NEAR",
    "Polkadot",
    "Zcash",
    "Bitcoin Gold",
    "Decred",
    "Kaspa",
    "Dash",
    "X Layer",
    "Bittensor",
    "Sei",
    "Celo",
    "Cronos",
    "opBNB",
    "zkSync Era",
    "Sonic",
    "Berachain",
    "Unichain",
    "Ink",
];

fn is_private_key_chain_supported(chain_name: &str) -> bool {
    PRIVATE_KEY_SUPPORTED_CHAINS.contains(&chain_name)
}

/// Returns the ordered list of chain display names that support private-key
/// import. Used by both iOS and (eventually) Android to gate the PK import
/// flow — keeping the list here means adding a new chain is one edit.
#[uniffi::export]
pub fn core_supported_private_key_chain_names() -> Vec<String> {
    PRIVATE_KEY_SUPPORTED_CHAINS
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn validate_watch_only_draft_addresses(
    selected_chains: &[String],
    entries: &WalletImportWatchOnlyEntries,
) -> bool {
    for chain_name in selected_chains {
        // Unknown chain, or one that cannot be watched from an address alone
        // (Monero needs a view key) — the draft is not importable.
        let Some(chain) = Chain::from_display_name(chain_name) else {
            return false;
        };
        if !chain.supports_watch_only_import() {
            return false;
        }
        let kind = chain.address_validation_kind();
        let addresses = entries.addresses_for(chain);
        let xpub_fallback = if chain == Chain::Bitcoin {
            entries.bitcoin_xpub.as_deref()
        } else {
            None
        };

        // Must have at least one address (or xpub for Bitcoin)
        let has_xpub = xpub_fallback
            .map(|x| {
                let t = x.trim();
                t.starts_with("xpub") || t.starts_with("ypub") || t.starts_with("zpub")
            })
            .unwrap_or(false);

        if addresses.is_empty() && !has_xpub {
            return false;
        }

        // Validate each address (Bitcoin xpub skips per-address validation).
        // Watch-only Bitcoin imports tolerate either network family because
        // the user may type a testnet address against the mainnet "Bitcoin"
        // chain row and we still want to accept it for mistake-resilience.
        if !has_xpub || !addresses.is_empty() {
            for addr in addresses {
                let result = validate_address(AddressValidationRequest {
                    kind: kind.to_string(),
                    value: addr.clone(),
                    network_mode: None,
                });
                if !result.is_valid {
                    if kind == "bitcoin" {
                        let testnet = validate_address(AddressValidationRequest {
                            kind: "bitcoinTestnet".to_string(),
                            value: addr.clone(),
                            network_mode: None,
                        });
                        if !testnet.is_valid {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots(addresses: &WalletImportAddresses) -> Vec<(String, String)> {
        let mut pairs: Vec<_> = addresses
            .by_slot
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        pairs.sort();
        pairs
    }

    #[test]
    fn plans_multi_chain_seed_import() {
        let plan = plan_wallet_import(WalletImportRequest {
            wallet_name: "Main".to_string(),
            default_wallet_name_start_index: 4,
            primary_selected_chain_name: "Bitcoin".to_string(),
            selected_chain_names: vec!["Bitcoin".to_string(), "Ethereum".to_string()],
            planned_wallet_ids: vec!["1".to_string(), "2".to_string()],
            is_watch_only_import: false,
            is_private_key_import: false,
            has_wallet_password: true,
            resolved_addresses: WalletImportAddresses {
                by_slot: HashMap::from([
                    ("bitcoin".to_string(), "bc1qexample".to_string()),
                    ("ethereum".to_string(), "0x1234".to_string()),
                    ("ethereum-classic".to_string(), "0x5678".to_string()),
                ]),
                bitcoin_xpub: None,
            },
            watch_only_entries: WalletImportWatchOnlyEntries::default(),
        })
        .expect("plan");

        assert_eq!(plan.wallets.len(), 2);
        assert_eq!(plan.wallets[0].name, "Main 1");
        assert_eq!(plan.secret_instructions[0].secret_kind, "seedPhrase");
        // Each wallet carries only its own chain's slot — the Bitcoin wallet
        // must not receive the Ethereum address.
        assert_eq!(
            slots(&plan.wallets[0].addresses),
            vec![("bitcoin".to_string(), "bc1qexample".to_string())]
        );
        assert_eq!(
            slots(&plan.wallets[1].addresses),
            vec![("ethereum".to_string(), "0x1234".to_string())]
        );
    }

    #[test]
    fn evm_chains_share_one_address_slot() {
        let request = |chain: &str| WalletImportRequest {
            wallet_name: "W".to_string(),
            default_wallet_name_start_index: 0,
            primary_selected_chain_name: chain.to_string(),
            selected_chain_names: vec![chain.to_string()],
            planned_wallet_ids: vec!["1".to_string()],
            is_watch_only_import: false,
            is_private_key_import: false,
            has_wallet_password: false,
            resolved_addresses: WalletImportAddresses {
                by_slot: HashMap::from([("ethereum".to_string(), "0xabc".to_string())]),
                bitcoin_xpub: None,
            },
            watch_only_entries: WalletImportWatchOnlyEntries::default(),
        };

        // Including chains the old hand-written match never listed.
        for chain in ["Ethereum", "Arbitrum", "Base", "Polygon", "Ink", "X Layer"] {
            let plan = plan_wallet_import(request(chain)).expect("plan");
            assert_eq!(
                plan.wallets[0].addresses.by_slot.get("ethereum").map(String::as_str),
                Some("0xabc"),
                "{chain} should read the shared ethereum slot"
            );
        }
    }

    #[test]
    fn ethereum_classic_fills_both_its_own_slot_and_the_evm_slot() {
        let plan = plan_wallet_import(WalletImportRequest {
            wallet_name: "W".to_string(),
            default_wallet_name_start_index: 0,
            primary_selected_chain_name: "Ethereum Classic".to_string(),
            selected_chain_names: vec!["Ethereum Classic".to_string()],
            planned_wallet_ids: vec!["1".to_string()],
            is_watch_only_import: false,
            is_private_key_import: false,
            has_wallet_password: false,
            resolved_addresses: WalletImportAddresses {
                by_slot: HashMap::from([
                    ("ethereum".to_string(), "0xmainnet".to_string()),
                    ("ethereum-classic".to_string(), "0xclassic".to_string()),
                ]),
                bitcoin_xpub: None,
            },
            watch_only_entries: WalletImportWatchOnlyEntries::default(),
        })
        .expect("plan");

        // Both slots carry the *ETC* address; the plain Ethereum address must
        // not leak into an ETC wallet.
        assert_eq!(
            slots(&plan.wallets[0].addresses),
            vec![
                ("ethereum".to_string(), "0xclassic".to_string()),
                ("ethereum-classic".to_string(), "0xclassic".to_string()),
            ]
        );
    }

    #[test]
    fn seed_import_carries_bitcoin_xpub_only_on_the_bitcoin_wallet() {
        let plan = plan_wallet_import(WalletImportRequest {
            wallet_name: "Main".to_string(),
            default_wallet_name_start_index: 0,
            primary_selected_chain_name: "Bitcoin".to_string(),
            selected_chain_names: vec!["Bitcoin".to_string(), "Solana".to_string()],
            planned_wallet_ids: vec!["1".to_string(), "2".to_string()],
            is_watch_only_import: false,
            is_private_key_import: false,
            has_wallet_password: false,
            resolved_addresses: WalletImportAddresses {
                by_slot: HashMap::from([
                    ("bitcoin".to_string(), "bc1qexample".to_string()),
                    ("solana".to_string(), "SoLaNa".to_string()),
                ]),
                bitcoin_xpub: Some("zpub999".to_string()),
            },
            watch_only_entries: WalletImportWatchOnlyEntries::default(),
        })
        .expect("plan");

        assert_eq!(
            plan.wallets[0].addresses.bitcoin_xpub.as_deref(),
            Some("zpub999")
        );
        assert_eq!(plan.wallets[1].addresses.bitcoin_xpub, None);
    }

    #[test]
    fn plans_watch_only_bitcoin_xpub_import() {
        let plan = plan_wallet_import(WalletImportRequest {
            wallet_name: String::new(),
            default_wallet_name_start_index: 7,
            primary_selected_chain_name: "Bitcoin".to_string(),
            selected_chain_names: vec!["Bitcoin".to_string()],
            planned_wallet_ids: vec!["watch-1".to_string()],
            is_watch_only_import: true,
            is_private_key_import: false,
            has_wallet_password: false,
            resolved_addresses: WalletImportAddresses::empty(),
            watch_only_entries: WalletImportWatchOnlyEntries {
                by_slot: HashMap::new(),
                bitcoin_xpub: Some("xpub123".to_string()),
            },
        })
        .expect("plan");

        assert_eq!(plan.wallets.len(), 1);
        assert_eq!(plan.wallets[0].name, "Wallet 7");
        assert_eq!(
            plan.wallets[0].addresses.bitcoin_xpub.as_deref(),
            Some("xpub123")
        );
        assert_eq!(plan.secret_kind, "watchOnly");
    }

    #[test]
    fn watch_only_expands_one_wallet_per_address() {
        let plan = plan_wallet_import(WalletImportRequest {
            wallet_name: "Watch".to_string(),
            default_wallet_name_start_index: 0,
            primary_selected_chain_name: "Solana".to_string(),
            selected_chain_names: vec!["Solana".to_string()],
            planned_wallet_ids: vec!["a".to_string(), "b".to_string()],
            is_watch_only_import: true,
            is_private_key_import: false,
            has_wallet_password: false,
            resolved_addresses: WalletImportAddresses::empty(),
            watch_only_entries: WalletImportWatchOnlyEntries {
                by_slot: HashMap::from([(
                    "solana".to_string(),
                    vec!["addr1".to_string(), "addr2".to_string()],
                )]),
                bitcoin_xpub: None,
            },
        })
        .expect("plan");

        assert_eq!(plan.wallets.len(), 2);
        assert_eq!(
            plan.wallets[0].addresses.by_slot.get("solana").map(String::as_str),
            Some("addr1")
        );
        assert_eq!(
            plan.wallets[1].addresses.by_slot.get("solana").map(String::as_str),
            Some("addr2")
        );
    }

    #[test]
    fn watch_only_rejects_chains_that_need_more_than_an_address() {
        let plan = plan_wallet_import(WalletImportRequest {
            wallet_name: "Watch".to_string(),
            default_wallet_name_start_index: 0,
            primary_selected_chain_name: "Monero".to_string(),
            selected_chain_names: vec!["Monero".to_string()],
            planned_wallet_ids: vec!["a".to_string()],
            is_watch_only_import: true,
            is_private_key_import: false,
            has_wallet_password: false,
            resolved_addresses: WalletImportAddresses::empty(),
            watch_only_entries: WalletImportWatchOnlyEntries {
                by_slot: HashMap::from([("monero".to_string(), vec!["4addr".to_string()])]),
                bitcoin_xpub: None,
            },
        });

        // Monero watch-only needs a view key, so an address alone is refused.
        assert!(plan.is_err());
        assert!(plan.unwrap_err().contains("not available"));
    }

    #[test]
    fn unknown_chain_is_not_importable_watch_only() {
        let plan = plan_wallet_import(WalletImportRequest {
            wallet_name: "Watch".to_string(),
            default_wallet_name_start_index: 0,
            primary_selected_chain_name: "Nonexistent Chain".to_string(),
            selected_chain_names: vec!["Nonexistent Chain".to_string()],
            planned_wallet_ids: vec!["a".to_string()],
            is_watch_only_import: true,
            is_private_key_import: false,
            has_wallet_password: false,
            resolved_addresses: WalletImportAddresses::empty(),
            watch_only_entries: WalletImportWatchOnlyEntries::default(),
        });
        assert!(plan.is_err());
    }

    #[test]
    fn testnet_chains_do_not_borrow_their_mainnet_address() {
        // A Bitcoin testnet address is not a Bitcoin address, so a testnet
        // chain must not resolve into the mainnet slot.
        assert_ne!(
            Chain::BitcoinTestnet.address_slot(),
            Chain::Bitcoin.address_slot()
        );
        let addresses = WalletImportAddresses {
            by_slot: HashMap::from([("bitcoin".to_string(), "bc1qexample".to_string())]),
            bitcoin_xpub: None,
        };
        assert_eq!(addresses.address_for(Chain::BitcoinTestnet), None);
        assert_eq!(addresses.address_for(Chain::Bitcoin), Some("bc1qexample"));
    }
}
