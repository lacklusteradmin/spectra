// The send preview: one record for every chain, carrying what a quote costs
// and how much it can move, plus the fee terms a chain's signed transaction
// repeats.
//
// Every amount — fee, balance, maximum — is an exact decimal string. Nothing
// on the send path is a float.

use serde::Serialize;

/// A quote for a transfer of one asset.
#[derive(Debug, Clone, Default, Serialize, PartialEq, uniffi::Record)]
pub struct SendPreview {
    /// The fee, in the chain's gas asset.
    pub estimated_network_fee: String,
    /// What the sent asset holds, in its own unit.
    pub spendable_balance: String,
    /// The most of the sent asset that can go once the fee is paid.
    pub max_sendable: String,
    pub fee_rate_description: Option<String>,
    /// The transaction's size, where the builder measured it.
    pub estimated_transaction_bytes: Option<i64>,
    pub terms: SendFeeTerms,
}

/// What the fee is made of, where a chain's transaction states more than its
/// amount.
#[derive(Debug, Clone, Default, Serialize, PartialEq, uniffi::Enum)]
pub enum SendFeeTerms {
    /// The fee amount is all there is to it.
    #[default]
    Flat,
    /// The EIP-1559 fields the signed transaction carries; review and
    /// replacement start from them.
    Evm { terms: EvmFeeTerms },
    /// The coins a UTXO send selects, at the rate it pays.
    Utxo {
        fee_rate_sat_vb: u64,
        selected_input_count: i64,
        uses_change_output: bool,
    },
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, uniffi::Record)]
pub struct EvmFeeTerms {
    pub nonce: i64,
    pub gas_limit: i64,
    pub max_fee_per_gas_gwei: String,
    pub max_priority_fee_per_gas_gwei: String,
}
