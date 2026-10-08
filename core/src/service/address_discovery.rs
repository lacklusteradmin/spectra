//! UTXO discovery and receive-address derivation.
use super::*;

#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
pub struct WalletAddressDiscovery {
    pub wallet_id: String,
    pub addresses: Vec<String>,
    pub error: Option<String>,
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Select the wallets and their networks from core state for a chain rescan.
    pub async fn discover_chain_addresses(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<Vec<WalletAddressDiscovery>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let wallets: Vec<_> = {
                let state = this.wallet_state.read().await;
                state
                    .wallets
                    .iter()
                    .filter_map(|w| {
                        let network = w.chain_id;
                        (network.uses_account_utxo()
                            && if chain.is_testnet() {
                                network == chain
                            } else {
                                network.mainnet_counterpart() == chain
                            })
                        .then(|| (w.id.clone(), network))
                    })
                    .collect()
            };
            let mut results = Vec::new();
            for (wallet_id, network) in wallets {
                let result = this
                    .discover_utxo_addresses(wallet_id.clone(), network)
                    .await;
                results.push(match result {
                    Ok(addresses) => WalletAddressDiscovery {
                        wallet_id,
                        addresses,
                        error: None,
                    },
                    Err(error) => WalletAddressDiscovery {
                        wallet_id,
                        addresses: Vec::new(),
                        error: Some(error.to_string()),
                    },
                });
            }
            Ok(results)
        })
        .await
    }

    /// Select and optionally reserve a receive address from stored wallet identity.
    pub async fn receive_address(
        &self,
        wallet_id: String,
        chain: crate::registry::Chain,
        reserve: bool,
    ) -> Result<Option<String>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let (wallet_id, network, stored) = {
                let state = this.wallet_state.read().await;
                let wallet = state
                    .wallets
                    .iter()
                    .find(|w| w.id.eq_ignore_ascii_case(&wallet_id))
                    .ok_or_else(|| SpectraBridgeError::InvalidInput {
                        message: "Wallet not found".into(),
                    })?;
                // A mainnet of the wallet's own family names the wallet's network.
                let network =
                    if !chain.is_testnet() && wallet.family() == chain.mainnet_counterpart() {
                        wallet.chain_id
                    } else {
                        chain
                    };
                (
                    wallet.id.clone(),
                    network,
                    wallet.address_on(network).map(str::to_string),
                )
            };
            if network.uses_account_utxo()
                && let Some(address) = this
                    .utxo_receive_address(wallet_id.clone(), network, reserve)
                    .await?
            {
                return Ok(Some(address));
            }
            // A Monero wallet rotates through account 0's subaddresses.
            if network.mainnet_counterpart() == crate::registry::Chain::Monero
                && let Some(primary) = stored.as_deref()
                && let Some(address) = this
                    .monero_receive_address(&wallet_id, network, primary, reserve)
                    .await?
            {
                return Ok(Some(address));
            }
            let Some(address) = stored.filter(|a| !a.trim().is_empty()) else {
                return Ok(None);
            };
            if !crate::send::flow::is_valid_send_address(network, address.clone()) {
                return Err(SpectraBridgeError::InvalidInput {
                    message: "stored receive address is invalid for its network".into(),
                });
            }
            if reserve {
                this.register_owned_address(wallet_id, network, address.clone(), None, None, None)
                    .await?;
            }
            Ok(Some(address))
        })
        .await
    }

    /// Walk a wallet's receive and change addresses and record the ones that have been
    /// used, returning every address the wallet is known to hold on `chain_id`.
    ///
    /// Core reads the seed, the derivation path, the keypool bound, the
    /// balance and the history, so the seed phrase never leaves core for this.
    /// The gap limit and the ceiling are core's too.
    pub async fn discover_utxo_addresses(
        &self,
        wallet_id: String,
        chain: crate::registry::Chain,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            const GAP_LIMIT: u32 = 20;
            const MAX_INDEX: u32 = 999;
            if !chain.uses_account_utxo() {
                return Ok(Vec::new());
            }
            let mut ordered = this.known_utxo_addresses(wallet_id.clone(), chain).await?;
            let mut seen: std::collections::HashSet<String> = ordered.iter().cloned().collect();

            // An account scans from its public key, stored or watched, even
            // while the seed is sealed. A wallet of one address has no
            // account to scan and knows its addresses already.
            let Some(context) = this.utxo_derivation_context(&wallet_id, chain).await? else {
                return Ok(ordered);
            };

            let state = this.keypool_state(wallet_id.clone(), chain).await?;
            use futures::stream::{self, StreamExt};
            let receive_floor = state
                .next_external_index
                .max(state.reserved_receive_index.map_or(0, |index| index + 1));
            for (branch, name, floor) in [
                (0, "external", receive_floor),
                (1, "change", state.next_change_index),
            ] {
                let minimum_end = u32::try_from(floor.max(0))
                    .ok()
                    .and_then(|floor| floor.checked_add(GAP_LIMIT - 1))
                    .ok_or_else(|| SpectraBridgeError::failure("UTXO discovery floor is out of range"))?;
                if minimum_end > MAX_INDEX {
                    return Err(SpectraBridgeError::failure(format!(
                        "UTXO discovery ceiling {MAX_INDEX} is below the {name} keypool floor"
                    )));
                }
                let context = &context;
                let mut probes = stream::iter(0..=MAX_INDEX)
                    .map(|index| async move {
                        let (address, path) = context.derive_on_branch(branch, index)?;
                        let active = this.utxo_address_has_activity(chain, &address).await?;
                        Ok::<_, SpectraBridgeError>((index, address, path, active))
                    })
                    .buffered(4);
                let mut unused = 0;
                let mut complete = false;
                while let Some(probe) = probes.next().await {
                    let (index, address, path, active) = probe?;
                    if active {
                        unused = 0;
                        this.register_owned_address(
                            wallet_id.clone(),
                            chain,
                            address.clone(),
                            path,
                            Some(name.to_string()),
                            Some(i64::from(index)),
                        )
                        .await?;
                        push_utxo_address(chain, &address, &mut ordered, &mut seen);
                    } else {
                        unused += 1;
                    }
                    if index >= minimum_end && unused >= GAP_LIMIT {
                        complete = true;
                        break;
                    }
                }
                if !complete {
                    return Err(SpectraBridgeError::failure(format!(
                        "UTXO {name} discovery reached index {MAX_INDEX} before finding {GAP_LIMIT} unused addresses"
                    )));
                }
            }
            this.mark_account_discovered(wallet_id, chain).await?;
            Ok(ordered)
        })
        .await
    }
}

