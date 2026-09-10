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
    fn nothing() -> Self {
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
const SENTINEL_CREATED_AT_UNIX: f64 = -62_135_596_800.0;

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

/// How many records a Bitcoin page asks for when the caller names no limit.
const DEFAULT_BITCOIN_LIMIT: u32 = 20;
const MIN_BITCOIN_LIMIT: u32 = 10;
const MAX_BITCOIN_LIMIT: u32 = 100;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Fetch and merge Bitcoin history for its wallets.
    ///
    /// Bitcoin is the one chain with an account xpub, so its history is the HD
    /// range's rather than one address's: with a seed to hand, the account
    /// xpub is derived and the whole range walked; failing that a stored xpub
    /// is walked, and failing that the single stored address is fetched. The
    /// front end held all three arms — it read the seed out of the Keychain,
    /// cut the account path out of the wallet's derivation path by string
    /// surgery, derived the xpub, chose between the results and built the
    /// records. All of it is here, over the seed and paths core already holds.
    ///
    /// A wallet whose seed is behind a password derives no xpub — the same as
    /// before, where the Keychain read simply returned nothing — and falls
    /// through to its stored address.
    pub async fn refresh_bitcoin_history(
        &self,
        wallet_ids: Vec<String>,
        load_more: bool,
        limit: Option<u32>,
    ) -> Result<HistoryRefreshOutcome, SpectraBridgeError> {
        let chain = Chain::Bitcoin;
        let chain_id = chain.str_id().to_string();
        let limit = limit
            .unwrap_or(DEFAULT_BITCOIN_LIMIT)
            .clamp(MIN_BITCOIN_LIMIT, MAX_BITCOIN_LIMIT);
        let (wallets, settings): (Vec<crate::store::state::WalletSummary>, _) = {
            let state = self.app_state().await;
            let wallets = state
                .wallets
                .iter()
                .filter(|wallet| wallet.chain_name == chain.chain_display_name())
                .filter(|wallet| {
                    wallet_ids.is_empty()
                        || wallet_ids
                            .iter()
                            .any(|id| id.eq_ignore_ascii_case(&wallet.id))
                })
                .cloned()
                .collect();
            (wallets, state.settings.clone())
        };
        if wallets.is_empty() {
            return Ok(HistoryRefreshOutcome::nothing());
        }

        let mut incoming = Vec::new();
        let mut diagnostics = Vec::new();
        let mut wallets_refreshed = 0;
        let mut wallets_failed = 0;
        let mut exhausted = true;
        for mut wallet in wallets {
            // The clone carries the wallet's derivation secrets in the clear.
            // Taking them into the guard wipes them at the end of the
            // iteration rather than leaving them in a dropped `WalletSummary`.
            let overrides = crate::store::wallet_domain::SensitiveOverrides::take_from(&mut wallet);
            if load_more {
                if self
                    .history_cursor(chain_id.clone(), wallet.id.clone())
                    .is_exhausted
                {
                    continue;
                }
            } else {
                self.reset_history(
                    crate::service::history_cursor::HistoryScope::ChainAndWallet {
                        chain_id: chain_id.clone(),
                        wallet_id: wallet.id.clone(),
                    },
                );
            }
            let has_cursor = self
                .history_cursor(chain_id.clone(), wallet.id.clone())
                .next_cursor
                .is_some();

            let network = wallet
                .network_chain(&settings)
                .filter(|network| network.mainnet_counterpart() == chain)
                .unwrap_or(chain);
            match self
                .bitcoin_history_page(&wallet, &overrides, network, limit, has_cursor)
                .await
            {
                Ok(page) => {
                    wallets_refreshed += 1;
                    exhausted = exhausted && page.next_cursor.is_none();
                    self.advance_history_cursor(
                        chain_id.clone(),
                        wallet.id.clone(),
                        page.next_cursor.clone(),
                    );
                    diagnostics.push(HistoryWalletDiagnostics {
                        wallet_id: wallet.id.clone(),
                        identifier: page.identifier.clone(),
                        source_used: page.source_used.clone(),
                        transaction_count: page.snapshots.len() as u32,
                        next_cursor: page.next_cursor.clone(),
                        error: None,
                    });
                    incoming.extend(page.snapshots.into_iter().map(|snapshot| {
                        bitcoin_record(&wallet, chain, &page.source_used, snapshot)
                    }));
                }
                Err(error) => {
                    wallets_failed += 1;
                    exhausted = false;
                    // The cursor is left where it was. `advance_history_cursor`
                    // with `None` means "the chain says there is no more",
                    // which a fetch that failed did not say: it marked the
                    // wallet exhausted, so this outcome reported more to load
                    // while the wallet's own cursor refused to load it, and
                    // "load more" did nothing until a pull-to-refresh reset it.
                    diagnostics.push(HistoryWalletDiagnostics {
                        wallet_id: wallet.id.clone(),
                        identifier: bitcoin_identifier(&wallet).unwrap_or_default(),
                        source_used: "none".to_string(),
                        transaction_count: 0,
                        next_cursor: None,
                        error: Some(error.to_string()),
                    });
                }
            }
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
            exhausted,
            diagnostics,
        })
    }
}

