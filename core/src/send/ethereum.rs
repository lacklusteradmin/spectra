// EVM fee and nonce input, and the preview of a prepared transfer.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmCustomFeeConfiguration {
    /// Exact decimal gwei.
    pub max_fee_per_gas_gwei: String,
    /// Exact decimal gwei.
    pub max_priority_fee_per_gas_gwei: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum EvmCustomFeeError {
    #[error("Enter a valid Max Fee in gwei.")]
    InvalidMaxFee,
    #[error("Enter a valid Priority Fee in gwei.")]
    InvalidPriorityFee,
    #[error("Max Fee must be greater than or equal to Priority Fee.")]
    MaxBelowPriority,
}

impl EvmCustomFeeConfiguration {
    /// The send path uses whole wei. A fee of zero, one finer than a wei, or
    /// one past the u64 the signer carries is refused, never rounded.
    pub(crate) fn to_wei(&self) -> Result<(u128, u128), EvmCustomFeeError> {
        fn wei(gwei: &str) -> Option<u128> {
            crate::amount::decimal::to_units(gwei, 9)
                .filter(|&wei| wei > 0 && wei <= u128::from(u64::MAX))
        }
        let max = wei(&self.max_fee_per_gas_gwei).ok_or(EvmCustomFeeError::InvalidMaxFee)?;
        let priority = wei(&self.max_priority_fee_per_gas_gwei)
            .ok_or(EvmCustomFeeError::InvalidPriorityFee)?;
        if max < priority {
            return Err(EvmCustomFeeError::MaxBelowPriority);
        }
        Ok((max, priority))
    }
}

/// Parse once in core; front ends render errors or use the returned fees.
#[uniffi::export]
pub fn parse_evm_custom_fees(
    max_fee_gwei_raw: String,
    priority_fee_gwei_raw: String,
) -> Result<EvmCustomFeeConfiguration, EvmCustomFeeError> {
    let fees = EvmCustomFeeConfiguration {
        max_fee_per_gas_gwei: crate::amount::decimal::canonical(&max_fee_gwei_raw)
            .ok_or(EvmCustomFeeError::InvalidMaxFee)?,
        max_priority_fee_per_gas_gwei: crate::amount::decimal::canonical(&priority_fee_gwei_raw)
            .ok_or(EvmCustomFeeError::InvalidPriorityFee)?,
    };
    fees.to_wei()?;
    Ok(fees)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum EvmNonceError {
    #[error("Enter a nonce value for manual nonce mode.")]
    Empty,
    #[error("Nonce must be a non-negative integer.")]
    InvalidInteger,
    #[error("Nonce value is too large.")]
    TooLarge,
}

/// Decimal nonce within the signed 64-bit range used by the preview and FFI.
#[uniffi::export]
pub fn parse_evm_nonce(raw: String) -> Result<i64, EvmNonceError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(EvmNonceError::Empty);
    }
    if !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(EvmNonceError::InvalidInteger);
    }
    raw.parse().map_err(|_| EvmNonceError::TooLarge)
}

/// Typed EVM overrides crossing the FFI from Swift. `resolve` validates them
/// and produces the overrides the signer consumes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmSendOverridesInput {
    pub nonce: Option<i64>,
    pub custom_fees: Option<EvmCustomFeeConfiguration>,
    /// Pin the gas limit. Defaults: 21_000 for plain ETH, node-estimated for
    /// ERC-20 / contract calls. Must be set for arbitrary calldata sends.
    pub gas_limit: Option<i64>,
    /// Hex-encoded calldata (with or without 0x prefix). For native ETH sends
    /// this appends arbitrary data (e.g. a memo). For ERC-20 sends, this
    /// overrides the auto-encoded `transfer(to, amount)` calldata entirely,
    /// enabling approvals, swaps, multicall, or any ABI-encoded function call.
    pub calldata_hex: Option<String>,
    /// EIP-2930 access list as a flat JSON string (array of
    /// `{address, storageKeys}` objects). Pre-warms storage slots to reduce
    /// gas cost for contracts with known read patterns. Non-empty lists require
    /// an explicit gas limit.
    pub access_list_json: Option<String>,
}