impl WalletService {
    /// Run the account's gap scan when it has never run to its end: a
    /// restored phrase or a watched account key finds its used addresses
    /// before its balance or history is first read, without being asked. A
    /// wallet of one address has no account to scan.
    pub(crate) async fn discover_account_once(
        &self,
        wallet_id: &str,
        chain: crate::registry::Chain,
    ) -> Result<(), SpectraBridgeError> {
        if !chain.uses_account_utxo()
            || self
                .keypool
                .read()
                .await
                .is_discovered(&super::keypool::keypool_key(wallet_id, chain))
            || self
                .utxo_derivation_context(wallet_id, chain)
                .await?
                .is_none()
        {
            return Ok(());
        }
        self.discover_utxo_addresses(wallet_id.to_string(), chain)
            .await
            .map(drop)
    }

    /// Every address this wallet is already known to hold on `chain_id`.
    ///
    /// No network and no derivation: the wallet's own address, what the owned
    /// table records, and the ends of transactions it has made. Three callers
    /// want exactly this and not the scan below it.
    pub(crate) async fn known_utxo_addresses(
        &self,
        wallet_id: String,
        chain: crate::registry::Chain,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        if !chain.uses_account_utxo() {
            return Ok(Vec::new());
        }
        let mut ordered: Vec<String> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

        let wallet = {
            let state = self.wallet_state.read().await;
            state.wallets.iter().find(|w| w.id == wallet_id).cloned()
        };
        let Some(wallet) = wallet else {
            return Ok(Vec::new());
        };

        if let Some(address) = wallet.address_on(chain) {
            push_utxo_address(chain, address, &mut ordered, &mut seen);
        }
        for address in self
            .owned_addresses_for_wallet(wallet_id.clone(), Some(chain))
            .await
        {
            push_utxo_address(chain, &address, &mut ordered, &mut seen);
        }
        for record in self
            .transactions_for_wallet(wallet_id)
            .await?
            .iter()
            .filter(|r| r.chain_id == chain)
        {
            if let Some(address) = &record.source_address {
                push_utxo_address(chain, address, &mut ordered, &mut seen);
            }
            if let Some(address) = &record.change_address {
                push_utxo_address(chain, address, &mut ordered, &mut seen);
            }
        }
        Ok(ordered)
    }