/// One wallet's Bitcoin history page, and which source answered.
pub(crate) struct BitcoinHistoryPage {
    pub snapshots: Vec<crate::history::CoreBitcoinHistorySnapshot>,
    pub next_cursor: Option<String>,
    pub source_used: String,
    pub identifier: String,
}

/// What a diagnostics row calls this wallet: its address, else its xpub, else
/// its name.
fn bitcoin_identifier(wallet: &crate::store::state::WalletSummary) -> Option<String> {
    wallet
        .address_on(Chain::Bitcoin)
        .map(str::to_string)
        .or_else(|| wallet.xpub.clone())
        .or_else(|| Some(wallet.name.clone()))
}

impl WalletService {
    /// Bitcoin's three sources, in order: the HD range under an account xpub
    /// derived from the seed, the stored address, then a stored xpub.
    async fn bitcoin_history_page(
        &self,
        wallet: &crate::store::state::WalletSummary,
        overrides: &crate::store::wallet_domain::SensitiveOverrides,
        network: Chain,
        limit: u32,
        has_cursor: bool,
    ) -> Result<BitcoinHistoryPage, SpectraBridgeError> {
        // Only on a first page: walking the HD range again for page two would
        // return the same records.
        if !has_cursor {
            if let Some(xpub) = self.bitcoin_account_xpub(wallet, overrides) {
                let snapshots = self
                    .fetch_bitcoin_hd_history_page(xpub.clone(), limit as u64)
                    .await?;
                // TODO: the HD walk reads Bitcoin's endpoints; a testnet wallet
                // walks its own network's once `fetch_bitcoin_hd_history_page`
                // takes a chain.
                if !snapshots.is_empty() {
                    return Ok(BitcoinHistoryPage {
                        snapshots,
                        next_cursor: None,
                        source_used: "rust.hd".to_string(),
                        identifier: xpub,
                    });
                }
            }
        }
        if let Some(address) = wallet
            .address_on(Chain::Bitcoin)
            .map(str::trim)
            .filter(|address| !address.is_empty())
            .map(str::to_string)
        {
            let entries = self
                .fetch_normalized_history(network.str_id().to_string(), address.clone())
                .await?;
            let has_more = entries.len() > limit as usize;
            let snapshots: Vec<_> = entries
                .into_iter()
                .take(limit as usize)
                .map(|entry| crate::history::CoreBitcoinHistorySnapshot {
                    txid: entry.tx_hash,
                    amount_btc: entry.amount,
                    kind: entry.kind,
                    status: entry.status,
                    counterparty_address: entry.counterparty,
                    block_height: entry.block_height,
                    // An entry with no timestamp keeps the sentinel the merge
                    // recognises. The front end stamped it with the time of the
                    // refresh, which sorted an undated transaction to the top
                    // of the list and moved it there again on every refresh.
                    created_at_unix: if entry.timestamp > 0.0 {
                        entry.timestamp
                    } else {
                        SENTINEL_CREATED_AT_UNIX
                    },
                })
                .collect();
            let next_cursor = has_more
                .then(|| snapshots.last().map(|snapshot| snapshot.txid.clone()))
                .flatten();
            return Ok(BitcoinHistoryPage {
                snapshots,
                next_cursor,
                source_used: "rust".to_string(),
                identifier: address,
            });
        }
        if let Some(xpub) = wallet
            .xpub
            .as_deref()
            .map(str::trim)
            .filter(|xpub| !xpub.is_empty())
            .map(str::to_string)
        {
            let snapshots = self
                .fetch_bitcoin_hd_history_page(xpub.clone(), limit as u64)
                .await?;
            return Ok(BitcoinHistoryPage {
                snapshots,
                next_cursor: None,
                source_used: "rust.hd".to_string(),
                identifier: xpub,
            });
        }
        Err(SpectraBridgeError::from(
            "this wallet has no Bitcoin address or account xpub".to_string(),
        ))
    }

