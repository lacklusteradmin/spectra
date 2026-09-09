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
}

/// One wallet to fetch history for.
struct Target {
    wallet_id: String,
    wallet_name: String,
    address: String,
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
        chain_name: entry.chain_name,
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
            return Ok(HistoryRefreshOutcome {
                wallets_refreshed: 0,
                wallets_failed: 0,
                added: 0,
                updated: 0,
            });
        }

        // Owned pairs rather than borrows of `targets`: the exported method's
        // future has to be `'static`, and a closure borrowing from the
        // enclosing scope is not.
        let requests: Vec<(usize, String)> = targets
            .iter()
            .enumerate()
            .map(|(index, target)| (index, target.address.clone()))
            .collect();
        let fetched: Vec<(usize, Option<Vec<_>>)> = stream::iter(requests)
            .map(|(index, address)| {
                let chain_id = chain_id.clone();
                async move {
                    (
                        index,
                        self.fetch_normalized_history(chain_id, address).await.ok(),
                    )
                }
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
                            .map(|entry| record_for(&targets[index], entry)),
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
/// transfers, what failed) is the refresh's own account of itself, and the
/// front end used to assemble it from the pieces it happened to hold.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct EvmHistoryWalletDiagnostics {
    pub wallet_id: String,
    pub address: String,
    pub source_used: String,
    pub transaction_count: u32,
    pub error: Option<String>,
}

/// What one EVM chain's history refresh did.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct EvmHistoryRefreshOutcome {
    pub wallets_refreshed: u32,
    pub wallets_failed: u32,
    pub added: u32,
    pub updated: u32,
    /// Whether every group reported a short page, so there is nothing more to
    /// load. A caller shows or hides its "load more" control from this.
    pub exhausted: bool,
    pub diagnostics: Vec<EvmHistoryWalletDiagnostics>,
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
    ) -> Result<EvmHistoryRefreshOutcome, SpectraBridgeError> {
        let chain = Chain::from_str_id(&chain_id)
            .filter(|chain| chain.is_evm())
            .ok_or_else(|| SpectraBridgeError::from(format!("{chain_id:?} is not an EVM chain")))?;
        let (groups, descriptors, wallet_names) = {
            let state = self.app_state().await;
            let targets = targets(&state, chain, &wallet_ids);
            let plan = crate::fetch::plan_evm_refresh_targets(crate::fetch::EvmRefreshTargetsRequest {
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
            let names: std::collections::HashMap<String, String> = targets
                .into_iter()
                .map(|target| (target.wallet_id, target.wallet_name))
                .collect();
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
            )
        };
        if groups.is_empty() {
            return Ok(EvmHistoryRefreshOutcome {
                wallets_refreshed: 0,
                wallets_failed: 0,
                added: 0,
                updated: 0,
                exhausted: true,
                diagnostics: Vec::new(),
            });
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

            let fetched = self
                .fetch_evm_history_page(
                    chain_id.clone(),
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
                        diagnostics.push(EvmHistoryWalletDiagnostics {
                            wallet_id: wallet_id.clone(),
                            address: normalized_address.clone(),
                            source_used: "none".to_string(),
                            transaction_count: 0,
                            error: Some(error.to_string()),
                        });
                    }
                    continue;
                }
            };
            wallets_refreshed += group_wallet_ids.len() as u32;
            let token_count = decoded.tokens.len() as u32;
            let is_last_page =
                decoded.tokens.len() < page_size as usize && decoded.native.len() < page_size as usize;
            exhausted = exhausted && is_last_page;
            for wallet_id in &group_wallet_ids {
                self.set_history_page(chain_id.clone(), wallet_id.clone(), page, is_last_page);
                diagnostics.push(EvmHistoryWalletDiagnostics {
                    wallet_id: wallet_id.clone(),
                    address: normalized_address.clone(),
                    source_used: "rust/etherscan".to_string(),
                    transaction_count: token_count,
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

        Ok(EvmHistoryRefreshOutcome {
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

    /// A wallet on a testnet fetches the address for that network.
    #[test]
    fn a_target_follows_the_network_the_wallet_is_on() {
        let mut state = CoreAppState::default();
        state.wallets = vec![wallet(
            "w1",
            Chain::Bitcoin,
            &[(Chain::Bitcoin, "bc1main"), (Chain::BitcoinTestnet4, "tb1test")],
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
        };
        let record = record_for(
            &target,
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
        assert_eq!(record_for(&target, entry.clone()).transaction_hash, None);
        entry.tx_hash = "abc".to_string();
        assert_eq!(
            record_for(&target, entry).transaction_hash.as_deref(),
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
            entry("ethereum", "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", true),
            entry("ethereum", "0xBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB", false),
            entry("solana", "So11111111111111111111111111111111111111112", true),
        ];

        let descriptors = token_descriptors(&state, Chain::Ethereum);
        assert_eq!(descriptors.len(), 1);
        assert_eq!(
            descriptors[0].contract,
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
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
            .join(format!("spectra-evm-history-{}.sqlite", crate::store::new_event_id()))
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
            .refresh_evm_chain_history(
                Chain::Cronos.str_id().to_string(),
                Vec::new(),
                false,
                None,
            )
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
