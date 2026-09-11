// EVM input validation, transaction assembly and preview decoding.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmCustomFeeConfiguration {
    pub max_fee_per_gas_gwei: f64,
    pub max_priority_fee_per_gas_gwei: f64,
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
    /// The send path uses whole wei. Reject values that would round to zero or
    /// saturate its u64 conversion; checking only `> 0` also accepts infinity.
    /// Reject sub-wei inputs rather than quietly rounding them up.
    pub(crate) fn to_wei(&self) -> Result<(u128, u128), EvmCustomFeeError> {
        fn wei(gwei: f64) -> Option<u128> {
            let scaled = (gwei * 1e9).round();
            (gwei.is_finite() && gwei >= 1e-9 && scaled < u64::MAX as f64)
                .then_some(scaled as u64 as u128)
        }
        let max = wei(self.max_fee_per_gas_gwei).ok_or(EvmCustomFeeError::InvalidMaxFee)?;
        let priority =
            wei(self.max_priority_fee_per_gas_gwei).ok_or(EvmCustomFeeError::InvalidPriorityFee)?;
        if self.max_fee_per_gas_gwei < self.max_priority_fee_per_gas_gwei {
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
        max_fee_per_gas_gwei: max_fee_gwei_raw
            .trim()
            .parse()
            .map_err(|_| EvmCustomFeeError::InvalidMaxFee)?,
        max_priority_fee_per_gas_gwei: priority_fee_gwei_raw
            .trim()
            .parse()
            .map_err(|_| EvmCustomFeeError::InvalidPriorityFee)?,
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
    /// Sign the transaction without broadcasting. The signed raw transaction
    /// hex is returned in `SendExecutionResult.evm.raw_tx_hex`; `txid` is
    /// left empty. Useful for offline signing or pre-flight inspection.
    pub sign_only: Option<bool>,
    /// EIP-2930 access list as a flat JSON string (array of
    /// `{address, storageKeys}` objects). Pre-warms storage slots to reduce
    /// gas cost for contracts with known read patterns. Non-empty lists require
    /// an explicit gas limit.
    pub access_list_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmSupportedToken {
    pub symbol: String,
    pub contract_address: String,
    pub decimals: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmSendAssemblyInput {
    pub chain_name: String,
    pub symbol: String,
    pub from_address: String,
    // Caller passes the already-resolved destination (ENS resolved in Swift).
    pub resolved_destination: String,
    /// The amount as the user typed it, exactly. This was an `f64`, so an
    /// amount of `1.1` assembled 1100000000000000089 wei while `execute_send`
    /// — which has always taken the decimal string — signed
    /// 1100000000000000000. The preview priced one transaction and the send
    /// made another. A decimal string is also the only form that can carry an
    /// 18-decimal amount at all: an `f64` runs out of significant digits six
    /// orders of magnitude above a wei.
    pub amount: String,
    // If set, this is an ERC-20 transfer (symbol is the token symbol).
    pub token: Option<EvmSupportedToken>,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmSendAssembly {
    pub value_wei: String,
    pub to_address: String,
    pub data_hex: String,
    pub is_native: bool,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum EvmSendError {
    #[error("Invalid destination address")]
    InvalidDestination,
    #[error("Invalid from address")]
    InvalidFromAddress,
    #[error("Unsupported chain: {0}")]
    UnsupportedChain(String),
    #[error("Unsupported asset for chain")]
    UnsupportedAsset,
    #[error("Invalid amount")]
    InvalidAmount,
}

fn normalize_evm_address(address: &str) -> String {
    address.trim().to_lowercase()
}

fn is_valid_evm_address(address: &str) -> bool {
    let a = normalize_evm_address(address);
    a.len() == 42 && a.starts_with("0x") && a[2..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Whether this asset is the one the chain pays fees in, and so moves as a
/// plain value transfer rather than an ERC-20 call.
pub fn is_native_evm_asset(chain_name: &str, symbol: &str) -> bool {
    crate::registry::Chain::from_display_name(chain_name)
        .is_some_and(|chain| chain.is_evm() && chain.coin_symbol() == symbol)
}

/// Whether an EVM send can be assembled for this chain at all.
///
/// This named seven chains. The registry knows twenty-three EVM mainnets, and
/// the sixteen outside the list got `UnsupportedChain` here, no fee preview in
/// the send sheet, and a blocked send behind "Unable to estimate network fee".
pub fn is_supported_evm_chain(chain_name: &str) -> bool {
    crate::registry::Chain::from_display_name(chain_name)
        .is_some_and(crate::registry::Chain::is_evm)
}

/// Shift a typed decimal amount into the asset's smallest unit.
///
/// This is [`crate::send::amount_input::parse_raw_amount`], which is the same
/// conversion `execute_send` performs on the amount it signs. The function that
/// stood here took an `f64` and claimed in its own doc comment to "avoid float
/// rounding by doing string arithmetic" — but the rounding had already happened
/// in the caller's `f64`, and `format!("{:.18}", …)` then wrote it out in full:
/// `1.1` assembled as `1100000000000000089` wei, `0.1` as `100000000000000006`.
/// Two conversions of one thing, one exact and one not, and the inexact one was
/// what the preview priced and what `spectra send assemble` printed as the
/// transaction a send would sign.
fn amount_to_smallest_unit(amount: &str, decimals: u32) -> Result<u128, EvmSendError> {
    crate::send::amount_input::parse_raw_amount(amount, decimals)
        .map_err(|_| EvmSendError::InvalidAmount)
}

fn encode_erc20_transfer_data(
    destination: &str,
    amount_smallest: u128,
) -> Result<String, EvmSendError> {
    let dst = normalize_evm_address(destination);
    if !is_valid_evm_address(&dst) {
        return Err(EvmSendError::InvalidDestination);
    }
    let addr_body = &dst[2..];
    let addr_padded = format!("{:0>64}", addr_body);
    // amount as hex, zero-padded to 32 bytes. A `uint256` is wider than a
    // `u128`, but `parse_raw_amount` caps there and no real token holding
    // reaches it, so the padding always has room.
    let amount_padded = format!("{:0>64x}", amount_smallest);
    // The selector is the fetch layer's, so the preview that reads this
    // calldata back and the code that writes it cannot drift apart.
    Ok(format!(
        "0x{}{}{}",
        crate::fetch::chains::evm::erc20_transfer_selector_hex(),
        addr_padded,
        amount_padded
    ))
}

#[uniffi::export]
pub fn prepare_evm_send_assembly(
    input: EvmSendAssemblyInput,
) -> Result<EvmSendAssembly, EvmSendError> {
    if !is_supported_evm_chain(&input.chain_name) {
        return Err(EvmSendError::UnsupportedChain(input.chain_name));
    }
    if !is_valid_evm_address(&input.from_address) {
        return Err(EvmSendError::InvalidFromAddress);
    }
    if !is_valid_evm_address(&input.resolved_destination) {
        return Err(EvmSendError::InvalidDestination);
    }
    let destination = normalize_evm_address(&input.resolved_destination);

    if is_native_evm_asset(&input.chain_name, &input.symbol) {
        // Every EVM chain in the catalog is 18, but read it rather than
        // restate it — a chain that is not would be silently off by orders of
        // magnitude on the funds path.
        let decimals = crate::registry::Chain::from_display_name(&input.chain_name)
            .map(|chain| u32::from(chain.native_decimals()))
            .unwrap_or(18);
        let wei = amount_to_smallest_unit(&input.amount, decimals)?;
        return Ok(EvmSendAssembly {
            value_wei: wei.to_string(),
            to_address: destination,
            data_hex: "0x".to_string(),
            is_native: true,
        });
    }

    let Some(token) = input.token else {
        return Err(EvmSendError::UnsupportedAsset);
    };
    let smallest = amount_to_smallest_unit(&input.amount, token.decimals)?;
    let data_hex = encode_erc20_transfer_data(&destination, smallest)?;
    let contract = normalize_evm_address(&token.contract_address);
    if !is_valid_evm_address(&contract) {
        return Err(EvmSendError::UnsupportedAsset);
    }
    Ok(EvmSendAssembly {
        value_wei: "0".to_string(),
        to_address: contract,
        data_hex,
        is_native: false,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvmPreviewDecodeInput {
    pub raw_json: String,
    pub explicit_nonce: Option<i64>,
    pub custom_fees: Option<EvmCustomFeeConfiguration>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvmPreviewDecoded {
    pub nonce: i64,
    pub gas_limit: i64,
    pub max_fee_per_gas_gwei: f64,
    pub max_priority_fee_per_gas_gwei: f64,
    pub estimated_network_fee_eth: f64,
    pub spendable_balance: Option<f64>,
    pub fee_rate_description: Option<String>,
    pub max_sendable: Option<f64>,
}

pub fn decode_evm_send_preview(input: EvmPreviewDecodeInput) -> Option<EvmPreviewDecoded> {
    let value: serde_json::Value = serde_json::from_str(&input.raw_json).ok()?;
    let obj = value.as_object()?;

    let rpc_nonce = obj.get("nonce").and_then(|v| v.as_i64()).unwrap_or(0);
    let nonce = input.explicit_nonce.unwrap_or(rpc_nonce);
    if nonce < 0 {
        return None;
    }
    let gas_limit = obj
        .get("gas_limit")
        .and_then(|v| v.as_i64())
        .unwrap_or(21_000);
    let live_fee_gwei = obj
        .get("max_fee_per_gas_gwei")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let live_prio_gwei = obj
        .get("max_priority_fee_per_gas_gwei")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let (max_fee_gwei, prio_gwei, fee_eth, fee_desc) = match input.custom_fees {
        Some(cf) => {
            cf.to_wei().ok()?;
            let fee_wei = (gas_limit as f64) * cf.max_fee_per_gas_gwei * 1_000_000_000.0;
            let fee_eth = fee_wei / 1_000_000_000_000_000_000.0;
            let desc = format!(
                "Max {:.2} gwei / Priority {:.2} gwei (custom)",
                cf.max_fee_per_gas_gwei, cf.max_priority_fee_per_gas_gwei
            );
            (
                cf.max_fee_per_gas_gwei,
                cf.max_priority_fee_per_gas_gwei,
                fee_eth,
                Some(desc),
            )
        }
        None => {
            let fee_eth = obj
                .get("estimated_fee_eth")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let desc = obj
                .get("fee_rate_description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            (live_fee_gwei, live_prio_gwei, fee_eth, desc)
        }
    };
    let spendable = obj.get("spendable_balance").and_then(|v| v.as_f64());
    Some(EvmPreviewDecoded {
        nonce,
        gas_limit,
        max_fee_per_gas_gwei: max_fee_gwei,
        max_priority_fee_per_gas_gwei: prio_gwei,
        estimated_network_fee_eth: fee_eth,
        spendable_balance: spendable,
        fee_rate_description: fee_desc,
        max_sendable: spendable,
    })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmSendResultDecoded {
    pub txid: String,
    pub raw_tx_hex: String,
    pub nonce: i64,
    pub gas_limit: i64,
}

/// Internal helper: parse the broadcast result JSON into the typed EVM record.
/// Used by `execute_send` to populate `SendExecutionResult.evm` so Swift
/// doesn't have to re-parse the JSON.
pub(crate) fn decode_evm_send_result_internal(
    json: &str,
    fallback_nonce: i64,
) -> EvmSendResultDecoded {
    let v: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => {
            return EvmSendResultDecoded {
                nonce: fallback_nonce,
                ..Default::default()
            };
        }
    };
    let obj = v.as_object();
    let get_str = |k: &str| -> String {
        obj.and_then(|o| o.get(k))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let nonce = obj
        .and_then(|o| o.get("nonce"))
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
        })
        .unwrap_or(fallback_nonce);
    let gas_limit = obj
        .and_then(|o| o.get("gas_limit"))
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
        })
        .unwrap_or(0);
    EvmSendResultDecoded {
        txid: get_str("txid"),
        raw_tx_hex: get_str("raw_tx_hex"),
        nonce,
        gas_limit,
    }
}

/// The assembler and `execute_send` shift the same typed decimal into the same
/// integer. They used not to: this one took an `f64`.
#[cfg(test)]
mod one_amount_one_conversion {
    use super::*;

    const FROM: &str = "0x1111111111111111111111111111111111111111";
    const TO: &str = "0x2222222222222222222222222222222222222222";

    fn native_wei(amount: &str) -> Result<String, EvmSendError> {
        prepare_evm_send_assembly(EvmSendAssemblyInput {
            chain_name: "Ethereum".into(),
            symbol: "ETH".into(),
            from_address: FROM.into(),
            resolved_destination: TO.into(),
            amount: amount.into(),
            token: None,
        })
        .map(|assembly| assembly.value_wei)
    }

    /// The amounts an `f64` gets wrong. Every one of these came out of
    /// `format!("{:.18}", amount)` carrying the double's own error: `1.1`
    /// assembled 89 wei above the amount typed, and the send then signed the
    /// exact one — the preview priced a transaction that was never made.
    #[test]
    fn a_typed_decimal_becomes_exactly_its_own_integer() {
        for (typed, wei) in [
            ("1.1", "1100000000000000000"),
            ("0.1", "100000000000000000"),
            ("0.07", "70000000000000000"),
            ("1234.5678", "1234567800000000000000"),
            ("12345678.9", "12345678900000000000000000"),
            ("1.5", "1500000000000000000"),
            ("0", "0"),
            (" 2.25 ", "2250000000000000000"),
            (".25", "250000000000000000"),
        ] {
            assert_eq!(native_wei(typed).unwrap(), wei, "{typed}");
        }
    }

    /// Eighteen significant decimals do not fit in an `f64` at all — it runs
    /// out around the sixteenth — so this amount was unrepresentable before,
    /// not merely rounded.
    #[test]
    fn the_smallest_unit_of_an_eighteen_decimal_asset_survives() {
        assert_eq!(
            native_wei("1.234567890123456789").unwrap(),
            "1234567890123456789"
        );
        assert_eq!(native_wei("0.000000000000000001").unwrap(), "1");
    }

    /// Whatever the signing path refuses, the assembler refuses. Scientific
    /// notation and over-precision both used to assemble: `parse` took `1e3`
    /// as a thousand and `format!` truncated the extra digit, so a preview
    /// succeeded for an amount `execute_send` would then reject.
    #[test]
    fn what_the_send_refuses_the_preview_refuses() {
        for refused in [
            "1e3",
            "1.1234567890123456789", // 19 decimals against 18
            "-1",
            "NaN",
            "inf",
            "",
            "1..0",
            "abc",
        ] {
            assert!(native_wei(refused).is_err(), "{refused}");
        }
    }

    /// A token is shifted by its own contract's decimals, not the chain's.
    #[test]
    fn a_token_amount_uses_the_contract_precision() {
        let assembly = prepare_evm_send_assembly(EvmSendAssemblyInput {
            chain_name: "Ethereum".into(),
            symbol: "USDC".into(),
            from_address: FROM.into(),
            resolved_destination: TO.into(),
            amount: "1234.567891".into(),
            token: Some(EvmSupportedToken {
                symbol: "USDC".into(),
                contract_address: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".into(),
                decimals: 6,
            }),
        })
        .unwrap();
        // 1234.567891 at 6 decimals is 1_234_567_891 = 0x499602d3, right
        // aligned in the call's 32-byte amount word.
        assert!(
            assembly
                .data_hex
                .ends_with(&format!("{:0>64x}", 1_234_567_891u64)),
            "{}",
            assembly.data_hex
        );
        // One decimal past the contract's precision is refused, not truncated.
        assert!(prepare_evm_send_assembly(EvmSendAssemblyInput {
            chain_name: "Ethereum".into(),
            symbol: "USDC".into(),
            from_address: FROM.into(),
            resolved_destination: TO.into(),
            amount: "1.1234567".into(),
            token: Some(EvmSupportedToken {
                symbol: "USDC".into(),
                contract_address: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".into(),
                decimals: 6,
            }),
        })
        .is_err());
    }
}

#[cfg(test)]
mod every_evm_chain_can_assemble {
    use super::*;

    /// Every EVM mainnet the registry knows can assemble a send.
    ///
    /// `is_supported_evm_chain` named seven of twenty-three. On the other
    /// sixteen — Base, Polygon, Linea, Scroll, Blast, Mantle, Sei, Celo,
    /// Cronos, opBNB, zkSync Era, Sonic, Berachain, Unichain, Ink and X Layer
    /// — this returned `UnsupportedChain`, the send sheet showed no fee, and
    /// the send itself stopped at "Unable to estimate network fee".
    #[test]
    fn every_evm_mainnet_assembles_a_native_send() {
        let address = "0x742d35cc6634c0532925a3b844bc454e4438f44e";
        for chain in crate::registry::Chain::all().filter(|c| c.is_evm() && !c.is_testnet()) {
            let assembly = prepare_evm_send_assembly(EvmSendAssemblyInput {
                chain_name: chain.chain_display_name().to_string(),
                symbol: chain.coin_symbol().to_string(),
                from_address: address.to_string(),
                resolved_destination: address.to_string(),
                amount: "1".into(),
                token: None,
            })
            .unwrap_or_else(|e| {
                panic!(
                    "{} cannot assemble a send: {e:?}",
                    chain.chain_display_name()
                )
            });
            assert!(assembly.is_native, "{}", chain.chain_display_name());
            assert_eq!(assembly.data_hex, "0x", "{}", chain.chain_display_name());
        }
    }

    /// A token is never assembled as the gas asset.
    ///
    /// `("Arbitrum", "ARB")` and `("Optimism", "OP")` were listed as native, so
    /// a preview for either built a value transfer of that many ETH and
    /// discarded the contract it had been given — a 21,000-gas estimate for a
    /// transfer that is nearer 65,000, simulated against an ETH balance the
    /// wallet may not have.
    #[test]
    fn a_governance_token_is_not_the_gas_asset() {
        let address = "0x742d35cc6634c0532925a3b844bc454e4438f44e";
        for (chain_name, symbol, contract) in [
            (
                "Arbitrum",
                "ARB",
                "0x912ce59144191c1204e64559fe8253a0e49e6548",
            ),
            (
                "Optimism",
                "OP",
                "0x4200000000000000000000000000000000000042",
            ),
        ] {
            assert!(
                !is_native_evm_asset(chain_name, symbol),
                "{symbol} is not what {chain_name} pays fees in"
            );
            let assembly = prepare_evm_send_assembly(EvmSendAssemblyInput {
                chain_name: chain_name.to_string(),
                symbol: symbol.to_string(),
                from_address: address.to_string(),
                resolved_destination: address.to_string(),
                amount: "100".into(),
                token: Some(EvmSupportedToken {
                    symbol: symbol.to_string(),
                    contract_address: contract.to_string(),
                    decimals: 18,
                }),
            })
            .unwrap();
            assert!(!assembly.is_native, "{symbol}");
            assert_eq!(assembly.value_wei, "0", "{symbol} must move no gas asset");
            assert_eq!(
                assembly.to_address, contract,
                "{symbol} goes to its contract"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_eth_assembly() {
        let a = prepare_evm_send_assembly(EvmSendAssemblyInput {
            chain_name: "Ethereum".into(),
            symbol: "ETH".into(),
            from_address: "0x1111111111111111111111111111111111111111".into(),
            resolved_destination: "0x2222222222222222222222222222222222222222".into(),
            amount: "1.5".into(),
            token: None,
        })
        .unwrap();
        assert!(a.is_native);
        assert_eq!(a.to_address, "0x2222222222222222222222222222222222222222");
        assert_eq!(a.data_hex, "0x");
        // 1.5 ETH = 1_500_000_000_000_000_000 wei
        assert_eq!(a.value_wei, "1500000000000000000");
    }

    #[test]
    fn erc20_assembly_has_transfer_selector() {
        let a = prepare_evm_send_assembly(EvmSendAssemblyInput {
            chain_name: "Ethereum".into(),
            symbol: "USDC".into(),
            from_address: "0x1111111111111111111111111111111111111111".into(),
            resolved_destination: "0x2222222222222222222222222222222222222222".into(),
            amount: "100".into(),
            token: Some(EvmSupportedToken {
                symbol: "USDC".into(),
                contract_address: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".into(),
                decimals: 6,
            }),
        })
        .unwrap();
        assert!(!a.is_native);
        assert_eq!(a.value_wei, "0");
        assert_eq!(a.to_address, "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48");
        assert!(a.data_hex.starts_with("0xa9059cbb"));
        // 100 USDC at 6 decimals = 100_000_000 = 0x5F5E100, padded
        assert!(a
            .data_hex
            .ends_with("0000000000000000000000000000000000000000000000000000000005f5e100"));
    }

    #[test]
    fn invalid_destination_rejected() {
        let err = prepare_evm_send_assembly(EvmSendAssemblyInput {
            chain_name: "Ethereum".into(),
            symbol: "ETH".into(),
            from_address: "0x1111111111111111111111111111111111111111".into(),
            resolved_destination: "not-an-address".into(),
            amount: "1".into(),
            token: None,
        })
        .unwrap_err();
        matches!(err, EvmSendError::InvalidDestination);
    }

    #[test]
    fn preview_decode_with_custom_fees() {
        let json = r#"{"nonce":7,"gas_limit":21000,"max_fee_per_gas_gwei":30.0,"max_priority_fee_per_gas_gwei":2.0,"estimated_fee_eth":0.00063,"fee_rate_description":"live desc","spendable_balance":4.2}"#;
        let decoded = decode_evm_send_preview(EvmPreviewDecodeInput {
            raw_json: json.into(),
            explicit_nonce: Some(12),
            custom_fees: Some(EvmCustomFeeConfiguration {
                max_fee_per_gas_gwei: 50.0,
                max_priority_fee_per_gas_gwei: 3.0,
            }),
        })
        .unwrap();
        assert_eq!(decoded.nonce, 12);
        assert_eq!(decoded.max_fee_per_gas_gwei, 50.0);
        assert!(decoded
            .fee_rate_description
            .as_deref()
            .unwrap_or("")
            .contains("custom"));
        // 21000 * 50 gwei = 0.00105 ETH
        assert!((decoded.estimated_network_fee_eth - 0.00105).abs() < 1e-9);
        assert_eq!(decoded.spendable_balance, Some(4.2));
    }

    #[test]
    fn decode_send_result_pulls_fields() {
        let json = r#"{"txid":"0xabc","raw_tx_hex":"0xf86...","nonce":12,"gas_limit":21000}"#;
        let r = decode_evm_send_result_internal(json, 0);
        assert_eq!(r.txid, "0xabc");
        assert_eq!(r.nonce, 12);
        assert_eq!(r.gas_limit, 21000);
    }

    #[test]
    fn decode_send_result_uses_fallback_on_missing_nonce() {
        let r = decode_evm_send_result_internal(r#"{"txid":"x"}"#, 7);
        assert_eq!(r.nonce, 7);
        assert_eq!(r.gas_limit, 0);
    }

    #[test]
    fn preview_decode_without_overrides_uses_rpc() {
        let json = r#"{"nonce":3,"gas_limit":21000,"max_fee_per_gas_gwei":20.0,"max_priority_fee_per_gas_gwei":1.5,"estimated_fee_eth":0.00042,"fee_rate_description":"rpc desc","spendable_balance":2.0}"#;
        let decoded = decode_evm_send_preview(EvmPreviewDecodeInput {
            raw_json: json.into(),
            explicit_nonce: None,
            custom_fees: None,
        })
        .unwrap();
        assert_eq!(decoded.nonce, 3);
        assert_eq!(decoded.max_fee_per_gas_gwei, 20.0);
        assert_eq!(decoded.fee_rate_description.as_deref(), Some("rpc desc"));
    }
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

    #[test]
    fn preview_refuses_invalid_typed_fees() {
        let preview = decode_evm_send_preview(EvmPreviewDecodeInput {
            raw_json: r#"{"gas_limit":21000}"#.into(),
            explicit_nonce: None,
            custom_fees: Some(EvmCustomFeeConfiguration {
                max_fee_per_gas_gwei: f64::INFINITY,
                max_priority_fee_per_gas_gwei: 1.0,
            }),
        });
        assert!(preview.is_none());
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
    fn preview_refuses_negative_nonce_from_caller_or_rpc() {
        for (raw, explicit) in [(r#"{"nonce":1}"#, Some(-1)), (r#"{"nonce":-1}"#, None)] {
            assert!(decode_evm_send_preview(EvmPreviewDecodeInput {
                raw_json: raw.into(),
                explicit_nonce: explicit,
                custom_fees: None,
            })
            .is_none());
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