    /// The account xpub for this wallet's Bitcoin path, when its seed is
    /// readable. A sealed wallet has none without its password.
    ///
    /// The phrase never leaves a `Zeroizing`: `wallet_seed_phrase` hands back a
    /// plain `String` copy of it, which drops without being wiped, and this
    /// runs on every Bitcoin refresh rather than only at send time.
    /// `load_signing_material` is the same read the send identity does, and
    /// keeps the wipe.
    fn bitcoin_account_xpub(
        &self,
        wallet: &crate::store::state::WalletSummary,
        overrides: &crate::store::wallet_domain::SensitiveOverrides,
    ) -> Option<String> {
        use crate::store::wallet_secrets::{load_signing_material, SigningMaterial};
        let secrets = self.secrets().ok()?;
        // A private-key wallet has no range to walk; only a phrase derives one.
        let seed = match load_signing_material(&*secrets, &wallet.id, None).ok()? {
            SigningMaterial::Mnemonic(seed) => seed,
            SigningMaterial::PrivateKey(_) => return None,
        };
        if seed.trim().is_empty() {
            return None;
        }
        // The account is the first four segments — `m/84'/0'/0'` — of the path
        // the wallet derives with. This was string surgery on the front end
        // side, over a path it read out of its own copy of the wallet.
        let path = wallet.derivation_path.clone().unwrap_or_else(|| {
            crate::app_core::default_path_from_catalog(Chain::Bitcoin.chain_display_name())
                .unwrap_or_default()
        });
        let account_path = path.split('/').take(4).collect::<Vec<_>>().join("/");
        // The wallet's own passphrase, not the empty string. Derived without
        // it, the xpub is a different wallet's: the range walked belongs to
        // nobody here, comes back empty and the refresh quietly falls through
        // to the single stored address — so a passphrase wallet never had HD
        // history at all. The send identity has always derived with it.
        crate::derivation::xpub_walker::derive_account_xpub(
            &seed,
            overrides.passphrase().unwrap_or_default(),
            &account_path,
        )
        .ok()
    }
}