    /// Move each wallet's reservation past a receive address that has been used.
    ///
    /// Network reads happen outside the writer. Advance only the exact index
    /// whose address was checked; a stale probe cannot clear a newer reservation.
    pub(crate) async fn advance_used_utxo_reservations(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<(), SpectraBridgeError> {
        if !chain.uses_account_utxo() {
            return Ok(());
        }
        let wallets: Vec<_> = {
            let state = self.wallet_state.read().await;
            state
                .wallets
                .iter()
                .filter_map(|w| {
                    let network = w.chain_id;
                    (if chain.is_testnet() {
                        network == chain
                    } else {
                        network.mainnet_counterpart() == chain
                    })
                    .then(|| (w.id.clone(), network))
                })
                .collect()
        };
        for (wallet_id, network) in wallets {
            let id = network;
            let Some(used) = self
                .keypool_state(wallet_id.clone(), id)
                .await?
                .reserved_receive_index
            else {
                continue;
            };
            let Some(address) = self
                .receive_address(wallet_id.clone(), network, false)
                .await?
            else {
                continue;
            };
            if !self.utxo_address_has_activity(network, &address).await? {
                continue;
            }
            if self
                .advance_receive_index_if_current(wallet_id.clone(), id, used)
                .await?
                .is_some()
            {
                self.receive_address(wallet_id, network, true).await?;
            }
        }
        Ok(())
    }
}

impl WalletService {
    /// Has this address ever been used on chain? One question, asked of each
    /// account UTXO chain's indexer.
    pub(crate) async fn utxo_address_has_activity(
        &self,
        chain: crate::registry::Chain,
        address: &str,
    ) -> Result<bool, SpectraBridgeError> {
        let endpoints = || self.endpoints_for(chain, &[EndpointCapability::History]);
        Ok(match chain.mainnet_counterpart() {
            Chain::Decred => {
                crate::api::insight::InsightClient::new(endpoints().await)
                    .has_activity(address)
                    .await?
            }
            Chain::Kaspa => {
                crate::api::kaspa_rest::KaspaClient::new(endpoints().await)
                    .has_activity(address)
                    .await?
            }
            _ if chain.uses_utxo_client() => {
                self.utxo_client(chain, &[EndpointCapability::History])
                    .await
                    .has_activity(address)
                    .await?
            }
            _ => {
                return Err(SpectraBridgeError::failure(
                    "chain does not support UTXO discovery",
                ));
            }
        })
    }

