use crate::derivation::error::DerivationError;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::registry::Chain;
use crate::validation::address::{AddressValidationRequest, validate_address};

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

/// Watch-only address lists, keyed by concrete chain id. A watch-only import
/// can supply several addresses per chain; each becomes one wallet.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletImportWatchOnlyEntries {
    /// Chain → addresses, in the order the user entered them.
    pub by_chain_id: HashMap<Chain, Vec<String>>,
    pub bitcoin_xpub: Option<String>,
}

impl WalletImportWatchOnlyEntries {
    /// Addresses entered for `chain`, or an empty slice when none were.
    pub fn addresses_for(&self, chain: Chain) -> &[String] {
        self.by_chain_id
            .get(&chain)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

/// What a front end asks an import for: the chains, the kind of import, and
/// the addresses a watch-only import was typed with.
///
/// It also carried a primary chain (always the first selected), wallet ids
/// (always empty — core mints them), a password flag (overwritten from the
/// password on the commit) and resolved addresses (overwritten by core's own
/// derivation, and empty from every caller). Those are the planner's inputs,
/// not the caller's, and live on [`WalletImportPlanRequest`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletImportRequest {
    pub wallet_name: String,
    pub selected_chain_ids: Vec<Chain>,
    pub is_watch_only_import: bool,
    pub is_private_key_import: bool,
    pub watch_only_entries: WalletImportWatchOnlyEntries,
}

/// The planner's input: a request, plus what core resolved for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletImportPlanRequest {
    pub wallet_name: String,
    pub selected_chain_ids: Vec<Chain>,
    /// Ids for the wallets this import will create, or empty to mint them.
    pub planned_wallet_ids: Vec<String>,
    pub is_watch_only_import: bool,
    pub is_private_key_import: bool,
    pub has_wallet_password: bool,
    pub resolved_addresses: WalletImportAddresses,
    pub watch_only_entries: WalletImportWatchOnlyEntries,
}

impl WalletImportPlanRequest {
    pub fn new(
        request: WalletImportRequest,
        resolved_addresses: WalletImportAddresses,
        has_wallet_password: bool,
    ) -> Self {
        Self {
            wallet_name: request.wallet_name,
            selected_chain_ids: request.selected_chain_ids,
            planned_wallet_ids: Vec::new(),
            is_watch_only_import: request.is_watch_only_import,
            is_private_key_import: request.is_private_key_import,
            has_wallet_password,
            resolved_addresses,
            watch_only_entries: request.watch_only_entries,
        }
    }
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
    pub chain_id: crate::registry::Chain,
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
    pub password: Option<String>,
    pub request: WalletImportRequest,
    pub seed_derivation_preset: crate::store::wallet_domain::CoreSeedDerivationPreset,
    pub seed_derivation_paths: crate::store::wallet_domain::CoreSeedDerivationPaths,
    pub derivation_overrides: crate::store::wallet_domain::CoreWalletDerivationOverrides,
    /// Seed used to derive selected chains when `resolved_addresses` is empty.
    pub seed_phrase: Option<String>,
    /// The key to derive a private-key import's address from, when the caller
    /// has not derived it itself.
    ///
    /// The same rule as `seed_phrase`, for the one import path that still had
    /// both front ends deriving first and passing the result over. A key that
    /// derives no address must not reach a sealed wallet, and that refusal
    /// belongs beside the derivation, not in each caller.
    pub private_key: Option<String>,
}

impl WalletImportCommit {
    /// What the imported wallets sign with. Only `None` stores the material
    /// unsealed; a blank password is refused before this is asked, which is
    /// the rule `store_seed_phrase` applies.
    pub fn signing(&self) -> crate::store::state::WalletSigning {
        use crate::store::state::WalletSigning;
        let password_protected = self.password.is_some();
        if self.request.is_watch_only_import {
            WalletSigning::WatchOnly
        } else if self.request.is_private_key_import {
            WalletSigning::PrivateKey { password_protected }
        } else {
            WalletSigning::SeedPhrase { password_protected }
        }
    }
}

