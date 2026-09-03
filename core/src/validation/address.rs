//! Per-chain address and string-identifier rules.

use serde::{Deserialize, Serialize};

use crate::derivation::chains::bitcoin::{parse_bitcoin_address, BitcoinNetworkKind};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AddressValidationRequest {
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AddressValidationResult {
    pub is_valid: bool,
    pub normalized_value: Option<String>,
}



#[uniffi::export]
pub fn core_validate_address(request: AddressValidationRequest) -> AddressValidationResult {
    validate_address(request)
}

pub fn validate_address(request: AddressValidationRequest) -> AddressValidationResult {
    let normalized_input = trim_string(&request.value);
    if normalized_input.is_empty() {
        return invalid_result();
    }

    // Each testnet has its own `kind` string (e.g. `"bitcoinTestnet"`,
    // `"litecoinTestnet"`), so the kind says which network to judge against.
    // The request used to also carry a `network_mode` "for backwards
    // compatibility with stored wallets" — nothing read it, and prelaunch
    // there are no stored wallets to be compatible with.
    match request.kind.as_str() {
        "bitcoin" => validate_bitcoin_address(&normalized_input, BitcoinNetworkKind::Mainnet),
        "bitcoinTestnet" | "bitcoinTestnet4" | "bitcoinSignet" => {
            validate_bitcoin_address(&normalized_input, BitcoinNetworkKind::Testnet)
        }
        "bitcoinCash" => validate_bitcoin_cash_address(&normalized_input, false),
        "bitcoinCashTestnet" => validate_bitcoin_cash_address(&normalized_input, true),
        "bitcoinSV" => validate_bitcoin_sv_address(&normalized_input),
        "bitcoinSVTestnet" => validate_bitcoin_sv_address(&normalized_input),
        "litecoin" => validate_litecoin_address(&normalized_input, false),
        "litecoinTestnet" => validate_litecoin_address(&normalized_input, true),
        "dogecoin" => validate_dogecoin_address(&normalized_input, false),
        "dogecoinTestnet" => validate_dogecoin_address(&normalized_input, true),
        // EVM addresses are network-agnostic on the wire — same validator
        // for mainnet + every EVM testnet.
        "evm" | "evmTestnet" => validate_evm_address(&normalized_input),
        "tron" | "tronTestnet" => validate_tron_address(&normalized_input),
        "solana" | "solanaDevnet" => validate_solana_address(&normalized_input),
        "stellar" | "stellarTestnet" => validate_stellar_address(&normalized_input),
        "xrp" | "xrpTestnet" => validate_xrp_address(&normalized_input),
        "sui" | "suiTestnet" => validate_sui_address(&normalized_input),
        "aptos" | "aptosTestnet" => validate_aptos_address(&normalized_input),
        "ton" | "tonTestnet" => validate_ton_address(&normalized_input),
        "internetComputer" => validate_icp_address(&normalized_input),
        "near" | "nearTestnet" => validate_near_address(&normalized_input),
        "polkadot" | "polkadotTestnet" => validate_polkadot_address(&normalized_input),
        "monero" => validate_monero_address(&normalized_input, false),
        "moneroStagenet" => validate_monero_address(&normalized_input, true),
        "cardano" | "cardanoTestnet" => validate_cardano_address(&normalized_input),
        "zcash" => validate_zcash_address(&normalized_input, false),
        "zcashTestnet" => validate_zcash_address(&normalized_input, true),
        "bitcoinGold" => validate_bitcoin_gold_address(&normalized_input),
        "decred" => validate_decred_address(&normalized_input, false),
        "decredTestnet" => validate_decred_address(&normalized_input, true),
        "kaspa" | "kaspaTestnet" => validate_kaspa_address(&normalized_input),
        "dash" => validate_dash_address(&normalized_input, false),
        "dashTestnet" => validate_dash_address(&normalized_input, true),
        "bittensor" => validate_bittensor_address(&normalized_input),
        // Not an address, but the same question in the same shape: a typed
        // string, is it well formed, and what is its canonical spelling. It had
        // its own export, its own request record and its own result record,
        // each identical to these, to dispatch on one kind.
        "aptosTokenType" => validate_aptos_token_type(&normalized_input),
        _ => invalid_result(),
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

fn make_string_result(normalized_value: String) -> AddressValidationResult {
    AddressValidationResult {
        is_valid: true,
        normalized_value: Some(normalized_value),
    }
}

const BASE58_LUT: [bool; 128] = {
    let mut lut = [false; 128];
    let alphabet = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut i = 0;
    while i < alphabet.len() {
        lut[alphabet[i] as usize] = true;
        i += 1;
    }
    lut
};

fn is_base58(value: &str) -> bool {
    value.bytes().all(|b| (b < 128) && BASE58_LUT[b as usize])
}

fn is_lower_hex(value: &str) -> bool {
    value.chars().all(|character| character.is_ascii_hexdigit())
}

fn validate_legacy_base58_payload(value: &str, allowed_versions: &[u8]) -> Option<Vec<u8>> {
    let decoded = bs58::decode(value).with_check(None).into_vec().ok()?;
    if decoded.len() != 21 || !allowed_versions.contains(&decoded[0]) {
        return None;
    }
    Some(decoded)
}

fn validate_segwit_hrp(value: &str, allowed_hrps: &[&str]) -> bool {
    bech32::segwit::decode(value)
        .map(|(hrp, _version, _program)| {
            let hrp = hrp.to_string().to_ascii_lowercase();
            allowed_hrps.iter().any(|candidate| *candidate == hrp)
        })
        .unwrap_or(false)
}

fn validate_bch_cashaddr(value: &str, testnet: bool) -> Option<String> {
    const CHARSET: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
    const GENERATORS: [u64; 5] = [
        0x98f2bc8e61,
        0x79b76d99e2,
        0xf33e5fb3c4,
        0xae2eabe2a8,
        0x1e4f43e470,
    ];

    fn polymod(values: &[u8]) -> u64 {
        let mut chk = 1u64;
        for value in values {
            let top = chk >> 35;
            chk = ((chk & 0x07_ffff_ffff) << 5) ^ (*value as u64);
            for (i, generator) in GENERATORS.iter().enumerate() {
                if ((top >> i) & 1) != 0 {
                    chk ^= generator;
                }
            }
        }
        chk
    }

    let lower = value.to_ascii_lowercase();
    if lower != value && value.chars().any(|c| c.is_ascii_lowercase()) {
        return None;
    }
    let expected_prefix = if testnet { "bchtest" } else { "bitcoincash" };
    let (prefix, payload) = match lower.split_once(':') {
        Some((prefix, payload)) if prefix == expected_prefix => (prefix.to_string(), payload),
        Some(_) => return None,
        None => (expected_prefix.to_string(), lower.as_str()),
    };
    if payload.len() < 9 {
        return None;
    }
    let mut payload_values = Vec::with_capacity(payload.len());
    for ch in payload.chars() {
        payload_values.push(CHARSET.find(ch)? as u8);
    }
    let data_len = payload_values.len().checked_sub(8)?;
    let version = *payload_values.first()?;
    let address_type = version >> 3;
    let hash_size = version & 0x07;
    if address_type > 1 || hash_size != 0 {
        return None;
    }
    let mut values = Vec::with_capacity(prefix.len() + 1 + payload_values.len());
    values.extend(prefix.bytes().map(|b| b & 0x1f));
    values.push(0);
    values.extend_from_slice(&payload_values);
    if polymod(&values) != 1 {
        return None;
    }
    if data_len == 0 {
        return None;
    }
    Some(if lower.contains(':') {
        lower
    } else {
        payload.to_string()
    })
}

fn validate_bitcoin_address(
    value: &str,
    expected_network: BitcoinNetworkKind,
) -> AddressValidationResult {
    let parsed = match parse_bitcoin_address(value) {
        Ok(parsed) => parsed,
        Err(_) => return invalid_result(),
    };
    let network = match &parsed {
        crate::derivation::chains::bitcoin::ParsedBitcoinAddress::Legacy { network, .. }
        | crate::derivation::chains::bitcoin::ParsedBitcoinAddress::SegWit { network, .. } => {
            network
        }
    };
    let is_valid = match expected_network {
        BitcoinNetworkKind::Mainnet => matches!(network, BitcoinNetworkKind::Mainnet),
        BitcoinNetworkKind::Testnet => matches!(network, BitcoinNetworkKind::Testnet),
    };
    if !is_valid {
        return invalid_result();
    }
    make_result(value.to_string())
}

fn validate_bitcoin_cash_address(value: &str, testnet: bool) -> AddressValidationResult {
    if let Some(normalized) = validate_bch_cashaddr(value, testnet) {
        return make_result(normalized);
    }
    let versions = if testnet {
        &[0x6f, 0xc4][..]
    } else {
        &[0x00, 0x05][..]
    };
    if validate_legacy_base58_payload(value, versions).is_some() {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_bitcoin_sv_address(value: &str) -> AddressValidationResult {
    // BSV is legacy-only: base58check P2PKH (version 0x00) or P2SH (0x05),
    // plus the testnet variants 0x6f / 0xc4. SegWit/Taproot are not valid.
    if crate::derivation::chains::bitcoin_sv::validate_bsv_address(value) {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_litecoin_address(value: &str, testnet: bool) -> AddressValidationResult {
    if testnet {
        if validate_segwit_hrp(value, &["tltc"])
            || crate::derivation::chains::litecoin::parse_mweb_address(value)
                .map(|_| value.to_ascii_lowercase().starts_with("tmweb1"))
                .unwrap_or(false)
            || validate_legacy_base58_payload(value, &[0x6f, 0x3a, 0xc4]).is_some()
        {
            return make_result(value.to_string());
        }
        return invalid_result();
    }
    if validate_segwit_hrp(value, &["ltc"])
        || crate::derivation::chains::litecoin::parse_mweb_address(value)
            .map(|_| value.to_ascii_lowercase().starts_with("ltcmweb1"))
            .unwrap_or(false)
        || validate_legacy_base58_payload(value, &[0x30, 0x32, 0x05]).is_some()
    {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_zcash_address(value: &str, testnet: bool) -> AddressValidationResult {
    if crate::derivation::chains::zcash::validate_zcash_address(value, testnet) {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_bitcoin_gold_address(value: &str) -> AddressValidationResult {
    if crate::derivation::chains::bitcoin_gold::validate_bitcoin_gold_address(value) {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_decred_address(value: &str, testnet: bool) -> AddressValidationResult {
    if crate::derivation::chains::decred::validate_decred_address(value, testnet) {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_kaspa_address(value: &str) -> AddressValidationResult {
    if crate::derivation::chains::kaspa::validate_kaspa_address(value) {
        return make_result(value.trim().to_ascii_lowercase());
    }
    invalid_result()
}

fn validate_dash_address(value: &str, testnet: bool) -> AddressValidationResult {
    if crate::derivation::chains::dash::validate_dash_address(value, testnet) {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_bittensor_address(value: &str) -> AddressValidationResult {
    if crate::derivation::chains::bittensor::validate_bittensor_address(value) {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_dogecoin_address(value: &str, testnet: bool) -> AddressValidationResult {
    let versions = if testnet {
        &[0x71, 0xc4][..]
    } else {
        &[0x1e, 0x16][..]
    };
    if validate_legacy_base58_payload(value, versions).is_some() {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_evm_address(value: &str) -> AddressValidationResult {
    let trimmed = value.trim();
    let normalized = trimmed.to_lowercase();
    if normalized.len() != 42 || !normalized.starts_with("0x") {
        return invalid_result();
    }
    if !is_lower_hex(&normalized[2..]) {
        return invalid_result();
    }
    // An address whose letters are not all one case carries an EIP-55
    // checksum, and the point of that checksum is to catch a mistyped or
    // corrupted character. Lowercasing first and never checking discards it:
    // any forty hex digits passed, so a pasted address with one letter changed
    // was accepted and the funds went somewhere nobody owns.
    //
    // All-lowercase and all-uppercase carry no checksum — that is the
    // pre-EIP-55 form, and it is still valid — so only the mixed case is
    // verified.
    let body = &trimmed[2..];
    let has_upper = body.chars().any(|c| c.is_ascii_uppercase());
    let has_lower = body.chars().any(|c| c.is_ascii_lowercase());
    if has_upper && has_lower {
        let Ok(bytes) = hex::decode(&normalized[2..]) else {
            return invalid_result();
        };
        if crate::derivation::chains::evm::eip55_checksum(&bytes) != trimmed {
            return invalid_result();
        }
    }
    make_result(normalized)
}

fn validate_tron_address(value: &str) -> AddressValidationResult {
    if crate::derivation::chains::tron::tron_base58_to_evm_hex(value).is_ok() {
        return make_result(value.to_string());
    }
    invalid_result()
}

/// A Solana address is the base58 of a 32-byte public key.
///
/// This checked the length range and the alphabet, which any base58 string of
/// the right length passes — including one three characters short of a real
/// address. Decoding is what tells them apart.
fn validate_solana_address(value: &str) -> AddressValidationResult {
    if !(32..=44).contains(&value.len()) || !is_base58(value) {
        return invalid_result();
    }
    match bs58::decode(value).into_vec() {
        Ok(bytes) if bytes.len() == 32 => make_result(value.to_string()),
        _ => invalid_result(),
    }
}

fn validate_stellar_address(value: &str) -> AddressValidationResult {
    if crate::derivation::chains::stellar::decode_stellar_address(value).is_ok() {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_xrp_address(value: &str) -> AddressValidationResult {
    if crate::derivation::chains::xrp::decode_xrp_address(value).is_ok() {
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
    if crate::derivation::chains::polkadot::decode_ss58(value).is_ok() {
        return make_result(value.to_string());
    }
    invalid_result()
}

fn validate_monero_address(value: &str, stagenet: bool) -> AddressValidationResult {
    if !is_base58(value) {
        return invalid_result();
    }
    if value.len() != 95 && value.len() != 106 {
        return invalid_result();
    }
    let valid = if stagenet {
        // Stagenet primary: starts with `5`. Sub-addresses: `7`.
        value.starts_with('5') || value.starts_with('7')
    } else {
        value.starts_with('4') || value.starts_with('8')
    };
    if valid {
        make_result(value.to_string())
    } else {
        invalid_result()
    }
}

/// A Shelley address is bech32, and bech32 carries a checksum.
///
/// This checked the prefix and a minimum length and nothing else, so a
/// truncated or mistyped address passed — which is the one thing the checksum
/// exists to catch.
fn validate_cardano_address(value: &str) -> AddressValidationResult {
    let lowered = value.to_lowercase();
    if !(lowered.starts_with("addr1") || lowered.starts_with("addr_test1")) {
        return invalid_result();
    }
    match bech32::decode(&lowered) {
        Ok((hrp, data)) if !data.is_empty() && (hrp.as_str() == "addr" || hrp.as_str() == "addr_test") => {
            make_result(value.to_string())
        }
        _ => invalid_result(),
    }
}

fn validate_aptos_token_type(value: &str) -> AddressValidationResult {
    let normalized = value.trim().to_lowercase();
    if normalized.is_empty() {
        return AddressValidationResult {
            is_valid: false,
            normalized_value: None,
        };
    }

    let addr_result = validate_aptos_address(&normalized);
    if addr_result.is_valid {
        return make_string_result(addr_result.normalized_value.unwrap_or(normalized));
    }

    if !normalized.contains("::") {
        return AddressValidationResult {
            is_valid: false,
            normalized_value: None,
        };
    }

    let address_component = normalized.split("::").next().unwrap_or_default();
    if !validate_aptos_address(address_component).is_valid {
        return AddressValidationResult {
            is_valid: false,
            normalized_value: None,
        };
    }

    make_string_result(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MNEMONIC: &str =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    fn validate(kind: &str, value: String) -> AddressValidationResult {
        validate_address(AddressValidationRequest {
            kind: kind.to_string(),
            value,
        })
    }

    fn mutate_last_char(value: &str) -> String {
        let mut out = value.to_string();
        let replacement = if out.ends_with('q') { 'p' } else { 'q' };
        out.pop();
        out.push(replacement);
        out
    }

    #[test]
    fn normalizes_evm_addresses() {
        let result = validate_address(AddressValidationRequest {
            kind: "evm".to_string(),
            // All one case: no EIP-55 checksum to verify, so this stays a
            // test about trimming and lowercasing.
            value: " 0XABCDABCDABCDABCDABCDABCDABCDABCDABCDABCD ".to_string(),
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
        });

        assert!(result.is_valid);
        assert_eq!(result.normalized_value.as_deref(), Some("0xabcd"));
    }

    /// A mixed-case EVM address is checked against its EIP-55 checksum.
    ///
    /// The validator lowercased first and never looked, so any forty hex
    /// digits passed. A checksummed address with one letter's case changed —
    /// which is what a mistyped or corrupted paste looks like — was accepted,
    /// and a send to it goes to an address nobody holds a key for.
    #[test]
    fn a_mixed_case_evm_address_must_checksum() {
        let valid = "0x742d35Cc6634C0532925a3b844Bc454e4438f44e";
        assert!(
            validate_address(AddressValidationRequest {
                kind: "evm".to_string(),
                value: valid.to_string(),
            })
            .is_valid
        );

        // One letter's case flipped: still forty hex digits, no longer a
        // checksum.
        let corrupted = "0x742d35cC6634C0532925a3b844Bc454e4438f44e";
        assert!(
            !validate_address(AddressValidationRequest {
                kind: "evm".to_string(),
                value: corrupted.to_string(),
            })
            .is_valid,
            "a broken checksum must be refused"
        );

        // All one case carries no checksum — the pre-EIP-55 form, still valid.
        for unchecked in [
            "0x742d35cc6634c0532925a3b844bc454e4438f44e",
            "0X742D35CC6634C0532925A3B844BC454E4438F44E",
        ] {
            assert!(
                validate_address(AddressValidationRequest {
                    kind: "evm".to_string(),
                    value: unchecked.to_string(),
                })
                .is_valid,
                "{unchecked}"
            );
        }
    }

    #[test]
    fn rejects_invalid_near_addresses() {
        let result = validate_address(AddressValidationRequest {
            kind: "near".to_string(),
            value: "bad..near".to_string(),
        });

        assert!(!result.is_valid);
    }

    #[test]
    fn validates_aptos_token_types() {
        let result = validate_address(AddressValidationRequest {
            kind: "aptosTokenType".to_string(),
            value: "0x1::aptos_coin::AptosCoin".to_string(),
        });

        assert!(result.is_valid);
        assert_eq!(
            result.normalized_value.as_deref(),
            Some("0x1::aptos_coin::aptoscoin")
        );
    }

    #[test]
    fn rejects_mutated_checksum_addresses() {
        let xrp = crate::derivation::chains::xrp::derive_xrp(
            MNEMONIC.to_string(),
            "m/44'/144'/0'/0/0".to_string(),
            None,
            true,
            false,
            false,
        )
        .unwrap()
        .address
        .unwrap();
        assert!(validate("xrp", xrp.clone()).is_valid);
        assert!(!validate("xrp", mutate_last_char(&xrp)).is_valid);

        let tron = crate::derivation::chains::tron::derive_tron(
            MNEMONIC.to_string(),
            "m/44'/195'/0'/0/0".to_string(),
            None,
            true,
            false,
            false,
        )
        .unwrap()
        .address
        .unwrap();
        assert!(validate("tron", tron.clone()).is_valid);
        assert!(!validate("tron", mutate_last_char(&tron)).is_valid);

        let stellar = crate::derivation::chains::stellar::derive_stellar(
            MNEMONIC.to_string(),
            "m/44'/148'/0'".to_string(),
            None,
            None,
            true,
            false,
            false,
        )
        .unwrap()
        .address
        .unwrap();
        assert!(validate("stellar", stellar.clone()).is_valid);
        assert!(!validate("stellar", mutate_last_char(&stellar)).is_valid);

        let bittensor = crate::derivation::chains::bittensor::derive_bittensor(
            MNEMONIC.to_string(),
            None,
            true,
            false,
            false,
        )
        .unwrap()
        .address
        .unwrap();
        assert!(validate("bittensor", bittensor.clone()).is_valid);
        assert!(!validate("bittensor", mutate_last_char(&bittensor)).is_valid);
    }

    #[test]
    fn validates_utxo_family_by_decoded_network() {
        let bch_cashaddr = "bitcoincash:qpm2qsznhks23z7629mms6s4cwef74vcwvy22gdx6a".to_string();
        assert!(validate("bitcoinCash", bch_cashaddr.clone()).is_valid);
        assert!(!validate("bitcoinCash", mutate_last_char(&bch_cashaddr)).is_valid);

        let doge = crate::derivation::chains::dogecoin::derive_dogecoin(
            MNEMONIC.to_string(),
            "m/44'/3'/0'/0/0".to_string(),
            None,
            crate::derivation::types::BitcoinScriptType::P2pkh,
            true,
            false,
            false,
        )
        .unwrap()
        .address
        .unwrap();
        assert!(validate("dogecoin", doge.clone()).is_valid);
        assert!(!validate("dogecoinTestnet", doge).is_valid);

        let ltc = crate::derivation::chains::litecoin::derive_litecoin(
            MNEMONIC.to_string(),
            "m/44'/2'/0'/0/0".to_string(),
            None,
            crate::derivation::types::BitcoinScriptType::P2pkh,
            true,
            false,
            false,
        )
        .unwrap()
        .address
        .unwrap();
        assert!(validate("litecoin", ltc.clone()).is_valid);
        assert!(!validate("litecoinTestnet", ltc).is_valid);
    }
}

#[cfg(test)]
mod every_chain_accepts_what_it_derives {
    use crate::registry::Chain;

    const MNEMONIC: &str =
        "legal winner thank year wave sausage worth useful legal winner thank yellow";

    /// A chain's validator accepts the address that chain derives.
    ///
    /// The two halves are written separately — `derivation/chains/*` produces
    /// the address, `validation/address.rs` judges it — so nothing made them
    /// agree. A chain whose derived address its own validator refuses can be
    /// imported and then cannot be sent to, and neither side's tests would
    /// show it.
    #[test]
    fn a_derived_address_passes_its_own_validator() {
        let mut checked = 0;
        let mut failures: Vec<String> = Vec::new();
        for chain in Chain::all().filter(|c| !c.is_testnet()) {
            let Some(path) = crate::store::wallet_domain::CoreSeedDerivationPaths::default()
                .path_for(chain)
                .map(str::to_string)
                .or_else(|| crate::app_core::default_path_from_catalog(chain.chain_display_name()).ok())
            else {
                continue;
            };
            let derived = crate::derivation::dispatch::derive_for_chain_name(
                chain.chain_display_name(),
                MNEMONIC,
                &path,
                None,
                None,
                None,
                true,
                false,
                false,
            );
            let Ok(result) = derived else { continue };
            let Some(address) = result.address.filter(|a| !a.is_empty()) else {
                continue;
            };
            checked += 1;
            let verdict = super::validate_address(super::AddressValidationRequest {
                kind: chain.address_validation_kind().to_string(),
                value: address.clone(),
            });
            if !verdict.is_valid {
                failures.push(format!(
                    "{} derived {address} and its own `{}` validator refuses it",
                    chain.chain_display_name(),
                    chain.address_validation_kind()
                ));
            }
        }
        // Every mainnet that derives at all, which is most of them.
        assert!(checked >= 40, "only {checked} chains derived — the probe is broken");
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