    /// The public branches and base path used to derive UTXO addresses,
    /// resolved once so a scan does not redo it per index.
    ///
    /// A phrase wallet's account is its stored public key, checked against
    /// its root address, so a scan needs no password; a watched account is
    /// the key it watches. `None` for a wallet of one address: a private key
    /// or a watched address has no account to walk.
    pub(crate) async fn utxo_derivation_context(
        &self,
        wallet_id: &str,
        chain: crate::registry::Chain,
    ) -> Result<Option<UtxoDerivation>, SpectraBridgeError> {
        if !chain.uses_account_utxo() {
            return Ok(None);
        }
        let mut wallet = {
            let state = self.wallet_state.read().await;
            let Some(wallet) = state.wallets.iter().find(|w| w.id == wallet_id).cloned() else {
                return Ok(None);
            };
            wallet
        };
        let _overrides = crate::store::wallet_domain::SensitiveOverrides::take_from(&mut wallet);
        // A multisig account is its policy, whoever signs it.
        if let Some(descriptor) = wallet.multisig_descriptor.as_deref() {
            return UtxoDerivation::from_multisig(chain, descriptor).map(Some);
        }
        let xpub = wallet
            .xpub
            .as_deref()
            .filter(|value| !value.trim().is_empty());
        match wallet.signing {
            crate::store::state::WalletSigning::SeedPhrase { .. } => {
                // An empty path resolves to the chain's default.
                let raw_path = wallet
                    .address_record_on(chain)
                    .and_then(|a| a.derivation_path.clone())
                    .or_else(|| {
                        (wallet.chain_id == chain)
                            .then(|| wallet.derivation_path.clone())
                            .flatten()
                    })
                    .unwrap_or_default();
                let resolved = crate::derivation::path::resolve_derivation_path(chain, raw_path)?;
                let xpub = xpub.ok_or_else(|| {
                    SpectraBridgeError::invalid(
                        "UTXO mnemonic wallet is missing its account public key",
                    )
                })?;
                let root = wallet.address_on(chain).ok_or_else(|| {
                    SpectraBridgeError::invalid("UTXO mnemonic wallet is missing its root address")
                })?;
                UtxoDerivation::from_account_xpub(chain, xpub, resolved, root).map(Some)
            }
            crate::store::state::WalletSigning::WatchOnly => xpub
                .map(|xpub| UtxoDerivation::from_watched_xpub(chain, xpub))
                .transpose(),
            crate::store::state::WalletSigning::PrivateKey { .. } => Ok(None),
        }
    }
}

/// Scan-local receive/change xpubs. No mnemonic or private key is kept while probing.
pub(crate) struct UtxoDerivation {
    chain: crate::registry::Chain,
    shape: AccountShape,
    secp: secp256k1::Secp256k1<secp256k1::All>,
    /// A phrase wallet's root path, whose branch and index each derived
    /// address's path replaces: what a send derives the address's key
    /// along. `None` for a watched account key, which signs nothing.
    base_path: Option<String>,
}

/// What an account's addresses are made of.
enum AccountShape {
    /// One key per address: the receive and change branch keys, and the
    /// script every address pays.
    Single {
        branches: [crate::derivation::bitcoin::ExtendedPublicKey; 2],
        script: crate::derivation::types::BitcoinScriptType,
    },
    /// A multisig account's policy over its cosigners' keys.
    Multisig(crate::derivation::multisig::MultisigPolicy),
}

impl UtxoDerivation {
    #[cfg(test)]
    pub(super) fn new(
        chain: crate::registry::Chain,
        phrase: &str,
        base_path: String,
    ) -> Result<Self, SpectraBridgeError> {
        Self::with_overrides(chain, phrase, base_path, &Default::default())
    }

    /// A phrase's account, derived from the phrase itself: the public key a
    /// stored one must equal.
    #[cfg(test)]
    pub(super) fn with_overrides(
        chain: crate::registry::Chain,
        phrase: &str,
        base_path: String,
        overrides: &crate::store::wallet_domain::WalletDerivationOverrides,
    ) -> Result<Self, SpectraBridgeError> {
        let secp = secp256k1::Secp256k1::new();
        let account = Self::account_private_key(chain, phrase, &base_path, overrides)?;
        let branches = [
            account.derive_child(&secp, 0)?.to_neutered(&secp),
            account.derive_child(&secp, 1)?.to_neutered(&secp),
        ];
        let context = Self {
            chain,
            shape: AccountShape::Single {
                branches,
                script: crate::derivation::dispatch::script_type_for_path(&base_path),
            },
            secp,
            base_path: Some(base_path),
        };
        context.derive_on_branch(0, 0)?;
        Ok(context)
    }

    fn path_indices(chain: Chain, base_path: &str) -> Result<Vec<u32>, SpectraBridgeError> {
        let indices = crate::derivation::bitcoin::parse_bip32_path(base_path)?;
        if indices.len() < 2 {
            return Err(SpectraBridgeError::failure(
                "missing UTXO branch and address index",
            ));
        }
        let suffix = &indices[indices.len() - 2..];
        if suffix[0] > 1 || suffix[1] >= crate::derivation::primitives::HARDENED_OFFSET {
            return Err(SpectraBridgeError::InvalidInput {
                message:
                    "UTXO discovery requires a non-hardened receive/change branch and address index"
                        .into(),
            });
        }
        if crate::derivation::path::utxo_discovery_index(base_path, chain, suffix[0])
            != Some(suffix[1])
        {
            return Err(SpectraBridgeError::InvalidInput {
                message:
                    "UTXO discovery requires a catalog-supported purpose, coin and account path"
                        .into(),
            });
        }
        Ok(indices)
    }