/// The address a private-key import stores on its one chain.
///
/// A chain with no private-key derivation refuses here, before the key is
/// sealed, rather than storing a wallet that could never sign with it.
pub fn derive_private_key_import_address(
    private_key: &str,
    chain: Chain,
) -> Result<std::collections::HashMap<Chain, String>, DerivationError> {
    let address = crate::derivation::dispatch::derive_from_private_key(
        chain,
        private_key.trim().trim_start_matches("0x").to_string(),
        true,
        false,
    )
    .map_err(DerivationError::invalid)?
    .and_then(|result| result.address)
    .ok_or_else(|| {
        DerivationError::refused(
            "%@ cannot derive an address from a private key.",
            [chain.chain_display_name()],
        )
    })?;
    Ok(std::iter::once((chain, address)).collect())
}

/// Derive an address for every selected chain, keyed by chain.
///
/// The path comes from `CoreSeedDerivationPaths::path_for` for the concrete
/// network. A chain whose derivation fails is left out here; the planner
/// refuses an import that leaves any selected chain without an address.
pub fn derive_import_addresses(
    seed_phrase: &str,
    selected_chain_ids: &[Chain],
    paths: &crate::store::wallet_domain::CoreSeedDerivationPaths,
    overrides: &crate::store::wallet_domain::CoreWalletDerivationOverrides,
) -> std::collections::HashMap<Chain, String> {
    let mut by_chain_id = std::collections::HashMap::new();
    // Every EVM chain's address is derived under Ethereum's entry, because the
    // family shares one address slot. Including Ethereum when only an L2 was
    // selected is what makes that slot get filled.
    let mut chains = selected_chain_ids.to_vec();
    if chains.iter().any(|c| c.is_evm()) && !chains.contains(&Chain::Ethereum) {
        chains.push(Chain::Ethereum);
    }
    for chain in chains {
        let path = match paths.path_for(chain) {
            Some(path) => path,
            None if !chain.uses_derivation_path() => "",
            None => continue,
        };
        let derived = crate::derivation::dispatch::derive_for_chain(
            chain,
            seed_phrase,
            path,
            overrides.passphrase.as_deref(),
            overrides.hmac_key.as_deref(),
            None,
            true,
            false,
            false,
        );
        if let Ok(result) = derived
            && let Some(address) = result.address
        {
            by_chain_id.insert(chain, address);
        }
    }
    by_chain_id
}

/// The committed wallets. SecretStore writes and rollback belong to core.
#[derive(Debug, Clone, uniffi::Record)]
pub struct WalletImportOutcome {
    pub secret_kind: String,
    pub wallets: Vec<crate::store::wallet_domain::WalletView>,
    /// Addresses the import refused, in the form they were supplied.
    ///
    /// Refusals are reported rather than silent. Dropping them quietly means a
    /// watch-only import of one bad address succeeds and stores a wallet with
    /// no address at all, which reads to the user as "imported" — the same
    /// mistake the address book already fixed with `addressBookRejected`.
    pub rejected_addresses: Vec<String>,
}

/// Keep a Bitcoin account xpub only if it carries a mainnet serialization
/// prefix: one per BIP, 44 → `xpub`, 49 → `ypub`, 84 → `zpub`. Only Bitcoin
/// mainnet takes an account xpub (`Chain::accepts_account_xpub`). An xpub is
/// not an address, so `validate_address` has nothing to say about it — but
/// storing an arbitrary string as one means a watch wallet that derives
/// nothing and shows no address. `None` in, `None` out.
fn validated_bitcoin_xpub(xpub: Option<&String>) -> (Option<String>, Option<String>) {
    let Some(trimmed) = xpub.map(|value| value.trim()).filter(|v| !v.is_empty()) else {
        return (None, None);
    };
    if ["xpub", "ypub", "zpub"]
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
    {
        (Some(trimmed.to_string()), None)
    } else {
        (None, Some(trimmed.to_string()))
    }
}

