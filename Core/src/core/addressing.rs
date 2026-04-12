use bitcoin::{Address, Network};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AddressValidationRequest {
    pub kind: String,
    pub value: String,
    pub network_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AddressValidationResult {
    pub is_valid: bool,
    pub normalized_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StringValidationRequest {
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StringValidationResult {
    pub is_valid: bool,
    pub normalized_value: Option<String>,
}

pub fn validate_address(request: AddressValidationRequest) -> AddressValidationResult {
    let normalized_input = trim_string(&request.value);
    if normalized_input.is_empty() {
        return invalid_result();
    }

    match request.kind.as_str() {
        "bitcoin" => validate_bitcoin_address(&normalized_input, request.network_mode.as_deref()),
        "bitcoinCash" => validate_bitcoin_cash_address(&normalized_input),
        "bitcoinSV" => validate_bitcoin_sv_address(&normalized_input),
        "litecoin" => validate_litecoin_address(&normalized_input),
        "dogecoin" => validate_dogecoin_address(&normalized_input, request.network_mode.as_deref()),
        "evm" => validate_evm_address(&normalized_input),
        "tron" => validate_tron_address(&normalized_input),
        "solana" => validate_solana_address(&normalized_input),
        "stellar" => validate_stellar_address(&normalized_input),
        "xrp" => validate_xrp_address(&normalized_input),
        "sui" => validate_sui_address(&normalized_input),
        "aptos" => validate_aptos_address(&normalized_input),
        "ton" => validate_ton_address(&normalized_input),
        "internetComputer" => validate_icp_address(&normalized_input),
        "near" => validate_near_address(&normalized_input),
        "polkadot" => validate_polkadot_address(&normalized_input),
        "monero" => validate_monero_address(&normalized_input),
        "cardano" => validate_cardano_address(&normalized_input),
        _ => invalid_result(),
    }
}

pub fn validate_string_identifier(request: StringValidationRequest) -> StringValidationResult {
    let normalized_input = trim_string(&request.value);
    if normalized_input.is_empty() {
        return StringValidationResult {
            is_valid: false,
            normalized_value: None,
        };
    }

    match request.kind.as_str() {
        "aptosTokenType" => validate_aptos_token_type(&normalized_input),
        _ => StringValidationResult {
            is_valid: false,
            normalized_value: None,
        },
    }
}

fn invalid_result() -> AddressValidationResult {
    AddressValidationResult {
        is_valid: false,
        normalized_value: None,
    }
}

fn trim_string(value: &str) -> String {
    value.trim().to_string()
}

fn make_result(normalized_value: String) -> AddressValidationResult {
    AddressValidationResult {
        is_valid: true,
        normalized_value: Some(normalized_value),
    }
}

fn make_string_result(normalized_value: String) -> StringValidationResult {
    StringValidationResult {
        is_valid: true,
        normalized_value: Some(normalized_value),
    }
}

fn is_base58(value: &str) -> bool {
    value.chars().all(|character| {
        "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".contains(character)
    })
}

fn is_lower_hex(value: &str) -> bool {
    value.chars().all(|character| character.is_ascii_hexdigit())
}

fn validate_bitcoin_address(value: &str, network_mode: Option<&str>) -> AddressValidationResult {
    let address = match Address::from_str(value) {
        Ok(address) => address,
        Err(_) => return invalid_result(),
    };

    let is_valid = match network_mode.unwrap_or("mainnet") {
        "mainnet" => address.require_network(Network::Bitcoin).is_ok(),
        "testnet" | "testnet4" | "signet" => {
            address.clone().require_network(Network::Testnet).is_ok()
                || address.clone().require_network(Network::Signet).is_ok()
                || address.require_network(Network::Regtest).is_ok()
        }
        _ => false,
    };

    if !is_valid {
        return invalid_result();
    }

    make_result(value.to_string())
}

fn validate_bitcoin_cash_address(value: &str) -> AddressValidationResult {
    let lowered = value.to_lowercase();
    if let Some(stripped) = lowered.strip_prefix("bitcoincash:") {
        if !stripped.is_empty()
            && stripped
                .chars()
                .all(|character| "023456789acdefghjklmnpqrstuvwxyz".contains(character))
        {
            return make_result(lowered);
        }
        return invalid_result();
    }

    if lowered.starts_with('q')
        || lowered.starts_with('p')
        || value.starts_with('1')
        || value.starts_with('3')
    {
        return make_result(value.to_string());
    }

    invalid_result()
}

fn validate_bitcoin_sv_address(value: &str) -> AddressValidationResult {
    // BSV is legacy-only: base58check P2PKH (version 0x00) or P2SH (0x05),
    // plus the testnet variants 0x6f / 0xc4. SegWit/Taproot are not valid.
    if crate::core::chains::bitcoin_sv::validate_bsv_address(value) {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_litecoin_address(value: &str) -> AddressValidationResult {
    let lowered = value.to_lowercase();
    if lowered.starts_with("ltc1")
        || value.starts_with('L')
        || value.starts_with('M')
        || value.starts_with('3')
    {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_dogecoin_address(value: &str, network_mode: Option<&str>) -> AddressValidationResult {
    if !is_base58(value) {
        return invalid_result();
    }

    let is_valid = match network_mode.unwrap_or("mainnet") {
        "mainnet" => value.starts_with('D') || value.starts_with('A') || value.starts_with('9'),
        "testnet" => value.starts_with('n') || value.starts_with('2'),
        _ => false,
    };

    if !is_valid {
        return invalid_result();
    }

    make_result(value.to_string())
}

fn validate_evm_address(value: &str) -> AddressValidationResult {
    let normalized = value.to_lowercase();
    if normalized.len() != 42 || !normalized.starts_with("0x") {
        return invalid_result();
    }
    if !is_lower_hex(&normalized[2..]) {
        return invalid_result();
    }
    make_result(normalized)
}

fn validate_tron_address(value: &str) -> AddressValidationResult {
    if value.len() == 34 && value.starts_with('T') && is_base58(value) {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_solana_address(value: &str) -> AddressValidationResult {
    if (32..=44).contains(&value.len()) && is_base58(value) {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_stellar_address(value: &str) -> AddressValidationResult {
    if value.len() == 56
        && value.starts_with('G')
        && value
            .chars()
            .all(|character| "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".contains(character))
    {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_xrp_address(value: &str) -> AddressValidationResult {
    if (25..=35).contains(&value.len()) && value.starts_with('r') && is_base58(value) {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_sui_address(value: &str) -> AddressValidationResult {
    let normalized = value.to_lowercase();
    if !normalized.starts_with("0x") {
        return invalid_result();
    }
    let body = &normalized[2..];
    if body.is_empty() || body.len() > 64 || !is_lower_hex(body) {
        return invalid_result();
    }
    make_result(normalized)
}

fn validate_aptos_address(value: &str) -> AddressValidationResult {
    let lowered = value.to_lowercase();
    let body = lowered.strip_prefix("0x").unwrap_or(&lowered);
    if body.is_empty() || body.len() > 64 || !is_lower_hex(body) {
        return invalid_result();
    }
    make_result(format!("0x{body}"))
}

fn validate_ton_address(value: &str) -> AddressValidationResult {
    let normalized = value.to_lowercase();
    if normalized.len() == 66 && normalized.starts_with("0:") && is_lower_hex(&normalized[2..]) {
        return make_result(normalized);
    }

    if value.len() == 48
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
    {
        return make_result(value.to_string());
    }

    invalid_result()
}

fn validate_icp_address(value: &str) -> AddressValidationResult {
    let normalized = value.to_lowercase();
    if normalized.len() == 64 && is_lower_hex(&normalized) {
        return make_result(normalized);
    }
    invalid_result()
}

fn validate_near_address(value: &str) -> AddressValidationResult {
    let normalized = value.to_lowercase();

    if normalized.len() == 64 && is_lower_hex(&normalized) {
        return make_result(normalized);
    }

    if !(2..=64).contains(&normalized.len()) {
        return invalid_result();
    }
    if normalized.starts_with('.') || normalized.ends_with('.') {
        return invalid_result();
    }
    if normalized.starts_with('-')
        || normalized.ends_with('-')
        || normalized.starts_with('_')
        || normalized.ends_with('_')
    {
        return invalid_result();
    }
    if !normalized.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || "._-".contains(character)
    }) {
        return invalid_result();
    }

    let mut previous_was_separator = false;
    for character in normalized.chars() {
        let is_separator = "._-".contains(character);
        if is_separator && previous_was_separator {
            return invalid_result();
        }
        previous_was_separator = is_separator;
    }

    make_result(normalized)
}

fn validate_polkadot_address(value: &str) -> AddressValidationResult {
    if (47..=50).contains(&value.len()) && is_base58(value) {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_monero_address(value: &str) -> AddressValidationResult {
    if !is_base58(value) {
        return invalid_result();
    }
    if value.len() != 95 && value.len() != 106 {
        return invalid_result();
    }
    if value.starts_with('4') || value.starts_with('8') {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_cardano_address(value: &str) -> AddressValidationResult {
    let lowered = value.to_lowercase();
    if (lowered.starts_with("addr1") || lowered.starts_with("addr_test1")) && value.len() >= 40 {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_aptos_token_type(value: &str) -> StringValidationResult {
    let normalized = value.trim().to_lowercase();
    if normalized.is_empty() {
        return StringValidationResult {
            is_valid: false,
            normalized_value: None,
        };
    }

    if validate_aptos_address(&normalized).is_valid {
        return make_string_result(
            validate_aptos_address(&normalized)
                .normalized_value
                .unwrap_or(normalized),
        );
    }

    if !normalized.contains("::") {
        return StringValidationResult {
            is_valid: false,
            normalized_value: None,
        };
    }

    let address_component = normalized
        .split("::")
        .next()
        .unwrap_or_default()
        .to_string();
    let validated_address = validate_aptos_address(&address_component);
    if !validated_address.is_valid {
        return StringValidationResult {
            is_valid: false,
            normalized_value: None,
        };
    }

    make_string_result(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_evm_addresses() {
        let result = validate_address(AddressValidationRequest {
            kind: "evm".to_string(),
            value: " 0xABCDabcdABCDabcdABCDabcdABCDabcdABCDabcd ".to_string(),
            network_mode: None,
        });

        assert!(result.is_valid);
        assert_eq!(
            result.normalized_value.as_deref(),
            Some("0xabcdabcdabcdabcdabcdabcdabcdabcdabcdabcd")
        );
    }

    #[test]
    fn normalizes_aptos_addresses() {
        let result = validate_address(AddressValidationRequest {
            kind: "aptos".to_string(),
            value: "ABCD".to_string(),
            network_mode: None,
        });

        assert!(result.is_valid);
        assert_eq!(result.normalized_value.as_deref(), Some("0xabcd"));
    }

    #[test]
    fn rejects_invalid_near_addresses() {
        let result = validate_address(AddressValidationRequest {
            kind: "near".to_string(),
            value: "bad..near".to_string(),
            network_mode: None,
        });

        assert!(!result.is_valid);
    }

    #[test]
    fn validates_aptos_token_types() {
        let result = validate_string_identifier(StringValidationRequest {
            kind: "aptosTokenType".to_string(),
            value: "0x1::aptos_coin::AptosCoin".to_string(),
        });

        assert!(result.is_valid);
        assert_eq!(
            result.normalized_value.as_deref(),
            Some("0x1::aptos_coin::aptoscoin")
        );
    }
}
