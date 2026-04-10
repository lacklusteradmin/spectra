use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SendAssetRoutingInput {
    pub chain_name: String,
    pub symbol: String,
    pub is_evm_chain: bool,
    pub supports_solana_send_coin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SendAssetRoutingPlan {
    pub preview_kind: Option<String>,
    pub submit_kind: Option<String>,
    pub native_evm_symbol: Option<String>,
    pub is_native_evm_asset: bool,
    pub allows_zero_amount: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SendPreviewRoutingRequest {
    pub asset: Option<SendAssetRoutingInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SendPreviewRoutingPlan {
    pub active_preview_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SendSubmitPreflightRequest {
    pub wallet_found: bool,
    pub asset_found: bool,
    pub destination_address: String,
    pub amount_input: String,
    pub available_balance: f64,
    pub asset: Option<SendAssetRoutingInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SendSubmitPreflightPlan {
    pub submit_kind: String,
    pub preview_kind: Option<String>,
    pub normalized_destination_address: String,
    pub amount: f64,
    pub chain_name: String,
    pub symbol: String,
    pub native_evm_symbol: Option<String>,
    pub is_native_evm_asset: bool,
    pub allows_zero_amount: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TransferRequest {
    pub chain_name: String,
    pub from_address: String,
    pub to_address: String,
    pub amount: String,
    pub asset_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TransferPlan {
    pub chain_name: String,
    pub estimated_fee: String,
    pub signing_payload_hex: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SignedTransfer {
    pub chain_name: String,
    pub raw_transaction_hex: String,
    pub txid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastReceipt {
    pub chain_name: String,
    pub txid: String,
    pub source_id: String,
}

pub trait TransferPlanner: Send + Sync {
    fn build_plan(&self, request: &TransferRequest) -> Result<TransferPlan, String>;
}

pub trait TransactionBroadcaster: Send + Sync {
    fn broadcast(&self, signed_transfer: &SignedTransfer) -> Result<BroadcastReceipt, String>;
}

pub fn route_send_asset(input: &SendAssetRoutingInput) -> SendAssetRoutingPlan {
    let submit_kind = match (input.chain_name.as_str(), input.symbol.as_str()) {
        ("Bitcoin", "BTC") => Some("bitcoin"),
        ("Bitcoin Cash", "BCH") => Some("bitcoinCash"),
        ("Bitcoin SV", "BSV") => Some("bitcoinSV"),
        ("Litecoin", "LTC") => Some("litecoin"),
        ("Dogecoin", "DOGE") => Some("dogecoin"),
        ("Tron", "TRX") | ("Tron", "USDT") => Some("tron"),
        ("XRP Ledger", "XRP") => Some("xrp"),
        ("Stellar", "XLM") => Some("stellar"),
        ("Monero", "XMR") => Some("monero"),
        ("Cardano", "ADA") => Some("cardano"),
        ("Sui", "SUI") => Some("sui"),
        ("Aptos", "APT") => Some("aptos"),
        ("TON", "TON") => Some("ton"),
        ("Internet Computer", "ICP") => Some("icp"),
        ("NEAR", "NEAR") => Some("near"),
        ("Polkadot", "DOT") => Some("polkadot"),
        _ if input.is_evm_chain => Some("ethereum"),
        _ if input.supports_solana_send_coin => Some("solana"),
        _ => None,
    }
    .map(str::to_string);

    let native_evm_symbol = native_evm_symbol_for_chain(&input.chain_name);
    let is_native_evm_asset = native_evm_symbol
        .as_ref()
        .map(|symbol| input.symbol == symbol.as_str())
        .unwrap_or(false);

    SendAssetRoutingPlan {
        preview_kind: submit_kind.clone(),
        submit_kind,
        native_evm_symbol,
        is_native_evm_asset,
        allows_zero_amount: is_native_evm_asset,
    }
}

pub fn plan_send_preview_routing(request: SendPreviewRoutingRequest) -> SendPreviewRoutingPlan {
    let active_preview_kind = request
        .asset
        .as_ref()
        .and_then(|asset| route_send_asset(asset).preview_kind);
    SendPreviewRoutingPlan {
        active_preview_kind,
    }
}

pub fn plan_send_submit_preflight(
    request: SendSubmitPreflightRequest,
) -> Result<SendSubmitPreflightPlan, String> {
    if !request.wallet_found {
        return Err("Select a wallet".to_string());
    }
    if !request.asset_found {
        return Err("Select an asset".to_string());
    }

    let asset = request.asset.ok_or_else(|| "Select an asset".to_string())?;
    let route = route_send_asset(&asset);
    let submit_kind = route
        .submit_kind
        .clone()
        .ok_or_else(|| format!("{} transfers are not enabled yet.", asset.symbol))?;

    let normalized_destination_address = request.destination_address.trim().to_string();
    if normalized_destination_address.is_empty() {
        return Err("Enter a destination address".to_string());
    }

    let amount_input = request.amount_input.trim();
    let amount = amount_input
        .parse::<f64>()
        .map_err(|_| "Enter a valid amount".to_string())?;

    if !amount.is_finite() || amount < 0.0 {
        return Err("Enter a valid amount".to_string());
    }

    if !route.allows_zero_amount && amount <= 0.0 {
        return Err("Enter a valid amount".to_string());
    }

    if amount > request.available_balance {
        return Err("Amount exceeds the available balance".to_string());
    }

    Ok(SendSubmitPreflightPlan {
        submit_kind,
        preview_kind: route.preview_kind,
        normalized_destination_address,
        amount,
        chain_name: asset.chain_name,
        symbol: asset.symbol,
        native_evm_symbol: route.native_evm_symbol,
        is_native_evm_asset: route.is_native_evm_asset,
        allows_zero_amount: route.allows_zero_amount,
    })
}

fn native_evm_symbol_for_chain(chain_name: &str) -> Option<String> {
    match chain_name {
        "Ethereum" | "Arbitrum" | "Optimism" => Some("ETH".to_string()),
        "Ethereum Classic" => Some("ETC".to_string()),
        "BNB Chain" => Some("BNB".to_string()),
        "Avalanche" => Some("AVAX".to_string()),
        "Hyperliquid" => Some("HYPE".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        plan_send_preview_routing, plan_send_submit_preflight, route_send_asset,
        SendAssetRoutingInput, SendPreviewRoutingRequest, SendSubmitPreflightRequest,
    };

    #[test]
    fn routes_evm_native_assets_with_native_symbol_metadata() {
        let route = route_send_asset(&SendAssetRoutingInput {
            chain_name: "Avalanche".to_string(),
            symbol: "AVAX".to_string(),
            is_evm_chain: true,
            supports_solana_send_coin: false,
        });

        assert_eq!(route.preview_kind.as_deref(), Some("ethereum"));
        assert_eq!(route.native_evm_symbol.as_deref(), Some("AVAX"));
        assert!(route.is_native_evm_asset);
        assert!(route.allows_zero_amount);
    }

    #[test]
    fn routes_supported_solana_assets_to_solana_preview_and_submit() {
        let plan = plan_send_preview_routing(SendPreviewRoutingRequest {
            asset: Some(SendAssetRoutingInput {
                chain_name: "Solana".to_string(),
                symbol: "USDC".to_string(),
                is_evm_chain: false,
                supports_solana_send_coin: true,
            }),
        });

        assert_eq!(plan.active_preview_kind.as_deref(), Some("solana"));
    }

    #[test]
    fn rejects_zero_amount_for_non_evm_native_sends() {
        let error = plan_send_submit_preflight(SendSubmitPreflightRequest {
            wallet_found: true,
            asset_found: true,
            destination_address: "bc1qdestination".to_string(),
            amount_input: "0".to_string(),
            available_balance: 1.0,
            asset: Some(SendAssetRoutingInput {
                chain_name: "Bitcoin".to_string(),
                symbol: "BTC".to_string(),
                is_evm_chain: false,
                supports_solana_send_coin: false,
            }),
        })
        .expect_err("bitcoin zero-value sends should be rejected in preflight");

        assert_eq!(error, "Enter a valid amount");
    }

    #[test]
    fn preserves_zero_amount_for_native_evm_preflight() {
        let plan = plan_send_submit_preflight(SendSubmitPreflightRequest {
            wallet_found: true,
            asset_found: true,
            destination_address: "0xabc".to_string(),
            amount_input: "0".to_string(),
            available_balance: 1.0,
            asset: Some(SendAssetRoutingInput {
                chain_name: "Ethereum".to_string(),
                symbol: "ETH".to_string(),
                is_evm_chain: true,
                supports_solana_send_coin: false,
            }),
        })
        .expect("native EVM zero-value sends remain allowed");

        assert_eq!(plan.submit_kind, "ethereum");
        assert_eq!(plan.amount, 0.0);
        assert!(plan.allows_zero_amount);
    }
}