/// Validate one address as `chain`'s, returning the normalized form to store,
/// or `None` when it does not parse for that chain.
fn normalized_import_address(chain: Chain, address: &str) -> Option<String> {
    let result = validate_address(AddressValidationRequest {
        kind: chain.address_validation_kind().to_string(),
        value: address.to_string(),
    });
    result.is_valid.then(|| {
        result
            .normalized_value
            .unwrap_or_else(|| address.to_string())
    })
}

/// Whether a watch-only import on `chain` would keep `address`: the rule
/// `validated_watch_only_entries` applies, for a form to judge each line by.
#[uniffi::export]
pub fn is_valid_watch_only_address(chain: Chain, address: String) -> bool {
    chain.supports_watch_only_import() && normalized_import_address(chain, address.trim()).is_some()
}

/// Validate one address against the chain that owns `slot`.
///
/// `Ok` carries the normalized form to store; `Err` means the address does not
/// parse for that chain.
fn validated_address_in_slot(slot: &str, address: &str) -> Result<String, ()> {
    let chain = Chain::all()
        .find(|chain| chain.address_slot() == slot)
        .ok_or(())?;
    normalized_import_address(chain, address).ok_or(())
}

/// Drop any address that does not validate for its chain, reporting what was
/// dropped.
///
/// One rule for every chain. Storing a malformed address is worse than
/// storing none: it renders as the wallet's receive address.
pub(crate) fn validated_addresses(
    addresses: &WalletImportAddresses,
) -> (WalletImportAddresses, Vec<String>) {
    let mut kept = HashMap::new();
    let mut rejected = Vec::new();
    for (slot, address) in &addresses.by_slot {
        let trimmed = address.trim();
        if trimmed.is_empty() {
            continue;
        }
        match validated_address_in_slot(slot, trimmed) {
            Ok(normalized) => {
                kept.insert(slot.clone(), normalized);
            }
            Err(()) => rejected.push(trimmed.to_string()),
        }
    }
    let (bitcoin_xpub, refused_xpub) = validated_bitcoin_xpub(addresses.bitcoin_xpub.as_ref());
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
) -> (WalletImportWatchOnlyEntries, Vec<String>) {
    let mut kept: HashMap<Chain, Vec<String>> = HashMap::new();
    let mut rejected = Vec::new();
    for (&chain, addresses) in &entries.by_chain_id {
        for address in addresses {
            let trimmed = address.trim();
            if trimmed.is_empty() {
                continue;
            }
            let normalized = Some(chain)
                .filter(|chain| chain.supports_watch_only_import())
                .and_then(|chain| normalized_import_address(chain, trimmed));
            match normalized {
                Some(normalized) => kept.entry(chain).or_default().push(normalized),
                None => rejected.push(trimmed.to_string()),
            }
        }
    }
    let (bitcoin_xpub, refused_xpub) = validated_bitcoin_xpub(entries.bitcoin_xpub.as_ref());
    rejected.extend(refused_xpub);
    (
        WalletImportWatchOnlyEntries {
            by_chain_id: kept,
            bitcoin_xpub,
        },
        rejected,
    )
}

/// Build imported wallets without storing them. Each wallet is on the network
/// the import selected for it, for good: there is no switching it later.
pub(crate) fn wallets_for_import(
    commit: &WalletImportCommit,
    plan: &WalletImportPlan,
) -> Vec<crate::store::wallet_domain::WalletView> {
    plan.wallets
        .iter()
        .map(|planned| {
            let network = planned.chain_id;
            crate::store::wallet_domain::WalletView {
                id: planned.wallet_id.clone(),
                name: planned.name.clone(),
                chain_id: network,
                addresses: planned.addresses.by_slot.clone(),
                bitcoin_xpub: if planned.chain_id.accepts_account_xpub() {
                    planned.addresses.bitcoin_xpub.clone()
                } else {
                    None
                },
                seed_derivation_preset: commit.seed_derivation_preset,
                seed_derivation_paths: commit.seed_derivation_paths.clone(),
                derivation_overrides: commit.derivation_overrides.clone(),
                holdings: vec![network.native_holding_template()],
                include_in_portfolio_total: true,
                signing: commit.signing(),
            }
        })
        .collect()
}

