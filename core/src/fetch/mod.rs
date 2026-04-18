pub mod balance_cache;
pub mod balance_decoder;
pub mod balance_observer;
pub mod endpoint_reliability;
pub mod history;
pub mod history_cache;
pub mod history_decode;
pub mod history_store;
pub mod http;
pub mod http_ffi;
pub mod price;
pub mod refresh;
pub mod refresh_engine;
pub mod transactions;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BalanceRequest {
    pub chain_name: String,
    pub address: String,
    pub asset_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BalanceSnapshot {
    pub chain_name: String,
    pub address: String,
    pub asset_id: Option<String>,
    pub amount: String,
    pub block_height: Option<u64>,
    pub source_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRequest {
    pub chain_name: String,
    pub address: String,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedTransaction {
    pub txid: String,
    pub chain_name: String,
    pub status: String,
    pub sent_amount: Option<String>,
    pub received_amount: Option<String>,
    pub fee_amount: Option<String>,
    pub timestamp_unix: Option<u64>,
}

pub trait BalanceProvider: Send + Sync {
    fn fetch_balance(&self, request: &BalanceRequest) -> Result<BalanceSnapshot, String>;
}

pub trait HistoryProvider: Send + Sync {
    fn fetch_history(&self, request: &HistoryRequest)
        -> Result<Vec<NormalizedTransaction>, String>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletBalanceRefreshRequest {
    pub selected_chain: String,
    pub has_seed_phrase: bool,
    pub has_extended_public_key: bool,
    pub available_address_kinds: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletBalanceRefreshPlan {
    pub service_kind: Option<String>,
    pub uses_bulk_refresh: bool,
    pub needs_tracked_tokens: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmRefreshWalletInput {
    pub index: u64,
    pub wallet_id: String,
    pub selected_chain: String,
    pub address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmRefreshTargetsRequest {
    pub chain_name: String,
    pub wallets: Vec<EvmRefreshWalletInput>,
    pub allowed_wallet_ids: Option<Vec<String>>,
    pub group_by_normalized_address: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmRefreshWalletTarget {
    pub index: u64,
    pub wallet_id: String,
    pub address: String,
    pub normalized_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmGroupedTarget {
    pub wallet_ids: Vec<String>,
    pub address: String,
    pub normalized_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmRefreshPlan {
    pub wallet_targets: Vec<EvmRefreshWalletTarget>,
    pub grouped_targets: Vec<EvmGroupedTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct DogecoinRefreshWalletInput {
    pub index: u64,
    pub wallet_id: String,
    pub selected_chain: String,
    pub addresses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct DogecoinRefreshTargetsRequest {
    pub wallets: Vec<DogecoinRefreshWalletInput>,
    pub allowed_wallet_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct DogecoinRefreshWalletTarget {
    pub index: u64,
    pub wallet_id: String,
    pub addresses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedRefreshWalletInput {
    pub index: u64,
    pub wallet_id: String,
    pub selected_chain: String,
    pub address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedRefreshTargetsRequest {
    pub chain_name: String,
    pub wallets: Vec<NormalizedRefreshWalletInput>,
    pub allowed_wallet_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedRefreshWalletTarget {
    pub index: u64,
    pub wallet_id: String,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BalanceRefreshHealthRequest {
    pub chain_name: String,
    pub attempted_wallet_count: u64,
    pub resolved_wallet_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct BalanceRefreshHealthPlan {
    pub should_mark_healthy: bool,
    pub should_note_successful_sync: bool,
    pub degraded_detail: Option<String>,
}

pub fn plan_wallet_balance_refresh(
    request: WalletBalanceRefreshRequest,
) -> WalletBalanceRefreshPlan {
    let address_kinds = request
        .available_address_kinds
        .into_iter()
        .collect::<BTreeSet<_>>();

    let plan = match request.selected_chain.as_str() {
        "Bitcoin"
            if request.has_seed_phrase
                || request.has_extended_public_key
                || address_kinds.contains("bitcoin") =>
        {
            Some(("bitcoinBulk", true, false))
        }
        "Bitcoin Cash" if address_kinds.contains("bitcoinCash") => {
            Some(("utxoSingleAddress", false, false))
        }
        "Bitcoin SV" if address_kinds.contains("bitcoinSV") => {
            Some(("utxoSingleAddress", false, false))
        }
        "Litecoin" if address_kinds.contains("litecoin") => {
            Some(("utxoSingleAddress", false, false))
        }
        "Dogecoin" if address_kinds.contains("dogecoin") => Some(("dogecoinBulk", true, false)),
        "Ethereum" | "Ethereum Classic" | "Arbitrum" | "Optimism" | "BNB Chain" | "Avalanche"
        | "Hyperliquid"
            if address_kinds.contains("evm") =>
        {
            Some(("evmPortfolio", false, true))
        }
        "Tron" if address_kinds.contains("tron") => Some(("tronPortfolio", false, true)),
        "Solana" if address_kinds.contains("solana") => Some(("solanaPortfolio", false, true)),
        "Cardano" if address_kinds.contains("cardano") => Some(("singleBalance", false, false)),
        "XRP Ledger" if address_kinds.contains("xrp") => Some(("singleBalance", false, false)),
        "Stellar" if address_kinds.contains("stellar") => Some(("singleBalance", false, false)),
        "Monero" if address_kinds.contains("monero") => Some(("singleBalance", false, false)),
        "Sui" if address_kinds.contains("sui") => Some(("suiPortfolio", false, true)),
        "Aptos" if address_kinds.contains("aptos") => Some(("aptosPortfolio", false, true)),
        "TON" if address_kinds.contains("ton") => Some(("tonPortfolio", false, true)),
        "Internet Computer" if address_kinds.contains("icp") => {
            Some(("singleBalance", false, false))
        }
        "NEAR" if address_kinds.contains("near") => Some(("nearPortfolio", false, true)),
        "Polkadot" if address_kinds.contains("polkadot") => Some(("singleBalance", false, false)),
        _ => None,
    };

    let (service_kind, uses_bulk_refresh, needs_tracked_tokens) = match plan {
        Some((kind, uses_bulk_refresh, needs_tracked_tokens)) => (
            Some(kind.to_string()),
            uses_bulk_refresh,
            needs_tracked_tokens,
        ),
        None => (None, false, false),
    };

    WalletBalanceRefreshPlan {
        service_kind,
        uses_bulk_refresh,
        needs_tracked_tokens,
    }
}

pub fn plan_balance_refresh_health(
    request: BalanceRefreshHealthRequest,
) -> BalanceRefreshHealthPlan {
    if request.attempted_wallet_count == 0 {
        return BalanceRefreshHealthPlan {
            should_mark_healthy: false,
            should_note_successful_sync: false,
            degraded_detail: None,
        };
    }

    if request.resolved_wallet_count == request.attempted_wallet_count {
        return BalanceRefreshHealthPlan {
            should_mark_healthy: true,
            should_note_successful_sync: false,
            degraded_detail: None,
        };
    }

    if request.resolved_wallet_count > 0 {
        return BalanceRefreshHealthPlan {
            should_mark_healthy: false,
            should_note_successful_sync: true,
            degraded_detail: Some(format!(
                "{} providers are partially reachable. Showing the latest available balances.",
                request.chain_name
            )),
        };
    }

    BalanceRefreshHealthPlan {
        should_mark_healthy: false,
        should_note_successful_sync: false,
        degraded_detail: Some(format!(
            "{} providers are unavailable. Using cached balances and history.",
            request.chain_name
        )),
    }
}

pub fn plan_evm_refresh_targets(request: EvmRefreshTargetsRequest) -> EvmRefreshPlan {
    let allowed_wallet_ids = request
        .allowed_wallet_ids
        .map(|wallet_ids| wallet_ids.into_iter().collect::<BTreeSet<_>>());
    let wallet_targets = request
        .wallets
        .into_iter()
        .filter(|wallet| wallet.selected_chain == request.chain_name)
        .filter(|wallet| {
            allowed_wallet_ids
                .as_ref()
                .map(|wallet_ids| wallet_ids.contains(&wallet.wallet_id))
                .unwrap_or(true)
        })
        .filter_map(|wallet| {
            let address = trim_optional(wallet.address.as_deref())?;
            Some(EvmRefreshWalletTarget {
                index: wallet.index,
                wallet_id: wallet.wallet_id,
                address: address.to_string(),
                normalized_address: normalize_evm_address(address),
            })
        })
        .collect::<Vec<_>>();

    let grouped_targets = if request.group_by_normalized_address {
        let mut grouped: BTreeMap<String, Vec<&EvmRefreshWalletTarget>> = BTreeMap::new();
        let mut ordered_keys = Vec::new();
        for target in &wallet_targets {
            if !grouped.contains_key(&target.normalized_address) {
                ordered_keys.push(target.normalized_address.clone());
            }
            grouped
                .entry(target.normalized_address.clone())
                .or_default()
                .push(target);
        }

        ordered_keys
            .into_iter()
            .filter_map(|key| {
                let group = grouped.get(&key)?;
                let address = group.first()?.address.clone();
                Some(EvmGroupedTarget {
                    wallet_ids: group
                        .iter()
                        .map(|target| target.wallet_id.clone())
                        .collect(),
                    address,
                    normalized_address: key,
                })
            })
            .collect()
    } else {
        wallet_targets
            .iter()
            .map(|target| EvmGroupedTarget {
                wallet_ids: vec![target.wallet_id.clone()],
                address: target.address.clone(),
                normalized_address: target.normalized_address.clone(),
            })
            .collect()
    };

    EvmRefreshPlan {
        wallet_targets,
        grouped_targets,
    }
}

pub fn plan_normalized_refresh_targets(
    request: NormalizedRefreshTargetsRequest,
) -> Vec<NormalizedRefreshWalletTarget> {
    let allowed_wallet_ids = request
        .allowed_wallet_ids
        .map(|wallet_ids| wallet_ids.into_iter().collect::<BTreeSet<_>>());

    request
        .wallets
        .into_iter()
        .filter(|wallet| wallet.selected_chain == request.chain_name)
        .filter(|wallet| {
            allowed_wallet_ids
                .as_ref()
                .map(|wallet_ids| wallet_ids.contains(&wallet.wallet_id))
                .unwrap_or(true)
        })
        .filter_map(|wallet| {
            let address = trim_optional(wallet.address.as_deref())?.to_string();
            Some(NormalizedRefreshWalletTarget {
                index: wallet.index,
                wallet_id: wallet.wallet_id,
                address,
            })
        })
        .collect()
}

pub fn plan_dogecoin_refresh_targets(
    request: DogecoinRefreshTargetsRequest,
) -> Vec<DogecoinRefreshWalletTarget> {
    let allowed_wallet_ids = request
        .allowed_wallet_ids
        .map(|wallet_ids| wallet_ids.into_iter().collect::<BTreeSet<_>>());

    request
        .wallets
        .into_iter()
        .filter(|wallet| wallet.selected_chain == "Dogecoin")
        .filter(|wallet| {
            allowed_wallet_ids
                .as_ref()
                .map(|wallet_ids| wallet_ids.contains(&wallet.wallet_id))
                .unwrap_or(true)
        })
        .filter_map(|wallet| {
            let addresses = wallet
                .addresses
                .into_iter()
                .filter_map(|address| {
                    trim_optional(Some(address.as_str())).map(|value| value.to_string())
                })
                .collect::<Vec<_>>();
            if addresses.is_empty() {
                return None;
            }
            Some(DogecoinRefreshWalletTarget {
                index: wallet.index,
                wallet_id: wallet.wallet_id,
                addresses,
            })
        })
        .collect()
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

fn normalize_evm_address(address: &str) -> String {
    address.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{
        plan_balance_refresh_health, plan_dogecoin_refresh_targets, plan_evm_refresh_targets,
        plan_normalized_refresh_targets, plan_wallet_balance_refresh, BalanceRefreshHealthRequest,
        DogecoinRefreshTargetsRequest, DogecoinRefreshWalletInput, EvmRefreshTargetsRequest,
        EvmRefreshWalletInput, NormalizedRefreshTargetsRequest, NormalizedRefreshWalletInput,
        WalletBalanceRefreshRequest,
    };

    #[test]
    fn groups_evm_targets_by_normalized_address() {
        let plan = plan_evm_refresh_targets(EvmRefreshTargetsRequest {
            chain_name: "Ethereum".to_string(),
            wallets: vec![
                EvmRefreshWalletInput {
                    index: 0,
                    wallet_id: "wallet-a".to_string(),
                    selected_chain: "Ethereum".to_string(),
                    address: Some(" 0xABC ".to_string()),
                },
                EvmRefreshWalletInput {
                    index: 1,
                    wallet_id: "wallet-b".to_string(),
                    selected_chain: "Ethereum".to_string(),
                    address: Some("0xabc".to_string()),
                },
                EvmRefreshWalletInput {
                    index: 2,
                    wallet_id: "wallet-c".to_string(),
                    selected_chain: "Arbitrum".to_string(),
                    address: Some("0xdef".to_string()),
                },
            ],
            allowed_wallet_ids: None,
            group_by_normalized_address: true,
        });

        assert_eq!(plan.wallet_targets.len(), 2);
        assert_eq!(plan.grouped_targets.len(), 1);
        assert_eq!(
            plan.grouped_targets[0].wallet_ids,
            vec!["wallet-a", "wallet-b"]
        );
        assert_eq!(plan.grouped_targets[0].normalized_address, "0xabc");
    }

    #[test]
    fn preserves_per_wallet_evm_targets_for_load_more_mode() {
        let plan = plan_evm_refresh_targets(EvmRefreshTargetsRequest {
            chain_name: "Ethereum".to_string(),
            wallets: vec![
                EvmRefreshWalletInput {
                    index: 0,
                    wallet_id: "wallet-a".to_string(),
                    selected_chain: "Ethereum".to_string(),
                    address: Some("0xABC".to_string()),
                },
                EvmRefreshWalletInput {
                    index: 1,
                    wallet_id: "wallet-b".to_string(),
                    selected_chain: "Ethereum".to_string(),
                    address: Some("0xabc".to_string()),
                },
            ],
            allowed_wallet_ids: None,
            group_by_normalized_address: false,
        });

        assert_eq!(plan.grouped_targets.len(), 2);
        assert_eq!(plan.grouped_targets[0].wallet_ids, vec!["wallet-a"]);
        assert_eq!(plan.grouped_targets[1].wallet_ids, vec!["wallet-b"]);
    }

    #[test]
    fn filters_dogecoin_targets_by_allowed_wallets_and_nonempty_addresses() {
        let targets = plan_dogecoin_refresh_targets(DogecoinRefreshTargetsRequest {
            wallets: vec![
                DogecoinRefreshWalletInput {
                    index: 0,
                    wallet_id: "wallet-a".to_string(),
                    selected_chain: "Dogecoin".to_string(),
                    addresses: vec!["Dabc".to_string(), " ".to_string()],
                },
                DogecoinRefreshWalletInput {
                    index: 1,
                    wallet_id: "wallet-b".to_string(),
                    selected_chain: "Dogecoin".to_string(),
                    addresses: vec![],
                },
                DogecoinRefreshWalletInput {
                    index: 2,
                    wallet_id: "wallet-c".to_string(),
                    selected_chain: "Bitcoin".to_string(),
                    addresses: vec!["Dskip".to_string()],
                },
            ],
            allowed_wallet_ids: Some(vec!["wallet-a".to_string(), "wallet-b".to_string()]),
        });

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].wallet_id, "wallet-a");
        assert_eq!(targets[0].addresses, vec!["Dabc"]);
    }

    #[test]
    fn filters_normalized_targets_by_chain_and_nonempty_address() {
        let targets = plan_normalized_refresh_targets(NormalizedRefreshTargetsRequest {
            chain_name: "Solana".to_string(),
            wallets: vec![
                NormalizedRefreshWalletInput {
                    index: 0,
                    wallet_id: "wallet-a".to_string(),
                    selected_chain: "Solana".to_string(),
                    address: Some(" SoLaNaAddr ".to_string()),
                },
                NormalizedRefreshWalletInput {
                    index: 1,
                    wallet_id: "wallet-b".to_string(),
                    selected_chain: "Solana".to_string(),
                    address: Some("".to_string()),
                },
                NormalizedRefreshWalletInput {
                    index: 2,
                    wallet_id: "wallet-c".to_string(),
                    selected_chain: "Tron".to_string(),
                    address: Some("T123".to_string()),
                },
                NormalizedRefreshWalletInput {
                    index: 3,
                    wallet_id: "wallet-d".to_string(),
                    selected_chain: "Solana".to_string(),
                    address: None,
                },
            ],
            allowed_wallet_ids: None,
        });

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].wallet_id, "wallet-a");
        assert_eq!(targets[0].address, "SoLaNaAddr");
    }

    #[test]
    fn plans_balance_refresh_for_bitcoin_when_xpub_is_available() {
        let plan = plan_wallet_balance_refresh(WalletBalanceRefreshRequest {
            selected_chain: "Bitcoin".to_string(),
            has_seed_phrase: false,
            has_extended_public_key: true,
            available_address_kinds: vec![],
        });

        assert_eq!(plan.service_kind.as_deref(), Some("bitcoinBulk"));
        assert!(plan.uses_bulk_refresh);
    }

    #[test]
    fn plans_evm_portfolio_refresh_only_when_evm_address_exists() {
        let missing_address = plan_wallet_balance_refresh(WalletBalanceRefreshRequest {
            selected_chain: "Arbitrum".to_string(),
            has_seed_phrase: false,
            has_extended_public_key: false,
            available_address_kinds: vec![],
        });
        assert_eq!(missing_address.service_kind, None);

        let with_address = plan_wallet_balance_refresh(WalletBalanceRefreshRequest {
            selected_chain: "Arbitrum".to_string(),
            has_seed_phrase: false,
            has_extended_public_key: false,
            available_address_kinds: vec!["evm".to_string()],
        });
        assert_eq!(with_address.service_kind.as_deref(), Some("evmPortfolio"));
        assert!(with_address.needs_tracked_tokens);
    }

    #[test]
    fn plans_balance_refresh_health_for_partial_results() {
        let plan = plan_balance_refresh_health(BalanceRefreshHealthRequest {
            chain_name: "Bitcoin".to_string(),
            attempted_wallet_count: 3,
            resolved_wallet_count: 1,
        });

        assert!(!plan.should_mark_healthy);
        assert!(plan.should_note_successful_sync);
        assert_eq!(
            plan.degraded_detail.as_deref(),
            Some(
                "Bitcoin providers are partially reachable. Showing the latest available balances."
            )
        );
    }
}
