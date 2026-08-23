pub mod history;
pub mod history_decode;
pub mod history_store;
pub mod http;

pub mod price;
pub mod refresh;
pub mod transactions;

// Per-chain read-path clients: client struct + shared types + balance /
// history / metadata / fee-estimate RPC methods.
pub mod chains;

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

/// A wallet a refresh might visit, and the addresses it would visit it at.
///
/// One record for all three families. There were three — `EvmRefreshWalletInput`,
/// `DogecoinRefreshWalletInput`, `NormalizedRefreshWalletInput` — differing in
/// `address: Option<String>` versus `addresses: Vec<String>`, which is one
/// address and many of them.
///
/// The `index` field is gone. It was the caller's position in its own array,
/// passed in so it could be passed back, and **no call site ever read it**:
/// every one of them mapped the results by `wallet_id`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct RefreshWalletInput {
    pub wallet_id: String,
    pub selected_chain: String,
    /// One entry for most chains; a UTXO chain supplies its known address set.
    pub addresses: Vec<String>,
}

/// Which wallets a refresh on `chain_name` should visit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct RefreshTargetsRequest {
    pub chain_name: String,
    pub wallets: Vec<RefreshWalletInput>,
    /// `None` refreshes every wallet on the chain.
    pub allowed_wallet_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct RefreshWalletTarget {
    pub wallet_id: String,
    pub addresses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmRefreshTargetsRequest {
    pub chain_name: String,
    pub wallets: Vec<RefreshWalletInput>,
    pub allowed_wallet_ids: Option<Vec<String>>,
    pub group_by_normalized_address: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmRefreshWalletTarget {
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
            let address = wallet
                .addresses
                .iter()
                .find_map(|address| trim_optional(Some(address)))?
                .to_string();
            let normalized_address = normalize_evm_address(&address);
            Some(EvmRefreshWalletTarget {
                wallet_id: wallet.wallet_id,
                address,
                normalized_address,
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

/// Which wallets a refresh on `chain_name` should visit, and at which
/// addresses.
///
/// Two functions before. The Dogecoin one hardcoded
/// `selected_chain == "Dogecoin"` — a chain name written into core for a caller
/// that only ever called it for Dogecoin — and returned `addresses: Vec<String>`
/// where the other returned one `address`. The filters were otherwise
/// identical: on the chain, in the allowed set, and having somewhere to look.
pub fn plan_refresh_targets(request: RefreshTargetsRequest) -> Vec<RefreshWalletTarget> {
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
            let addresses = wallet
                .addresses
                .iter()
                .filter_map(|address| trim_optional(Some(address)).map(str::to_string))
                .collect::<Vec<_>>();
            if addresses.is_empty() {
                return None;
            }
            Some(RefreshWalletTarget {
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
        plan_evm_refresh_targets, plan_refresh_targets, EvmRefreshTargetsRequest,
        RefreshTargetsRequest, RefreshWalletInput,
    };

    fn wallet(id: &str, chain: &str, addresses: &[&str]) -> RefreshWalletInput {
        RefreshWalletInput {
            wallet_id: id.to_string(),
            selected_chain: chain.to_string(),
            addresses: addresses.iter().map(|a| a.to_string()).collect(),
        }
    }

    #[test]
    fn groups_evm_targets_by_normalized_address() {
        let plan = plan_evm_refresh_targets(EvmRefreshTargetsRequest {
            chain_name: "Ethereum".to_string(),
            wallets: vec![
                wallet("wallet-a", "Ethereum", &[" 0xABC "]),
                wallet("wallet-b", "Ethereum", &["0xabc"]),
                wallet("wallet-c", "Arbitrum", &["0xdef"]),
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
                wallet("wallet-a", "Ethereum", &["0xABC"]),
                wallet("wallet-b", "Ethereum", &["0xabc"]),
            ],
            allowed_wallet_ids: None,
            group_by_normalized_address: false,
        });

        assert_eq!(plan.grouped_targets.len(), 2);
        assert_eq!(plan.grouped_targets[0].wallet_ids, vec!["wallet-a"]);
        assert_eq!(plan.grouped_targets[1].wallet_ids, vec!["wallet-b"]);
    }

    /// One planner for every family: on the chain, in the allowed set, and
    /// having somewhere to look.
    ///
    /// This was two tests over two functions, one of which hardcoded
    /// `"Dogecoin"`. A wallet with many addresses and a wallet with one go
    /// through the same filter now, which is what says they always did.
    #[test]
    fn refresh_targets_filter_by_chain_allowed_set_and_having_an_address() {
        let wallets = vec![
            wallet("wallet-a", "Dogecoin", &["D1", " D2 "]),
            wallet("wallet-b", "Dogecoin", &["  ", ""]),
            wallet("wallet-c", "Dogecoin", &["D3"]),
            wallet("wallet-d", "Litecoin", &["L1"]),
        ];

        let all = plan_refresh_targets(RefreshTargetsRequest {
            chain_name: "Dogecoin".to_string(),
            wallets: wallets.clone(),
            allowed_wallet_ids: None,
        });
        assert_eq!(
            all.iter().map(|t| t.wallet_id.as_str()).collect::<Vec<_>>(),
            vec!["wallet-a", "wallet-c"],
            "wallet-b has no usable address and wallet-d is on another chain"
        );
        assert_eq!(all[0].addresses, vec!["D1", "D2"], "entries are trimmed");

        let restricted = plan_refresh_targets(RefreshTargetsRequest {
            chain_name: "Dogecoin".to_string(),
            wallets: wallets.clone(),
            allowed_wallet_ids: Some(vec!["wallet-c".to_string()]),
        });
        assert_eq!(restricted.len(), 1);
        assert_eq!(restricted[0].wallet_id, "wallet-c");

        // The single-address family is the same call with one entry.
        let single = plan_refresh_targets(RefreshTargetsRequest {
            chain_name: "Litecoin".to_string(),
            wallets,
            allowed_wallet_ids: None,
        });
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].addresses, vec!["L1"]);
    }
}

// ── FFI surface ─────────────────────────────────────────────────────────────

#[uniffi::export]
pub fn core_evm_refresh_targets(request: EvmRefreshTargetsRequest) -> EvmRefreshPlan {
    plan_evm_refresh_targets(request)
}

/// One export for the Dogecoin and normalized families, which were two.
#[uniffi::export]
pub fn core_refresh_targets(request: RefreshTargetsRequest) -> Vec<RefreshWalletTarget> {
    plan_refresh_targets(request)
}
