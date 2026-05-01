// Per-chain send-preview record types. Formerly also hosted a full Rust-side
// WalletCore state mirror that Swift forwarded @Published mutations into; that
// mirror is dead code now that Swift no longer treats WalletCore as canonical
// state, so only the shared preview structs remain.

use serde::{Deserialize, Serialize};

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, uniffi::Record)]
pub struct EthereumSendPreview {
    pub nonce: i64,
    pub gasLimit: i64,
    pub maxFeePerGasGwei: f64,
    pub maxPriorityFeePerGasGwei: f64,
    pub estimatedNetworkFeeEth: f64,
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
    pub estimatedNetworkFeeBtc: f64,
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
    pub estimatedNetworkFeeDoge: f64,
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
    pub estimatedNetworkFeeTrx: f64,
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
    pub estimatedNetworkFeeSol: f64,
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
    pub estimatedNetworkFeeXrp: f64,
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
    pub estimatedNetworkFeeXlm: f64,
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
    pub estimatedNetworkFeeXmr: f64,
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
    pub estimatedNetworkFeeAda: f64,
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
    pub estimatedNetworkFeeSui: f64,
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
    pub estimatedNetworkFeeApt: f64,
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
    pub estimatedNetworkFeeTon: f64,
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
    pub estimatedNetworkFeeIcp: f64,
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
    pub estimatedNetworkFeeNear: f64,
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
    pub estimatedNetworkFeeDot: f64,
    pub spendableBalance: f64,
    pub feeRateDescription: Option<String>,
    pub estimatedTransactionBytes: Option<i64>,
    pub selectedInputCount: Option<i64>,
    pub usesChangeOutput: Option<bool>,
    pub maxSendable: f64,
}