impl WalletImportRequest {
    /// Refuse a request whose parts do not belong together, before anything
    /// is derived or sealed.
    ///
    /// A private key and a watched address each belong to one network, so
    /// those imports take exactly one chain. Narrowing a longer list to its
    /// first entry, or ignoring addresses typed for a chain the import is not
    /// on, would drop part of what the caller asked for without saying so.
    pub fn check_shape(&self) -> Result<(), DerivationError> {
        check_import_shape(
            &self.selected_chain_ids,
            self.is_watch_only_import,
            self.is_private_key_import,
            &self.watch_only_entries,
        )
    }
}

fn check_import_shape(
    chains: &[Chain],
    is_watch_only: bool,
    is_private_key: bool,
    entries: &WalletImportWatchOnlyEntries,
) -> Result<(), DerivationError> {
    let Some(&chain) = chains.first() else {
        return Err(DerivationError::invalid("Select a chain first."));
    };
    if is_watch_only && is_private_key {
        return Err(DerivationError::invalid(
            "An import is either watch-only or from a private key, not both.",
        ));
    }
    if is_private_key && chains.len() > 1 {
        return Err(DerivationError::invalid(
            "A private key imports on one chain. Select one chain.",
        ));
    }
    if !is_watch_only {
        if !entries.by_chain_id.is_empty() || entries.bitcoin_xpub.is_some() {
            return Err(DerivationError::invalid(
                "Watched addresses belong to a watch-only import.",
            ));
        }
        return Ok(());
    }
    if chains.len() > 1 {
        return Err(DerivationError::invalid(
            "A watch-only import uses one chain. Select one chain.",
        ));
    }
    if entries
        .by_chain_id
        .keys()
        .any(|entry_chain| *entry_chain != chain)
    {
        return Err(DerivationError::invalid(
            "Watched addresses must be on the selected chain.",
        ));
    }
    if entries.bitcoin_xpub.is_some() && !chain.accepts_account_xpub() {
        return Err(DerivationError::refused(
            "%@ does not take an account xpub.",
            [chain.chain_display_name()],
        ));
    }
    Ok(())
}

pub fn plan_wallet_import(
    request: WalletImportPlanRequest,
) -> Result<WalletImportPlan, DerivationError> {
    check_import_shape(
        &request.selected_chain_ids,
        request.is_watch_only_import,
        request.is_private_key_import,
        &request.watch_only_entries,
    )?;
    if request.is_watch_only_import {
        plan_watch_only_import(request)
    } else {
        plan_signing_import(request)
    }
}