/// One HD snapshot as a stored record.
fn bitcoin_record(
    wallet: &crate::store::state::WalletSummary,
    chain: Chain,
    source_used: &str,
    snapshot: crate::history::CoreBitcoinHistorySnapshot,
) -> crate::fetch::transactions::CoreTransactionRecord {
    crate::fetch::transactions::CoreTransactionRecord {
        id: crate::store::new_transaction_id(),
        wallet_id: Some(wallet.id.clone()),
        kind: snapshot.kind,
        status: snapshot.status,
        wallet_name: wallet.name.clone(),
        asset_name: chain.chain_display_name().to_string(),
        symbol: chain.coin_symbol().to_string(),
        chain_name: chain.chain_display_name().to_string(),
        amount: snapshot.amount_btc,
        address: snapshot.counterparty_address,
        transaction_hash: Some(snapshot.txid).filter(|txid| !txid.is_empty()),
        ethereum_nonce: None,
        receipt_block_number: snapshot.block_height,
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
        transaction_history_source: Some(source_used.to_string()),
        created_at_unix: snapshot.created_at_unix,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::state::{AppSettings, WalletAddress, WalletSummary};

    fn wallet(id: &str, chain: Chain, addresses: &[(Chain, &str)]) -> WalletSummary {
        WalletSummary {
            id: id.to_string(),
            name: format!("{id} wallet"),
            is_watch_only: false,
            chain_name: chain.chain_display_name().to_string(),
            include_in_portfolio_total: true,
            network_mode: None,
            xpub: None,
            derivation_preset: "standard".to_string(),
            derivation_path: None,
            derivation_overrides: Default::default(),
            holdings: Vec::new(),
            addresses: addresses
                .iter()
                .map(|(chain, address)| WalletAddress {
                    chain_name: chain.chain_display_name().to_string(),
                    address: (*address).to_string(),
                    kind: "receive".to_string(),
                    derivation_path: None,
                })
                .collect(),
        }
    }

    /// Only the wallets on the chain, only the ones with an address, and the
    /// address for the network each is on.
    #[test]
    fn targets_are_the_chains_wallets_that_have_an_address() {
        let mut state = CoreAppState::default();
        state.wallets = vec![
            wallet("w1", Chain::Solana, &[(Chain::Solana, "So1")]),
            wallet("w2", Chain::Solana, &[]),
            wallet("w3", Chain::Bitcoin, &[(Chain::Bitcoin, "bc1")]),
        ];

        let solana = targets(&state, Chain::Solana, &[]);
        assert_eq!(solana.len(), 1);
        assert_eq!(solana[0].wallet_id, "w1");
        assert_eq!(solana[0].address, "So1");
        assert_eq!(solana[0].wallet_name, "w1 wallet");

        // Scoped to one wallet, case-insensitively — ids cross the boundary in
        // whichever case the front end holds them.
        assert!(targets(&state, Chain::Solana, &["W1".to_string()]).len() == 1);
        assert!(targets(&state, Chain::Solana, &["w3".to_string()]).is_empty());
    }

    /// A wallet on a testnet fetches that network's history, and the record
    /// still lands under the family.
    ///
    /// Filing under the network would rename the asset and leave the mainnet
    /// holding beside it; fetching from the family read the mainnet chain and
    /// found nothing. The two are separate answers.
    #[test]
    fn a_testnet_wallet_fetches_its_network_and_files_under_its_family() {
        let mut state = CoreAppState::default();
        state.wallets = vec![wallet(
            "w1",
            Chain::Bitcoin,
            &[
                (Chain::Bitcoin, "bc1main"),
                (Chain::BitcoinTestnet4, "tb1test"),
            ],
        )];
        state.settings.network_chain_by_family.insert(
            Chain::Bitcoin.str_id().to_string(),
            Chain::BitcoinTestnet4.str_id().to_string(),
        );

        let target = &targets(&state, Chain::Bitcoin, &[])[0];
        assert_eq!(target.network, Chain::BitcoinTestnet4, "fetched from");
        assert_eq!(target.address, "tb1test");

        let record = record_for(
            target,
            Chain::Bitcoin,
            crate::fetch::history_decode::NormalizedHistoryItem {
                kind: "receive".to_string(),
                status: "confirmed".to_string(),
                asset_name: "Bitcoin Testnet4".to_string(),
                symbol: "BTC".to_string(),
                chain_name: "Bitcoin Testnet4".to_string(),
                amount: 1.0,
                counterparty: "tb1other".to_string(),
                tx_hash: "abc".to_string(),
                block_height: None,
                timestamp: 1.0,
            },
        );
        assert_eq!(record.chain_name, "Bitcoin", "filed under");
    }

    /// A wallet on a testnet fetches the address for that network.
    #[test]
    fn a_target_follows_the_network_the_wallet_is_on() {
        let mut state = CoreAppState::default();
        state.wallets = vec![wallet(
            "w1",
            Chain::Bitcoin,
            &[
                (Chain::Bitcoin, "bc1main"),
                (Chain::BitcoinTestnet4, "tb1test"),
            ],
        )];
        assert_eq!(targets(&state, Chain::Bitcoin, &[])[0].address, "bc1main");

        state.settings = AppSettings {
            network_chain_by_family: [(
                Chain::Bitcoin.str_id().to_string(),
                Chain::BitcoinTestnet4.str_id().to_string(),
            )]
            .into_iter()
            .collect(),
            ..AppSettings::default()
        };
        assert_eq!(targets(&state, Chain::Bitcoin, &[])[0].address, "tb1test");
    }

    /// The record carries the wallet the entry belongs to, and an id a front
    /// end can parse back.
    #[test]
    fn a_record_names_its_wallet_and_carries_a_uuid() {
        let target = Target {
            wallet_id: "w1".to_string(),
            wallet_name: "Main".to_string(),
            address: "So1".to_string(),
            network: Chain::Solana,
        };
        let record = record_for(
            &target,
            Chain::Solana,
            crate::fetch::history_decode::NormalizedHistoryItem {
                kind: "receive".to_string(),
                status: "confirmed".to_string(),
                asset_name: "Solana".to_string(),
                symbol: "SOL".to_string(),
                chain_name: "Solana".to_string(),
                amount: 1.5,
                counterparty: "So2".to_string(),
                tx_hash: "sig".to_string(),
                block_height: Some(7),
                timestamp: 1_700_000_000.0,
            },
        );
        assert_eq!(record.wallet_id.as_deref(), Some("w1"));
        assert_eq!(record.wallet_name, "Main");
        assert_eq!(record.transaction_history_source.as_deref(), Some("rust"));
        assert_eq!(record.created_at_unix, 1_700_000_000.0);
        assert_eq!(record.receipt_block_number, Some(7));
        // Parseable as a UUID: a front end drops a row whose id is not.
        assert_eq!(record.id.len(), 36);
        assert_eq!(
            record.id.chars().filter(|c| *c == '-').count(),
            4,
            "{}",
            record.id
        );
        assert_eq!(&record.id[14..15], "4", "version nibble: {}", record.id);

        // An empty hash is no hash, not an empty one.
        let mut entry = crate::fetch::history_decode::NormalizedHistoryItem {
            kind: "send".to_string(),
            status: "confirmed".to_string(),
            asset_name: "Solana".to_string(),
            symbol: "SOL".to_string(),
            chain_name: "Solana".to_string(),
            amount: 0.0,
            counterparty: String::new(),
            tx_hash: String::new(),
            block_height: None,
            timestamp: 0.0,
        };
        assert_eq!(
            record_for(&target, Chain::Solana, entry.clone()).transaction_hash,
            None
        );
        entry.tx_hash = "abc".to_string();
        assert_eq!(
            record_for(&target, Chain::Solana, entry)
                .transaction_hash
                .as_deref(),
            Some("abc")
        );
    }

    /// The tokens a page decodes with are the user's enabled ones for that
    /// chain, contracts in their canonical form.
    #[test]
    fn descriptors_are_the_enabled_tokens_for_the_chain() {
        use crate::store::wallet_domain::{CoreTokenPreferenceCategory, CoreTokenPreferenceEntry};
        fn entry(chain: &str, contract: &str, enabled: bool) -> CoreTokenPreferenceEntry {
            CoreTokenPreferenceEntry {
                token: crate::tokens::TokenEntry {
                    chain: chain.to_string(),
                    name: "Token".to_string(),
                    symbol: "TKN".to_string(),
                    token_standard: "erc20".to_string(),
                    contract: contract.to_string(),
                    coingecko_id: String::new(),
                    decimals: 6,
                    tags: Vec::new(),
                    color: String::new(),
                    asset_name: String::new(),
                    enabled: true,
                },
                category: CoreTokenPreferenceCategory::Stablecoin,
                is_built_in: true,
                is_enabled: enabled,
            }
        }
        let mut state = CoreAppState::default();
        state.token_preferences = vec![
            entry(
                "ethereum",
                "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                true,
            ),
            entry(
                "ethereum",
                "0xBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB",
                false,
            ),
            entry(
                "solana",
                "So11111111111111111111111111111111111111112",
                true,
            ),
        ];

        let descriptors = token_descriptors(&state, Chain::Ethereum);
        assert_eq!(descriptors.len(), 1);
        assert_eq!(
            descriptors[0].contract, "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "the contract crosses in its canonical form"
        );
        assert_eq!(descriptors[0].decimals, 6);
        // A chain that hosts no known tokens decodes none.
        assert!(token_descriptors(&state, Chain::EthereumClassic).is_empty());
    }

    /// A chain no explorer serves fails per wallet and says so, rather than
    /// raising and losing the wallets that did answer.
    ///
    /// Offline: `explorer_query_url` refuses before any request is made.
    #[tokio::test]
    async fn a_chain_no_explorer_serves_counts_a_failure_and_reports_it() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        // The merge writes, so the store has to be open — a failed page still
        // ends in a merge of nothing.
        let db = std::env::temp_dir()
            .join(format!(
                "spectra-evm-history-{}.sqlite",
                crate::store::new_event_id()
            ))
            .to_string_lossy()
            .into_owned();
        service.open_state(db).await.expect("open");
        service
            .apply_state_command(crate::store::state::StateCommand::UpsertWallet {
                wallet: wallet("w1", Chain::Cronos, &[(Chain::Ethereum, "0xabc")]),
            })
            .await
            .expect("wallet");

        let outcome = service
            .refresh_evm_chain_history(Chain::Cronos.str_id().to_string(), Vec::new(), false, None)
            .await
            .expect("refresh");
        assert_eq!(outcome.wallets_refreshed, 0);
        assert_eq!(outcome.wallets_failed, 1);
        assert_eq!(outcome.added, 0);
        assert!(!outcome.exhausted, "a failed page is not the last page");
        assert_eq!(outcome.diagnostics.len(), 1);
        assert_eq!(outcome.diagnostics[0].wallet_id, "w1");
        assert_eq!(outcome.diagnostics[0].source_used, "none");
        assert!(outcome.diagnostics[0].error.is_some());
    }

    /// A UTXO wallet with no known addresses is not refreshed, and asking for
    /// an unknown chain is refused.
    ///
    /// Offline: the keypool is empty, so no provider is reached.
    #[tokio::test]
    async fn a_utxo_wallet_with_no_known_addresses_is_skipped() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let db = std::env::temp_dir()
            .join(format!(
                "spectra-utxo-history-{}.sqlite",
                crate::store::new_event_id()
            ))
            .to_string_lossy()
            .into_owned();
        service.open_state(db).await.expect("open");
        service
            .apply_state_command(crate::store::state::StateCommand::UpsertWallet {
                wallet: wallet("w1", Chain::Litecoin, &[(Chain::Litecoin, "ltc1")]),
            })
            .await
            .expect("wallet");

        let outcome = service
            .refresh_utxo_chain_history(Chain::Litecoin.str_id().to_string(), Vec::new(), false)
            .await
            .expect("refresh");
        assert_eq!(outcome.wallets_refreshed, 0);
        assert_eq!(outcome.wallets_failed, 0);
        assert_eq!(outcome.added, 0);

        assert!(service
            .refresh_utxo_chain_history("not-a-chain".to_string(), Vec::new(), false)
            .await
            .is_err());
    }

    /// A UTXO wallet one of whose addresses did not answer stores nothing.
    ///
    /// Netting is over the whole address set, so an address that did not
    /// answer is a wrong amount rather than a missing row: a transaction whose
    /// change went there nets to the legs that did answer. The refresh used to
    /// aggregate and merge whatever came back, so a figure no address agreed
    /// with was stored — here the send leg alone, unnetted by the change leg
    /// the failing address holds. The wallet is counted failed, nothing is
    /// merged for it, and its cursor is left loadable so a later refresh nets
    /// the whole set again.
    ///
    /// One address answers with a transaction and the other refuses, which is
    /// the case the offline gate cannot reach — hence the mock backend.
    #[tokio::test]
    async fn a_utxo_wallet_whose_address_did_not_answer_stores_nothing() {
        use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};
        const ANSWERS: &str = "ltc1qw508d6qejxtdg4y5r3zarvary0c5xw7kgmn4n9";
        const REFUSES: &str = "LhK2kQwiaAvhjWY799cZvMyYwnQAcxkarr";

        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(|request: &Request| {
                if !request.url.path().contains(ANSWERS) {
                    return ResponseTemplate::new(500);
                }
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "transactions": [{
                        "txid": "aa11",
                        "blockTime": 1_700_000_000u64,
                        "blockHeight": 900_000u64,
                        "value": "100000",
                        "fees": "500",
                        "vin": [{ "addresses": [ANSWERS] }],
                    }]
                }))
            })
            .mount(&server)
            .await;

        let service = WalletService::new_typed(vec![crate::service::ChainEndpoints {
            chain_id: Chain::Litecoin.str_id().into(),
            endpoints: vec![server.uri()],
            api_key: None,
        }])
        .expect("service");
        let db = std::env::temp_dir()
            .join(format!(
                "spectra-utxo-fail-{}.sqlite",
                crate::store::new_event_id()
            ))
            .to_string_lossy()
            .into_owned();
        service.open_state(db).await.expect("open");
        service
            .apply_state_command(crate::store::state::StateCommand::UpsertWallet {
                wallet: wallet("w1", Chain::Litecoin, &[(Chain::Litecoin, ANSWERS)]),
            })
            .await
            .expect("wallet");
        // The second address is the wallet's keypool, which is where the
        // refresh reads the rest of the set from.
        service
            .register_owned_address(
                "w1".to_string(),
                Chain::Litecoin.chain_display_name().to_string(),
                REFUSES.to_string(),
                None,
                None,
                None,
            )
            .await
            .expect("owned address");

        let outcome = service
            .refresh_utxo_chain_history(Chain::Litecoin.str_id().to_string(), Vec::new(), false)
            .await
            .expect("refresh");
        assert_eq!(outcome.wallets_refreshed, 0);
        assert_eq!(outcome.wallets_failed, 1);
        assert_eq!(
            outcome.added, 0,
            "a half-fetched transaction nets to a figure no address agrees with"
        );
        assert_eq!(outcome.updated, 0);
        assert!(
            !service
                .history_cursor(Chain::Litecoin.str_id().to_string(), "w1".to_string())
                .is_exhausted,
            "a wallet that failed must stay loadable"
        );
    }

    /// A Bitcoin wallet with neither an address nor an xpub is a failure with
    /// a reason, not a silent skip.
    ///
    /// Offline: the three sources are tried in order and none of them has an
    /// identifier to fetch for, so no provider is reached.
    #[tokio::test]
    async fn a_bitcoin_wallet_with_nothing_to_fetch_for_says_so() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let db = std::env::temp_dir()
            .join(format!(
                "spectra-btc-history-{}.sqlite",
                crate::store::new_event_id()
            ))
            .to_string_lossy()
            .into_owned();
        service.open_state(db).await.expect("open");
        service
            .apply_state_command(crate::store::state::StateCommand::UpsertWallet {
                wallet: wallet("w1", Chain::Bitcoin, &[]),
            })
            .await
            .expect("wallet");

        let outcome = service
            .refresh_bitcoin_history(Vec::new(), false, None)
            .await
            .expect("refresh");
        assert_eq!(outcome.wallets_refreshed, 0);
        assert_eq!(outcome.wallets_failed, 1);
        assert_eq!(outcome.added, 0);
        assert_eq!(outcome.diagnostics.len(), 1);
        assert_eq!(outcome.diagnostics[0].source_used, "none");
        assert!(outcome.diagnostics[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("no Bitcoin address")));
        // The row names the wallet when it has nothing else to be named by.
        assert_eq!(outcome.diagnostics[0].identifier, "w1 wallet");
        // A failure leaves the cursor where it was. Writing `None` there says
        // "the chain confirms there is no more", which a fetch that failed did
        // not say: it marked the wallet exhausted, so this outcome reported
        // more to load while the wallet's own cursor refused to load it.
        assert!(!outcome.exhausted, "a failed page is not the last page");
        assert!(
            !service
                .history_cursor(Chain::Bitcoin.str_id().to_string(), "w1".to_string())
                .is_exhausted,
            "a failure must not mark the wallet exhausted"
        );

        // No Bitcoin wallets at all is not a failure.
        let empty = WalletService::new_typed(Vec::new()).expect("service");
        let outcome = empty
            .refresh_bitcoin_history(Vec::new(), false, None)
            .await
            .expect("refresh");
        assert_eq!(outcome.wallets_failed, 0);
        assert!(outcome.exhausted);
    }

    /// Only an EVM chain has an explorer page to fetch.
    #[tokio::test]
    async fn a_non_evm_chain_is_refused() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        assert!(service
            .refresh_evm_chain_history(Chain::Solana.str_id().to_string(), Vec::new(), false, None)
            .await
            .is_err());
        // With no wallets there is nothing to fetch, no error and no store to
        // write to.
        let outcome = service
            .refresh_evm_chain_history(
                Chain::Ethereum.str_id().to_string(),
                Vec::new(),
                false,
                None,
            )
            .await
            .expect("refresh");
        assert_eq!(outcome.wallets_refreshed, 0);
        assert!(outcome.exhausted);
    }

    /// A chain with no wallets is not an error and not a network call.
    #[tokio::test]
    async fn a_chain_with_no_wallets_refreshes_nothing() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let outcome = service
            .refresh_chain_history(Chain::Solana.str_id().to_string(), Vec::new())
            .await
            .expect("refresh");
        assert_eq!(outcome.wallets_refreshed, 0);
        assert_eq!(outcome.added, 0);
        assert!(service
            .refresh_chain_history("not-a-chain".to_string(), Vec::new())
            .await
            .is_err());
    }
}
