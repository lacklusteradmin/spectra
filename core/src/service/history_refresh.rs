//! Fetching one chain's history for the wallets core holds, and merging it.
//!
//! Core plans which wallets to fetch for, builds each transaction record and
//! merges the result; a caller asks for a chain and is told what changed.

use futures::{StreamExt as _, stream};
use std::collections::HashMap;

use crate::SpectraBridgeError;
use crate::registry::Chain;
use crate::service::WalletService;
use crate::store::state::ResidentState;

/// What one chain's history refresh did.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct HistoryRefreshOutcome {
    /// Wallets whose history was fetched and merged.
    pub wallets_refreshed: u32,
    /// Wallets whose provider failed. The stored history is left alone; a
    /// caller shows its degraded banner from this rather than from an error,
    /// because a partial refresh still merged what it got.
    pub wallets_failed: u32,
    pub added: u32,
    pub updated: u32,
    /// Whether every wallet reported a last page, so there is nothing more to
    /// load. A caller shows or hides its "load more" control from this.
    pub exhausted: bool,
    /// One row per wallet fetched, for the diagnostics screen. Empty for the
    /// paths whose screen has no such table.
    pub diagnostics: Vec<HistoryWalletDiagnostics>,
}

impl HistoryRefreshOutcome {
    /// Nothing to fetch: no wallet on this chain had an address.
    pub(super) fn nothing() -> Self {
        Self {
            wallets_refreshed: 0,
            wallets_failed: 0,
            added: 0,
            updated: 0,
            exhausted: true,
            diagnostics: Vec::new(),
        }
    }
}

/// One wallet to fetch history for.
pub(super) struct Target {
    pub wallet_id: String,
    pub wallet_name: String,
    pub address: String,
    /// Exact network used both for fetching and persisted transaction identity.
    pub network: Chain,
}

/// The wallets on `chain` that have an address, with the address for the
/// network each is on. `wallet_ids` scopes it; empty means every wallet.
fn targets(state: &ResidentState, chain: Chain, wallet_ids: &[String]) -> Vec<Target> {
    state
        .wallets
        .iter()
        .filter(|wallet| wallet.family() == chain)
        .filter(|wallet| {
            wallet_ids.is_empty()
                || wallet_ids
                    .iter()
                    .any(|id| id.eq_ignore_ascii_case(&wallet.id))
        })
        .filter_map(|wallet| {
            Some(Target {
                wallet_id: wallet.id.clone(),
                wallet_name: wallet.name.clone(),
                address: wallet.active_address()?.to_string(),
                network: wallet.chain_id,
            })
        })
        .collect()
}

/// Only identical addresses on the same network may share a provider response.
fn evm_history_groups(targets: &[Target], load_more: bool) -> Vec<(Vec<String>, String)> {
    let mut groups = std::collections::BTreeMap::new();
    for target in targets {
        let address = target.address.trim().to_lowercase();
        let key = (
            target.network.str_id(),
            address,
            load_more.then_some(&target.wallet_id),
        );
        groups
            .entry(key)
            .or_insert_with(Vec::new)
            .push(target.wallet_id.clone());
    }
    groups
        .into_iter()
        .map(|((_, address, _), ids)| (ids, address))
        .collect()
}

/// The name and symbol of every token the user lists, by deployment: what a
/// row of a token the catalog does not know is called, as its sends are.
fn token_names(state: &ResidentState) -> HashMap<String, (String, String)> {
    state
        .token_preferences
        .iter()
        .map(|entry| {
            (
                entry.token.deployment_id.clone(),
                (entry.token.name.clone(), entry.token.symbol.clone()),
            )
        })
        .collect()
}