fn plan_signing_import(
    request: WalletImportPlanRequest,
) -> Result<WalletImportPlan, DerivationError> {
    let mut request = request;
    if request.planned_wallet_ids.is_empty() {
        request.planned_wallet_ids = request
            .selected_chain_ids
            .iter()
            .map(|_| crate::store::new_transaction_id())
            .collect();
    } else if request.selected_chain_ids.len() != request.planned_wallet_ids.len() {
        return Err(DerivationError::Invalid(
            "Wallet ID plan did not match selected chains.".into(),
        ));
    }

    let selected_chain_count = request.selected_chain_ids.len();
    let mut wallets = Vec::with_capacity(selected_chain_count);
    let mut secret_instructions = Vec::with_capacity(selected_chain_count);
    let secret_kind = if request.is_private_key_import {
        "privateKey"
    } else {
        "seedPhrase"
    };

    for (index, (chain_id, wallet_id)) in request
        .selected_chain_ids
        .iter()
        .zip(request.planned_wallet_ids.iter())
        .enumerate()
    {
        // Every wallet a secret is sealed under answers on its own chain. A
        // chain the secret derived nothing for refuses the import rather than
        // storing a wallet with no address beside a key it cannot use.
        let addresses = addresses_for_chain(*chain_id, &request.resolved_addresses);
        if addresses.address_for(*chain_id).is_none() {
            return Err(DerivationError::refused(
                "Could not derive a %@ address from this secret.",
                [chain_id.chain_display_name()],
            ));
        }
        wallets.push(PlannedWallet {
            wallet_id: wallet_id.clone(),
            name: wallet_display_name(
                &request.wallet_name,
                index + 1,
                index + 1,
                selected_chain_count,
            ),
            chain_id: *chain_id,
            addresses,
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

fn plan_watch_only_import(
    request: WalletImportPlanRequest,
) -> Result<WalletImportPlan, DerivationError> {
    // `check_import_shape` has held this to exactly one chain.
    let primary = request.selected_chain_ids[0];
    let watch_entries = watch_only_addresses_for_chain(primary, &request.watch_only_entries)?;
    if watch_entries.is_empty() {
        return Err(DerivationError::Invalid(
            "Enter at least one valid address to import.".into(),
        ));
    }
    let mut request = request;
    if request.planned_wallet_ids.is_empty() {
        request.planned_wallet_ids = watch_entries
            .iter()
            .map(|_| crate::store::new_transaction_id())
            .collect();
    } else if request.planned_wallet_ids.len() != watch_entries.len() {
        return Err(DerivationError::Internal(
            "Watch-only wallet ID plan did not match expanded requests.".into(),
        ));
    }

    let selected_chain_count = watch_entries.len();
    let wallets = watch_entries
        .into_iter()
        .zip(request.planned_wallet_ids.iter())
        .enumerate()
        .map(
            |(index, ((chain_id, addresses), wallet_id))| PlannedWallet {
                wallet_id: wallet_id.clone(),
                name: wallet_display_name(
                    &request.wallet_name,
                    index + 1,
                    index + 1,
                    selected_chain_count,
                ),
                chain_id,
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
    chain: Chain,
    entries: &WalletImportWatchOnlyEntries,
) -> Result<Vec<(Chain, WalletImportAddresses)>, DerivationError> {
    if !chain.supports_watch_only_import() {
        return Err(DerivationError::refused(
            "%@ cannot be imported as watch-only.",
            [chain.chain_display_name()],
        ));
    }

    // Bitcoin has a second form: one xpub stands in for the whole account, so
    // it plans a single wallet instead of one per address.
    if chain == Chain::Bitcoin
        && let Some(xpub) = trim_optional(entries.bitcoin_xpub.as_deref())
    {
        return Ok(vec![(
            chain,
            WalletImportAddresses {
                by_slot: HashMap::new(),
                bitcoin_xpub: Some(xpub.to_string()),
            },
        )]);
    }

    Ok(entries
        .addresses_for(chain)
        .iter()
        .map(|address| (chain, WalletImportAddresses::single(chain, address.clone())))
        .collect())
}

/// The address slots a wallet on `chain_id` should carry.
///
/// A wallet is per-chain, so it takes only the slots its own chain reads.
/// Bitcoin additionally carries the account xpub when one was supplied.
fn addresses_for_chain(chain: Chain, addresses: &WalletImportAddresses) -> WalletImportAddresses {
    let mut by_slot = HashMap::new();
    if let Some(address) = addresses.address_for(chain) {
        by_slot.insert(chain.address_slot().to_string(), address.to_string());
    }
    // Ethereum Classic is EVM-shaped but has its own slot, while every other
    // EVM chain shares Ethereum's. A derived EVM wallet holds one key, and its
    // address is that key's on either side, so both slots are filled in both
    // directions. Filling only the ETC→Ethereum direction meant an Ethereum
    // wallet answered for twenty-two EVM mainnets and not for the
    // twenty-third.
    if chain.is_evm() {
        let sibling = if chain == Chain::EthereumClassic {
            Chain::Ethereum
        } else {
            Chain::EthereumClassic
        };
        // This wallet's own address, in both EVM slots — not the other slot's,
        // which belongs to a different wallet. An ETC wallet must not show the
        // plain Ethereum address, and an Ethereum wallet must not show ETC's.
        if let Some(address) = addresses.address_for(chain) {
            by_slot.insert(sibling.address_slot().to_string(), address.to_string());
        }
    }

    WalletImportAddresses {
        by_slot,
        bitcoin_xpub: if chain.accepts_account_xpub() {
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

// ── Import draft validation ──

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

    /// A testnet is never offered on its own — the importer picks a mainnet and
    /// the network selector decides which network it lands on.
    #[test]
    fn the_private_key_picker_offers_mainnets_only() {
        for chain in Chain::all().filter(|c| c.derives_from_private_key()) {
            assert!(
                !chain.is_testnet() || chain.mainnet_counterpart() != chain,
                "{} is a testnet and would be offered",
                chain.str_id()
            );
        }
    }

    #[test]
    fn plans_multi_chain_seed_import() {
        let plan = plan_wallet_import(WalletImportPlanRequest {
            wallet_name: "Main".to_string(),
            selected_chain_ids: vec![
                crate::registry::Chain::Bitcoin,
                crate::registry::Chain::Ethereum,
            ],
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
        // A wallet carries its own family's slots and no others — the Bitcoin
        // wallet must not receive the Ethereum address. The EVM wallet carries
        // both EVM slots with *its own* address, because one key answers on
        // Ethereum and on Ethereum Classic; the `0x5678` supplied for the ETC
        // slot belongs to an ETC wallet, and this import created none.
        assert_eq!(
            slots(&plan.wallets[0].addresses),
            vec![("bitcoin".to_string(), "bc1qexample".to_string())]
        );
        assert_eq!(
            slots(&plan.wallets[1].addresses),
            vec![
                ("ethereum".to_string(), "0x1234".to_string()),
                ("ethereum-classic".to_string(), "0x1234".to_string()),
            ]
        );
    }

    #[test]
    fn evm_chains_share_one_address_slot() {
        let request = |chain: Chain| WalletImportPlanRequest {
            wallet_name: "W".to_string(),
            selected_chain_ids: vec![chain],
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

        // Every EVM chain, not only Ethereum.
        for chain in [
            Chain::Ethereum,
            Chain::Arbitrum,
            Chain::Base,
            Chain::Polygon,
            Chain::Ink,
            Chain::XLayer,
        ] {
            let plan = plan_wallet_import(request(chain)).expect("plan");
            assert_eq!(
                plan.wallets[0]
                    .addresses
                    .by_slot
                    .get("ethereum")
                    .map(String::as_str),
                Some("0xabc"),
                "{chain} should read the shared ethereum slot"
            );
        }
    }

    #[test]
    fn ethereum_classic_fills_both_its_own_slot_and_the_evm_slot() {
        let plan = plan_wallet_import(WalletImportPlanRequest {
            wallet_name: "W".to_string(),
            selected_chain_ids: vec![crate::registry::Chain::EthereumClassic],
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
        let plan = plan_wallet_import(WalletImportPlanRequest {
            wallet_name: "Main".to_string(),
            selected_chain_ids: vec![
                crate::registry::Chain::Bitcoin,
                crate::registry::Chain::Solana,
            ],
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
        let plan = plan_wallet_import(WalletImportPlanRequest {
            wallet_name: String::new(),
            selected_chain_ids: vec![crate::registry::Chain::Bitcoin],
            planned_wallet_ids: vec!["watch-1".to_string()],
            is_watch_only_import: true,
            is_private_key_import: false,
            has_wallet_password: false,
            resolved_addresses: WalletImportAddresses::default(),
            watch_only_entries: WalletImportWatchOnlyEntries {
                by_chain_id: HashMap::new(),
                bitcoin_xpub: Some("xpub123".to_string()),
            },
        })
        .expect("plan");

        assert_eq!(plan.wallets.len(), 1);
        assert_eq!(plan.wallets[0].name, "Wallet 1");
        assert_eq!(
            plan.wallets[0].addresses.bitcoin_xpub.as_deref(),
            Some("xpub123")
        );
        assert_eq!(plan.secret_kind, "watchOnly");
    }

    #[test]
    fn watch_only_expands_one_wallet_per_address() {
        let plan = plan_wallet_import(WalletImportPlanRequest {
            wallet_name: "Watch".to_string(),
            selected_chain_ids: vec![crate::registry::Chain::Solana],
            planned_wallet_ids: vec!["a".to_string(), "b".to_string()],
            is_watch_only_import: true,
            is_private_key_import: false,
            has_wallet_password: false,
            resolved_addresses: WalletImportAddresses::default(),
            watch_only_entries: WalletImportWatchOnlyEntries {
                by_chain_id: HashMap::from([(
                    Chain::Solana,
                    vec!["addr1".to_string(), "addr2".to_string()],
                )]),
                bitcoin_xpub: None,
            },
        })
        .expect("plan");

        assert_eq!(plan.wallets.len(), 2);
        assert_eq!(
            plan.wallets[0]
                .addresses
                .by_slot
                .get("solana")
                .map(String::as_str),
            Some("addr1")
        );
        assert_eq!(
            plan.wallets[1]
                .addresses
                .by_slot
                .get("solana")
                .map(String::as_str),
            Some("addr2")
        );
    }

    #[test]
    fn watch_only_rejects_chains_that_need_more_than_an_address() {
        let plan = plan_wallet_import(WalletImportPlanRequest {
            wallet_name: "Watch".to_string(),
            selected_chain_ids: vec![crate::registry::Chain::Monero],
            planned_wallet_ids: vec!["a".to_string()],
            is_watch_only_import: true,
            is_private_key_import: false,
            has_wallet_password: false,
            resolved_addresses: WalletImportAddresses::default(),
            watch_only_entries: WalletImportWatchOnlyEntries {
                by_chain_id: HashMap::from([(Chain::Monero, vec!["4addr".to_string()])]),
                bitcoin_xpub: None,
            },
        });

        // Monero watch-only needs a view key, so an address alone is refused.
        assert!(plan.is_err());
        assert!(
            plan.unwrap_err()
                .to_string()
                .contains("cannot be imported as watch-only")
        );
    }

    fn shaped(
        chains: &[Chain],
        watch_only: bool,
        private_key: bool,
        entries: WalletImportWatchOnlyEntries,
    ) -> Result<WalletImportPlan, DerivationError> {
        plan_wallet_import(WalletImportPlanRequest {
            wallet_name: "W".to_string(),
            selected_chain_ids: chains.to_vec(),
            planned_wallet_ids: Vec::new(),
            is_watch_only_import: watch_only,
            is_private_key_import: private_key,
            has_wallet_password: false,
            resolved_addresses: WalletImportAddresses {
                by_slot: HashMap::from([("ethereum".to_string(), "0xabc".to_string())]),
                bitcoin_xpub: None,
            },
            watch_only_entries: entries,
        })
    }

    /// A private key belongs to one network. A second chain used to plan a
    /// second wallet with no address and seal the key under it too.
    #[test]
    fn a_private_key_import_takes_one_chain() {
        let refused = shaped(
            &[Chain::Ethereum, Chain::Solana],
            false,
            true,
            WalletImportWatchOnlyEntries::default(),
        );
        assert!(refused.unwrap_err().to_string().contains("one chain"));
        assert!(
            shaped(
                &[Chain::Ethereum],
                false,
                true,
                WalletImportWatchOnlyEntries::default()
            )
            .is_ok()
        );
    }

    /// A signing wallet without an address on its own chain is refused, not
    /// stored beside a secret it cannot use.
    #[test]
    fn every_signing_wallet_needs_its_own_address() {
        let refused = shaped(
            &[Chain::Ethereum, Chain::Solana],
            false,
            false,
            WalletImportWatchOnlyEntries::default(),
        )
        .unwrap_err();
        assert_eq!(
            refused,
            DerivationError::Invalid(crate::CoreMessage::new(
                "Could not derive a %@ address from this secret.",
                ["Solana"],
            ))
        );
    }

    /// Watched addresses were narrowed to the first selected chain, and lines
    /// typed for any other chain were dropped without a word.
    #[test]
    fn a_watch_only_import_takes_one_chain_and_only_its_addresses() {
        let entries = |chain: Chain| WalletImportWatchOnlyEntries {
            by_chain_id: HashMap::from([(chain, vec!["addr".to_string()])]),
            bitcoin_xpub: None,
        };
        assert!(
            shaped(
                &[Chain::Solana, Chain::Ethereum],
                true,
                false,
                entries(Chain::Solana)
            )
            .unwrap_err()
            .to_string()
            .contains("one chain")
        );
        assert!(
            shaped(&[Chain::Solana], true, false, entries(Chain::Ethereum))
                .unwrap_err()
                .to_string()
                .contains("selected chain")
        );
        assert!(shaped(&[Chain::Solana], true, false, entries(Chain::Solana)).is_ok());
    }

    #[test]
    fn an_account_xpub_needs_a_chain_that_takes_one() {
        let xpub = WalletImportWatchOnlyEntries {
            by_chain_id: HashMap::new(),
            bitcoin_xpub: Some("xpub123".to_string()),
        };
        assert!(
            shaped(&[Chain::Litecoin], true, false, xpub.clone())
                .unwrap_err()
                .to_string()
                .contains("account xpub")
        );
        assert!(
            shaped(&[Chain::Ethereum], false, false, xpub)
                .unwrap_err()
                .to_string()
                .contains("watch-only import")
        );
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

#[cfg(test)]
mod minted_wallet_id_tests {
    use super::*;

    fn request(chains: &[Chain], planned: Vec<String>) -> WalletImportPlanRequest {
        WalletImportPlanRequest {
            wallet_name: "Main".to_string(),
            selected_chain_ids: chains.to_vec(),
            planned_wallet_ids: planned,
            is_watch_only_import: false,
            is_private_key_import: false,
            has_wallet_password: false,
            resolved_addresses: WalletImportAddresses {
                by_slot: [(
                    "bitcoin".to_string(),
                    "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu".to_string(),
                )]
                .into_iter()
                .collect(),
                bitcoin_xpub: None,
            },
            watch_only_entries: WalletImportWatchOnlyEntries::default(),
        }
    }

    /// Core mints an id per wallet it plans, so a caller does not have to
    /// predict how many there will be.
    #[test]
    fn an_empty_id_plan_is_minted_here() {
        let plan = plan_wallet_import(request(&[Chain::Bitcoin], Vec::new())).expect("plan");
        assert_eq!(plan.wallets.len(), 1);
        let id = &plan.wallets[0].wallet_id;
        // Parseable as a UUID, like every other id that crosses the boundary.
        assert_eq!(id.len(), 36, "{id}");
        assert_eq!(id.chars().filter(|c| *c == '-').count(), 4, "{id}");
        // The secret instruction names the same wallet.
        assert_eq!(plan.secret_instructions[0].wallet_id, *id);

        // Two chains, two distinct ids.
        let plan = plan_wallet_import(request(&[Chain::Bitcoin, Chain::Bitcoin], Vec::new()))
            .expect("plan");
        assert_ne!(plan.wallets[0].wallet_id, plan.wallets[1].wallet_id);
    }

    /// A caller that does supply ids still has to supply the right number:
    /// silently ignoring a mismatched plan would file a wallet under an id
    /// nothing else knows.
    #[test]
    fn a_supplied_id_plan_must_match() {
        let plan = plan_wallet_import(request(&[Chain::Bitcoin], vec!["given-id".to_string()]))
            .expect("plan");
        assert_eq!(plan.wallets[0].wallet_id, "given-id");
        assert!(
            plan_wallet_import(request(
                &[Chain::Bitcoin],
                vec!["one".to_string(), "two".to_string()]
            ))
            .is_err()
        );
    }
}
