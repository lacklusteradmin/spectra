// Send-preview decoders for the non-EVM chains: a provider's JSON in, a typed
// preview record out.
//
// Four shapes — the shared UTXO one (Bitcoin, BCH, BSV, Litecoin), Bitcoin's
// xpub variant, Dogecoin and Tron — plus the shared simple-fee path, whose
// members are `SimpleChain` below and `Chain::simple_preview_chain` in the
// registry.

use serde::{Deserialize, Serialize};

/// Extract a top-level JSON field as a string. Numeric/bool values are stringified.
/// Missing keys or invalid JSON return "". Callers use this for loose scraping of
/// send-result JSON where the value may be a hash, signature, index, digest, etc.
pub fn extract_json_string_field(json: String, key: String) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) else {
        return String::new();
    };
    let Some(obj) = v.as_object() else {
        return String::new();
    };
    match obj.get(&key) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Null) | None => String::new(),
        Some(other) => other.to_string().trim_matches('"').to_string(),
    }
}

/// A money field of core's own preview JSON: an exact decimal string, or a
/// JSON integer. A JSON float is refused — it is the rounding this path no
/// longer does.
fn obj_decimal(o: &serde_json::Map<String, serde_json::Value>, k: &str) -> Option<String> {
    match o.get(k)? {
        serde_json::Value::String(s) => crate::decimal::canonical(s),
        serde_json::Value::Number(n) if n.is_u64() => Some(n.to_string()),
        _ => None,
    }
}

fn obj_u64(o: &serde_json::Map<String, serde_json::Value>, k: &str) -> Option<u64> {
    o.get(k).and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
    })
}

fn obj_str(o: &serde_json::Map<String, serde_json::Value>, k: &str) -> Option<String> {
    o.get(k).and_then(|v| v.as_str().map(|s| s.to_string()))
}

/// Every UTXO chain this previews counts in eight-decimal units.
const UTXO_DECIMALS: u32 = 8;

fn coins(sat: u64) -> String {
    crate::decimal::from_units(u128::from(sat), UTXO_DECIMALS)
}

/// The fields of core's UTXO capacity quote, in satoshis.
struct UtxoQuote {
    fee_rate_sat_vb: u64,
    fee_sat: u64,
    tx_bytes: i64,
    input_count: i64,
    spendable_sat: u64,
    max_sendable_sat: u64,
}

fn utxo_quote(json: &str) -> Option<UtxoQuote> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let o = v.as_object()?;
    Some(UtxoQuote {
        fee_rate_sat_vb: obj_u64(o, "fee_rate_svb").unwrap_or(1),
        fee_sat: obj_u64(o, "estimated_fee_sat").unwrap_or(0),
        tx_bytes: obj_u64(o, "estimated_tx_bytes").unwrap_or(0) as i64,
        input_count: obj_u64(o, "selected_input_count").unwrap_or(0) as i64,
        spendable_sat: obj_u64(o, "spendable_balance_sat").unwrap_or(0),
        max_sendable_sat: obj_u64(o, "max_sendable_sat").unwrap_or(0),
    })
}

/// Which decode shape a shared-path preview comes back in.
///
/// `Chain::simple_preview_chain` is the one place that answers it; this does
/// not cross the FFI.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SimpleChain {
    Solana,
    Xrp,
    Stellar,
    Monero,
    Cardano,
    Sui,
    Aptos,
    Ton,
    Icp,
    Near,
    Polkadot,
    Bittensor,
}

// Record builders: the final UniFFI preview record, straight from smallest
// units.

