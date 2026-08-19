// Per-chain send-preview record types, plus the `SendPreview` tagged enum in
// `send::flow` that carries one of them across the FFI.

use serde::{Deserialize, Serialize};

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct EthereumSendPreview {
    pub nonce: i64,
    pub gasLimit: i64,
    pub maxFeePerGasGwei: f64,
    pub maxPriorityFeePerGasGwei: f64,
    pub estimatedNetworkFee: f64,
    pub spendableBalance: Option<f64>,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: Option<f64>,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct BitcoinSendPreview {
    pub estimatedFeeRateSatVb: u64,
    pub estimatedNetworkFee: f64,
    pub feeRateDescription: Option<String>,
    pub spendableBalance: Option<f64>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: Option<f64>,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct DogecoinSendPreview {
    pub spendableBalanceDoge: f64,
    pub requestedAmountDoge: f64,
    pub estimatedNetworkFee: f64,
    pub estimatedFeeRateDogePerKb: f64,
    pub estimatedTransactionBytes: i64,
    pub selectedInputCount: i64,
    pub usesChangeOutput: bool,
    pub feePriority: String,
    pub maxSendableDoge: f64,
    pub spendableBalance: f64,
    pub feeRateDescription: Option<String>,
    pub maxSendable: f64,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct TronSendPreview {
    pub estimatedNetworkFee: f64,
    pub feeLimitSun: i64,
    pub simulationUsed: bool,
    pub spendableBalance: f64,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: f64,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct SolanaSendPreview {
    pub estimatedNetworkFee: f64,
    pub spendableBalance: f64,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: f64,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct XRPSendPreview {
    pub estimatedNetworkFee: f64,
    pub feeDrops: i64,
    pub sequence: i64,
    pub lastLedgerSequence: i64,
    pub spendableBalance: f64,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: f64,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct StellarSendPreview {
    pub estimatedNetworkFee: f64,
    pub feeStroops: i64,
    pub sequence: i64,
    pub spendableBalance: f64,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: f64,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct MoneroSendPreview {
    pub estimatedNetworkFee: f64,
    pub priorityLabel: String,
    pub spendableBalance: f64,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: f64,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct CardanoSendPreview {
    pub estimatedNetworkFee: f64,
    pub ttlSlot: u64,
    pub spendableBalance: f64,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: f64,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct SuiSendPreview {
    pub estimatedNetworkFee: f64,
    pub gasBudgetMist: u64,
    pub referenceGasPrice: u64,
    pub spendableBalance: f64,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: f64,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct AptosSendPreview {
    pub estimatedNetworkFee: f64,
    pub maxGasAmount: u64,
    pub gasUnitPriceOctas: u64,
    pub spendableBalance: f64,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: f64,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct TONSendPreview {
    pub estimatedNetworkFee: f64,
    pub sequenceNumber: u32,
    pub spendableBalance: f64,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: f64,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct ICPSendPreview {
    pub estimatedNetworkFee: f64,
    pub feeE8s: u64,
    pub spendableBalance: f64,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: f64,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct NearSendPreview {
    pub estimatedNetworkFee: f64,
    pub gasPriceYoctoNear: String,
    pub spendableBalance: f64,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: f64,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct PolkadotSendPreview {
    pub estimatedNetworkFee: f64,
    pub spendableBalance: f64,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: f64,
}