/// One normalized entry as a stored record.
///
/// `created_at` is the entry's own timestamp in unix seconds, which is what
/// the merge orders and de-duplicates by.
pub(super) fn record_for(
    target: &Target,
    names: &HashMap<String, (String, String)>,
    entry: crate::fetch::history_decode::NormalizedHistoryItem,
) -> crate::fetch::transactions::FetchedTransactionRecord {
    // A token the user lists is called what the list calls it; the feed's
    // own name for an unknown contract is the contract itself.
    let (asset_display_name, symbol) = entry
        .deployment_id
        .as_ref()
        .and_then(|id| names.get(id))
        .cloned()
        .unwrap_or((entry.asset_display_name, entry.symbol));
    crate::fetch::transactions::FetchedTransactionRecord {
        // The feed names every row's deployment; the ticker is display text.
        deployment_id: entry.deployment_id,
        id: crate::store::new_transaction_id(),
        wallet_id: Some(target.wallet_id.clone()),
        kind: entry.kind,
        status: entry.status,
        wallet_name: target.wallet_name.clone(),
        asset_display_name,
        symbol,
        chain_id: target.network,
        amount: entry.amount,
        address: entry.counterparty,
        transaction_hash: Some(entry.tx_hash).filter(|hash| !hash.is_empty()),
        nonce: None,
        receipt_block_number: entry.block_height,
        receipt_gas_used: None,
        receipt_effective_gas_price_gwei: None,
        receipt_network_fee: None,
        fee_rate_description: None,
        confirmation_count: None,
        confirmed_network_fee: None,
        used_change_output: None,
        source_derivation_path: None,
        change_derivation_path: None,
        source_address: None,
        change_address: None,
        signed_transaction_payload: None,
        signed_transaction_payload_format: None,
        failure_reason: None,
        transaction_history_source: Some("rust".to_string()),
        // A provider that gives no time (an unconfirmed transaction, a
        // Solana block without `blockTime`) gives 0. That is stored as unknown,
        // the sentinel every other history path stores, not as the Unix epoch.
        created_at_unix: if entry.timestamp > 0.0 {
            entry.timestamp
        } else {
            SENTINEL_CREATED_AT_UNIX
        },
    }
}

impl WalletService {
    /// Fetch one chain's history for its wallets and merge it into the store.
    ///
    /// `wallet_ids` scopes the refresh; an empty list means every wallet on the
    /// chain. A provider failure for one wallet is counted, not raised: the
    /// wallets that answered are still merged, which is what the front end's
    /// "loaded with partial provider failures" banner was already saying.
    pub(crate) async fn refresh_chain_history_page(
        &self,
        chain: Chain,
        wallet_ids: Vec<String>,
        load_more: bool,
    ) -> Result<HistoryRefreshOutcome, SpectraBridgeError> {
        let _operation = self.history_pagination.operation_lock.lock().await;
        let (targets, names) = {
            let state = self.app_state().await;
            (targets(&state, chain, &wallet_ids), token_names(&state))
        };
        if targets.is_empty() {
            return Ok(HistoryRefreshOutcome::nothing());
        }
        let mut requests = Vec::new();
        for (index, target) in targets.iter().enumerate() {
            let saved = self.history_cursor(chain, target.wallet_id.clone());
            if load_more && saved.is_exhausted {
                continue;
            }
            requests.push((
                index,
                target.network,
                target.address.clone(),
                if load_more { saved.next_cursor } else { None },
            ));
        }
        let fetched: Vec<_> = stream::iter(requests)
            .map(|(index, network, address, cursor)| async move {
                (
                    index,
                    cursor.clone(),
                    self.fetch_normalized_history_page(network, &address, cursor.as_deref())
                        .await,
                )
            })
            .buffer_unordered(4)
            .collect()
            .await;
        let mut incoming = Vec::new();
        let mut updates = Vec::new();
        let mut diagnostics = Vec::new();
        let mut wallets_refreshed = 0;
        let mut wallets_failed = 0;
        let mut exhausted = true;
        for (index, previous, fetched) in fetched {
            let target = &targets[index];
            match fetched {
                Ok(page) => {
                    if page.next_cursor.is_some() && page.next_cursor == previous {
                        wallets_failed += 1;
                        exhausted = false;
                        diagnostics.push(HistoryWalletDiagnostics {
                            wallet_id: target.wallet_id.clone(),
                            identifier: target.address.clone(),
                            source_used: "none".into(),
                            transaction_count: 0,
                            next_cursor: previous,
                            error: Some("provider repeated history cursor".into()),
                        });
                        continue;
                    }
                    wallets_refreshed += 1;
                    exhausted &= page.next_cursor.is_none();
                    diagnostics.push(HistoryWalletDiagnostics {
                        wallet_id: target.wallet_id.clone(),
                        identifier: target.address.clone(),
                        source_used: "rust/provider".into(),
                        transaction_count: page.items.len() as u32,
                        next_cursor: page.next_cursor.clone(),
                        error: None,
                    });
                    updates.push((target.wallet_id.clone(), page.next_cursor));
                    incoming.extend(
                        page.items
                            .into_iter()
                            .map(|entry| record_for(target, &names, entry)),
                    );
                }
                Err(error) => {
                    wallets_failed += 1;
                    exhausted = false;
                    diagnostics.push(HistoryWalletDiagnostics {
                        wallet_id: target.wallet_id.clone(),
                        identifier: target.address.clone(),
                        source_used: "none".into(),
                        transaction_count: 0,
                        next_cursor: previous,
                        error: Some(error.to_string()),
                    });
                }
            }
        }
        let change = self.merge_fetched_history(incoming).await?;
        for (id, next) in updates {
            self.advance_history_cursor(chain, id, next)?;
        }
        Ok(HistoryRefreshOutcome {
            wallets_refreshed,
            wallets_failed,
            added: change.added.len() as u32,
            updated: change.updated.len() as u32,
            exhausted,
            diagnostics,
        })
    }
}

