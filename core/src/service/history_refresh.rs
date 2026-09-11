//! Fetching one chain's history for the wallets core holds, and merging it.
//!
//! The fetch, the record it becomes and the merge used to be three steps on
//! the front end's side of the boundary: it planned which wallets to fetch for
//! (from its own projection of core's wallets), built a transaction record per
//! entry — minting the id, naming the wallet, stamping the source — and handed
//! the result back to be merged. Core owns all three; a caller asks for a
//! chain and is told what changed.

use futures::{stream, StreamExt as _};

use crate::registry::Chain;
use crate::service::WalletService;
use crate::store::state::CoreAppState;
use crate::SpectraBridgeError;

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
struct Target {
    wallet_id: String,
    wallet_name: String,
    address: String,
    /// The chain to fetch *from*: the network this wallet is on. What the
    /// records are filed *under* is the family, which is what the store groups
    /// by and what the app names the asset after — so a wallet on a testnet
    /// reads its testnet history and it lands in the same list, rather than
    /// reading the mainnet chain's and finding nothing.
    network: Chain,
}

/// The wallets on `chain` that have an address, with the address for the
/// network each is on. `wallet_ids` scopes it; empty means every wallet.
fn targets(state: &CoreAppState, chain: Chain, wallet_ids: &[String]) -> Vec<Target> {
    state
        .wallets
        .iter()
        .filter(|wallet| wallet.chain_name == chain.chain_display_name())
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
                address: wallet.active_address(&state.settings)?.to_string(),
                network: wallet.network_chain(&state.settings).unwrap_or(chain),
            })
        })
        .collect()
}

