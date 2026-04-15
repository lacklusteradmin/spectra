use crate::addressing::{validate_address, AddressValidationRequest};
use crate::derivation_derive_all_addresses_json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const CANONICAL_MNEMONIC: &str =
    "test test test test test test test test test test test junk";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChainSelfTestResult {
    pub name: String,
    pub passed: bool,
    pub message: String,
}

struct ChainSpec {
    chain_key: &'static str,
    chain_label: &'static str,
    address_kind: &'static str,
    network_mode: Option<&'static str>,
    valid_address: &'static str,
    invalid_address: &'static str,
    derivation_chain: Option<&'static str>,
    derivation_path: &'static str,
}

const CHAIN_SPECS: &[ChainSpec] = &[
    ChainSpec {
        chain_key: "Bitcoin",
        chain_label: "Bitcoin",
        address_kind: "bitcoin",
        network_mode: Some("mainnet"),
        valid_address: "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080",
        invalid_address: "bc1_not_valid",
        derivation_chain: Some("Bitcoin"),
        derivation_path: "m/84'/0'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Bitcoin Cash",
        chain_label: "Bitcoin Cash",
        address_kind: "bitcoinCash",
        network_mode: None,
        valid_address: "bitcoincash:qq07d3s9k4u8x7n5e9qj6m4eht0n5k7n3w6d5m9c8w",
        invalid_address: "bitcoincash:not_valid",
        derivation_chain: Some("Bitcoin Cash"),
        derivation_path: "m/44'/145'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Bitcoin SV",
        chain_label: "Bitcoin SV",
        address_kind: "bitcoinSV",
        network_mode: None,
        valid_address: "1MirQ9bwyQcGVJPwKUgapu5ouK2E2Ey4gX",
        invalid_address: "bsv_not_valid",
        derivation_chain: Some("Bitcoin SV"),
        derivation_path: "m/44'/236'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Litecoin",
        chain_label: "Litecoin",
        address_kind: "litecoin",
        network_mode: None,
        valid_address: "ltc1qg82u8my75w4q8k4s4w9q3k6v7d9s8g0j4qg3s6",
        invalid_address: "ltc_not_valid",
        derivation_chain: Some("Litecoin"),
        derivation_path: "m/44'/2'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Cardano",
        chain_label: "Cardano",
        address_kind: "cardano",
        network_mode: None,
        valid_address: "addr1q9d6m0vxj4j6f0r2k6zk6n6w6r0v9x9k5n0d5u7r3q8v9w7c5m0h2g8t7u6k5a4s3d2f1g0h9j8k7l6m5n4p3q2r1s",
        invalid_address: "addr_not_valid",
        derivation_chain: Some("Cardano"),
        derivation_path: "m/1852'/1815'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Solana",
        chain_label: "Solana",
        address_kind: "solana",
        network_mode: None,
        valid_address: "Vote111111111111111111111111111111111111111",
        invalid_address: "sol_not_valid",
        derivation_chain: Some("Solana"),
        derivation_path: "m/44'/501'/0'/0'",
    },
    ChainSpec {
        chain_key: "Stellar",
        chain_label: "Stellar",
        address_kind: "stellar",
        network_mode: None,
        valid_address: "GBRPYHIL2C4F7Q4W6H6OL5K2C4BFRJHC7YQ7AZZLQ6G4Z7D4VJ4M6N4K",
        invalid_address: "stellar_not_valid",
        derivation_chain: Some("Stellar"),
        derivation_path: "m/44'/148'/0'",
    },
    ChainSpec {
        chain_key: "XRP",
        chain_label: "XRP",
        address_kind: "xrp",
        network_mode: None,
        valid_address: "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh",
        invalid_address: "xrp_not_valid",
        derivation_chain: Some("XRP Ledger"),
        derivation_path: "m/44'/144'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Tron",
        chain_label: "Tron",
        address_kind: "tron",
        network_mode: None,
        valid_address: "TNPeeaaFB7K9cmo4uQpcU32zGK8G1NYqeL",
        invalid_address: "tron_not_valid",
        derivation_chain: Some("Tron"),
        derivation_path: "m/44'/195'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Sui",
        chain_label: "Sui",
        address_kind: "sui",
        network_mode: None,
        valid_address: "0x5f1e6bc4b4f4d7e4d4b5e7a6c3b2a1f0e9d8c7b6a5f4e3d2c1b0a9876543210f",
        invalid_address: "0xnotvalid",
        derivation_chain: Some("Sui"),
        derivation_path: "m/44'/784'/0'/0'/0'",
    },
    ChainSpec {
        chain_key: "Aptos",
        chain_label: "Aptos",
        address_kind: "aptos",
        network_mode: None,
        valid_address: "0x1",
        invalid_address: "aptos_not_valid",
        derivation_chain: Some("Aptos"),
        derivation_path: "m/44'/637'/0'/0'/0'",
    },
    ChainSpec {
        chain_key: "TON",
        chain_label: "TON",
        address_kind: "ton",
        network_mode: None,
        valid_address: "UQBm--PFwDv1yCeS-QTJ-L8oiUpqo9IT1BwgVptlSq3ts4DV",
        invalid_address: "ton_not_valid",
        derivation_chain: Some("TON"),
        derivation_path: "m/44'/607'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Internet Computer",
        chain_label: "Internet Computer",
        address_kind: "internetComputer",
        network_mode: None,
        valid_address: "be2us-64aaa-aaaaa-qaabq-cai",
        invalid_address: "icp_not_valid",
        derivation_chain: Some("Internet Computer"),
        derivation_path: "m/44'/223'/0'/0/0",
    },
    ChainSpec {
        chain_key: "NEAR",
        chain_label: "NEAR",
        address_kind: "near",
        network_mode: None,
        valid_address: "example.near",
        invalid_address: "-not-valid.near",
        derivation_chain: Some("NEAR"),
        derivation_path: "m/44'/397'/0'",
    },
    ChainSpec {
        chain_key: "Polkadot",
        chain_label: "Polkadot",
        address_kind: "polkadot",
        network_mode: None,
        valid_address: "15oF4u3gP5xY8J8cH7W5WqJ9wS6XtK9vYw7R1oL2nQm1QdKp",
        invalid_address: "dot_not_valid",
        derivation_chain: Some("Polkadot"),
        derivation_path: "m/44'/354'/0'",
    },
    ChainSpec {
        chain_key: "Monero",
        chain_label: "Monero",
        address_kind: "monero",
        network_mode: None,
        valid_address: "47zQ5w3QJ9P4hJ2sD7v8QnE9mQfQv7s3y6Fq1v6F5g4Yv7dL1m4rV4bW2tK4w9W8nS2b8S8i3Q2vX5M8Q1n7w6Jp1q2x3Q",
        invalid_address: "xmr_not_valid",
        derivation_chain: None,
        derivation_path: "",
    },
    ChainSpec {
        chain_key: "BNB Chain",
        chain_label: "BNB Chain",
        address_kind: "evm",
        network_mode: None,
        valid_address: "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
        invalid_address: "0x_not_valid",
        derivation_chain: Some("Ethereum"),
        derivation_path: "m/44'/60'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Avalanche",
        chain_label: "Avalanche",
        address_kind: "evm",
        network_mode: None,
        valid_address: "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
        invalid_address: "0x_not_valid",
        derivation_chain: Some("Avalanche"),
        derivation_path: "m/44'/60'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Ethereum Classic",
        chain_label: "Ethereum Classic",
        address_kind: "evm",
        network_mode: None,
        valid_address: "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
        invalid_address: "0x_not_valid",
        derivation_chain: Some("Ethereum Classic"),
        derivation_path: "m/44'/61'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Hyperliquid",
        chain_label: "Hyperliquid",
        address_kind: "evm",
        network_mode: None,
        valid_address: "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
        invalid_address: "0x_not_valid",
        derivation_chain: Some("Hyperliquid"),
        derivation_path: "m/44'/60'/0'/0/0",
    },
];

