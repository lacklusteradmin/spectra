use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransferHoldingInput {
    pub index: u64,
    pub chain_name: String,
    pub symbol: String,
    pub supports_send: bool,
    pub supports_receive_address: bool,
    pub is_live_chain: bool,
    pub supports_evm_token: bool,
    pub supports_solana_send_coin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransferWalletInput {
    pub wallet_id: String,
    pub has_signing_material: bool,
    pub holdings: Vec<TransferHoldingInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransferAvailabilityRequest {
    pub wallets: Vec<TransferWalletInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletTransferAvailability {
    pub wallet_id: String,
    pub send_holding_indices: Vec<u64>,
    pub receive_holding_indices: Vec<u64>,
    pub receive_chains: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransferAvailabilityPlan {
    pub wallets: Vec<WalletTransferAvailability>,
    pub send_enabled_wallet_ids: Vec<String>,
    pub receive_enabled_wallet_ids: Vec<String>,
}

use crate::registry::SendRule;

pub fn plan_transfer_availability(
    request: TransferAvailabilityRequest,
) -> TransferAvailabilityPlan {
    let mut wallet_plans = Vec::with_capacity(request.wallets.len());
    let mut send_enabled_wallet_ids = Vec::new();
    let mut receive_enabled_wallet_ids = Vec::new();

    for wallet in request.wallets {
        let send_holding_indices = wallet
            .holdings
            .iter()
            .filter(|holding| can_send_holding(holding, wallet.has_signing_material))
            .map(|holding| holding.index)
            .collect::<Vec<_>>();
        let receive_holding_indices = wallet
            .holdings
            .iter()
            .filter(|holding| holding.supports_receive_address)
            .map(|holding| holding.index)
            .collect::<Vec<_>>();

        let mut receive_chains = Vec::new();
        for holding in wallet
            .holdings
            .iter()
            .filter(|holding| receive_holding_indices.contains(&holding.index))
        {
            if !receive_chains
                .iter()
                .any(|chain| chain == &holding.chain_name)
            {
                receive_chains.push(holding.chain_name.clone());
            }
        }

        if !send_holding_indices.is_empty() {
            send_enabled_wallet_ids.push(wallet.wallet_id.clone());
        }
        if !receive_holding_indices.is_empty() {
            receive_enabled_wallet_ids.push(wallet.wallet_id.clone());
        }

        wallet_plans.push(WalletTransferAvailability {
            wallet_id: wallet.wallet_id,
            send_holding_indices,
            receive_holding_indices,
            receive_chains,
        });
    }

    TransferAvailabilityPlan {
        wallets: wallet_plans,
        send_enabled_wallet_ids,
        receive_enabled_wallet_ids,
    }
}

/// Whether a holding can be sent, applied to a resolved coin.
///
/// Same rules as `can_send_holding`, expressed against the record core actually
/// holds instead of an index-and-flags input the caller had to assemble.
pub(crate) fn can_send_coin(
    coin: &crate::store::wallet_domain::CoreCoin,
    has_signing_material: bool,
    chain_supports_send: bool,
    is_live_chain: bool,
    token_preferences: &[crate::store::wallet_domain::CoreTokenPreferenceEntry],
) -> bool {
    if !chain_supports_send {
        return false;
    }
    if is_live_chain && !has_signing_material {
        return false;
    }
    let Some(chain) = crate::registry::Chain::from_display_name(&coin.chain_name) else {
        return true;
    };
    let is_tracked_token = || {
        token_preferences.iter().any(|entry| {
            entry.is_enabled
                && entry.symbol == coin.symbol
                && chain_display_name(entry.chain) == coin.chain_name
                && match coin.contract_address.as_deref() {
                    Some(contract) => entry.contract_address.eq_ignore_ascii_case(contract),
                    None => true,
                }
        })
    };
    match chain.send_rule() {
        SendRule::Any => true,
        SendRule::NativeOnly => coin.symbol == chain.coin_symbol(),
        SendRule::NativeOrSupportedToken => {
            coin.symbol == chain.coin_symbol() || is_tracked_token()
        }
        SendRule::SupportedSolanaCoin => coin.symbol == chain.coin_symbol() || is_tracked_token(),
    }
}

/// `CoreTokenTrackingChain`'s serde names are the chain display names.
fn chain_display_name(chain: crate::store::wallet_domain::CoreTokenTrackingChain) -> &'static str {
    use crate::store::wallet_domain::CoreTokenTrackingChain as C;
    match chain {
        C::Ethereum => "Ethereum",
        C::Arbitrum => "Arbitrum",
        C::Optimism => "Optimism",
        C::Bnb => "BNB Chain",
        C::Avalanche => "Avalanche",
        C::Hyperliquid => "Hyperliquid",
        C::Polygon => "Polygon",
        C::Base => "Base",
        C::Linea => "Linea",
        C::Scroll => "Scroll",
        C::Blast => "Blast",
        C::Mantle => "Mantle",
        C::Solana => "Solana",
        C::Sui => "Sui",
        C::Aptos => "Aptos",
        C::Ton => "TON",
        C::Near => "NEAR",
        C::Tron => "Tron",
    }
}

fn can_send_holding(holding: &TransferHoldingInput, has_signing_material: bool) -> bool {
    if !holding.supports_send {
        return false;
    }
    if holding.is_live_chain && !has_signing_material {
        return false;
    }

    // The per-chain rule lives on `registry::Chain`; an unknown chain name
    // keeps the old fallthrough rather than silently blocking sends.
    let Some(chain) = crate::registry::Chain::from_display_name(&holding.chain_name) else {
        return true;
    };
    match chain.send_rule() {
        SendRule::Any => true,
        SendRule::NativeOnly => holding.symbol == chain.coin_symbol(),
        SendRule::NativeOrSupportedToken => {
            holding.symbol == chain.coin_symbol() || holding.supports_evm_token
        }
        SendRule::SupportedSolanaCoin => holding.supports_solana_send_coin,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        plan_transfer_availability, TransferAvailabilityRequest, TransferHoldingInput,
        TransferWalletInput,
    };

    #[test]
    fn plans_send_and_receive_holdings_with_chain_specific_rules() {
        let plan = plan_transfer_availability(TransferAvailabilityRequest {
            wallets: vec![TransferWalletInput {
                wallet_id: "wallet-1".to_string(),
                has_signing_material: true,
                holdings: vec![
                    TransferHoldingInput {
                        index: 0,
                        chain_name: "Ethereum".to_string(),
                        symbol: "ETH".to_string(),
                        supports_send: true,
                        supports_receive_address: true,
                        is_live_chain: true,
                        supports_evm_token: false,
                        supports_solana_send_coin: false,
                    },
                    TransferHoldingInput {
                        index: 1,
                        chain_name: "Ethereum".to_string(),
                        symbol: "USDC".to_string(),
                        supports_send: true,
                        supports_receive_address: true,
                        is_live_chain: true,
                        supports_evm_token: true,
                        supports_solana_send_coin: false,
                    },
                    TransferHoldingInput {
                        index: 2,
                        chain_name: "Solana".to_string(),
                        symbol: "BONK".to_string(),
                        supports_send: true,
                        supports_receive_address: true,
                        is_live_chain: true,
                        supports_evm_token: false,
                        supports_solana_send_coin: false,
                    },
                ],
            }],
        });

        assert_eq!(plan.send_enabled_wallet_ids, vec!["wallet-1".to_string()]);
        assert_eq!(
            plan.receive_enabled_wallet_ids,
            vec!["wallet-1".to_string()]
        );
        assert_eq!(plan.wallets[0].send_holding_indices, vec![0u64, 1u64]);
        assert_eq!(
            plan.wallets[0].receive_holding_indices,
            vec![0u64, 1u64, 2u64]
        );
        assert_eq!(
            plan.wallets[0].receive_chains,
            vec!["Ethereum".to_string(), "Solana".to_string()]
        );
    }

    #[test]
    fn blocks_live_chain_send_without_signing_material() {
        let plan = plan_transfer_availability(TransferAvailabilityRequest {
            wallets: vec![TransferWalletInput {
                wallet_id: "wallet-2".to_string(),
                has_signing_material: false,
                holdings: vec![TransferHoldingInput {
                    index: 0,
                    chain_name: "Bitcoin".to_string(),
                    symbol: "BTC".to_string(),
                    supports_send: true,
                    supports_receive_address: true,
                    is_live_chain: true,
                    supports_evm_token: false,
                    supports_solana_send_coin: false,
                }],
            }],
        });

        assert!(plan.send_enabled_wallet_ids.is_empty());
        assert_eq!(plan.wallets[0].send_holding_indices, Vec::<u64>::new());
        assert_eq!(plan.wallets[0].receive_holding_indices, vec![0u64]);
    }
}

// ── FFI surface ─────────────────────────────────────────────────────────────