/// `Date.distantPast` in unix seconds — the stamp a front end puts on a record
/// it created locally before the chain confirmed it.
pub(super) const SENTINEL_CREATED_AT_UNIX: f64 = -62_135_596_800.0;

/// One wallet's row for the history-diagnostics screen.
///
/// The screen is a front end's, so the rows cross the boundary rather than
/// being written here — but what they say (which source answered, how many
/// records, what failed) is the refresh's own account of itself.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct HistoryWalletDiagnostics {
    pub wallet_id: String,
    /// What was fetched for: an address, an account xpub, or the wallet's name
    /// when it has neither.
    pub identifier: String,
    pub source_used: String,
    pub transaction_count: u32,
    pub next_cursor: Option<String>,
    pub error: Option<String>,
}

/// The page size a refresh asks a provider for when the caller names none.
const DEFAULT_EVM_PAGE_SIZE: u32 = 20;
const MIN_EVM_PAGE_SIZE: u32 = 20;
const MAX_EVM_PAGE_SIZE: u32 = 500;

/// The tokens this chain's history should decode, as the user has them.
fn token_descriptors(state: &ResidentState, chain: Chain) -> Vec<crate::service::TokenDescriptor> {
    if !chain.hosts_tokens() {
        return Vec::new();
    }
    state
        .token_preferences
        .iter()
        .filter(|entry| entry.hosting_chain() == Some(chain))
        .filter_map(|entry| {
            let contract = crate::tokens::normalize_token_identifier(
                Some(entry.token.contract.clone()),
                chain,
            )?;
            Some(crate::service::TokenDescriptor {
                standard: entry.token.token_standard.clone(),
                contract,
                symbol: entry.token.symbol.clone(),
                decimals: u8::try_from(entry.token.decimals).unwrap_or(u8::MAX),
                name: Some(entry.token.name.clone()),
            })
        })
        .collect()
}