fn validate(kind: &str, value: &str, network_mode: Option<&str>) -> bool {
    validate_address(AddressValidationRequest {
        kind: kind.to_string(),
        value: value.to_string(),
        network_mode: network_mode.map(str::to_string),
    })
    .is_valid
}

fn run_address_accepts(spec: &ChainSpec) -> ChainSelfTestResult {
    let passed = validate(spec.address_kind, spec.valid_address, spec.network_mode);
    ChainSelfTestResult {
        name: format!("{} Address Validation", spec.chain_label),
        passed,
        message: if passed {
            format!("Valid {} address accepted.", spec.chain_label)
        } else {
            format!("Valid {} address rejected.", spec.chain_label)
        },
    }
}

fn run_address_rejects(spec: &ChainSpec) -> ChainSelfTestResult {
    let passed = !validate(spec.address_kind, spec.invalid_address, spec.network_mode);
    ChainSelfTestResult {
        name: format!("{} Address Rejects Invalid", spec.chain_label),
        passed,
        message: if passed {
            format!("Invalid {} address rejected.", spec.chain_label)
        } else {
            format!("Invalid {} address unexpectedly accepted.", spec.chain_label)
        },
    }
}

fn derive_one(chain_name: &str, path: &str) -> Option<String> {
    let chain_paths_json = format!("{{\"{}\":\"{}\"}}", chain_name, path);
    let response_json =
        derivation_derive_all_addresses_json(CANONICAL_MNEMONIC.to_string(), chain_paths_json)
            .ok()?;
    let map: HashMap<String, Option<String>> = serde_json::from_str(&response_json).ok()?;
    map.get(chain_name).cloned().flatten()
}