/// The quote for a prepared EVM transfer. Its fee is the most the signed
/// transaction can spend on gas; a native send's maximum is the balance that
/// leaves, and a token's is its whole balance, since gas is paid in the coin.
///
/// `token_balance` is the token's balance when the transfer moves one;
/// `custom_fees` says the fees are the user's rather than the network's.
pub(crate) fn evm_send_preview(
    chain: crate::registry::Chain,
    prepared: &crate::send::evm::PreparedEvmTransaction,
    native_balance_wei: u128,
    token_balance: Option<String>,
    custom_fees: bool,
) -> Result<crate::send::preview_types::SendPreview, crate::send::error::SendError> {
    let nonce = i64::try_from(prepared.nonce)
        .map_err(|_| crate::send::error::SendError::invalid("EVM nonce exceeds supported range"))?;
    let gas_limit = i64::try_from(prepared.gas_limit)
        .ok()
        .filter(|gas| *gas > 0)
        .ok_or_else(|| crate::send::error::SendError::invalid("EVM gas limit must be positive"))?;
    let fee_wei = prepared.maximum_fee_wei()?;
    let gwei = |wei: u128| crate::amount::decimal::from_units(wei, 9);
    let (max_fee, priority_fee) = (
        gwei(prepared.max_fee_per_gas),
        gwei(prepared.max_priority_fee_per_gas),
    );
    let native =
        |wei: u128| crate::amount::decimal::from_units(wei, u32::from(chain.native_decimals()));
    let spendable =
        token_balance.unwrap_or_else(|| native(native_balance_wei.saturating_sub(fee_wei)));
    Ok(crate::send::preview_types::SendPreview {
        estimated_network_fee: native(fee_wei),
        max_sendable: spendable.clone(),
        spendable_balance: spendable,
        fee_rate_description: Some(format!(
            "Max {max_fee} gwei / Priority {priority_fee} gwei{}",
            if custom_fees { " (custom)" } else { "" }
        )),
        estimated_transaction_bytes: None,
        terms: crate::send::preview_types::SendFeeTerms::Evm {
            terms: crate::send::preview_types::EvmFeeTerms {
                nonce,
                gas_limit,
                max_fee_per_gas_gwei: max_fee,
                max_priority_fee_per_gas_gwei: priority_fee,
            },
        },
    })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvmSendDetails {
    pub txid: String,
    pub raw_tx_hex: String,
    pub nonce: i64,
    pub gas_limit: i64,
}

#[cfg(test)]
mod custom_fee_tests {
    use super::*;

    #[test]
    fn parsed_fees_convert_to_the_expected_wei() {
        let fees = parse_evm_custom_fees(" 30.25 ".into(), "0.000000001".into()).unwrap();
        assert_eq!(fees.to_wei().unwrap(), (30_250_000_000, 1));
        let equal = parse_evm_custom_fees("2".into(), "2".into()).unwrap();
        assert_eq!(equal.to_wei().unwrap(), (2_000_000_000, 2_000_000_000));
    }

    #[test]
    fn nonfinite_underflow_and_overflow_fees_are_refused() {
        for raw in [
            "",
            "nonsense",
            "NaN",
            "inf",
            "-inf",
            "0",
            "-1",
            "1e-10",
            "1e100",
            "18446744074",
        ] {
            assert_eq!(
                parse_evm_custom_fees(raw.into(), "1".into()).unwrap_err(),
                EvmCustomFeeError::InvalidMaxFee,
                "max: {raw}"
            );
            assert_eq!(
                parse_evm_custom_fees("30".into(), raw.into()).unwrap_err(),
                EvmCustomFeeError::InvalidPriorityFee,
                "priority: {raw}"
            );
        }
        assert_eq!(
            parse_evm_custom_fees("1".into(), "2".into()).unwrap_err(),
            EvmCustomFeeError::MaxBelowPriority
        );
    }
}

#[cfg(test)]
mod nonce_tests {
    use super::*;

    #[test]
    fn parses_decimal_nonce_across_the_ffi_range() {
        for (raw, expected) in [
            (" 00012 ", 12),
            ("0", 0),
            ("2147483648", 2147483648),
            ("9223372036854775807", i64::MAX),
        ] {
            assert_eq!(parse_evm_nonce(raw.into()), Ok(expected));
        }
    }

    #[test]
    fn refuses_malformed_or_overflowing_manual_nonce() {
        assert_eq!(parse_evm_nonce(" ".into()), Err(EvmNonceError::Empty));
        for raw in ["-1", "+1", "1.0", "1e2", "0x10", "1 2", "１２"] {
            assert_eq!(
                parse_evm_nonce(raw.into()),
                Err(EvmNonceError::InvalidInteger)
            );
        }
        for raw in ["9223372036854775808", "18446744073709551616"] {
            assert_eq!(parse_evm_nonce(raw.into()), Err(EvmNonceError::TooLarge));
        }
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    use crate::send::evm::PreparedEvmTransaction;
    use crate::send::preview_types::{EvmFeeTerms, SendFeeTerms};

    fn prepared(gas_limit: u64, max_fee: u128, priority: u128) -> PreparedEvmTransaction {
        PreparedEvmTransaction {
            chain_id: 1,
            nonce: 7,
            max_fee_per_gas: max_fee,
            max_priority_fee_per_gas: priority,
            gas_limit,
            to: "0x1111111111111111111111111111111111111111".into(),
            value_wei: 0,
            data: Vec::new(),
            access_list: Vec::new(),
            additional_fee_wei: 0,
        }
    }

    #[test]
    fn the_fee_is_the_signed_gas_budget_with_its_terms() {
        let preview = evm_send_preview(
            crate::registry::Chain::Ethereum,
            &prepared(21_000, 20_000_000_000, 1_500_000_000),
            0,
            None,
            false,
        )
        .unwrap();
        assert_eq!(preview.estimated_network_fee, "0.00042");
        assert_eq!(
            preview.fee_rate_description.as_deref(),
            Some("Max 20 gwei / Priority 1.5 gwei")
        );
        assert_eq!(
            preview.terms,
            SendFeeTerms::Evm {
                terms: EvmFeeTerms {
                    nonce: 7,
                    gas_limit: 21_000,
                    max_fee_per_gas_gwei: "20".into(),
                    max_priority_fee_per_gas_gwei: "1.5".into(),
                }
            }
        );
        let custom = evm_send_preview(
            crate::registry::Chain::Ethereum,
            &prepared(21_000, 50_000_000_000, 3_000_000_000),
            0,
            None,
            true,
        )
        .unwrap();
        // 21000 × 50 gwei, and the description says whose fees they are.
        assert_eq!(custom.estimated_network_fee, "0.00105");
        assert!(custom.fee_rate_description.unwrap().ends_with("(custom)"));
    }

    #[test]
    fn an_operator_fee_is_part_of_the_budget() {
        let mut transfer = prepared(21_000, 1_000_000_000, 0);
        transfer.additional_fee_wei = 5;
        let preview =
            evm_send_preview(crate::registry::Chain::Ethereum, &transfer, 0, None, false).unwrap();
        assert_eq!(preview.estimated_network_fee, "0.000021000000000005");
    }

    /// A coin pays its own fee; a token's gas comes out of the coin.
    #[test]
    fn the_fee_reduces_a_native_maximum_but_not_a_token_balance() {
        let transfer = prepared(21_000, 20_000_000_000, 1_000_000_000);
        // 1.00021 ETH less 21000 × 20 gwei (0.00042), exactly.
        let native = evm_send_preview(
            crate::registry::Chain::Ethereum,
            &transfer,
            1_000_210_000_000_000_000,
            None,
            false,
        )
        .unwrap();
        assert_eq!(native.max_sendable, "0.99979");
        assert_eq!(native.spendable_balance, "0.99979");
        let token = evm_send_preview(
            crate::registry::Chain::Ethereum,
            &transfer,
            1_000_210_000_000_000_000,
            Some("1".into()),
            false,
        )
        .unwrap();
        assert_eq!(token.max_sendable, "1");
        // Less than the fee is no maximum, not a negative one.
        let short = evm_send_preview(
            crate::registry::Chain::Ethereum,
            &transfer,
            1_000_000_000_000,
            None,
            false,
        )
        .unwrap();
        assert_eq!(short.max_sendable, "0");
    }

    #[test]
    fn a_zero_gas_limit_or_an_unrepresentable_nonce_refuses_the_preview() {
        assert!(
            evm_send_preview(
                crate::registry::Chain::Ethereum,
                &prepared(0, 1, 0),
                0,
                None,
                false
            )
            .is_err()
        );
        let mut transfer = prepared(21_000, 1, 0);
        transfer.nonce = u64::MAX;
        assert!(
            evm_send_preview(crate::registry::Chain::Ethereum, &transfer, 0, None, false).is_err()
        );
        transfer.nonce = i64::MAX as u64;
        assert!(
            evm_send_preview(crate::registry::Chain::Ethereum, &transfer, 0, None, false).is_ok()
        );
    }
}
