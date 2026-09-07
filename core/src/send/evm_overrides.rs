//! Validate front-end EVM overrides before deriving keys or fetching metadata.

use serde::Deserialize;

use super::chains::evm::{AccessListEntry, EvmSendOverrides};
use super::ethereum::EvmSendOverridesInput;
use crate::SpectraBridgeError;

fn invalid(message: impl Into<String>) -> SpectraBridgeError {
    SpectraBridgeError::InvalidInput {
        message: message.into(),
    }
}

fn bytes(raw: &str, field: &str) -> Result<Vec<u8>, SpectraBridgeError> {
    let raw = raw.trim();
    let digits = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .unwrap_or(raw);
    hex::decode(digits).map_err(|_| invalid(format!("{field} must contain whole hex bytes")))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AccessListInput {
    address: String,
    storage_keys: Vec<String>,
}

impl EvmSendOverridesInput {
    /// Used by the service and CLI. This Rust-only method adds no FFI export.
    pub fn resolve(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<EvmSendOverrides, SpectraBridgeError> {
        if !chain.is_evm() {
            return Err(invalid("EVM overrides require an EVM chain"));
        }
        let nonce = self
            .nonce
            .map(|value| u64::try_from(value).map_err(|_| invalid("nonce must be non-negative")))
            .transpose()?;
        let gas_limit = self
            .gas_limit
            .map(|value| {
                u64::try_from(value)
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| invalid("gas limit must be positive"))
            })
            .transpose()?;
        let fees = self
            .custom_fees
            .as_ref()
            .map(|fees| fees.to_wei())
            .transpose()
            .map_err(|error| invalid(error.to_string()))?;
        let calldata = self
            .calldata_hex
            .as_deref()
            .map(|raw| bytes(raw, "calldata"))
            .transpose()?;
        let mut access_list = Vec::new();
        if let Some(raw) = &self.access_list_json {
            let entries: Vec<AccessListInput> = serde_json::from_str(raw)
                .map_err(|error| invalid(format!("invalid access list: {error}")))?;
            for (index, entry) in entries.into_iter().enumerate() {
                let address = entry.address.trim();
                let validation = crate::validation::address::validate_address(
                    crate::validation::address::AddressValidationRequest {
                        kind: chain.address_validation_kind().into(),
                        value: address.into(),
                    },
                );
                if !validation.is_valid {
                    return Err(invalid(format!(
                        "access list entry {index} has an invalid address"
                    )));
                }
                let address: [u8; 20] = bytes(address, "access list address")?
                    .try_into()
                    .map_err(|_| invalid("access list address must be 20 bytes"))?;
                let storage_keys = entry
                    .storage_keys
                    .iter()
                    .map(|key| {
                        bytes(key, "storage key")?
                            .try_into()
                            .map_err(|_| invalid("storage key must be 32 bytes"))
                    })
                    .collect::<Result<Vec<[u8; 32]>, _>>()?;
                access_list.push(AccessListEntry {
                    address,
                    storage_keys,
                });
            }
        }
        // Native sends otherwise default to 21,000; ERC-20 estimation does not
        // account for arbitrary replacement calldata or access-list entries.
        if gas_limit.is_none() && (calldata.is_some() || !access_list.is_empty()) {
            return Err(invalid(
                "calldata and non-empty access lists require an explicit gas limit",
            ));
        }
        Ok(EvmSendOverrides {
            nonce,
            max_fee_per_gas_wei: fees.map(|(max, _)| max),
            max_priority_fee_per_gas_wei: fees.map(|(_, priority)| priority),
            gas_limit,
            calldata,
            access_list,
            sign_only: self.sign_only.unwrap_or(false),
            gas_buffer_pct: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Chain;

    #[test]
    fn resolves_every_supplied_field_without_dropping_bytes() {
        let input = EvmSendOverridesInput {
            nonce: Some(0),
            gas_limit: Some(50_000),
            calldata_hex: Some("0x0102ff".into()),
            access_list_json: Some(format!(
                r#"[{{"address":"0x{}","storageKeys":["0x{}"]}}]"#,
                "11".repeat(20),
                "22".repeat(32)
            )),
            sign_only: Some(true),
            ..Default::default()
        };
        let value = input.resolve(Chain::Ethereum).unwrap();
        assert_eq!(value.nonce, Some(0));
        assert_eq!(value.gas_limit, Some(50_000));
        assert_eq!(value.calldata.unwrap(), [1, 2, 255]);
        assert_eq!(value.access_list[0].address, [0x11; 20]);
        assert_eq!(value.access_list[0].storage_keys, vec![[0x22; 32]]);
        assert!(value.sign_only);
    }

    #[test]
    fn malformed_fields_are_refused_instead_of_defaulted() {
        let malformed = [
            EvmSendOverridesInput {
                nonce: Some(-1),
                ..Default::default()
            },
            EvmSendOverridesInput {
                gas_limit: Some(-1),
                ..Default::default()
            },
            EvmSendOverridesInput {
                gas_limit: Some(0),
                ..Default::default()
            },
            EvmSendOverridesInput {
                calldata_hex: Some("0x0".into()),
                gas_limit: Some(50_000),
                ..Default::default()
            },
            EvmSendOverridesInput {
                calldata_hex: Some("0xzz".into()),
                gas_limit: Some(50_000),
                ..Default::default()
            },
            EvmSendOverridesInput {
                calldata_hex: Some("0x0x00".into()),
                gas_limit: Some(50_000),
                ..Default::default()
            },
        ];
        for input in malformed {
            assert!(matches!(
                input.resolve(Chain::Ethereum),
                Err(SpectraBridgeError::InvalidInput { .. })
            ));
        }
        for raw in [
            "null",
            "{}",
            "[{}]",
            r#"[{"address":"0x11","storageKeys":[]}]"#,
            r#"[{"address":"0x1111111111111111111111111111111111111111","storageKeys":["0x01"]}]"#,
            r#"[{"address":"0x1111111111111111111111111111111111111111","storageKeys":[],"typo":true}]"#,
            r#"[{"address":"0x742d35cC6634C0532925a3b844Bc454e4438f44e","storageKeys":[]}]"#,
        ] {
            let input = EvmSendOverridesInput {
                access_list_json: Some(raw.into()),
                gas_limit: Some(50_000),
                ..Default::default()
            };
            assert!(input.resolve(Chain::Ethereum).is_err(), "{raw}");
        }
    }

    #[test]
    fn defaults_and_explicit_empty_calldata_are_distinct() {
        let defaults = EvmSendOverridesInput::default()
            .resolve(Chain::Ethereum)
            .unwrap();
        assert!(defaults.calldata.is_none());
        assert!(defaults.gas_limit.is_none());
        let input = EvmSendOverridesInput {
            calldata_hex: Some("0x".into()),
            ..Default::default()
        };
        assert!(
            input.resolve(Chain::Ethereum).is_err(),
            "custom data needs explicit gas"
        );
        let empty = EvmSendOverridesInput {
            gas_limit: Some(21_000),
            ..input
        }
        .resolve(Chain::Ethereum)
        .unwrap();
        assert_eq!(empty.calldata, Some(Vec::new()));
        assert!(EvmSendOverridesInput::default()
            .resolve(Chain::Bitcoin)
            .is_err());
        let input = EvmSendOverridesInput {
            access_list_json: Some(
                r#"[{"address":"0x1111111111111111111111111111111111111111","storageKeys":[]}]"#
                    .into(),
            ),
            ..Default::default()
        };
        assert!(
            input.resolve(Chain::Ethereum).is_err(),
            "access list needs explicit gas"
        );
    }
}