    pub(crate) fn account_private_key(
        chain: Chain,
        phrase: &str,
        base_path: &str,
        overrides: &crate::store::wallet_domain::WalletDerivationOverrides,
    ) -> Result<crate::derivation::bitcoin::ExtendedPrivateKey, SpectraBridgeError> {
        overrides.validate_for_chain(chain)?;
        let mut indices = Self::path_indices(chain, base_path)?;
        indices.truncate(indices.len() - 2);
        use crate::derivation::bitcoin::{ExtendedPrivateKey, derive_bip39_seed};
        let seed = derive_bip39_seed(
            phrase,
            overrides.passphrase.as_deref().unwrap_or_default(),
            0,
            None,
            None,
        )?;
        let master = ExtendedPrivateKey::master_from_seed(
            overrides
                .hmac_key
                .as_deref()
                .unwrap_or("Bitcoin seed")
                .as_bytes(),
            seed.as_ref(),
        )?;
        Ok(master.derive_path(&secp256k1::Secp256k1::new(), &indices)?)
    }

    pub(crate) fn account_xpub(
        chain: Chain,
        phrase: &str,
        base_path: &str,
        overrides: &crate::store::wallet_domain::WalletDerivationOverrides,
    ) -> Result<String, SpectraBridgeError> {
        let account = Self::account_private_key(chain, phrase, base_path, overrides)?;
        let version = if chain.is_testnet() {
            crate::derivation::bitcoin::XPUB_VERSION_TESTNET
        } else {
            crate::derivation::bitcoin::XPUB_VERSION_MAINNET
        };
        Ok(account
            .to_neutered(&secp256k1::Secp256k1::new())
            .to_xpub_string(version))
    }

    /// A phrase wallet's account from the public key it stores, which must
    /// be its path's account and derive its root address.
    pub(crate) fn from_account_xpub(
        chain: Chain,
        xpub: &str,
        base_path: String,
        root_address: &str,
    ) -> Result<Self, SpectraBridgeError> {
        if !chain.uses_account_utxo() {
            return Err(SpectraBridgeError::invalid(
                "expected a public account UTXO network",
            ));
        }
        let indices = Self::path_indices(chain, &base_path)?;
        let (account, version) =
            crate::derivation::bitcoin::ExtendedPublicKey::from_xpub_string(xpub)?;
        let expected_version = if chain.is_testnet() {
            crate::derivation::bitcoin::XPUB_VERSION_TESTNET
        } else {
            crate::derivation::bitcoin::XPUB_VERSION_MAINNET
        };
        if version != expected_version || account.depth != 3 || account.child_number != indices[2] {
            return Err(SpectraBridgeError::invalid(
                "UTXO account public key has the wrong network, depth or account",
            ));
        }
        let secp = secp256k1::Secp256k1::new();
        let branches = [
            account.derive_child(&secp, 0)?,
            account.derive_child(&secp, 1)?,
        ];
        let context = Self {
            chain,
            shape: AccountShape::Single {
                branches,
                script: crate::derivation::dispatch::script_type_for_path(&base_path),
            },
            secp,
            base_path: Some(base_path),
        };
        let (root, _) = context.derive_on_branch(indices[3], indices[4])?;
        if normalized_utxo_address(chain, &root).is_none()
            || normalized_utxo_address(chain, &root) != normalized_utxo_address(chain, root_address)
        {
            return Err(SpectraBridgeError::invalid(
                "UTXO account public key does not derive the stored root address",
            ));
        }
        Ok(context)
    }

    /// A watched account from its extended public key, paying the script its
    /// encoding names on the key's own network.
    pub(crate) fn from_watched_xpub(chain: Chain, xpub: &str) -> Result<Self, SpectraBridgeError> {
        let account = crate::derivation::account_key::parse(chain, xpub)?;
        let secp = secp256k1::Secp256k1::new();
        let branches = [
            account.key.derive_child(&secp, 0)?,
            account.key.derive_child(&secp, 1)?,
        ];
        Ok(Self {
            chain,
            shape: AccountShape::Single {
                branches,
                script: account.version.script,
            },
            secp,
            base_path: None,
        })
    }

