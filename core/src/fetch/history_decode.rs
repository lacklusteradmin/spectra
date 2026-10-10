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
// `WalletService::fetch_evm_history_page` (tokens, NFTs and native).
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct EvmTokenTransferItem {
    pub standard: String,
    pub contract_address: String,
    pub token_name: String,
    pub symbol: String,
    pub from_address: String,
    pub to_address: String,
    /// Decimal amount serialized as a string so Swift can reconstruct a
    /// `Decimal` without floating-point loss.
    pub amount_decimal: String,
    pub transaction_hash: String,
    pub block_number: i64,
    pub timestamp: f64,
}

/// One ERC-721 or ERC-1155 transfer: a token id and a whole quantity.
#[derive(Debug, Clone)]
pub struct EvmNftTransferItem {
    pub standard: crate::api::evm_nft::NftStandard,
    pub contract_address: String,
    pub token_id: String,
    pub quantity: String,
    pub collection: String,
    pub symbol: String,
    pub from_address: String,
    pub to_address: String,
    pub transaction_hash: String,
    pub block_number: i64,
    pub timestamp: f64,
}

#[derive(Debug, Clone)]
pub struct EvmNativeTransferItem {
    pub status: String,
    pub from_address: String,
    pub to_address: String,
    pub amount_decimal: String,
    pub transaction_hash: String,
    pub block_number: i64,
    pub timestamp: f64,
}

#[derive(Debug, Clone, Default)]
pub struct EvmHistoryPageDecoded {
    pub tokens: Vec<EvmTokenTransferItem>,
    pub nfts: Vec<EvmNftTransferItem>,
    pub native: Vec<EvmNativeTransferItem>,
}

// ────────────────────────────────────────────────────────────────────
// EVM history page → per-wallet transaction record projection.
// Given a decoded page and the target wallets, emits one record per
// (wallet × matching transfer) where "matching" means the transfer
// touches the wallet's normalized address as sender or receiver.
// The history service merges these records into core-owned transaction state.
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct EvmHistoryTransactionRecord {
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

#[derive(Debug, Clone)]
pub(crate) struct EvmTransactionRecordWalletInput {
    pub wallet_id: String,
    pub wallet_name: String,
}

#[derive(Debug, Clone)]
pub(crate) struct EvmTransactionRecordRequest {
    pub decoded_page: EvmHistoryPageDecoded,
    pub normalized_address: String,
    pub chain_id: crate::registry::Chain,
    pub token_source_used: Option<String>,
    pub wallets: Vec<EvmTransactionRecordWalletInput>,
    pub unknown_timestamp_sentinel_unix: f64,
}

pub(crate) fn build_evm_transaction_records(
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
                deployment_id: if transfer.standard.is_empty() {
                    crate::tokens::deployment_id_for(
                        request.chain_id,
                        Some(&transfer.contract_address),
                    )
                } else {
                    crate::tokens::protocol_deployment_id(
                        request.chain_id,
                        &transfer.standard,
                        &transfer.contract_address,
                    )
                },
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
        for transfer in &request.decoded_page.nfts {
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
            // The token is its own asset: its quantity is the amount, and no
            // catalog token, price or holding shares its identity.
            out.push(EvmHistoryTransactionRecord {
                status: "confirmed".into(),
                deployment_id: Some(crate::tokens::nft_deployment_id(
                    request.chain_id,
                    transfer.standard,
                    &transfer.contract_address,
                    &transfer.token_id,
                )),
                wallet_id: wallet.wallet_id.clone(),
                wallet_name: wallet.wallet_name.clone(),
                kind: if is_outgoing { "send" } else { "receive" }.to_string(),
                asset_display_name: crate::tokens::nft_display_name(
                    &transfer.collection,
                    &transfer.token_id,
                ),
                symbol: crate::tokens::nft_symbol(&transfer.symbol),
                chain_id: request.chain_id,
                amount_decimal: transfer.quantity.clone(),
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
                asset_display_name: request.chain_id.coin_name().to_string(),
                symbol: request.chain_id.coin_symbol().to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_evm_transaction_records_for_matching_transfers() {
        let page = EvmHistoryPageDecoded {
            tokens: vec![EvmTokenTransferItem {
                standard: String::new(),
                contract_address: "0xabc".into(),
                token_name: "USD Coin".into(),
                symbol: "USDC".into(),
                from_address: "0xself".into(),
                to_address: "0xother".into(),
                amount_decimal: "1.5".into(),
                transaction_hash: "0xhash".into(),
                block_number: 100,
                timestamp: 1700000000.0,
            }],
            nfts: vec![],
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
                standard: String::new(),
                contract_address: "0xabc".into(),
                token_name: "USD Coin".into(),
                symbol: "USDC".into(),
                from_address: "0xA".into(),
                to_address: "0xB".into(),
                amount_decimal: "1".into(),
                transaction_hash: "0xhash".into(),
                block_number: 100,
                timestamp: 1700000000.0,
            }],
            ..Default::default()
        };
        let out = build_evm_transaction_records(EvmTransactionRecordRequest {
            decoded_page: page,
            normalized_address: "0xself".into(),
            chain_id: crate::registry::Chain::Ethereum,
            token_source_used: None,
            wallets: vec![EvmTransactionRecordWalletInput {
                wallet_id: "w1".into(),
                wallet_name: "Primary".into(),
            }],
            unknown_timestamp_sentinel_unix: -1.0,
        });
        assert!(out.is_empty());
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
