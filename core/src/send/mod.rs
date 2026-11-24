use crate::send::error::SendError;
pub mod error;
pub mod ethereum;
mod evm_overrides;
pub mod flow;
pub mod keys;
pub mod message;

pub mod payload;
pub mod payment_memo;
pub mod prepared_fields;
pub mod preview_types;
pub mod scanned_payment;
pub mod stages;
pub mod transfer;
pub mod verification;

// Per-chain write-path: build / sign / broadcast transaction methods.
pub(crate) mod account_utxo;
pub(crate) mod accounting;
pub mod aptos;
pub(crate) mod aptos_multikey;
mod bcs;
pub mod bitcoin;
pub(crate) mod bitcoin_wire;
pub mod cardano;
pub(crate) mod cardano_multisig;
pub mod decred;
pub mod evm;
pub(crate) mod icp_stages;
pub(crate) mod icp_staking;
pub mod kaspa;
pub(crate) mod leb128;
pub(crate) mod legacy_p2pkh;
pub mod litecoin;
pub(crate) mod litecoin_mweb;
pub(crate) mod litecoin_quote;
pub(crate) mod monero_local;
pub mod near;
pub(crate) mod p2sh_multisig;
pub mod peercoin;
pub mod polkadot;
pub mod polkadot_pools;
pub(crate) mod psbt;
pub(crate) mod reservation;
pub(crate) mod safe;
pub mod solana;
pub(crate) mod solana_token;
pub mod stellar;
pub(crate) mod stellar_issued;
pub(crate) mod stellar_multisig;
pub mod substrate;
pub(crate) mod substrate_multisig;
pub mod sui;
pub(crate) mod sui_multisig;
pub mod ton;
pub(crate) mod ton_multisig;
pub mod tron;
pub(crate) mod tron_multisig;
pub mod xrp;
pub(crate) mod xrp_issued;
pub(crate) mod xrp_multisig;
pub mod zcash;
pub(crate) mod zcash_shielded;
pub(crate) mod zcash_stages;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

pub use transfer::{SendAsset, SendAssetKind};

/// Whether a send can be made, with what core resolved to make it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SendPreflight {
    pub chain: crate::registry::Chain,
    pub symbol: String,
    pub normalized_destination_address: String,
    /// The amount as typed, trimmed: an exact decimal, never a float.
    pub amount: String,
    /// The token this send moves, or `None` for a native asset. A token core
    /// cannot identify is refused rather than sent with a guessed scale.
    pub token_contract_address: Option<String>,
    pub token_decimals: Option<u32>,
}

/// A send request: what `WalletService::build_send` prepares as an artifact.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SendExecutionRequest {
    /// Spectra chain ID string (e.g. "bitcoin", "ethereum").
    pub chain_id: crate::registry::Chain,
    /// Core-owned wallet whose stored signing identity is used.
    pub wallet_id: String,
    /// Required only for a password-sealed wallet. No seed or raw key crosses here.
    pub password: Option<String>,
    /// Destination/recipient address.
    pub to_address: String,
    /// Exact decimal input, validated and converted to integer units in core.
    pub amount_str: String,
    // ── Token-specific ──────────────────────────────────────────────────
    /// Contract/mint address for token sends (ERC-20, SPL, TRC-20, NEP-141).
    pub contract_address: Option<String>,
    /// Actual token protocol; omitted values are resolved by core before reads.
    pub token_standard: Option<crate::registry::TokenStandard>,
    /// Token decimals for raw-unit conversion.
    pub token_decimals: Option<u32>,
    // ── Chain-specific optional fields ───────────────────────────────────
    /// Fee rate as an exact decimal: sat/vB for Bitcoin, DOGE per kB for
    /// Dogecoin.
    pub fee_rate_svb: Option<String>,
    /// UTXO fee in satoshis (BCH, BSV, LTC, DOGE).
    pub fee_sat: Option<u64>,
    /// Sui gas budget in SUI, as an exact decimal.
    pub gas_budget: Option<String>,
    /// Cardano fee in ADA, as an exact decimal.
    pub fee_amount: Option<String>,
    /// EVM overrides (nonce, custom gas fees). Typed; Rust assembles the
    /// payload fragment internally — no JSON shuttle from Swift.
    pub evm_overrides: Option<crate::send::ethereum::EvmSendOverridesInput>,
    /// The XRP destination tag, Stellar memo or TON comment the payment carries.
    pub memo: Option<payment_memo::PaymentMemo>,
}

impl SendExecutionRequest {
    pub(crate) fn zeroize_sensitive_fields(&mut self) {
        if let Some(password) = &mut self.password {
            password.zeroize();
        }
    }
}

/// What a send that built, signed and broadcast in one call produced.
#[derive(Debug, Clone)]
pub struct SendExecutionResult {
    pub transaction_hash: String,
    /// EVM-specific details (nonce, raw_tx_hex, gas_limit), converted directly
    /// from the protocol result. `None` for non-EVM chains.
    pub evm: Option<crate::send::ethereum::EvmSendDetails>,
}

