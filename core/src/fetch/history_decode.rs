// Typed decode helpers for chain-history JSON shapes, so front ends get native
// records instead of re-parsing JSON.

use crate::store::persistence_models::TransactionRecord;
use crate::store::wallet_domain::{TransactionKind, TransactionStatus};

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
    pub status: TransactionStatus,
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

/// One transfer of any kind, as a record needs it.
struct Transfer {
    status: TransactionStatus,
    deployment_id: Option<String>,
    asset_display_name: String,
    symbol: String,
    amount: String,
    from_address: String,
    to_address: String,
    transaction_hash: String,
    block_number: i64,
    timestamp: f64,
    source: String,
}

pub(crate) fn build_evm_transaction_records(
    request: EvmTransactionRecordRequest,
) -> Vec<TransactionRecord> {
    let normalized = request.normalized_address;
    let chain = request.chain_id;
    let token_source = request
        .token_source_used
        .unwrap_or_else(|| "none".to_string());
    let page = request.decoded_page;
    let tokens = page.tokens.into_iter().map(|transfer| Transfer {
        status: TransactionStatus::Confirmed,
        deployment_id: if transfer.standard.is_empty() {
            crate::tokens::deployment_id_for(chain, Some(&transfer.contract_address))
        } else {
            crate::tokens::protocol_deployment_id(
                chain,
                &transfer.standard,
                &transfer.contract_address,
            )
        },
        asset_display_name: transfer.token_name,
        symbol: transfer.symbol,
        amount: transfer.amount_decimal,
        from_address: transfer.from_address,
        to_address: transfer.to_address,
        transaction_hash: transfer.transaction_hash,
        block_number: transfer.block_number,
        timestamp: transfer.timestamp,
        source: token_source.clone(),
    });
    // The token is its own asset: its quantity is the amount, and no catalog
    // token, price or holding shares its identity.
    let nfts = page.nfts.into_iter().map(|transfer| Transfer {
        status: TransactionStatus::Confirmed,
        deployment_id: Some(crate::tokens::nft_deployment_id(
            chain,
            transfer.standard,
            &transfer.contract_address,
            &transfer.token_id,
        )),
        asset_display_name: crate::tokens::nft_display_name(
            &transfer.collection,
            &transfer.token_id,
        ),
        symbol: crate::tokens::nft_symbol(&transfer.symbol),
        amount: transfer.quantity,
        from_address: transfer.from_address,
        to_address: transfer.to_address,
        transaction_hash: transfer.transaction_hash,
        block_number: transfer.block_number,
        timestamp: transfer.timestamp,
        source: token_source.clone(),
    });
    let native = page.native.into_iter().map(|transfer| Transfer {
        status: transfer.status,
        deployment_id: crate::tokens::deployment_id_for(chain, None),
        asset_display_name: chain.coin_name().to_string(),
        symbol: chain.coin_symbol().to_string(),
        amount: transfer.amount_decimal,
        from_address: transfer.from_address,
        to_address: transfer.to_address,
        transaction_hash: transfer.transaction_hash,
        block_number: transfer.block_number,
        timestamp: transfer.timestamp,
        source: "etherscan".to_string(),
    });
    // A transfer whose amount cannot be read is refused, as every history
    // row's is, not stored as 0.
    let transfers: Vec<Transfer> = tokens
        .chain(nfts)
        .chain(native)
        .filter_map(|mut transfer| {
            transfer.amount = crate::decimal::canonical(&transfer.amount)?;
            Some(transfer)
        })
        .collect();

    let mut out = Vec::new();
    for wallet in &request.wallets {
        for transfer in &transfers {
            let is_outgoing = transfer.from_address == normalized;
            if !is_outgoing && transfer.to_address != normalized {
                continue;
            }
            let (counterparty, wallet_side) = if is_outgoing {
                (&transfer.to_address, &transfer.from_address)
            } else {
                (&transfer.from_address, &transfer.to_address)
            };
            out.push(TransactionRecord {
                actions: Default::default(),
                deployment_id: transfer.deployment_id.clone(),
                id: crate::store::new_transaction_id(),
                wallet_id: Some(wallet.wallet_id.clone()),
                kind: if is_outgoing {
                    TransactionKind::Send
                } else {
                    TransactionKind::Receive
                },
                status: transfer.status,
                wallet_name: wallet.wallet_name.clone(),
                asset_display_name: transfer.asset_display_name.clone(),
                symbol: transfer.symbol.clone(),
                chain_id: chain,
                amount: transfer.amount.clone(),
                address: counterparty.clone(),
                transaction_hash: Some(transfer.transaction_hash.clone())
                    .filter(|hash| !hash.is_empty()),
                nonce: None,
                receipt_block_number: Some(transfer.block_number),
                receipt_gas_used: None,
                receipt_effective_gas_price_gwei: None,
                receipt_network_fee: None,
                fee_rate_description: None,
                confirmation_count: None,
                confirmed_network_fee: None,
                used_change_output: None,
                source_derivation_path: None,
                change_derivation_path: None,
                source_address: Some(wallet_side.clone()).filter(|address| !address.is_empty()),
                change_address: None,
                signed_transaction_payload: None,
                signed_transaction_payload_format: None,
                failure_reason: None,
                transaction_history_source: Some(transfer.source.clone()),
                created_at_unix: if transfer.timestamp > 0.0 {
                    transfer.timestamp
                } else {
                    request.unknown_timestamp_sentinel_unix
                },
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
                status: TransactionStatus::Confirmed,
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
        assert_eq!(out[0].kind, TransactionKind::Send);
        assert_eq!(out[0].symbol, "USDC");
        assert_eq!(out[0].address, "0xother");
        assert_eq!(out[0].source_address.as_deref(), Some("0xself"));
        assert_eq!(out[0].created_at_unix, 1700000000.0);
        assert_eq!(out[1].kind, TransactionKind::Receive);
        assert_eq!(out[1].symbol, "ETH");
        assert_eq!(
            out[1].transaction_history_source.as_deref(),
            Some("etherscan")
        );
        assert_eq!(out[1].created_at_unix, -1.0);
    }

    /// Each NFT transfer is its own token: its deployment names the
    /// collection and id, its name is the collection's and `#id`, and its
    /// quantity is the amount, never a balance scaled by decimals.
    #[test]
    fn an_nft_transfer_is_recorded_as_its_own_token_and_quantity() {
        use crate::api::evm_nft::NftStandard;
        let transfer = |standard,
                        contract: &str,
                        id: &str,
                        quantity: &str,
                        collection: &str,
                        symbol: &str,
                        from: &str,
                        to: &str| EvmNftTransferItem {
            standard,
            contract_address: contract.into(),
            token_id: id.into(),
            quantity: quantity.into(),
            collection: collection.into(),
            symbol: symbol.into(),
            from_address: from.into(),
            to_address: to.into(),
            transaction_hash: format!("0x{id}"),
            block_number: 100,
            timestamp: 1_780_000_000.0,
        };
        let out = build_evm_transaction_records(EvmTransactionRecordRequest {
            decoded_page: EvmHistoryPageDecoded {
                nfts: vec![
                    transfer(
                        NftStandard::Erc721,
                        "0xABC",
                        "1234",
                        "1",
                        "Apes",
                        "APE",
                        "0xself",
                        "0xother",
                    ),
                    transfer(
                        NftStandard::Erc1155,
                        "0xdef",
                        "7",
                        "4",
                        "",
                        "",
                        "0xother",
                        "0xself",
                    ),
                ],
                ..Default::default()
            },
            normalized_address: "0xself".into(),
            chain_id: crate::registry::Chain::Ethereum,
            token_source_used: None,
            wallets: vec![EvmTransactionRecordWalletInput {
                wallet_id: "w1".into(),
                wallet_name: "Collector".into(),
            }],
            unknown_timestamp_sentinel_unix: -1.0,
        });
        let rows: Vec<_> = out
            .iter()
            .map(|row| {
                (
                    row.deployment_id.as_deref().unwrap(),
                    row.kind,
                    row.amount.as_str(),
                    row.asset_display_name.as_str(),
                    row.symbol.as_str(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                (
                    "ethereum:erc-721:0xabc:1234",
                    TransactionKind::Send,
                    "1",
                    "Apes #1234",
                    "APE"
                ),
                (
                    "ethereum:erc-1155:0xdef:7",
                    TransactionKind::Receive,
                    "4",
                    "#7",
                    "NFT"
                ),
            ]
        );
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

    /// A transfer whose amount cannot be read is no row, never a row of 0:
    /// the rule every other history row already follows.
    #[test]
    fn an_unreadable_amount_is_refused_not_stored_as_zero() {
        let transfer = |hash: &str, quantity: &str| EvmNftTransferItem {
            standard: crate::api::evm_nft::NftStandard::Erc1155,
            contract_address: "0xdef".into(),
            token_id: "7".into(),
            quantity: quantity.into(),
            collection: String::new(),
            symbol: String::new(),
            from_address: "0xother".into(),
            to_address: "0xself".into(),
            transaction_hash: hash.into(),
            block_number: 100,
            timestamp: 1_780_000_000.0,
        };
        let out = build_evm_transaction_records(EvmTransactionRecordRequest {
            decoded_page: EvmHistoryPageDecoded {
                nfts: vec![
                    transfer("0xgarbled", "lots"),
                    transfer("0xempty", ""),
                    transfer("0xread", "3"),
                ],
                ..Default::default()
            },
            normalized_address: "0xself".into(),
            chain_id: crate::registry::Chain::Ethereum,
            token_source_used: None,
            wallets: vec![EvmTransactionRecordWalletInput {
                wallet_id: "w1".into(),
                wallet_name: "Collector".into(),
            }],
            unknown_timestamp_sentinel_unix: -1.0,
        });
        let rows: Vec<_> = out
            .iter()
            .map(|row| (row.transaction_hash.as_deref(), row.amount.as_str()))
            .collect();
        assert_eq!(rows, [(Some("0xread"), "3")]);
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
