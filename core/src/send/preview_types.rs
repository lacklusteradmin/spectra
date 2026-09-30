// Per-chain send-preview record types, plus the `SendPreview` tagged enum in
// `send::flow` that carries one of them across the FFI.
//
// Every amount — fee, balance, maximum, fee rate — is an exact decimal
// string in the asset's own unit. Nothing on the send path is a float.

use serde::{Deserialize, Serialize};

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct EvmSendPreview {
    pub nonce: i64,
    pub gasLimit: i64,
    pub maxFeePerGasGwei: String,
    pub maxPriorityFeePerGasGwei: String,
    pub estimatedNetworkFee: String,
    pub spendableBalance: Option<String>,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct BitcoinSendPreview {
    pub estimatedFeeRateSatVb: u64,
    pub estimatedNetworkFee: String,
    pub feeRateDescription: Option<String>,
    pub spendableBalance: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct DogecoinSendPreview {
    pub estimatedNetworkFee: String,
    pub estimatedFeeRateDogePerKb: String,
    pub estimatedTransactionBytes: i64,
    pub selectedInputCount: i64,
    pub usesChangeOutput: bool,
    pub spendableBalance: String,
    pub feeRateDescription: Option<String>,
    pub maxSendable: String,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct TronSendPreview {
    pub estimatedNetworkFee: String,
    pub feeLimitSun: i64,
    pub simulationUsed: bool,
    pub spendableBalance: String,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: String,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct SolanaSendPreview {
    pub estimatedNetworkFee: String,
    pub spendableBalance: String,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: String,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct XrpSendPreview {
    pub estimatedNetworkFee: String,
    pub feeDrops: i64,
    pub sequence: i64,
    pub lastLedgerSequence: i64,
    pub spendableBalance: String,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: String,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct StellarSendPreview {
    pub estimatedNetworkFee: String,
    pub feeStroops: i64,
    pub sequence: i64,
    pub spendableBalance: String,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: String,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct MoneroSendPreview {
    pub estimatedNetworkFee: String,
    pub priorityLabel: String,
    pub spendableBalance: String,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: String,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct CardanoSendPreview {
    pub estimatedNetworkFee: String,
    pub ttlSlot: u64,
    pub spendableBalance: String,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: String,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct SuiSendPreview {
    pub estimatedNetworkFee: String,
    pub gasBudgetMist: u64,
    pub referenceGasPrice: u64,
    pub spendableBalance: String,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: String,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct AptosSendPreview {
    pub estimatedNetworkFee: String,
    pub maxGasAmount: u64,
    pub gasUnitPriceOctas: u64,
    pub spendableBalance: String,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: String,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct TonSendPreview {
    pub estimatedNetworkFee: String,
    pub sequenceNumber: u32,
    pub spendableBalance: String,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: String,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct IcpSendPreview {
    pub estimatedNetworkFee: String,
    pub feeE8s: u64,
    pub spendableBalance: String,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: String,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct NearSendPreview {
    pub estimatedNetworkFee: String,
    pub gasPriceYoctoNear: String,
    pub spendableBalance: String,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: String,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct PolkadotSendPreview {
    pub estimatedNetworkFee: String,
    pub spendableBalance: String,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: String,
}