    /// A multisig account from its descriptor. Its keys sign through a
    /// PSBT, so no address has a path of its own.
    pub(crate) fn from_multisig(
        chain: Chain,
        descriptor: &str,
    ) -> Result<Self, SpectraBridgeError> {
        Ok(Self {
            chain,
            shape: AccountShape::Multisig(crate::derivation::multisig::MultisigPolicy::parse(
                chain, descriptor,
            )?),
            secp: secp256k1::Secp256k1::new(),
            base_path: None,
        })
    }

    pub(crate) fn derive(
        &self,
        index: u32,
    ) -> Result<(String, Option<String>), SpectraBridgeError> {
        self.derive_on_branch(0, index)
    }

    /// The address at `index` on `branch` (0 receive, 1 change), and the
    /// path its key derives along when the account is a phrase wallet's.
    pub(crate) fn derive_on_branch(
        &self,
        branch: u32,
        index: u32,
    ) -> Result<(String, Option<String>), SpectraBridgeError> {
        if branch > 1 {
            return Err(SpectraBridgeError::failure(
                "UTXO discovery branch must be receive or change",
            ));
        }
        if index >= crate::derivation::primitives::HARDENED_OFFSET {
            return Err(SpectraBridgeError::failure(
                "UTXO discovery index must be non-hardened",
            ));
        }
        let path = self.base_path.as_ref().map(|base| {
            crate::derivation::path::derivation_path_replacing_last_two(
                base.clone(),
                branch,
                index,
                base.clone(),
            )
        });
        let address = match &self.shape {
            AccountShape::Single { branches, script } => {
                let child = branches[branch as usize].derive_child(&self.secp, index)?;
                self.chain
                    .encode_discovery_address(&child.public_key, *script)?
            }
            AccountShape::Multisig(policy) => policy.address(self.chain, (branch, index))?,
        };
        Ok((address, path))
    }
}

/// `address` as the chain's validator normalizes it, or `None` when it is
/// not one of the chain's addresses.
pub(crate) fn normalized_utxo_address(chain: Chain, address: &str) -> Option<String> {
    let validated = crate::validation::address::validate_address(
        crate::validation::address::AddressValidationRequest {
            kind: chain.address_validation_kind().to_string(),
            value: address.to_string(),
        },
    );
    validated.normalized_value.filter(|_| validated.is_valid)
}

/// Append an address if it is valid for the chain and not already listed.
///
/// Validation is the registry's, judged against the chain the wallet is on.
fn push_utxo_address(
    chain: Chain,
    address: &str,
    ordered: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
) {
    let Some(normalized) = normalized_utxo_address(chain, address) else {
        return;
    };
    if seen.insert(normalized.clone()) {
        ordered.push(normalized);
    }
}

impl WalletService {
    /// The reserved receive address for a wallet on an account UTXO chain.
    ///
    /// `reserve` takes the next index when none is held; without it this only
    /// reads. An account never hands out index 0 as a receive address, which
    /// is why the reservation floor is 1.
    ///
    /// `None` for a chain without accounts or a wallet of one address. A
    /// phrase wallet uses its stored account key and a watched account its
    /// own; a missing or mismatched key fails before reserving any index.
    pub async fn utxo_receive_address(
        &self,
        wallet_id: String,
        chain: crate::registry::Chain,
        reserve: bool,
    ) -> Result<Option<String>, SpectraBridgeError> {
        if !chain.uses_account_utxo() {
            return Ok(None);
        }

        let Some(context) = self.utxo_derivation_context(&wallet_id, chain).await? else {
            return Ok(None);
        };
        let index = if reserve {
            Some(
                self.reserve_receive_index(wallet_id.clone(), chain, 1)
                    .await?,
            )
        } else {
            // A watched account has no address of its own to fall back to:
            // until one is reserved, it shows its first.
            self.keypool_state(wallet_id.clone(), chain)
                .await?
                .reserved_receive_index
                .or(context.base_path.is_none().then_some(0))
        };
        let Some(index) = index.filter(|i| *i >= 0) else {
            return Ok(None);
        };

        let index = u32::try_from(index)
            .map_err(|_| SpectraBridgeError::failure("receive index is out of range"))?;
        let (address, path) = context.derive(index)?;
        if reserve {
            self.register_owned_address(
                wallet_id,
                chain,
                address.clone(),
                path,
                Some("external".to_string()),
                Some(i64::from(index)),
            )
            .await?;
        }
        Ok(Some(address))
    }
}