fn run_derivation(spec: &ChainSpec) -> Option<ChainSelfTestResult> {
    let derivation_chain = spec.derivation_chain?;
    let derived = derive_one(derivation_chain, spec.derivation_path);
    let name = format!("{} Seed Derivation", spec.chain_label);
    let Some(address) = derived else {
        return Some(ChainSelfTestResult {
            name,
            passed: false,
            message: format!(
                "Failed to derive a {} address from a valid mnemonic.",
                spec.chain_label
            ),
        });
    };
    let passed = validate(spec.address_kind, &address, spec.network_mode);
    Some(ChainSelfTestResult {
        name,
        passed,
        message: if passed {
            format!("Mnemonic-derived {} address is valid.", spec.chain_label)
        } else {
            format!("Derived {} address format is invalid.", spec.chain_label)
        },
    })
}

fn run_spec(spec: &ChainSpec) -> Vec<ChainSelfTestResult> {
    let mut results = vec![run_address_accepts(spec), run_address_rejects(spec)];
    if let Some(derivation_result) = run_derivation(spec) {
        results.push(derivation_result);
    }
    results
}

fn run_dogecoin() -> Vec<ChainSelfTestResult> {
    let valid_mainnet = "DBus3bamQjgJULBJtYXpEzDWQRwF5iwxgC";
    let mainnet_passed = validate("dogecoin", valid_mainnet, Some("mainnet"));
    let garbage_rejected = !validate("dogecoin", "not_a_real_address", Some("mainnet"));
    let mutated = "DA7Q2K7f1k3wX6sVzP8fCBxNf31xHn3v7H";
    let checksum_rejected = !validate("dogecoin", mutated, Some("mainnet"));
    vec![
        ChainSelfTestResult {
            name: "DOGE Address Mainnet Validation".to_string(),
            passed: mainnet_passed,
            message: if mainnet_passed {
                "Mainnet address accepted.".to_string()
            } else {
                "Mainnet address validation failed.".to_string()
            },
        },
        ChainSelfTestResult {
            name: "DOGE Address Rejects Invalid".to_string(),
            passed: garbage_rejected,
            message: if garbage_rejected {
                "Invalid address rejected.".to_string()
            } else {
                "Invalid address unexpectedly accepted.".to_string()
            },
        },
        ChainSelfTestResult {
            name: "DOGE Address Rejects Bad Checksum".to_string(),
            passed: checksum_rejected,
            message: if checksum_rejected {
                "Checksum mutation rejected.".to_string()
            } else {
                "Checksum mutation unexpectedly accepted.".to_string()
            },
        },
    ]
}