impl WalletService {
    /// Fetch one EVM chain's history page for its wallets and merge it; what
    /// comes back is what changed and what to put on the diagnostics screen.
    ///
    /// `load_more` advances each group's page instead of restarting at the
    /// first, and leaves a group that already reported a short page alone.
    pub async fn refresh_evm_chain_history(
        &self,
        chain_id: crate::registry::Chain,
        wallet_ids: Vec<String>,
        load_more: bool,
        page_size: Option<u32>,
    ) -> Result<HistoryRefreshOutcome, SpectraBridgeError> {
        let _operation = self.history_pagination.operation_lock.lock().await;
        let chain = super::evm_network(chain_id)?;
        let (groups, descriptors, wallet_names, networks) = {
            let state = self.app_state().await;
            let targets = targets(&state, chain, &wallet_ids);
            let groups = evm_history_groups(&targets, load_more);
            let mut descriptors = std::collections::HashMap::new();
            let mut names = std::collections::HashMap::new();
            let mut networks = std::collections::HashMap::new();
            for target in targets {
                descriptors
                    .entry(target.network)
                    .or_insert_with(|| token_descriptors(&state, target.network));
                networks.insert(target.wallet_id.clone(), target.network);
                names.insert(target.wallet_id, target.wallet_name);
            }
            (groups, descriptors, names, networks)
        };
        if groups.is_empty() {
            return Ok(HistoryRefreshOutcome::nothing());
        }

        let page_size = page_size
            .unwrap_or(DEFAULT_EVM_PAGE_SIZE)
            .clamp(MIN_EVM_PAGE_SIZE, MAX_EVM_PAGE_SIZE);
        let mut incoming = Vec::new();
        let mut diagnostics = Vec::new();
        let mut wallets_refreshed = 0;
        let mut wallets_failed = 0;
        let mut exhausted = true;
        let mut cursor_updates = Vec::new();
        for (group_wallet_ids, normalized_address) in groups {
            let Some(first) = group_wallet_ids.first().cloned() else {
                continue;
            };
            if load_more && self.history_cursor(chain_id, first.clone()).is_exhausted {
                continue;
            }
            let current = self.history_cursor(chain_id, first.clone()).next_page;
            let page = if load_more { current + 1 } else { 1 };

            let network = networks.get(&first).copied().unwrap_or(chain);
            let fetched = self
                .fetch_evm_history_page(
                    network,
                    normalized_address.clone(),
                    descriptors.get(&network).cloned().unwrap_or_default(),
                    page,
                    page_size,
                )
                .await;
            let fetched = match fetched {
                Ok(fetched) => fetched,
                Err(error) => {
                    wallets_failed += group_wallet_ids.len() as u32;
                    exhausted = false;
                    for wallet_id in &group_wallet_ids {
                        diagnostics.push(HistoryWalletDiagnostics {
                            wallet_id: wallet_id.clone(),
                            identifier: normalized_address.clone(),
                            source_used: "none".to_string(),
                            transaction_count: 0,
                            next_cursor: None,
                            error: Some(error.to_string()),
                        });
                    }
                    continue;
                }
            };
            let decoded = fetched.decoded;
            wallets_refreshed += group_wallet_ids.len() as u32;
            let token_count = decoded.tokens.len() as u32;
            let is_last_page = fetched.exhausted;
            exhausted = exhausted && is_last_page;
            for wallet_id in &group_wallet_ids {
                cursor_updates.push((wallet_id.clone(), page, is_last_page));
                diagnostics.push(HistoryWalletDiagnostics {
                    wallet_id: wallet_id.clone(),
                    identifier: normalized_address.clone(),
                    source_used: "rust/etherscan".to_string(),
                    transaction_count: token_count,
                    next_cursor: None,
                    error: None,
                });
            }

            let planned = crate::fetch::history_decode::build_evm_transaction_records(
                crate::fetch::history_decode::EvmTransactionRecordRequest {
                    decoded_page: decoded,
                    normalized_address: normalized_address.clone(),
                    chain_id: network,
                    token_source_used: Some("rust/etherscan".to_string()),
                    wallets: group_wallet_ids
                        .iter()
                        .map(|wallet_id| {
                            crate::fetch::history_decode::EvmTransactionRecordWalletInput {
                                wallet_id: wallet_id.clone(),
                                wallet_name: wallet_names
                                    .get(wallet_id)
                                    .cloned()
                                    .unwrap_or_default(),
                            }
                        })
                        .collect(),
                    unknown_timestamp_sentinel_unix: SENTINEL_CREATED_AT_UNIX,
                },
            );
            incoming.extend(planned.into_iter().map(evm_record));
        }

        let change = self.merge_fetched_history(incoming).await?;

        for (id, page, exhausted) in cursor_updates {
            self.set_history_page(chain_id, id, page, exhausted)?;
        }
        Ok(HistoryRefreshOutcome {
            wallets_refreshed,
            wallets_failed,
            added: change.added.len() as u32,
            updated: change.updated.len() as u32,
            exhausted,
            diagnostics,
        })
    }
}

/// A planned EVM record as a stored one. The amount crosses as a decimal
/// string and lands as the `f64` the store holds.
fn evm_record(
    planned: crate::fetch::history_decode::EvmHistoryTransactionRecord,
) -> crate::fetch::transactions::FetchedTransactionRecord {
    crate::fetch::transactions::FetchedTransactionRecord {
        deployment_id: planned.deployment_id,
        id: crate::store::new_transaction_id(),
        wallet_id: Some(planned.wallet_id),
        kind: planned.kind,
        status: planned.status,
        wallet_name: planned.wallet_name,
        asset_display_name: planned.asset_display_name,
        symbol: planned.symbol,
        chain_id: planned.chain_id,
        amount: crate::decimal::canonical(&planned.amount_decimal).unwrap_or_else(|| "0".into()),
        address: planned.counterparty,
        transaction_hash: Some(planned.transaction_hash).filter(|hash| !hash.is_empty()),
        nonce: None,
        receipt_block_number: Some(planned.block_number),
        receipt_gas_used: None,
        receipt_effective_gas_price_gwei: None,
        receipt_network_fee: None,
        fee_rate_description: None,
        confirmation_count: None,
        confirmed_network_fee: None,
        used_change_output: None,
        source_derivation_path: None,
        change_derivation_path: None,
        source_address: Some(planned.source_address).filter(|address| !address.is_empty()),
        change_address: None,
        signed_transaction_payload: None,
        signed_transaction_payload_format: None,
        failure_reason: None,
        transaction_history_source: Some(planned.source_used),
        created_at_unix: planned.created_at_unix,
    }
}

