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
        chain_id: String,
    ) -> Result<Vec<WalletAddressDiscovery>, SpectraBridgeError> {
        let chain = chain_for_id(&chain_id)?;
        let wallets: Vec<_> = {
            let state = self.wallet_state.read().await;
            state
                .wallets
                .iter()
                .filter_map(|w| {
                    let network = w.chain()?;
                    (network.supports_deep_utxo_discovery()
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
            let result = self
                .discover_utxo_addresses(wallet_id.clone(), network.str_id().into())
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
    }

    /// Select and optionally reserve a receive address from stored wallet identity.
    pub async fn receive_address(
        &self,
        wallet_id: String,
        chain_id: String,
        reserve: bool,
    ) -> Result<Option<String>, SpectraBridgeError> {
        let requested = chain_for_id(&chain_id)?;
        let (wallet_id, network, stored, xpub) = {
            let state = self.wallet_state.read().await;
            let wallet = state
                .wallets
                .iter()
                .find(|w| w.id.eq_ignore_ascii_case(&wallet_id))
                .ok_or_else(|| SpectraBridgeError::InvalidInput {
                    message: "Wallet not found".into(),
                })?;
            let own = wallet.chain();
            let network = if requested.is_testnet() {
                requested
            } else {
                own.filter(|c| c.mainnet_counterpart() == requested.mainnet_counterpart())
                    .unwrap_or_else(|| state.settings.selected_chain_for_family(requested))
            };
            (
                wallet.id.clone(),
                network,
                wallet.address_on(network).map(str::to_string),
                wallet.xpub.clone(),
            )
        };
        // Extended public keys are sufficient for watch-only Bitcoin receiving.
        if network.mainnet_counterpart() == Chain::Bitcoin {
            if let Some(xpub) = xpub.filter(|value| !value.trim().is_empty()) {
                use crate::derivation::xpub_walker::{derive_children_on_network, HdNetwork};
                let name = network.chain_display_name().to_string();
                let hd_network = if network.is_testnet() {
                    HdNetwork::Testnet
                } else {
                    HdNetwork::Mainnet
                };
                // Validate before reserving any index.
                derive_children_on_network(&xpub, 0, 0, 1, hd_network, None)?;
                let index = if reserve {
                    self.reserve_receive_index(wallet_id.clone(), name.clone(), 1)
                        .await?
                } else {
                    self.keypool_state(wallet_id.clone(), name.clone())
                        .await?
                        .reserved_receive_index
                        .unwrap_or(0)
                };
                let index = u32::try_from(index)
                    .map_err(|_| SpectraBridgeError::from("receive index is out of range"))?;
                let address = derive_children_on_network(&xpub, 0, index, 1, hd_network, None)?
                    .pop()
                    .ok_or_else(|| SpectraBridgeError::from("missing derived address"))?
                    .address;
                if reserve {
                    self.register_owned_address(
                        wallet_id,
                        name,
                        address.clone(),
                        None,
                        Some("external".into()),
                        Some(i64::from(index)),
                    )
                    .await?;
                }
                return Ok(Some(address));
            }
        }
        if network.supports_deep_utxo_discovery() {
            if let Some(address) = self
                .utxo_receive_address(wallet_id.clone(), network.str_id().into(), reserve)
                .await?
            {
                return Ok(Some(address));
            }
        }
        let Some(address) = stored.filter(|a| !a.trim().is_empty()) else {
            return Ok(None);
        };
        if !crate::send::flow::is_valid_send_address(
            network.chain_display_name().into(),
            address.clone(),
        ) {
            return Err(SpectraBridgeError::InvalidInput {
                message: "stored receive address is invalid for its network".into(),
            });
        }
        if reserve {
            self.register_owned_address(
                wallet_id,
                network.chain_display_name().into(),
                address.clone(),
                None,
                None,
                None,
            )
            .await?;
        }
        Ok(Some(address))
    }

    /// Every address this wallet is already known to hold on `chain_id`.
    ///
    /// No network and no derivation: the wallet's own address, what the owned
    /// table records, and the ends of transactions it has made. Three callers
    /// want exactly this and not the scan below it.
    pub async fn known_utxo_addresses(
        &self,
        wallet_id: String,
        chain_id: String,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        let chain = chain_for_id(&chain_id)?;
        if !chain.supports_deep_utxo_discovery() {
            return Ok(Vec::new());
        }
        let chain_name = chain.chain_display_name().to_string();
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
            push_utxo_address(&chain_name, address, &mut ordered, &mut seen);
        }
        for address in self
            .owned_addresses_for_wallet(wallet_id.clone(), Some(chain_name.clone()))
            .await
        {
            push_utxo_address(&chain_name, &address, &mut ordered, &mut seen);
        }
        for record in self
            .transactions_for_wallet(wallet_id)
            .await?
            .iter()
            .filter(|r| r.chain_name == chain_name)
        {
            if let Some(address) = &record.source_address {
                push_utxo_address(&chain_name, address, &mut ordered, &mut seen);
            }
            if let Some(address) = &record.change_address {
                push_utxo_address(&chain_name, address, &mut ordered, &mut seen);
            }
        }
        Ok(ordered)
    }

    /// Walk a wallet's external addresses and record the ones that have been
    /// used, returning every address the wallet is known to hold on `chain_id`.
    ///
    /// This was `AppState.discoverUTXOAddresses` and the four methods under it
    /// — a derive-and-probe loop in the front end that needed the seed phrase
    /// in Swift's hands to run. Core reads the seed, the derivation path, the
    /// keypool bound, the balance and the history already; the loop was the
    /// last thing keeping the phrase on that side.
    ///
    /// The gap limit and the ceiling are core's now. They were
    /// `AppState.utxoDiscoveryGapLimit` and `utxoDiscoveryMaxIndex`.
    pub async fn discover_utxo_addresses(
        &self,
        wallet_id: String,
        chain_id: String,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        const GAP_LIMIT: u32 = 3;
        const MAX_INDEX: u32 = 40;

        let chain = chain_for_id(&chain_id)?;
        if !chain.supports_deep_utxo_discovery() {
            return Ok(Vec::new());
        }
        let chain_name = chain.chain_display_name().to_string();

        let mut ordered = self
            .known_utxo_addresses(wallet_id.clone(), chain_id.clone())
            .await?;
        let mut seen: std::collections::HashSet<String> =
            ordered.iter().map(|a| a.to_lowercase()).collect();

        // No readable phrase means no scan, which is what a sealed wallet
        // looks like from here: its material needs a password this path does
        // not have. The addresses gathered above are still returned.
        let Some(context) = self.utxo_derivation_context(&wallet_id, chain).await else {
            return Ok(ordered);
        };

        let state = self
            .keypool_state(wallet_id.clone(), chain_name.clone())
            .await?;
        let reserved = state.reserved_receive_index.unwrap_or(0).max(0) as u32;
        let upper =
            MAX_INDEX.min((state.next_external_index.max(0) as u32).max(reserved + 1) + GAP_LIMIT);

        use futures::stream::{self, StreamExt};
        let candidates: Vec<_> = (0..=upper)
            .filter_map(|index| {
                context
                    .derive(index)
                    .map(|(address, path)| (index, address, path))
            })
            .collect();
        let mut probes = stream::iter(candidates)
            .map(|(index, address, path)| async move {
                let active = self.utxo_address_has_activity(chain, &address).await;
                (index, address, path, active)
            })
            .buffered(4);
        while let Some((index, address, path, active)) = probes.next().await {
            push_utxo_address(&chain_name, &address, &mut ordered, &mut seen);
            if active? {
                self.register_owned_address(
                    wallet_id.clone(),
                    chain_name.clone(),
                    address,
                    Some(path),
                    Some("external".to_string()),
                    Some(index as i64),
                )
                .await?;
            }
        }
        Ok(ordered)
    }
}

impl WalletService {
    /// Move each wallet's reservation past a receive address that has been used.
    ///
    /// Network reads happen outside the writer. Advance only the exact index
    /// whose address was checked; a stale probe cannot clear a newer reservation.
    pub(crate) async fn advance_used_utxo_reservations(
        &self,
        chain_id: String,
    ) -> Result<(), SpectraBridgeError> {
        let chain = chain_for_id(&chain_id)?;
        if !chain.supports_deep_utxo_discovery() {
            return Ok(());
        }
        let wallets: Vec<_> = {
            let state = self.wallet_state.read().await;
            state
                .wallets
                .iter()
                .filter_map(|w| {
                    let network = w.chain()?;
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
            let name = network.chain_display_name().to_string();
            let Some(used) = self
                .keypool_state(wallet_id.clone(), name.clone())
                .await?
                .reserved_receive_index
            else {
                continue;
            };
            let Some(address) = self
                .receive_address(wallet_id.clone(), network.str_id().into(), false)
                .await?
            else {
                continue;
            };
            if !self.utxo_address_has_activity(network, &address).await? {
                continue;
            }
            if self
                .advance_receive_index_if_current(wallet_id.clone(), name, used)
                .await?
                .is_some()
            {
                self.receive_address(wallet_id, network.str_id().into(), true)
                    .await?;
            }
        }
        Ok(())
    }
}

impl WalletService {
    /// Has this address ever been used on chain?
    ///
    /// Swift asked this three ways: Bitcoin looked at UTXOs and a confirmed
    /// balance and never at history, while the other four looked at balance
    /// and history and never at the UTXO count. It is one question.
    pub(crate) async fn utxo_address_has_activity(
        &self,
        chain: crate::registry::Chain,
        address: &str,
    ) -> Result<bool, SpectraBridgeError> {
        use crate::fetch::chains::{
            bitcoin::BitcoinClient, bitcoin_sv::BitcoinSvClient, blockbook::BlockbookClient,
            dogecoin::DogecoinClient,
        };
        let endpoints = self.endpoints_for(chain.str_id()).await;
        let active = match chain.mainnet_counterpart() {
            crate::registry::Chain::Bitcoin => {
                BitcoinClient::new(crate::http::HttpClient::shared(), endpoints)
                    .has_activity(address)
                    .await?
            }
            crate::registry::Chain::BitcoinCash | crate::registry::Chain::Litecoin => {
                BlockbookClient::new(endpoints, chain)
                    .has_activity(address)
                    .await?
            }
            crate::registry::Chain::BitcoinSV => {
                BitcoinSvClient::new(endpoints)
                    .has_activity(address)
                    .await?
            }
            crate::registry::Chain::Dogecoin => {
                DogecoinClient::new(endpoints).has_activity(address).await?
            }
            _ => {
                return Err(SpectraBridgeError::from(
                    "chain does not support UTXO discovery",
                ))
            }
        };
        Ok(active)
    }

    /// The seed and base derivation path this wallet derives UTXO addresses
    /// from, resolved once so a scan does not redo it per index.
    ///
    /// `None` when the phrase is not readable without a password — a sealed
    /// wallet, or one with no signing material at all.
    pub(crate) async fn utxo_derivation_context(
        &self,
        wallet_id: &str,
        chain: crate::registry::Chain,
    ) -> Option<UtxoDerivation> {
        let mut wallet = {
            let state = self.wallet_state.read().await;
            state.wallets.iter().find(|w| w.id == wallet_id).cloned()?
        };
        let overrides = crate::store::wallet_domain::SensitiveOverrides::take_from(&mut wallet);
        let store = self.secrets().ok()?;
        let seed_phrase =
            crate::store::wallet_secrets::load_seed_phrase(&*store, wallet_id, None).ok()?;

        let defaults = crate::derivation_paths_for_preset(wallet.derivation_preset).ok()?;
        let imported = wallet.to_wallet_view(&defaults);
        let raw_path = wallet
            .addresses
            .iter()
            .find(|a| a.chain_name == chain.chain_display_name())
            .and_then(|a| a.derivation_path.clone())
            .or_else(|| {
                imported
                    .seed_derivation_paths
                    .by_chain
                    .get(chain.str_id())
                    .cloned()
            })
            .unwrap_or_default();
        let chain_name = chain.chain_display_name().to_string();
        let resolved = crate::resolve_derivation_path(chain_name.clone(), raw_path).ok()?;

        tokio::task::spawn_blocking(move || {
            UtxoDerivation::with_overrides(chain, &seed_phrase, resolved, &overrides.0)
        })
        .await
        .ok()?
        .ok()
    }
}

/// Scan-local external xpub. No mnemonic or private key is kept while probing.
pub(crate) struct UtxoDerivation {
    chain: crate::registry::Chain,
    branch: crate::derivation::chains::bitcoin::ExtendedPublicKey,
    secp: secp256k1::Secp256k1<secp256k1::All>,
    base_path: String,
}

impl UtxoDerivation {
    #[cfg(test)]
    pub(super) fn new(
        chain: crate::registry::Chain,
        phrase: &str,
        base_path: String,
    ) -> Result<Self, String> {
        Self::with_overrides(chain, phrase, base_path, &Default::default())
    }

    pub(super) fn with_overrides(
        chain: crate::registry::Chain,
        phrase: &str,
        base_path: String,
        overrides: &crate::store::wallet_domain::CoreWalletDerivationOverrides,
    ) -> Result<Self, String> {
        overrides
            .validate_for_chain(chain)
            .map_err(|e| e.to_string())?;
        use crate::derivation::chains::bitcoin::{
            derive_bip39_seed, parse_bip32_path, ExtendedPrivateKey,
        };
        let path = crate::app_core::derivation_path_replacing_last_two(
            base_path.clone(),
            0,
            0,
            base_path.clone(),
        );
        let mut indices = parse_bip32_path(&path)?;
        indices.pop().ok_or("missing address index")?;
        let secp = secp256k1::Secp256k1::new();
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
        let branch = master.derive_path(&secp, &indices)?.to_neutered(&secp);
        Ok(Self {
            chain,
            branch,
            secp,
            base_path,
        })
    }

    pub(crate) fn derive(&self, index: u32) -> Option<(String, String)> {
        let path = crate::app_core::derivation_path_replacing_last_two(
            self.base_path.clone(),
            0,
            index,
            self.base_path.clone(),
        );
        let child = self.branch.derive_child(&self.secp, index).ok()?;
        let address = self
            .chain
            .encode_discovery_address(
                &child.public_key,
                crate::derivation::dispatch::script_type_for_path(&path),
            )
            .ok()?;
        Some((address, path))
    }
}

/// Append an address if it is valid for the chain and not already listed.
///
/// Validation is the registry's, judged against the chain the wallet is on —
/// the Swift original ran the same check through `isValidAddressForPolicy`.
fn push_utxo_address(
    chain_name: &str,
    address: &str,
    ordered: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
) {
    let trimmed = address.trim();
    if trimmed.is_empty()
        || !crate::send::flow::is_valid_send_address(chain_name.to_string(), trimmed.to_string())
    {
        return;
    }
    if seen.insert(trimmed.to_lowercase()) {
        ordered.push(trimmed.to_string());
    }
}

impl WalletService {
    /// The reserved receive address for a wallet on a deep-UTXO chain.
    ///
    /// `reserve` takes the next index when none is held; without it this only
    /// reads. Deep-UTXO chains never hand out index 0 as a receive address,
    /// which is why the reservation floor is 1.
    ///
    /// `None` for a chain without the walk, or a wallet whose phrase this path
    /// cannot read — the caller falls back to the wallet's stored address, as
    /// the Swift original did.
    pub async fn utxo_receive_address(
        &self,
        wallet_id: String,
        chain_id: String,
        reserve: bool,
    ) -> Result<Option<String>, SpectraBridgeError> {
        let chain = chain_for_id(&chain_id)?;
        if !chain.supports_deep_utxo_discovery() {
            return Ok(None);
        }
        let chain_name = chain.chain_display_name().to_string();

        let Some(context) = self.utxo_derivation_context(&wallet_id, chain).await else {
            return Ok(None);
        };
        let index = if reserve {
            Some(
                self.reserve_receive_index(wallet_id.clone(), chain_name.clone(), 1)
                    .await?,
            )
        } else {
            self.keypool_state(wallet_id.clone(), chain_name.clone())
                .await?
                .reserved_receive_index
        };
        let Some(index) = index.filter(|i| *i >= 0) else {
            return Ok(None);
        };

        let Some((address, path)) = context.derive(index as u32) else {
            return Ok(None);
        };
        if reserve {
            self.register_owned_address(
                wallet_id,
                chain_name,
                address.clone(),
                Some(path),
                Some("external".to_string()),
                Some(index),
            )
            .await?;
        }
        Ok(Some(address))
    }
}