pub fn build_evm_send_preview_record(
    input: crate::send::ethereum::EvmPreviewDecodeInput,
) -> Option<crate::send::preview_types::EvmSendPreview> {
    let d = crate::send::ethereum::decode_evm_send_preview(input)?;
    Some(crate::send::preview_types::EvmSendPreview {
        nonce: d.nonce,
        gasLimit: d.gas_limit,
        maxFeePerGasGwei: d.max_fee_per_gas_gwei,
        maxPriorityFeePerGasGwei: d.max_priority_fee_per_gas_gwei,
        estimatedNetworkFee: d.estimated_network_fee_eth,
        spendableBalance: d.spendable_balance,
        feeRateDescription: d.fee_rate_description,
        estimatedTransactionBytes: None,
        selectedInputCount: None,
        usesChangeOutput: None,
        maxSendable: d.max_sendable,
    })
}

pub fn build_utxo_send_preview_record(
    json: String,
) -> Option<crate::send::preview_types::BitcoinSendPreview> {
    let q = utxo_quote(&json)?;
    if q.spendable_sat == 0 {
        return None;
    }
    Some(crate::send::preview_types::BitcoinSendPreview {
        estimatedFeeRateSatVb: q.fee_rate_sat_vb,
        estimatedNetworkFee: coins(q.fee_sat),
        feeRateDescription: Some(format!("{} sat/vB", q.fee_rate_sat_vb)),
        spendableBalance: Some(coins(q.spendable_sat)),
        estimatedTransactionBytes: Some(q.tx_bytes),
        selectedInputCount: Some(q.input_count),
        usesChangeOutput: None,
        maxSendable: Some(coins(q.max_sendable_sat)),
    })
}

/// The Bitcoin HD send preview, from the two numbers it actually needs. The
/// rate is a node's float estimate; it is rounded up to whole sat/vB here,
/// and everything after that is integer satoshis.
pub fn build_bitcoin_hd_send_preview_record(
    confirmed_sats: u64,
    sats_per_vbyte: f64,
) -> Option<crate::send::preview_types::BitcoinSendPreview> {
    let rate = sats_per_vbyte.ceil().max(1.0) as u64;
    let bytes: u64 = 250;
    let fee_sat = rate.checked_mul(bytes)?;
    Some(crate::send::preview_types::BitcoinSendPreview {
        estimatedFeeRateSatVb: rate,
        estimatedNetworkFee: coins(fee_sat),
        feeRateDescription: Some(format!("{rate} sat/vB")),
        spendableBalance: Some(coins(confirmed_sats)),
        estimatedTransactionBytes: Some(bytes as i64),
        selectedInputCount: None,
        usesChangeOutput: None,
        maxSendable: Some(coins(confirmed_sats.saturating_sub(fee_sat))),
    })
}

/// `requested_amount` is the exact amount typed; it decides only whether the
/// send leaves change.
pub fn build_dogecoin_send_preview_record(
    json: String,
    requested_amount: &str,
) -> Option<crate::send::preview_types::DogecoinSendPreview> {
    let q = utxo_quote(&json)?;
    if q.spendable_sat == 0 {
        return None;
    }
    let requested_sat =
        u64::try_from(crate::decimal::to_units(requested_amount, UTXO_DECIMALS)?).ok()?;
    let uses_change = q.spendable_sat > requested_sat.saturating_add(q.fee_sat);
    Some(crate::send::preview_types::DogecoinSendPreview {
        estimatedNetworkFee: coins(q.fee_sat),
        // sat/vB × 1000 is satoshis per kB.
        estimatedFeeRateDogePerKb: coins(q.fee_rate_sat_vb.checked_mul(1000)?),
        estimatedTransactionBytes: q.tx_bytes,
        selectedInputCount: q.input_count,
        usesChangeOutput: uses_change,
        spendableBalance: coins(q.spendable_sat),
        feeRateDescription: Some(format!("{} sat/vB", q.fee_rate_sat_vb)),
        maxSendable: coins(q.max_sendable_sat),
    })
}