impl WalletService {
    /// Fetch and merge one UTXO chain's history across each wallet's known
    /// addresses.
    ///
    /// A UTXO wallet spends from many addresses, so one transaction shows up
    /// once per address it touched; the records are netted per transaction
    /// before they are stored. The addresses are core's own keypool.
    ///
    pub async fn refresh_utxo_chain_history(
        &self,
        chain: Chain,
        wallet_ids: Vec<String>,
        load_more: bool,
    ) -> Result<HistoryRefreshOutcome, SpectraBridgeError> {
        let _operation = self.history_pagination.operation_lock.lock().await;
        let targets = {
            let state = self.app_state().await;
            targets(&state, chain, &wallet_ids)
        };
        let mut incoming = Vec::new();
        let mut updates = Vec::new();
        let mut diagnostics = Vec::new();
        let mut wallets_refreshed = 0;
        let mut wallets_failed = 0;
        let mut exhausted = true;
        for target in targets {
            let saved = self.history_cursor(chain, target.wallet_id.clone());
            if load_more && saved.is_exhausted {
                continue;
            }
            let previous = if load_more { saved.next_cursor } else { None };
            let fetched = async {
                let addresses = self
                    .known_utxo_addresses(target.wallet_id.clone(), target.network)
                    .await?;
                if addresses.is_empty() {
                    return Err(SpectraBridgeError::failure(
                        "UTXO wallet has no stored addresses",
                    ));
                }
                let client = self
                    .utxo_client(target.network, &[crate::EndpointCapability::History])
                    .await;
                Ok::<_, SpectraBridgeError>(
                        crate::fetch::bitcoin_history::page(
                            target.network,
                            &addresses,
                            previous.as_deref(),
                            20,
                            |address, after| {
                                let client = &client;
                                async move {
                                    client.fetch_history_page(&address, after.as_deref()).await
                                }
                            },
                        )
                        .await?,
                    )
            }
            .await;
            match fetched {
                Ok(page) => {
                    wallets_refreshed += 1;
                    exhausted &= page.next_cursor.is_none();
                    diagnostics.push(HistoryWalletDiagnostics {
                        wallet_id: target.wallet_id.clone(),
                        identifier: target.address.clone(),
                        source_used: "rust/utxo".into(),
                        transaction_count: page.items.len() as u32,
                        next_cursor: page.next_cursor.clone(),
                        error: None,
                    });
                    updates.push((target.wallet_id.clone(), page.next_cursor));
                    incoming.extend(page.items.into_iter().map(|entry| {
                        record_for(
                            &target,
                            &HashMap::new(),
                            crate::fetch::history_decode::NormalizedHistoryItem {
                                deployment_id: crate::tokens::deployment_id_for(
                                    target.network,
                                    None,
                                ),
                                kind: entry.kind,
                                status: entry.status,
                                asset_display_name: target.network.coin_name().into(),
                                symbol: target.network.coin_symbol().into(),
                                chain_id: target.network,
                                amount: entry.amount_btc,
                                // A row nets the wallet's every address in one
                                // transaction, so it names no single counterparty.
                                counterparty: String::new(),
                                tx_hash: entry.txid,
                                block_height: entry.block_height,
                                timestamp: entry.created_at_unix,
                            },
                        )
                    }));
                }
                Err(error) => {
                    wallets_failed += 1;
                    exhausted = false;
                    diagnostics.push(HistoryWalletDiagnostics {
                        wallet_id: target.wallet_id.clone(),
                        identifier: target.address.clone(),
                        source_used: "none".into(),
                        transaction_count: 0,
                        next_cursor: previous,
                        error: Some(error.to_string()),
                    });
                }
            }
        }
        let change = self.merge_fetched_history(incoming).await?;
        for (id, next) in updates {
            self.advance_history_cursor(chain, id, next)?;
        }
        Ok(HistoryRefreshOutcome {
            wallets_refreshed,
            wallets_failed,
            added: change.added.len() as u32,
            updated: change.updated.len() as u32,
            exhausted,
            diagnostics,
        })
    }
}

#[cfg(test)]
#[path = "tests/history_refresh.rs"]
mod tests;
