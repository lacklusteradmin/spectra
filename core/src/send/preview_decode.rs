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

fn obj_str(o: &serde_json::Map<String, serde_json::Value>, k: &str) -> Option<String> {
    o.get(k).and_then(|v| v.as_str().map(|s| s.to_string()))
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
        maxSendable: d.max_sendable,
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
        maxSendable: obj_decimal(o, "max_sendable").unwrap_or_else(|| spendable.clone()),
        spendableBalance: spendable,
        feeRateDescription: obj_str(o, "fee_rate_description"),
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
    /// Substrate's runtime quote and spendable balance use the same record.
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
                maxSendable: max,
            },
        },
        SimpleChain::Xrp => SimpleChainPreview::Xrp {
            preview: XrpSendPreview {
                estimatedNetworkFee: fee,
                feeDrops: raw.parse().unwrap_or(12),
                spendableBalance: bal,
                feeRateDescription: Some(desc),
                maxSendable: max,
            },
        },
        SimpleChain::Stellar => SimpleChainPreview::Stellar {
            preview: StellarSendPreview {
                estimatedNetworkFee: fee,
                feeStroops: raw.parse().unwrap_or(100),
                spendableBalance: bal,
                feeRateDescription: Some(desc),
                maxSendable: max,
            },
        },
        SimpleChain::Monero => SimpleChainPreview::Monero {
            preview: MoneroSendPreview {
                estimatedNetworkFee: fee,
                priorityLabel: "normal".into(),
                spendableBalance: bal,
                feeRateDescription: Some(desc),
                maxSendable: max,
            },
        },
        SimpleChain::Cardano => SimpleChainPreview::Cardano {
            preview: CardanoSendPreview {
                estimatedNetworkFee: fee,
                spendableBalance: bal,
                feeRateDescription: Some(desc),
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
                maxSendable: max,
            },
        },
        SimpleChain::Aptos => {
            let gas = o
                .get("gas_unit_price_octas")?
                .as_u64()
                .filter(|price| *price > 0)?;
            let max_gas = o
                .get("max_gas_amount")?
                .as_u64()
                .filter(|amount| *amount > 0)?;
            SimpleChainPreview::Aptos {
                preview: AptosSendPreview {
                    estimatedNetworkFee: fee,
                    maxGasAmount: max_gas,
                    gasUnitPriceOctas: gas,
                    spendableBalance: bal,
                    feeRateDescription: Some(format!("{} octas/unit", gas)),
                    maxSendable: max,
                },
            }
        }
        SimpleChain::Ton => SimpleChainPreview::Ton {
            preview: TonSendPreview {
                estimatedNetworkFee: fee,
                spendableBalance: bal,
                feeRateDescription: Some(desc),
                maxSendable: max,
            },
        },
        SimpleChain::Icp => SimpleChainPreview::Icp {
            preview: IcpSendPreview {
                estimatedNetworkFee: fee,
                feeE8s: raw.parse().unwrap_or(10_000),
                spendableBalance: bal,
                feeRateDescription: Some(desc),
                maxSendable: max,
            },
        },
        SimpleChain::Near => SimpleChainPreview::Near {
            preview: NearSendPreview {
                estimatedNetworkFee: fee,
                feeBudgetYoctoNear: raw,
                spendableBalance: bal,
                feeRateDescription: Some(desc),
                maxSendable: max,
            },
        },
        SimpleChain::Bittensor => SimpleChainPreview::Bittensor {
            preview: PolkadotSendPreview {
                estimatedNetworkFee: fee,
                spendableBalance: bal,
                feeRateDescription: Some(desc),
                estimatedTransactionBytes: o
                    .get("estimated_transaction_bytes")
                    .and_then(serde_json::Value::as_i64),
                maxSendable: max,
            },
        },
        SimpleChain::Polkadot => SimpleChainPreview::Polkadot {
            preview: PolkadotSendPreview {
                estimatedNetworkFee: fee,
                spendableBalance: bal,
                feeRateDescription: Some(desc),
                estimatedTransactionBytes: o
                    .get("estimated_transaction_bytes")
                    .and_then(serde_json::Value::as_i64),
                maxSendable: max,
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn aptos_requires_explicit_gas_parameters_instead_of_guessing_from_the_fee() {
        let mut value = serde_json::json!({
            "fee_display":"0.01", "fee_raw":"1000000", "balance_display":"2",
            "gas_unit_price_octas":100, "max_gas_amount":10000
        });
        let SimpleChainPreview::Aptos { preview } =
            build_simple_chain_preview(value.to_string(), SimpleChain::Aptos).unwrap()
        else {
            panic!("Aptos shape");
        };
        assert_eq!(preview.gasUnitPriceOctas, 100);
        assert_eq!(preview.estimatedNetworkFee, "0.01");
        assert_eq!(preview.maxSendable, "1.99");
        value["gas_unit_price_octas"] = serde_json::json!(0);
        assert!(build_simple_chain_preview(value.to_string(), SimpleChain::Aptos).is_none());
        value["gas_unit_price_octas"] = serde_json::json!(100);
        value.as_object_mut().unwrap().remove("max_gas_amount");
        assert!(build_simple_chain_preview(value.to_string(), SimpleChain::Aptos).is_none());
        value["max_gas_amount"] = serde_json::json!(10000);
        value
            .as_object_mut()
            .unwrap()
            .remove("gas_unit_price_octas");
        assert!(build_simple_chain_preview(value.to_string(), SimpleChain::Aptos).is_none());
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
