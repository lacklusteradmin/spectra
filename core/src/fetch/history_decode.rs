// Typed decode helpers for chain-history JSON shapes, so front ends get native
// records instead of re-parsing JSON.

// ────────────────────────────────────────────────────────────────────
// Normalized chain history — typed item produced by
// `WalletService::fetch_normalized_history` (see `history::ChainHistoryEntry`).
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, uniffi::Record)]
pub struct NormalizedHistoryItem {
    pub deployment_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub asset_display_name: String,
    pub symbol: String,
    pub chain_id: crate::registry::Chain,
    /// The magnitude, as an exact decimal; `kind` says which way it went.
    pub amount: String,
    pub counterparty: String,
    pub tx_hash: String,
    pub block_height: Option<i64>,
    pub timestamp: f64,
}

// ────────────────────────────────────────────────────────────────────
// EVM history page decode — shape produced by
// `WalletService::fetch_evm_history_page` (tokens and native).
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmTokenTransferItem {
    pub contract_address: String,
    pub token_name: String,
    pub symbol: String,
    pub decimals: i32,
    pub from_address: String,
    pub to_address: String,
    /// Decimal amount serialized as a string so Swift can reconstruct a
    /// `Decimal` without floating-point loss.
    pub amount_decimal: String,
    pub transaction_hash: String,
    pub block_number: i64,
    pub log_index: i64,
    pub timestamp: f64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmNativeTransferItem {
    pub status: String,
    pub from_address: String,
    pub to_address: String,
    pub amount_decimal: String,
    pub transaction_hash: String,
    pub block_number: i64,
    pub timestamp: f64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmHistoryPageDecoded {
    pub tokens: Vec<EvmTokenTransferItem>,
    pub native: Vec<EvmNativeTransferItem>,
}

// ────────────────────────────────────────────────────────────────────
// EVM history page → per-wallet transaction record projection.
// Given a decoded page and the target wallets, emits one record per
// (wallet × matching transfer) where "matching" means the transfer
// touches the wallet's normalized address as sender or receiver.
// The history service merges these records into core-owned transaction state.
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmHistoryTransactionRecord {
    pub status: String,
    pub deployment_id: Option<String>,
    pub wallet_id: String,
    pub wallet_name: String,
    pub kind: String,
    pub asset_display_name: String,
    pub symbol: String,
    pub chain_id: crate::registry::Chain,
    pub amount_decimal: String,
    pub counterparty: String,
    pub transaction_hash: String,
    pub block_number: i64,
    pub source_address: String,
    pub source_used: String,
    pub created_at_unix: f64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmTransactionRecordWalletInput {
    pub wallet_id: String,
    pub wallet_name: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmTransactionRecordRequest {
    pub decoded_page: EvmHistoryPageDecoded,
    pub normalized_address: String,
    pub chain_id: crate::registry::Chain,
    pub token_source_used: Option<String>,
    pub native_asset_display_name: String,
    pub native_asset_symbol: String,
    pub wallets: Vec<EvmTransactionRecordWalletInput>,
    pub unknown_timestamp_sentinel_unix: f64,
}

pub fn build_evm_transaction_records(
    request: EvmTransactionRecordRequest,
) -> Vec<EvmHistoryTransactionRecord> {
    let normalized = request.normalized_address;
    let token_source = request
        .token_source_used
        .unwrap_or_else(|| "none".to_string());
    let native_source = "etherscan".to_string();
    let mut out: Vec<EvmHistoryTransactionRecord> = Vec::new();

    for wallet in &request.wallets {
        for transfer in &request.decoded_page.tokens {
            let is_outgoing = transfer.from_address == normalized;
            let is_incoming = transfer.to_address == normalized;
            if !is_outgoing && !is_incoming {
                continue;
            }
            let (counterparty, wallet_side) = if is_outgoing {
                (transfer.to_address.clone(), transfer.from_address.clone())
            } else {
                (transfer.from_address.clone(), transfer.to_address.clone())
            };
            let created_at = if transfer.timestamp > 0.0 {
                transfer.timestamp
            } else {
                request.unknown_timestamp_sentinel_unix
            };
            out.push(EvmHistoryTransactionRecord {
                status: "confirmed".into(),
                deployment_id: crate::tokens::deployment_id_for(
                    request.chain_id,
                    Some(&transfer.contract_address),
                ),
                wallet_id: wallet.wallet_id.clone(),
                wallet_name: wallet.wallet_name.clone(),
                kind: if is_outgoing { "send" } else { "receive" }.to_string(),
                asset_display_name: transfer.token_name.clone(),
                symbol: transfer.symbol.clone(),
                chain_id: request.chain_id,
                amount_decimal: transfer.amount_decimal.clone(),
                counterparty,
                transaction_hash: transfer.transaction_hash.clone(),
                block_number: transfer.block_number,
                source_address: wallet_side,
                source_used: token_source.clone(),
                created_at_unix: created_at,
            });
        }
        for transfer in &request.decoded_page.native {
            let is_outgoing = transfer.from_address == normalized;
            let is_incoming = transfer.to_address == normalized;
            if !is_outgoing && !is_incoming {
                continue;
            }
            let (counterparty, wallet_side) = if is_outgoing {
                (transfer.to_address.clone(), transfer.from_address.clone())
            } else {
                (transfer.from_address.clone(), transfer.to_address.clone())
            };
            let created_at = if transfer.timestamp > 0.0 {
                transfer.timestamp
            } else {
                request.unknown_timestamp_sentinel_unix
            };
            out.push(EvmHistoryTransactionRecord {
                status: transfer.status.clone(),
                deployment_id: crate::tokens::deployment_id_for(request.chain_id, None),
                wallet_id: wallet.wallet_id.clone(),
                wallet_name: wallet.wallet_name.clone(),
                kind: if is_outgoing { "send" } else { "receive" }.to_string(),
                asset_display_name: request.native_asset_display_name.clone(),
                symbol: request.native_asset_symbol.clone(),
                chain_id: request.chain_id,
                amount_decimal: transfer.amount_decimal.clone(),
                counterparty,
                transaction_hash: transfer.transaction_hash.clone(),
                block_number: transfer.block_number,
                source_address: wallet_side,
                source_used: native_source.clone(),
                created_at_unix: created_at,
            });
        }
    }
    out
}

// ────────────────────────────────────────────────────────────────────
// Dogecoin per-wallet aggregation: groups normalized entries by
// transaction hash, nets signed amounts, picks a counterparty, and
// produces a single aggregated record per hash.
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, uniffi::Record)]
pub struct MultiAddressAggregateInput {
    pub own_addresses: Vec<String>,
    pub entries: Vec<NormalizedHistoryItem>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct AggregatedTransaction {
    pub hash: String,
    pub kind: String,
    pub status: String,
    /// The net magnitude, as an exact decimal.
    pub amount: String,
    pub counterparty: String,
    pub block_number: Option<i64>,
    /// Earliest known non-distant-past timestamp (Unix seconds). 0 when unknown.
    pub created_at_unix: f64,
}

/// Net a wallet's own addresses together into one record per transaction.
///
/// A UTXO transaction can touch several of a wallet's addresses; without this
/// it appears once per address, with each leg's amount instead of the net.
/// Nothing here is chain-specific — it groups `NormalizedHistoryItem` by hash
/// and signs each leg against `own_addresses`.
pub fn history_aggregate_by_transaction(
    input: MultiAddressAggregateInput,
) -> Vec<AggregatedTransaction> {
    use std::collections::HashMap;
    let own: std::collections::HashSet<String> = input
        .own_addresses
        .into_iter()
        .map(|a| a.to_lowercase())
        .collect();
    let mut by_hash: HashMap<String, Vec<NormalizedHistoryItem>> = HashMap::new();
    for e in input.entries {
        if e.tx_hash.is_empty() {
            continue;
        }
        by_hash.entry(e.tx_hash.clone()).or_default().push(e);
    }
    let mut out = Vec::new();
    for (_hash, group) in by_hash {
        let Some(first) = group.first().cloned() else {
            continue;
        };
        // Net in exact decimals: the received and sent legs summed apart,
        // then the smaller taken from the larger. A leg whose amount is not a
        // decimal drops the transaction rather than netting it wrongly.
        let total = |receive: bool| {
            group
                .iter()
                .filter(|s| (s.kind == "receive") == receive)
                .try_fold("0".to_string(), |sum, s| {
                    crate::decimal::add(&sum, &s.amount)
                })
        };
        let (Some(received), Some(sent)) = (total(true), total(false)) else {
            continue;
        };
        let (kind, amount) = match crate::decimal::compare(&received, &sent) {
            Some(std::cmp::Ordering::Greater) => {
                ("receive", crate::decimal::sub_or_zero(&received, &sent))
            }
            Some(std::cmp::Ordering::Less) => {
                ("send", crate::decimal::sub_or_zero(&sent, &received))
            }
            _ => continue,
        };
        let Some(amount) = amount else { continue };
        let status = if group.iter().any(|s| s.status == "pending") {
            "pending"
        } else {
            "confirmed"
        };
        let block_number = group.iter().filter_map(|s| s.block_height).max();
        let known_ts: Vec<f64> = group
            .iter()
            .filter_map(|s| {
                if s.timestamp > 0.0 {
                    Some(s.timestamp)
                } else {
                    None
                }
            })
            .collect();
        let created_at_unix = known_ts.iter().copied().fold(f64::INFINITY, f64::min);
        let created_at_unix = if created_at_unix.is_finite() {
            created_at_unix
        } else {
            first.timestamp
        };
        let counterparty = group
            .iter()
            .map(|s| s.counterparty.clone())
            .find(|c| {
                let trimmed = c.trim();
                !trimmed.is_empty() && !own.contains(&c.to_lowercase())
            })
            .unwrap_or_else(|| first.counterparty.clone());
        out.push(AggregatedTransaction {
            hash: first.tx_hash.clone(),
            kind: kind.into(),
            status: status.into(),
            amount,
            counterparty,
            block_number,
            created_at_unix,
        });
    }
    out
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct EvmNativeAsset {
    pub asset_display_name: String,
    pub symbol: String,
}

/// Native asset name/symbol for an EVM `chain_id`. Returns `None` when
/// the chain name is not a known EVM chain.
pub fn history_evm_native_asset(chain: crate::registry::Chain) -> Option<EvmNativeAsset> {
    // Both halves are catalog columns, so every EVM network has an asset to
    // name.
    if !chain.is_evm() {
        return None;
    }
    Some(EvmNativeAsset {
        asset_display_name: chain.entry().native_asset_display_name.clone(),
        symbol: chain.entry().gas_token_symbol.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_evm_transaction_records_for_matching_transfers() {
        let page = EvmHistoryPageDecoded {
            tokens: vec![EvmTokenTransferItem {
                contract_address: "0xabc".into(),
                token_name: "USD Coin".into(),
                symbol: "USDC".into(),
                decimals: 6,
                from_address: "0xself".into(),
                to_address: "0xother".into(),
                amount_decimal: "1.5".into(),
                transaction_hash: "0xhash".into(),
                block_number: 100,
                log_index: 0,
                timestamp: 1700000000.0,
            }],
            native: vec![EvmNativeTransferItem {
                status: "confirmed".into(),
                from_address: "0xother".into(),
                to_address: "0xself".into(),
                amount_decimal: "0.25".into(),
                transaction_hash: "0xhash2".into(),
                block_number: 101,
                timestamp: 0.0,
            }],
        };
        let out = build_evm_transaction_records(EvmTransactionRecordRequest {
            decoded_page: page,
            normalized_address: "0xself".into(),
            chain_id: crate::registry::Chain::Ethereum,
            token_source_used: Some("rust/etherscan".into()),
            native_asset_display_name: "Ether".into(),
            native_asset_symbol: "ETH".into(),
            wallets: vec![EvmTransactionRecordWalletInput {
                wallet_id: "w1".into(),
                wallet_name: "Primary".into(),
            }],
            unknown_timestamp_sentinel_unix: -1.0,
        });
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].kind, "send");
        assert_eq!(out[0].symbol, "USDC");
        assert_eq!(out[0].counterparty, "0xother");
        assert_eq!(out[0].source_address, "0xself");
        assert_eq!(out[0].created_at_unix, 1700000000.0);
        assert_eq!(out[1].kind, "receive");
        assert_eq!(out[1].symbol, "ETH");
        assert_eq!(out[1].source_used, "etherscan");
        assert_eq!(out[1].created_at_unix, -1.0);
    }

    #[test]
    fn plans_evm_transaction_records_skips_unrelated_transfers() {
        let page = EvmHistoryPageDecoded {
            tokens: vec![EvmTokenTransferItem {
                contract_address: "0xabc".into(),
                token_name: "USD Coin".into(),
                symbol: "USDC".into(),
                decimals: 6,
                from_address: "0xA".into(),
                to_address: "0xB".into(),
                amount_decimal: "1".into(),
                transaction_hash: "0xhash".into(),
                block_number: 100,
                log_index: 0,
                timestamp: 1700000000.0,
            }],
            native: vec![],
        };
        let out = build_evm_transaction_records(EvmTransactionRecordRequest {
            decoded_page: page,
            normalized_address: "0xself".into(),
            chain_id: crate::registry::Chain::Ethereum,
            token_source_used: None,
            native_asset_display_name: "Ether".into(),
            native_asset_symbol: "ETH".into(),
            wallets: vec![EvmTransactionRecordWalletInput {
                wallet_id: "w1".into(),
                wallet_name: "Primary".into(),
            }],
            unknown_timestamp_sentinel_unix: -1.0,
        });
        assert!(out.is_empty());
    }

    #[test]
    fn dogecoin_aggregate_nets_amounts() {
        let entry =
            |kind: &str, amount, counterparty: &str, ts: f64, status: &str| NormalizedHistoryItem {
                deployment_id: None,
                kind: kind.into(),
                status: status.into(),
                asset_display_name: "Dogecoin".into(),
                symbol: "DOGE".into(),
                chain_id: crate::registry::Chain::Dogecoin,
                amount,
                counterparty: counterparty.into(),
                tx_hash: "tx1".into(),
                block_height: Some(100),
                timestamp: ts,
            };
        let out = history_aggregate_by_transaction(MultiAddressAggregateInput {
            own_addresses: vec!["Own1".into()],
            entries: vec![
                entry(
                    "receive",
                    "10".into(),
                    "External",
                    1700000000.0,
                    "confirmed",
                ),
                entry("send", "3".into(), "Own1", 1700000005.0, "confirmed"),
            ],
        });
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, "receive");
        assert_eq!(out[0].amount, "7");
        assert_eq!(out[0].counterparty, "External");
        assert_eq!(out[0].created_at_unix, 1700000000.0);
    }

    #[test]
    fn evm_native_asset_lookup() {
        let eth = history_evm_native_asset(crate::registry::Chain::Ethereum).unwrap();
        assert_eq!(eth.symbol, "ETH");
        let bnb = history_evm_native_asset(crate::registry::Chain::BnbChain).unwrap();
        assert_eq!(bnb.asset_display_name, "BNB");
        assert!(history_evm_native_asset(crate::registry::Chain::Bitcoin).is_none());
    }

    /// Every chain the registry knows can be paged, and iOS's "load more"
    /// covers every chain that can be paged.
    #[test]
    fn every_chain_can_be_paged() {
        use crate::registry::Chain;

        // The lookup the paging needs round-trips for every chain.
        for chain in Chain::all() {
            assert_eq!(
                Chain::from_str_id(chain.str_id()).map(|c| c.str_id()),
                Some(chain.str_id()),
                "{} cannot be paged",
                chain.str_id()
            );
        }
        assert!(Chain::from_str_id("Nope").is_none());
    }
}

#[cfg(test)]
mod aggregation_is_not_chain_specific {
    use super::*;

    /// One record per transaction, netted across the wallet's own addresses.
    #[test]
    fn two_legs_of_one_transaction_become_one_record() {
        let leg = |addr: &str, kind: &str, amount: &str| NormalizedHistoryItem {
            deployment_id: None,
            kind: kind.to_string(),
            status: "confirmed".to_string(),
            asset_display_name: "Litecoin".to_string(),
            symbol: "LTC".to_string(),
            chain_id: crate::registry::Chain::Litecoin,
            amount: amount.into(),
            counterparty: addr.to_string(),
            tx_hash: "abc".to_string(),
            block_height: Some(10),
            timestamp: 1_700_000_000.0,
        };
        let out = history_aggregate_by_transaction(MultiAddressAggregateInput {
            own_addresses: vec!["ltc1own".into(), "ltc1change".into()],
            entries: vec![
                leg("ltc1own", "send", "5"),
                leg("ltc1change", "receive", "2"),
            ],
        });
        assert_eq!(out.len(), 1, "one transaction, one record");
        assert_eq!(out[0].hash, "abc");
        // Net of the legs: 5 out, 2 back as change.
        assert_eq!(out[0].amount, "3");
        assert_eq!(out[0].kind, "send");
    }
}
