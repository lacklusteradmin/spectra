use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryWallet {
    pub wallet_id: String,
    pub selected_chain: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryTransaction {
    pub id: String,
    pub wallet_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub wallet_name: String,
    pub asset_name: String,
    pub symbol: String,
    pub chain_name: String,
    pub address: String,
    pub transaction_hash: Option<String>,
    pub transaction_history_source: Option<String>,
    pub created_at_unix: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizeHistoryRequest {
    pub wallets: Vec<HistoryWallet>,
    pub transactions: Vec<HistoryTransaction>,
    pub unknown_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedHistoryEntry {
    pub id: String,
    pub transaction_id: String,
    pub dedupe_key: String,
    pub created_at_unix: f64,
    pub kind: String,
    pub status: String,
    pub wallet_name: String,
    pub asset_name: String,
    pub symbol: String,
    pub chain_name: String,
    pub address: String,
    pub transaction_hash: Option<String>,
    pub source_tag: String,
    pub provider_count: usize,
    pub search_index: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BitcoinHistorySnapshot {
    pub txid: String,
    pub amount_btc: f64,
    pub kind: String,
    pub status: String,
    pub counterparty_address: String,
    pub block_height: Option<i64>,
    pub created_at_unix: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MergeBitcoinHistorySnapshotsRequest {
    pub snapshots: Vec<BitcoinHistorySnapshot>,
    pub owned_addresses: Vec<String>,
    pub limit: usize,
}

pub fn normalize_history(request: NormalizeHistoryRequest) -> Vec<NormalizedHistoryEntry> {
    let wallet_by_id = request
        .wallets
        .into_iter()
        .map(|wallet| (wallet.wallet_id, wallet.selected_chain))
        .collect::<BTreeMap<_, _>>();

    let mut grouped_by_dedupe_key = BTreeMap::<String, Vec<NormalizedHistoryEntry>>::new();
    for transaction in request.transactions {
        let Some(wallet_id) = transaction.wallet_id.as_ref() else {
            continue;
        };
        let Some(selected_chain) = wallet_by_id.get(wallet_id) else {
            continue;
        };
        if *selected_chain != transaction.chain_name {
            continue;
        }
        let entry = normalized_entry(transaction, &request.unknown_label);
        grouped_by_dedupe_key
            .entry(entry.dedupe_key.clone())
            .or_default()
            .push(entry);
    }

    let mut deduped = grouped_by_dedupe_key
        .into_values()
        .filter_map(|entries| {
            if entries.is_empty() {
                return None;
            }
            let provider_count = entries
                .iter()
                .map(|entry| entry.source_tag.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                .max(1);
            let best = entries
                .into_iter()
                .max_by(|lhs, rhs| compare_entries(lhs, rhs))?;
            Some(NormalizedHistoryEntry {
                provider_count,
                ..best
            })
        })
        .collect::<Vec<_>>();

    deduped.sort_by(|lhs, rhs| {
        rhs.created_at_unix
            .partial_cmp(&lhs.created_at_unix)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| lhs.id.cmp(&rhs.id))
    });
    deduped
}

pub fn merge_bitcoin_history_snapshots(
    request: MergeBitcoinHistorySnapshotsRequest,
) -> Vec<BitcoinHistorySnapshot> {
    let owned_addresses = request
        .owned_addresses
        .into_iter()
        .map(|address| address.trim().to_lowercase())
        .filter(|address| !address.is_empty())
        .collect::<std::collections::BTreeSet<_>>();

    let grouped = request.snapshots.into_iter().fold(
        BTreeMap::<String, Vec<BitcoinHistorySnapshot>>::new(),
        |mut grouped, snapshot| {
            grouped
                .entry(snapshot.txid.clone())
                .or_default()
                .push(snapshot);
            grouped
        },
    );

    let mut merged = grouped
        .into_values()
        .filter_map(|entries| merge_bitcoin_snapshot_group(entries, &owned_addresses))
        .collect::<Vec<_>>();

    merged.sort_by(|lhs, rhs| {
        rhs.created_at_unix
            .partial_cmp(&lhs.created_at_unix)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| lhs.txid.cmp(&rhs.txid))
    });
    merged.truncate(request.limit.max(1));
    merged
}

fn normalized_entry(
    transaction: HistoryTransaction,
    unknown_label: &str,
) -> NormalizedHistoryEntry {
    let wallet_key = transaction
        .wallet_id
        .clone()
        .unwrap_or_else(|| "unknown-wallet".to_string())
        .to_lowercase();
    let normalized_chain = transaction.chain_name.to_lowercase();
    let normalized_symbol = transaction.symbol.trim().to_lowercase();
    let (dedupe_key, stable_id) = match transaction
        .transaction_hash
        .as_ref()
        .map(|value| value.to_lowercase())
        .filter(|value| !value.is_empty())
    {
        Some(transaction_hash) => {
            let key = format!(
                "{}|{}|{}|{}",
                wallet_key, normalized_chain, normalized_symbol, transaction_hash
            );
            (key.clone(), key)
        }
        None => {
            let key = format!("local|{}|{}", wallet_key, transaction.id.to_lowercase());
            (key.clone(), key)
        }
    };

    let source_tag = normalized_source_tag(
        transaction.transaction_history_source.as_deref(),
        unknown_label,
    );
    let search_index = [
        transaction.wallet_name.as_str(),
        transaction.asset_name.as_str(),
        transaction.symbol.as_str(),
        transaction.chain_name.as_str(),
        transaction.address.as_str(),
        transaction.transaction_hash.as_deref().unwrap_or(""),
        source_tag.as_str(),
    ]
    .join(" ")
    .to_lowercase();

    NormalizedHistoryEntry {
        id: stable_id,
        transaction_id: transaction.id,
        dedupe_key,
        created_at_unix: transaction.created_at_unix,
        kind: transaction.kind,
        status: transaction.status,
        wallet_name: transaction.wallet_name,
        asset_name: transaction.asset_name,
        symbol: transaction.symbol,
        chain_name: transaction.chain_name,
        address: transaction.address,
        transaction_hash: transaction.transaction_hash,
        source_tag,
        provider_count: 1,
        search_index,
    }
}

fn merge_bitcoin_snapshot_group(
    entries: Vec<BitcoinHistorySnapshot>,
    owned_addresses: &std::collections::BTreeSet<String>,
) -> Option<BitcoinHistorySnapshot> {
    if entries.is_empty() {
        return None;
    }

    let net_amount = entries.iter().fold(0.0, |amount, entry| {
        amount
            + if entry.kind == "receive" {
                entry.amount_btc
            } else {
                -entry.amount_btc
            }
    });
    if net_amount == 0.0 {
        return None;
    }

    let mut ordered_entries = entries;
    ordered_entries.sort_by(|lhs, rhs| {
        rhs.created_at_unix
            .partial_cmp(&lhs.created_at_unix)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                rhs.amount_btc
                    .partial_cmp(&lhs.amount_btc)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    let counterparty_address = ordered_entries
        .iter()
        .map(|entry| entry.counterparty_address.trim().to_string())
        .find(|address| {
            let normalized = address.to_lowercase();
            !normalized.is_empty() && !owned_addresses.contains(&normalized)
        })
        .or_else(|| {
            ordered_entries
                .first()
                .map(|entry| entry.counterparty_address.clone())
        })
        .unwrap_or_default();

    let representative = ordered_entries.first()?.clone();
    Some(BitcoinHistorySnapshot {
        txid: representative.txid,
        amount_btc: net_amount.abs(),
        kind: if net_amount > 0.0 {
            "receive".to_string()
        } else {
            "send".to_string()
        },
        status: if ordered_entries
            .iter()
            .any(|entry| entry.status == "pending")
        {
            "pending".to_string()
        } else {
            "confirmed".to_string()
        },
        counterparty_address,
        block_height: ordered_entries
            .iter()
            .filter_map(|entry| entry.block_height)
            .max(),
        created_at_unix: ordered_entries
            .iter()
            .map(|entry| entry.created_at_unix)
            .fold(representative.created_at_unix, f64::max),
    })
}

fn compare_entries(
    lhs: &NormalizedHistoryEntry,
    rhs: &NormalizedHistoryEntry,
) -> std::cmp::Ordering {
    status_rank(&lhs.status)
        .cmp(&status_rank(&rhs.status))
        .then_with(|| {
            lhs.created_at_unix
                .partial_cmp(&rhs.created_at_unix)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| lhs.transaction_id.cmp(&rhs.transaction_id))
}

fn status_rank(status: &str) -> i32 {
    match status {
        "confirmed" => 3,
        "pending" => 2,
        "failed" => 1,
        _ => 0,
    }
}

fn normalized_source_tag(raw_source: Option<&str>, unknown_label: &str) -> String {
    let trimmed = raw_source.unwrap_or("").trim().to_lowercase();
    if trimmed.is_empty() {
        return unknown_label.to_string();
    }
    match trimmed.as_str() {
        "esplora" => "Esplora".to_string(),
        "litecoinspace" => "LitecoinSpace".to_string(),
        "blockchair" => "Blockchair".to_string(),
        "blockcypher" => "BlockCypher".to_string(),
        "dogecoin.providers" => "DOGE Providers".to_string(),
        "rpc" => "RPC".to_string(),
        "etherscan" => "Etherscan".to_string(),
        "blockscout" => "Blockscout".to_string(),
        "ethplorer" => "Ethplorer".to_string(),
        "none" => unknown_label.to_string(),
        _ => title_case(&trimmed),
    }
}

fn title_case(value: &str) -> String {
    value
        .split_whitespace()
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupes_history_by_hash_and_prefers_confirmed() {
        let entries = normalize_history(NormalizeHistoryRequest {
            wallets: vec![HistoryWallet {
                wallet_id: "wallet-1".to_string(),
                selected_chain: "Ethereum".to_string(),
            }],
            transactions: vec![
                HistoryTransaction {
                    id: "tx-1".to_string(),
                    wallet_id: Some("wallet-1".to_string()),
                    kind: "send".to_string(),
                    status: "pending".to_string(),
                    wallet_name: "Main".to_string(),
                    asset_name: "Ether".to_string(),
                    symbol: "ETH".to_string(),
                    chain_name: "Ethereum".to_string(),
                    address: "0xabc".to_string(),
                    transaction_hash: Some("0xhash".to_string()),
                    transaction_history_source: Some("rpc".to_string()),
                    created_at_unix: 100.0,
                },
                HistoryTransaction {
                    id: "tx-2".to_string(),
                    wallet_id: Some("wallet-1".to_string()),
                    kind: "send".to_string(),
                    status: "confirmed".to_string(),
                    wallet_name: "Main".to_string(),
                    asset_name: "Ether".to_string(),
                    symbol: "ETH".to_string(),
                    chain_name: "Ethereum".to_string(),
                    address: "0xabc".to_string(),
                    transaction_hash: Some("0xhash".to_string()),
                    transaction_history_source: Some("etherscan".to_string()),
                    created_at_unix: 110.0,
                },
            ],
            unknown_label: "Unknown".to_string(),
        });

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "confirmed");
        assert_eq!(entries[0].provider_count, 2);
    }

    #[test]
    fn merges_bitcoin_inventory_snapshots_into_net_entries() {
        let merged = merge_bitcoin_history_snapshots(MergeBitcoinHistorySnapshotsRequest {
            snapshots: vec![
                BitcoinHistorySnapshot {
                    txid: "tx-1".to_string(),
                    amount_btc: 0.75,
                    kind: "receive".to_string(),
                    status: "confirmed".to_string(),
                    counterparty_address: "bc1-owned".to_string(),
                    block_height: Some(100),
                    created_at_unix: 100.0,
                },
                BitcoinHistorySnapshot {
                    txid: "tx-1".to_string(),
                    amount_btc: 0.25,
                    kind: "send".to_string(),
                    status: "pending".to_string(),
                    counterparty_address: "bc1-other".to_string(),
                    block_height: Some(101),
                    created_at_unix: 110.0,
                },
            ],
            owned_addresses: vec!["bc1-owned".to_string()],
            limit: 25,
        });

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].txid, "tx-1");
        assert_eq!(merged[0].amount_btc, 0.5);
        assert_eq!(merged[0].kind, "receive");
        assert_eq!(merged[0].status, "pending");
        assert_eq!(merged[0].counterparty_address, "bc1-other");
        assert_eq!(merged[0].block_height, Some(101));
        assert_eq!(merged[0].created_at_unix, 110.0);
    }
}