pub fn build_tron_send_preview_record(
    json: String,
) -> Option<crate::send::preview_types::TronSendPreview> {
    let v: serde_json::Value = serde_json::from_str(&json).ok()?;
    let o = v.as_object()?;
    let spendable = obj_decimal(o, "spendable_balance")?;
    Some(crate::send::preview_types::TronSendPreview {
        estimatedNetworkFee: obj_decimal(o, "estimated_fee_trx")?,
        feeLimitSun: o.get("fee_limit_sun").and_then(|v| v.as_i64()).unwrap_or(0),
        simulationUsed: false,
        maxSendable: obj_decimal(o, "max_sendable").unwrap_or_else(|| spendable.clone()),
        spendableBalance: spendable,
        feeRateDescription: obj_str(o, "fee_rate_description"),
        estimatedTransactionBytes: None,
        selectedInputCount: None,
        usesChangeOutput: None,
    })
}

// Tagged-union output for unified simple-chain preview builder.
// Swift dispatches on the enum variant to assign the right @Published preview.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum SimpleChainPreview {
    Solana {
        preview: crate::send::preview_types::SolanaSendPreview,
    },
    Xrp {
        preview: crate::send::preview_types::XrpSendPreview,
    },
    Stellar {
        preview: crate::send::preview_types::StellarSendPreview,
    },
    Monero {
        preview: crate::send::preview_types::MoneroSendPreview,
    },
    Cardano {
        preview: crate::send::preview_types::CardanoSendPreview,
    },
    Sui {
        preview: crate::send::preview_types::SuiSendPreview,
    },
    Aptos {
        preview: crate::send::preview_types::AptosSendPreview,
    },
    Ton {
        preview: crate::send::preview_types::TonSendPreview,
    },
    Icp {
        preview: crate::send::preview_types::IcpSendPreview,
    },
    Near {
        preview: crate::send::preview_types::NearSendPreview,
    },
    Polkadot {
        preview: crate::send::preview_types::PolkadotSendPreview,
    },
    /// Substrate, like Polkadot, so the same record shape. Its fee is the
    /// catalog's static 125,000 rao rather than an on-chain quote — the same
    /// arrangement Polkadot's preview has had.
    Bittensor {
        preview: crate::send::preview_types::PolkadotSendPreview,
    },
}