/// Why a send cannot land, once the fee is counted.
///
/// The preflight already refuses `amount > available_balance`.
/// That is half the question: the fee comes out of the chain's gas asset,
/// which for a token send is a different balance entirely, and the half that
/// knew about it lived in Swift.
#[derive(Debug, Clone, PartialEq)]
pub enum SendAffordability {
    Unavailable,
    /// Amount and fee both fit.
    Affordable,
    /// A native send: amount plus fee is more than the wallet holds.
    AmountPlusFeeExceedsBalance {
        symbol: String,
        required: String,
    },
    /// A token send: the amount alone is more than the token balance.
    AmountExceedsBalance {
        symbol: String,
    },
    /// A token send whose amount fits, with too little gas asset for the fee.
    FeeExceedsGasBalance {
        gas_symbol: String,
        fee: String,
        chain_id: crate::registry::Chain,
    },
}

/// What the caller knows. Everything else — whether this is the chain's own
/// asset, what that asset is called, how many decimals a fee is quoted to —
/// is read from the registry here.
#[derive(Debug, Clone)]
pub struct SendAffordabilityInput {
    pub is_native: bool,
    pub chain_id: crate::registry::Chain,
    /// The asset being sent.
    pub symbol: String,
    /// Exact decimals, all of them: this decides whether funds leave.
    pub amount: String,
    pub network_fee: String,
    /// What the wallet holds of the asset being sent.
    pub holding_balance: String,
    /// What it holds of the chain's gas asset. Ignored for a native send,
    /// where that is `holding_balance`.
    pub gas_balance: Option<String>,
}

/// Can this send land?
///
/// Naming the chain is enough: whether the asset is native, its symbol and
/// the fee are registry facts.
///
/// The verdict is an enum, not a sentence: the wording is localized in the
/// front end's own bundle.
pub fn send_affordability(input: SendAffordabilityInput) -> SendAffordability {
    if !input.is_native && input.gas_balance.is_none() {
        return SendAffordability::Unavailable;
    }
    let gas_symbol = input.chain_id.coin_symbol().to_string();
    use crate::amount::decimal::{add, compare};
    use std::cmp::Ordering::Greater;
    // An amount that is not a decimal cannot be judged, and is not sent.
    let exceeds = |a: &str, b: &str| compare(a, b).is_none_or(|o| o == Greater);

    if input.is_native {
        let Some(total) = add(&input.amount, &input.network_fee) else {
            return SendAffordability::Unavailable;
        };
        if exceeds(&total, &input.holding_balance) {
            return SendAffordability::AmountPlusFeeExceedsBalance {
                symbol: input.symbol,
                required: total,
            };
        }
        return SendAffordability::Affordable;
    }

    if exceeds(&input.amount, &input.holding_balance) {
        return SendAffordability::AmountExceedsBalance {
            symbol: input.symbol,
        };
    }
    match input.gas_balance {
        Some(gas_balance) if exceeds(&input.network_fee, &gas_balance) => {
            SendAffordability::FeeExceedsGasBalance {
                gas_symbol,
                fee: crate::amount::decimal::canonical(&input.network_fee)
                    .unwrap_or(input.network_fee),
                chain_id: input.chain_id,
            }
        }
        _ => SendAffordability::Affordable,
    }
}

/// Can this send be made? `asset` is `None` when the holding is missing or on
/// a chain core does not know.
pub fn validate_send_preflight(
    wallet_found: bool,
    asset: Option<&SendAsset>,
    available_balance: &str,
    destination_address: &str,
    amount_input: &str,
) -> Result<SendPreflight, SendError> {
    if !wallet_found {
        return Err(SendError::Invalid("Select a wallet".into()));
    }
    let asset = asset.ok_or_else(|| SendError::Invalid("Select an asset".into()))?;
    if !asset.is_sendable() {
        return Err(SendError::Invalid(
            format!("{} transfers are not enabled yet.", asset.symbol).into(),
        ));
    }

    let normalized_destination_address = destination_address.trim().to_string();
    if normalized_destination_address.is_empty() {
        return Err(SendError::Invalid("Enter a destination address".into()));
    }

    let amount_input = amount_input.trim();
    let exact = crate::amount::decimal::canonical(amount_input)
        .ok_or_else(|| SendError::Invalid("Enter a valid amount".into()))?;
    if !asset.allows_zero_amount() && crate::amount::decimal::is_zero(&exact) {
        return Err(SendError::Invalid("Enter a valid amount".into()));
    }
    if crate::amount::decimal::compare(&exact, available_balance)
        .is_none_or(|o| o == std::cmp::Ordering::Greater)
    {
        return Err(SendError::Invalid(
            "Amount exceeds the available balance".into(),
        ));
    }

    let token = asset.token();
    Ok(SendPreflight {
        chain: asset.chain,
        symbol: asset.symbol.clone(),
        normalized_destination_address,
        amount: amount_input.to_string(),
        token_contract_address: token.map(|token| token.contract.clone()),
        token_decimals: token.map(|token| token.decimals),
    })
}

// ── FFI surface ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