/// One normalized entry as a stored record.
///
/// `created_at` is the entry's own timestamp in unix seconds, which is what
/// the merge orders and de-duplicates by.
fn record_for(
    target: &Target,
    chain: Chain,
    entry: crate::fetch::history_decode::NormalizedHistoryItem,
) -> crate::fetch::transactions::CoreTransactionRecord {
    crate::fetch::transactions::CoreTransactionRecord {
        id: crate::store::new_transaction_id(),
        wallet_id: Some(target.wallet_id.clone()),
        kind: entry.kind,
        status: entry.status,
        wallet_name: target.wallet_name.clone(),
        asset_name: entry.asset_name,
        symbol: entry.symbol,
        // The family's name, not the network's: the store groups by it and a
        // testnet record has to land in the same list as the wallet's others.
        chain_name: chain.chain_display_name().to_string(),
        amount: entry.amount,
        address: entry.counterparty,
        transaction_hash: Some(entry.tx_hash).filter(|hash| !hash.is_empty()),
        ethereum_nonce: None,
        receipt_block_number: entry.block_height,
        receipt_gas_used: None,
        receipt_effective_gas_price_gwei: None,
        receipt_network_fee_eth: None,
        fee_priority_raw: None,
        fee_rate_description: None,
        confirmation_count: None,
        dogecoin_confirmed_network_fee_doge: None,
        dogecoin_estimated_fee_rate_doge_per_kb: None,
        used_change_output: None,
        source_derivation_path: None,
        change_derivation_path: None,
        source_address: None,
        change_address: None,
        signed_transaction_payload: None,
        signed_transaction_payload_format: None,
        failure_reason: None,
        transaction_history_source: Some("rust".to_string()),
        created_at_unix: entry.timestamp,
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Fetch one chain's history for its wallets and merge it into the store.
    ///
    /// `wallet_ids` scopes the refresh; an empty list means every wallet on the
    /// chain. A provider failure for one wallet is counted, not raised: the
    /// wallets that answered are still merged, which is what the front end's
    /// "loaded with partial provider failures" banner was already saying.
    pub async fn refresh_chain_history(
        &self,
        chain_id: String,
        wallet_ids: Vec<String>,
    ) -> Result<HistoryRefreshOutcome, SpectraBridgeError> {
        let chain = crate::registry::Chain::from_str_id(&chain_id)
            .ok_or_else(|| SpectraBridgeError::from(format!("unknown chain {chain_id:?}")))?;
        let targets = {
            let state = self.app_state().await;
            targets(&state, chain, &wallet_ids)
        };
        if targets.is_empty() {
            return Ok(HistoryRefreshOutcome::nothing());
        }

        // Owned pairs rather than borrows of `targets`: the exported method's
        // future has to be `'static`, and a closure borrowing from the
        // enclosing scope is not.
        let requests: Vec<(usize, String, String)> = targets
            .iter()
            .enumerate()
            .map(|(index, target)| {
                (
                    index,
                    target.network.str_id().to_string(),
                    target.address.clone(),
                )
            })
            .collect();
        let fetched: Vec<(usize, Option<Vec<_>>)> = stream::iter(requests)
            .map(|(index, network_id, address)| async move {
                (
                    index,
                    self.fetch_normalized_history(network_id, address)
                        .await
                        .ok(),
                )
            })
            .buffer_unordered(4)
            .collect()
            .await;

        let mut incoming = Vec::new();
        let mut wallets_refreshed = 0;
        let mut wallets_failed = 0;
        for (index, entries) in fetched {
            match entries {
                Some(entries) => {
                    wallets_refreshed += 1;
                    incoming.extend(
                        entries
                            .into_iter()
                            .map(|entry| record_for(&targets[index], chain, entry)),
                    );
                }
                None => wallets_failed += 1,
            }
        }

        let change = self
            .apply_transaction_command(crate::service::types::TransactionCommand::Merge {
                incoming,
                chain_name: chain.chain_display_name().to_string(),
                // Records the app created for a send it just broadcast carry a
                // sentinel time until the chain confirms them; the merge keeps
                // that rather than moving them to the provider's timestamp.
                preserve_created_at_sentinel_unix: Some(SENTINEL_CREATED_AT_UNIX),
            })
            .await?;

        Ok(HistoryRefreshOutcome {
            wallets_refreshed,
            wallets_failed,
            added: change.added.len() as u32,
            updated: change.updated.len() as u32,
            // These providers answer with a whole history at once, so the page
            // just merged is the last one.
            exhausted: true,
            diagnostics: Vec::new(),
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
/// records, what failed) is the refresh's own account of itself, and the front
/// end used to assemble it from the pieces it happened to hold.
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
///
/// Token preferences are core state; the front end used to filter its mirror
/// of them, normalise each contract and build the descriptor list to hand
/// back.
fn token_descriptors(state: &CoreAppState, chain: Chain) -> Vec<crate::service::TokenDescriptor> {
    let Some(hosting) = crate::store::wallet_domain::CoreTokenHostingChain::from_chain_name(
        chain.chain_display_name(),
    ) else {
        return Vec::new();
    };
    state
        .token_preferences
        .iter()
        // `hosting_chain()` reads the entry's own `chain` key, which is the
        // catalog's spelling — `"bnb"` for BNB Chain — rather than a display
        // name, so the comparison is between variants and not strings.
        .filter(|entry| entry.is_enabled && entry.hosting_chain() == Some(hosting))
        .filter_map(|entry| {
            let contract = crate::tokens::normalize_token_identifier(
                Some(entry.token.contract.clone()),
                chain.chain_display_name().to_string(),
            )?;
            Some(crate::service::TokenDescriptor {
                contract,
                symbol: entry.token.symbol.clone(),
                decimals: u8::try_from(entry.token.decimals).unwrap_or(u8::MAX),
                name: Some(entry.token.name.clone()),
            })
        })
        .collect()
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Fetch one EVM chain's history page for its wallets and merge it.
    ///
    /// The front end drove this in eight steps: plan the wallets, group them by
    /// normalized address, reset or advance each group's page, build the token
    /// descriptor list from its mirror of the token preferences, fetch, plan
    /// the records, convert them, and merge. All eight are core's; what comes
    /// back is what changed and what to put on the diagnostics screen.
    ///
    /// `load_more` advances each group's page instead of restarting at the
    /// first, and leaves a group that already reported a short page alone.
    pub async fn refresh_evm_chain_history(
        &self,
        chain_id: String,
        wallet_ids: Vec<String>,
        load_more: bool,
        page_size: Option<u32>,
    ) -> Result<HistoryRefreshOutcome, SpectraBridgeError> {
        let chain = Chain::from_str_id(&chain_id)
            .filter(|chain| chain.is_evm())
            .ok_or_else(|| SpectraBridgeError::from(format!("{chain_id:?} is not an EVM chain")))?;
        let (groups, descriptors, wallet_names, networks) = {
            let state = self.app_state().await;
            let targets = targets(&state, chain, &wallet_ids);
            let plan =
                crate::fetch::plan_evm_refresh_targets(crate::fetch::EvmRefreshTargetsRequest {
                    chain_name: chain.chain_display_name().to_string(),
                    wallets: targets
                        .iter()
                        .map(|target| crate::fetch::RefreshWalletInput {
                            wallet_id: target.wallet_id.clone(),
                            selected_chain: chain.chain_display_name().to_string(),
                            addresses: vec![target.address.clone()],
                        })
                        .collect(),
                    allowed_wallet_ids: None,
                    // Wallets that share an address share a page; loading more
                    // walks each wallet's own cursor.
                    group_by_normalized_address: !load_more,
                });
            let mut names = std::collections::HashMap::new();
            let mut networks = std::collections::HashMap::new();
            for target in targets {
                networks.insert(target.wallet_id.clone(), target.network);
                names.insert(target.wallet_id, target.wallet_name);
            }
            (
                if load_more {
                    plan.wallet_targets
                        .into_iter()
                        .map(|target| (vec![target.wallet_id], target.normalized_address))
                        .collect::<Vec<_>>()
                } else {
                    plan.grouped_targets
                        .into_iter()
                        .map(|group| (group.wallet_ids, group.normalized_address))
                        .collect()
                },
                token_descriptors(&state, chain),
                names,
                networks,
            )
        };
        if groups.is_empty() {
            return Ok(HistoryRefreshOutcome::nothing());
        }

        let page_size = page_size
            .unwrap_or(DEFAULT_EVM_PAGE_SIZE)
            .clamp(MIN_EVM_PAGE_SIZE, MAX_EVM_PAGE_SIZE);
        let native = crate::fetch::history_decode::history_evm_native_asset(
            chain.chain_display_name().to_string(),
        )
        .unwrap_or(crate::fetch::history_decode::EvmNativeAsset {
            asset_name: "Ether".to_string(),
            symbol: "ETH".to_string(),
        });

        let mut incoming = Vec::new();
        let mut diagnostics = Vec::new();
        let mut wallets_refreshed = 0;
        let mut wallets_failed = 0;
        let mut exhausted = true;
        for (group_wallet_ids, normalized_address) in groups {
            let Some(first) = group_wallet_ids.first().cloned() else {
                continue;
            };
            if !load_more {
                for wallet_id in &group_wallet_ids {
                    self.reset_history(
                        crate::service::history_cursor::HistoryScope::ChainAndWallet {
                            chain_id: chain_id.clone(),
                            wallet_id: wallet_id.clone(),
                        },
                    );
                    self.set_history_page(chain_id.clone(), wallet_id.clone(), 1, false);
                }
            } else if self
                .history_cursor(chain_id.clone(), first.clone())
                .is_exhausted
            {
                continue;
            }
            let current = self
                .history_cursor(chain_id.clone(), first.clone())
                .next_page
                .max(1);
            let page = if load_more { current + 1 } else { current };

            // Fetched from the network this group's wallets are on; the
            // records below are filed under the family.
            let network = networks.get(&first).copied().unwrap_or(chain);
            let fetched = self
                .fetch_evm_history_page(
                    network.str_id().to_string(),
                    normalized_address.clone(),
                    descriptors.clone(),
                    page,
                    page_size,
                )
                .await;
            let decoded = match fetched {
                Ok(decoded) => decoded,
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
            wallets_refreshed += group_wallet_ids.len() as u32;
            let token_count = decoded.tokens.len() as u32;
            let is_last_page = decoded.tokens.len() < page_size as usize
                && decoded.native.len() < page_size as usize;
            exhausted = exhausted && is_last_page;
            for wallet_id in &group_wallet_ids {
                self.set_history_page(chain_id.clone(), wallet_id.clone(), page, is_last_page);
                diagnostics.push(HistoryWalletDiagnostics {
                    wallet_id: wallet_id.clone(),
                    identifier: normalized_address.clone(),
                    source_used: "rust/etherscan".to_string(),
                    transaction_count: token_count,
                    next_cursor: None,
                    error: None,
                });
            }

            let planned = crate::fetch::history_decode::plan_evm_transaction_records(
                crate::fetch::history_decode::EvmTransactionRecordRequest {
                    decoded_page: decoded,
                    normalized_address: normalized_address.clone(),
                    chain_name: chain.chain_display_name().to_string(),
                    token_source_used: Some("rust/etherscan".to_string()),
                    native_asset_name: native.asset_name.clone(),
                    native_asset_symbol: native.symbol.clone(),
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

        let change = self
            .apply_transaction_command(crate::service::types::TransactionCommand::Merge {
                incoming,
                chain_name: chain.chain_display_name().to_string(),
                preserve_created_at_sentinel_unix: Some(SENTINEL_CREATED_AT_UNIX),
            })
            .await?;

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
    planned: crate::fetch::history_decode::EvmPlannedTransactionRecord,
) -> crate::fetch::transactions::CoreTransactionRecord {
    crate::fetch::transactions::CoreTransactionRecord {
        id: crate::store::new_transaction_id(),
        wallet_id: Some(planned.wallet_id),
        kind: planned.kind,
        status: "confirmed".to_string(),
        wallet_name: planned.wallet_name,
        asset_name: planned.asset_name,
        symbol: planned.symbol,
        chain_name: planned.chain_name,
        amount: planned.amount_decimal.parse().unwrap_or(0.0),
        address: planned.counterparty,
        transaction_hash: Some(planned.transaction_hash).filter(|hash| !hash.is_empty()),
        ethereum_nonce: None,
        receipt_block_number: Some(planned.block_number),
        receipt_gas_used: None,
        receipt_effective_gas_price_gwei: None,
        receipt_network_fee_eth: None,
        fee_priority_raw: None,
        fee_rate_description: None,
        confirmation_count: None,
        dogecoin_confirmed_network_fee_doge: None,
        dogecoin_estimated_fee_rate_doge_per_kb: None,
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

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Fetch and merge one UTXO chain's history across each wallet's known
    /// addresses.
    ///
    /// A UTXO wallet spends from many addresses, so one transaction shows up
    /// once per address it touched; the records are netted per transaction
    /// before they are stored. The front end drove this: it asked core for
    /// each wallet's known addresses, handed them back inside a planning
    /// request, fetched per address, called the aggregator, built the records
    /// and sent them to be merged. Core has the addresses — they are its
    /// keypool — and does the rest with them.
    ///
    /// These providers return a whole history in one call, so the first page is
    /// also the last; `load_more` therefore has nothing to fetch for a wallet
    /// already marked exhausted.
    pub async fn refresh_utxo_chain_history(
        &self,
        chain_id: String,
        wallet_ids: Vec<String>,
        load_more: bool,
    ) -> Result<HistoryRefreshOutcome, SpectraBridgeError> {
        let chain = Chain::from_str_id(&chain_id)
            .ok_or_else(|| SpectraBridgeError::from(format!("unknown chain {chain_id:?}")))?;
        let wallets: Vec<(String, String, Chain)> = {
            let state = self.app_state().await;
            targets(&state, chain, &wallet_ids)
                .into_iter()
                .map(|target| (target.wallet_id, target.wallet_name, target.network))
                .collect()
        };

        let mut incoming = Vec::new();
        let mut wallets_refreshed = 0;
        let mut wallets_failed = 0;
        for (wallet_id, wallet_name, network) in wallets {
            let addresses = self
                .known_utxo_addresses(wallet_id.clone(), chain_id.clone())
                .await
                .unwrap_or_default();
            if addresses.is_empty() {
                continue;
            }
            if load_more {
                if self
                    .history_cursor(chain_id.clone(), wallet_id.clone())
                    .is_exhausted
                {
                    continue;
                }
            } else {
                self.reset_history(
                    crate::service::history_cursor::HistoryScope::ChainAndWallet {
                        chain_id: chain_id.clone(),
                        wallet_id: wallet_id.clone(),
                    },
                );
            }

            let mut entries = Vec::new();
            let mut failed = false;
            for address in &addresses {
                match self
                    .fetch_normalized_history(network.str_id().to_string(), address.clone())
                    .await
                {
                    Ok(fetched) => entries.extend(fetched),
                    Err(_) => failed = true,
                }
            }
            // Netting is over the whole address set, so an address that did not
            // answer is a wrong amount rather than a missing row: a transaction
            // whose change went to that address nets to the legs that did
            // answer, and the figure stored is one no address agrees with.
            // Merge nothing for the wallet and count it failed — these
            // providers hand back a whole history at once, so a later refresh
            // has everything to net again, and the cursor below is not written,
            // which leaves the wallet loadable rather than exhausted.
            if failed {
                wallets_failed += 1;
                continue;
            }
            wallets_refreshed += 1;
            // A whole history in one call, so the page just fetched is the last.
            self.set_history_page(chain_id.clone(), wallet_id.clone(), 1, true);

            let aggregated = crate::fetch::history_decode::history_aggregate_by_transaction(
                crate::fetch::history_decode::MultiAddressAggregateInput {
                    own_addresses: addresses,
                    entries,
                },
            );
            incoming.extend(aggregated.into_iter().map(|aggregate| {
                aggregated_record(&wallet_id, &wallet_name, chain, &chain_id, aggregate)
            }));
        }

        if wallets_refreshed == 0 && wallets_failed == 0 {
            return Ok(HistoryRefreshOutcome::nothing());
        }

        let change = self
            .apply_transaction_command(crate::service::types::TransactionCommand::Merge {
                incoming,
                chain_name: chain.chain_display_name().to_string(),
                preserve_created_at_sentinel_unix: Some(SENTINEL_CREATED_AT_UNIX),
            })
            .await?;

        Ok(HistoryRefreshOutcome {
            wallets_refreshed,
            wallets_failed,
            added: change.added.len() as u32,
            updated: change.updated.len() as u32,
            // These providers answer with a whole history at once, so the page
            // just merged is the last one.
            exhausted: true,
            diagnostics: Vec::new(),
        })
    }
}

/// One netted transaction as a stored record.
fn aggregated_record(
    wallet_id: &str,
    wallet_name: &str,
    chain: Chain,
    chain_id: &str,
    aggregate: crate::fetch::history_decode::AggregatedTransaction,
) -> crate::fetch::transactions::CoreTransactionRecord {
    crate::fetch::transactions::CoreTransactionRecord {
        id: crate::store::new_transaction_id(),
        wallet_id: Some(wallet_id.to_string()),
        kind: aggregate.kind,
        status: aggregate.status,
        wallet_name: wallet_name.to_string(),
        asset_name: chain.chain_display_name().to_string(),
        symbol: chain.coin_symbol().to_string(),
        chain_name: chain.chain_display_name().to_string(),
        amount: aggregate.amount,
        address: aggregate.counterparty,
        transaction_hash: Some(aggregate.hash).filter(|hash| !hash.is_empty()),
        ethereum_nonce: None,
        receipt_block_number: aggregate.block_number,
        receipt_gas_used: None,
        receipt_effective_gas_price_gwei: None,
        receipt_network_fee_eth: None,
        fee_priority_raw: None,
        fee_rate_description: None,
        confirmation_count: None,
        dogecoin_confirmed_network_fee_doge: None,
        dogecoin_estimated_fee_rate_doge_per_kb: None,
        used_change_output: None,
        source_derivation_path: None,
        change_derivation_path: None,
        source_address: None,
        change_address: None,
        signed_transaction_payload: None,
        signed_transaction_payload_format: None,
        failure_reason: None,
        transaction_history_source: Some(format!("{chain_id}.providers")),
        // An aggregate with no known timestamp keeps the sentinel the merge
        // recognises rather than claiming it happened now.
        created_at_unix: if aggregate.created_at_unix > 0.0 {
            aggregate.created_at_unix
        } else {
            SENTINEL_CREATED_AT_UNIX
        },
    }
}

#[cfg(test)]
mod tests;