/// `None` when the fee or the balance is missing: a zero standing in for
/// either would offer a wrong maximum.
pub fn build_simple_chain_preview(json: String, chain: SimpleChain) -> Option<SimpleChainPreview> {
    use crate::send::preview_types::*;
    let v: serde_json::Value = serde_json::from_str(&json).ok()?;
    let o = v.as_object()?;
    let fee = obj_decimal(o, "fee_display")?;
    let bal = obj_decimal(o, "balance_display")?;
    let max = match obj_decimal(o, "max_sendable") {
        Some(max) => max,
        None => crate::decimal::sub_or_zero(&bal, &fee)?,
    };
    let desc = obj_str(o, "fee_rate_description").unwrap_or_default();
    let raw = obj_str(o, "fee_raw").unwrap_or_default();
    Some(match chain {
        SimpleChain::Solana => SimpleChainPreview::Solana {
            preview: SolanaSendPreview {
                estimatedNetworkFee: fee,
                spendableBalance: bal,
                feeRateDescription: Some(desc),
                estimatedTransactionBytes: None,
                selectedInputCount: None,
                usesChangeOutput: None,
                maxSendable: max,
            },
        },
        SimpleChain::Xrp => SimpleChainPreview::Xrp {
            preview: XrpSendPreview {
                estimatedNetworkFee: fee,
                feeDrops: raw.parse().unwrap_or(12),
                sequence: 0,
                lastLedgerSequence: 0,
                spendableBalance: bal,
                feeRateDescription: Some(desc),
                estimatedTransactionBytes: None,
                selectedInputCount: None,
                usesChangeOutput: None,
                maxSendable: max,
            },
        },
        SimpleChain::Stellar => SimpleChainPreview::Stellar {
            preview: StellarSendPreview {
                estimatedNetworkFee: fee,
                feeStroops: raw.parse().unwrap_or(100),
                sequence: 0,
                spendableBalance: bal,
                feeRateDescription: Some(desc),
                estimatedTransactionBytes: None,
                selectedInputCount: None,
                usesChangeOutput: None,
                maxSendable: max,
            },
        },
        SimpleChain::Monero => SimpleChainPreview::Monero {
            preview: MoneroSendPreview {
                estimatedNetworkFee: fee,
                priorityLabel: "normal".into(),
                spendableBalance: bal,
                feeRateDescription: Some(desc),
                estimatedTransactionBytes: None,
                selectedInputCount: None,
                usesChangeOutput: None,
                maxSendable: max,
            },
        },
        SimpleChain::Cardano => SimpleChainPreview::Cardano {
            preview: CardanoSendPreview {
                estimatedNetworkFee: fee,
                ttlSlot: 0,
                spendableBalance: bal,
                feeRateDescription: Some(desc),
                estimatedTransactionBytes: None,
                selectedInputCount: None,
                usesChangeOutput: None,
                maxSendable: max,
            },
        },
        SimpleChain::Sui => SimpleChainPreview::Sui {
            preview: SuiSendPreview {
                estimatedNetworkFee: fee,
                gasBudgetMist: raw.parse().unwrap_or(3_000_000),
                referenceGasPrice: 1_000,
                spendableBalance: bal,
                feeRateDescription: Some(desc),
                estimatedTransactionBytes: None,
                selectedInputCount: None,
                usesChangeOutput: None,
                maxSendable: max,
            },
        },
        SimpleChain::Aptos => {
            let gas: u64 = raw.parse().unwrap_or(100);
            SimpleChainPreview::Aptos {
                preview: AptosSendPreview {
                    estimatedNetworkFee: fee,
                    maxGasAmount: 10_000,
                    gasUnitPriceOctas: gas,
                    spendableBalance: bal,
                    feeRateDescription: Some(format!("{} octas/unit", gas)),
                    estimatedTransactionBytes: None,
                    selectedInputCount: None,
                    usesChangeOutput: None,
                    maxSendable: max,
                },
            }
        }
        SimpleChain::Ton => SimpleChainPreview::Ton {
            preview: TonSendPreview {
                estimatedNetworkFee: fee,
                sequenceNumber: 0,
                spendableBalance: bal,
                feeRateDescription: Some(desc),
                estimatedTransactionBytes: None,
                selectedInputCount: None,
                usesChangeOutput: None,
                maxSendable: max,
            },
        },
        SimpleChain::Icp => SimpleChainPreview::Icp {
            preview: IcpSendPreview {
                estimatedNetworkFee: fee,
                feeE8s: raw.parse().unwrap_or(10_000),
                spendableBalance: bal,
                feeRateDescription: Some(desc),
                estimatedTransactionBytes: None,
                selectedInputCount: None,
                usesChangeOutput: None,
                maxSendable: max,
            },
        },
        SimpleChain::Near => SimpleChainPreview::Near {
            preview: NearSendPreview {
                estimatedNetworkFee: fee,
                gasPriceYoctoNear: raw.clone(),
                spendableBalance: bal,
                feeRateDescription: Some(raw),
                estimatedTransactionBytes: None,
                selectedInputCount: None,
                usesChangeOutput: None,
                maxSendable: max,
            },
        },
        SimpleChain::Bittensor => SimpleChainPreview::Bittensor {
            preview: PolkadotSendPreview {
                estimatedNetworkFee: fee,
                spendableBalance: bal,
                feeRateDescription: Some(desc),
                estimatedTransactionBytes: None,
                selectedInputCount: None,
                usesChangeOutput: None,
                maxSendable: max,
            },
        },
        SimpleChain::Polkadot => SimpleChainPreview::Polkadot {
            preview: PolkadotSendPreview {
                estimatedNetworkFee: fee,
                spendableBalance: bal,
                feeRateDescription: Some(desc),
                estimatedTransactionBytes: None,
                selectedInputCount: None,
                usesChangeOutput: None,
                maxSendable: max,
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utxo_amounts_are_exact_coins() {
        let json = r#"{"fee_rate_svb":5,"estimated_fee_sat":1000,"estimated_tx_bytes":200,"selected_input_count":2,"spendable_balance_sat":500000001,"max_sendable_sat":499999001}"#;
        let d = build_utxo_send_preview_record(json.into()).unwrap();
        assert_eq!(d.estimatedFeeRateSatVb, 5);
        assert_eq!(d.feeRateDescription.as_deref(), Some("5 sat/vB"));
        assert_eq!(d.estimatedNetworkFee, "0.00001");
        assert_eq!(d.spendableBalance.as_deref(), Some("5.00000001"));
        assert_eq!(d.maxSendable.as_deref(), Some("4.99999001"));
        assert_eq!(d.selectedInputCount, Some(2));
    }

    #[test]
    fn bitcoin_hd_rounds_the_rate_up_then_counts_satoshis() {
        let d = build_bitcoin_hd_send_preview_record(100_000, 2.3).unwrap();
        assert_eq!(d.estimatedFeeRateSatVb, 3);
        assert_eq!(d.estimatedTransactionBytes, Some(250));
        // 100000 - 3*250
        assert_eq!(d.maxSendable.as_deref(), Some("0.0009925"));
    }

    #[test]
    fn dogecoin_change_and_rate_per_kb() {
        let json = r#"{"fee_rate_svb":1,"estimated_fee_sat":1000,"estimated_tx_bytes":200,"selected_input_count":1,"spendable_balance_sat":1000000000,"max_sendable_sat":999999000}"#;
        let d = build_dogecoin_send_preview_record(json.into(), "1").unwrap();
        assert!(d.usesChangeOutput);
        assert_eq!(d.estimatedFeeRateDogePerKb, "0.00001");
        assert_eq!(d.maxSendable, "9.99999");
        // The whole spendable amount leaves nothing to change.
        assert!(
            !build_dogecoin_send_preview_record(json.into(), "9.99999")
                .unwrap()
                .usesChangeOutput
        );
        // An amount finer than a satoshi is not quoted.
        assert!(build_dogecoin_send_preview_record(json.into(), "0.000000001").is_none());
    }

    #[test]
    fn tron_reads_exact_decimals_and_refuses_floats() {
        let json = r#"{"estimated_fee_trx":"0.27","fee_limit_sun":1000000,"spendable_balance":"42","max_sendable":"41.73","fee_rate_description":"bandwidth ok"}"#;
        let d = build_tron_send_preview_record(json.into()).unwrap();
        assert_eq!(d.feeLimitSun, 1_000_000);
        assert_eq!(d.maxSendable, "41.73");
        assert_eq!(d.feeRateDescription.as_deref(), Some("bandwidth ok"));
        let float = r#"{"estimated_fee_trx":0.27,"spendable_balance":"42"}"#;
        assert!(build_tron_send_preview_record(float.into()).is_none());
    }

    #[test]
    fn simple_preview_subtracts_the_fee_exactly_when_no_maximum_is_given() {
        let json = r#"{"fee_display":"0.000005","fee_raw":"5000","fee_rate_description":"rpc","balance_display":"2.5"}"#;
        let SimpleChainPreview::Solana { preview } =
            build_simple_chain_preview(json.into(), SimpleChain::Solana).unwrap()
        else {
            panic!("solana shape")
        };
        assert_eq!(preview.maxSendable, "2.499995");
        assert_eq!(preview.estimatedNetworkFee, "0.000005");
    }

    #[test]
    fn simple_preview_uses_an_explicit_maximum_and_refuses_missing_fields() {
        let json =
            r#"{"fee_display":"0.1","fee_raw":"100","balance_display":"10","max_sendable":"9.5"}"#;
        let SimpleChainPreview::Stellar { preview } =
            build_simple_chain_preview(json.into(), SimpleChain::Stellar).unwrap()
        else {
            panic!("stellar shape")
        };
        assert_eq!(preview.maxSendable, "9.5");
        assert!(build_simple_chain_preview("{}".into(), SimpleChain::Solana).is_none());
    }

    #[test]
    fn extract_field_handles_strings_numbers_and_missing() {
        let j = r#"{"txid":"0xabc","nonce":7,"flag":true}"#;
        assert_eq!(extract_json_string_field(j.into(), "txid".into()), "0xabc");
        assert_eq!(extract_json_string_field(j.into(), "nonce".into()), "7");
        assert_eq!(extract_json_string_field(j.into(), "flag".into()), "true");
        assert_eq!(extract_json_string_field(j.into(), "missing".into()), "");
        assert_eq!(
            extract_json_string_field("{not json".into(), "x".into()),
            ""
        );
    }
}

/// Add an extra output's bytes to a UTXO preview.
///
/// A destination that costs more than a standard output — Litecoin's MWEB
/// peg-in is the one the registry names — pays for those bytes at the same
/// rate, and both the estimate and what is left sendable move with it. The
/// arithmetic was on the front end's side, beside a registry fact it fetched
/// to do it, and had no test.
pub fn with_extra_output_overhead(
    preview: crate::send::preview_types::BitcoinSendPreview,
    overhead_bytes: u64,
) -> crate::send::preview_types::BitcoinSendPreview {
    if overhead_bytes == 0 {
        return preview;
    }
    let additional_fee = coins(overhead_bytes.saturating_mul(preview.estimatedFeeRateSatVb));
    crate::send::preview_types::BitcoinSendPreview {
        estimatedNetworkFee: crate::decimal::add(&preview.estimatedNetworkFee, &additional_fee)
            .unwrap_or_else(|| preview.estimatedNetworkFee.clone()),
        estimatedTransactionBytes: Some(
            preview.estimatedTransactionBytes.unwrap_or(0) + overhead_bytes as i64,
        ),
        maxSendable: preview
            .maxSendable
            .as_deref()
            .and_then(|max| crate::decimal::sub_or_zero(max, &additional_fee)),
        ..preview
    }
}

#[cfg(test)]
mod extra_output_overhead_tests {
    use super::with_extra_output_overhead;
    use crate::send::preview_types::BitcoinSendPreview;

    fn preview() -> BitcoinSendPreview {
        BitcoinSendPreview {
            estimatedFeeRateSatVb: 10,
            estimatedNetworkFee: "0.00002".into(),
            feeRateDescription: None,
            spendableBalance: Some("1".into()),
            estimatedTransactionBytes: Some(200),
            selectedInputCount: Some(1),
            usesChangeOutput: Some(true),
            maxSendable: Some("0.5".into()),
        }
    }

    /// The extra bytes are paid for at the preview's own rate, and what is
    /// left sendable comes down by the same amount.
    #[test]
    fn an_extra_output_costs_its_bytes_at_the_previewed_rate() {
        // Litecoin's MWEB peg-in: 1017 bytes at 10 sat/vB is 10,170 sats.
        let adjusted = with_extra_output_overhead(preview(), 1017);
        assert_eq!(adjusted.estimatedTransactionBytes, Some(1217));
        assert_eq!(adjusted.estimatedNetworkFee, "0.0001217");
        assert_eq!(adjusted.maxSendable.as_deref(), Some("0.4998983"));
        // Untouched fields stay put.
        assert_eq!(adjusted.spendableBalance.as_deref(), Some("1"));
        assert_eq!(adjusted.selectedInputCount, Some(1));
    }

    /// No overhead is no change at all, not a recomputation that rounds.
    #[test]
    fn no_overhead_leaves_the_preview_alone() {
        assert_eq!(with_extra_output_overhead(preview(), 0), preview());
    }

    /// What is left sendable cannot go negative.
    #[test]
    fn max_sendable_stops_at_zero() {
        let mut small = preview();
        small.maxSendable = Some("0.00001".into());
        assert_eq!(
            with_extra_output_overhead(small, 1017)
                .maxSendable
                .as_deref(),
            Some("0")
        );
    }
}