fn run_ethereum() -> Vec<ChainSelfTestResult> {
    let valid = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045";
    let valid_passed = validate("evm", valid, None);
    let garbage_rejected = !validate("evm", "0x_not_valid", None);
    let mixed_case = "0x52908400098527886E0F7030069857D2E4169EE7";
    let normalized_pass = validate_address(AddressValidationRequest {
        kind: "evm".to_string(),
        value: mixed_case.to_string(),
        network_mode: None,
    })
    .normalized_value
        .map(|v| v == mixed_case.to_lowercase())
        .unwrap_or(false);
    let derived = derive_one("Ethereum", "m/44'/60'/0'/0/0");
    let derivation_passed = derived
        .as_deref()
        .map(|address| validate("evm", address, None))
        .unwrap_or(false);
    vec![
        ChainSelfTestResult {
            name: "ETH Address Validation".to_string(),
            passed: valid_passed,
            message: if valid_passed {
                "Valid Ethereum address accepted.".to_string()
            } else {
                "Valid Ethereum address rejected.".to_string()
            },
        },
        ChainSelfTestResult {
            name: "ETH Address Rejects Invalid".to_string(),
            passed: garbage_rejected,
            message: if garbage_rejected {
                "Invalid Ethereum address rejected.".to_string()
            } else {
                "Invalid Ethereum address unexpectedly accepted.".to_string()
            },
        },
        ChainSelfTestResult {
            name: "ETH Receive Address Normalization".to_string(),
            passed: normalized_pass,
            message: if normalized_pass {
                "Receive address normalized successfully.".to_string()
            } else {
                "Receive address normalization failed.".to_string()
            },
        },
        ChainSelfTestResult {
            name: "ETH Seed Derivation".to_string(),
            passed: derivation_passed,
            message: if derivation_passed {
                "Mnemonic-derived Ethereum address is valid.".to_string()
            } else {
                "Derived address format is invalid.".to_string()
            },
        },
    ]
}

fn run_for_chain(chain_key: &str) -> Vec<ChainSelfTestResult> {
    match chain_key {
        "Dogecoin" => run_dogecoin(),
        "Ethereum" => run_ethereum(),
        _ => CHAIN_SPECS
            .iter()
            .find(|spec| spec.chain_key == chain_key)
            .map(run_spec)
            .unwrap_or_default(),
    }
}

#[uniffi::export]
pub fn self_tests_run_chain_json(chain_key: String) -> Result<String, crate::SpectraBridgeError> {
    let results = run_for_chain(&chain_key);
    serde_json::to_string(&results).map_err(crate::SpectraBridgeError::from)
}

#[uniffi::export]
pub fn self_tests_run_all_json() -> Result<String, crate::SpectraBridgeError> {
    let mut all: Vec<(&str, Vec<ChainSelfTestResult>)> = Vec::new();
    all.push(("Dogecoin", run_dogecoin()));
    all.push(("Ethereum", run_ethereum()));
    for spec in CHAIN_SPECS {
        all.push((spec.chain_key, run_spec(spec)));
    }
    let map: HashMap<&str, Vec<ChainSelfTestResult>> = all.into_iter().collect();
    serde_json::to_string(&map).map_err(crate::SpectraBridgeError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_tests_cover_all_chains() {
        let json = self_tests_run_all_json().expect("self tests run");
        let map: HashMap<String, Vec<ChainSelfTestResult>> =
            serde_json::from_str(&json).expect("decode");
        assert!(map.contains_key("Bitcoin"));
        assert!(map.contains_key("Ethereum"));
        assert!(map.contains_key("Dogecoin"));
    }

    #[test]
    fn ethereum_self_tests_pass() {
        let results = run_ethereum();
        assert!(results.iter().all(|r| r.passed), "{:#?}", results);
    }
}
