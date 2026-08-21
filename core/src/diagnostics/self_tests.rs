use crate::validation::address::{validate_address, AddressValidationRequest};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const CANONICAL_MNEMONIC: &str = "test test test test test test test test test test test junk";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum ChainSelfTestOutcome {
    ValidAddressAccepted,
    ValidAddressRejected,
    InvalidAddressRejected,
    InvalidAddressUnexpectedlyAccepted,
    DerivationFailed,
    DerivedAddressValid,
    DerivedAddressInvalid,
    NormalizationSuccess,
    NormalizationFailure,
    ChecksumMutationRejected,
    ChecksumMutationAccepted,
    Custom { text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ChainSelfTestResult {
    pub name: String,
    pub passed: bool,
    pub chain_label: String,
    pub outcome: ChainSelfTestOutcome,
}

struct ChainSpec {
    chain_key: &'static str,
    chain_label: &'static str,
    address_kind: &'static str,
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
        valid_address: "bc1qgkju4yvvtuz0s8vqn837q396jezu2h8ex7gk98",
        invalid_address: "bc1_not_valid",
        derivation_chain: Some("Bitcoin"),
        derivation_path: "m/84'/0'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Bitcoin Cash",
        chain_label: "Bitcoin Cash",
        address_kind: "bitcoinCash",
        valid_address: "19GmUu4QnfGirbAxnpDviczXKZ8LVCvvq8",
        invalid_address: "bitcoincash:not_valid",
        derivation_chain: Some("Bitcoin Cash"),
        derivation_path: "m/44'/145'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Bitcoin SV",
        chain_label: "Bitcoin SV",
        address_kind: "bitcoinSV",
        valid_address: "1MirQ9bwyQcGVJPwKUgapu5ouK2E2Ey4gX",
        invalid_address: "bsv_not_valid",
        derivation_chain: Some("Bitcoin SV"),
        derivation_path: "m/44'/236'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Litecoin",
        chain_label: "Litecoin",
        address_kind: "litecoin",
        valid_address: "LZHamZCxNf71EmnHgUkztqMyaWyBc5nrkb",
        invalid_address: "ltc_not_valid",
        derivation_chain: Some("Litecoin"),
        derivation_path: "m/44'/2'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Cardano",
        chain_label: "Cardano",
        address_kind: "cardano",
        valid_address: "addr1q9d6m0vxj4j6f0r2k6zk6n6w6r0v9x9k5n0d5u7r3q8v9w7c5m0h2g8t7u6k5a4s3d2f1g0h9j8k7l6m5n4p3q2r1s",
        invalid_address: "addr_not_valid",
        derivation_chain: Some("Cardano"),
        derivation_path: "m/1852'/1815'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Solana",
        chain_label: "Solana",
        address_kind: "solana",
        valid_address: "Vote111111111111111111111111111111111111111",
        invalid_address: "sol_not_valid",
        derivation_chain: Some("Solana"),
        derivation_path: "m/44'/501'/0'/0'",
    },
    ChainSpec {
        chain_key: "Stellar",
        chain_label: "Stellar",
        address_kind: "stellar",
        valid_address: "GAFOIIMIXWLSN66RYL32JHCI7AMKWZ3TYYSZTOXSTLPHJWPMZDERMXUO",
        invalid_address: "stellar_not_valid",
        derivation_chain: Some("Stellar"),
        derivation_path: "m/44'/148'/0'",
    },
    ChainSpec {
        chain_key: "XRP",
        chain_label: "XRP",
        address_kind: "xrp",
        valid_address: "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh",
        invalid_address: "xrp_not_valid",
        derivation_chain: Some("XRP Ledger"),
        derivation_path: "m/44'/144'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Tron",
        chain_label: "Tron",
        address_kind: "tron",
        valid_address: "TNPeeaaFB7K9cmo4uQpcU32zGK8G1NYqeL",
        invalid_address: "tron_not_valid",
        derivation_chain: Some("Tron"),
        derivation_path: "m/44'/195'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Sui",
        chain_label: "Sui",
        address_kind: "sui",
        valid_address: "0x5f1e6bc4b4f4d7e4d4b5e7a6c3b2a1f0e9d8c7b6a5f4e3d2c1b0a9876543210f",
        invalid_address: "0xnotvalid",
        derivation_chain: Some("Sui"),
        derivation_path: "m/44'/784'/0'/0'/0'",
    },
    ChainSpec {
        chain_key: "Aptos",
        chain_label: "Aptos",
        address_kind: "aptos",
        valid_address: "0x1",
        invalid_address: "aptos_not_valid",
        derivation_chain: Some("Aptos"),
        derivation_path: "m/44'/637'/0'/0'/0'",
    },
    ChainSpec {
        chain_key: "TON",
        chain_label: "TON",
        address_kind: "ton",
        valid_address: "UQBm--PFwDv1yCeS-QTJ-L8oiUpqo9IT1BwgVptlSq3ts4DV",
        invalid_address: "ton_not_valid",
        derivation_chain: Some("TON"),
        derivation_path: "m/44'/607'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Internet Computer",
        chain_label: "Internet Computer",
        address_kind: "internetComputer",
        valid_address: "3d67a090082c446abb79b91cfa4937cb69256d23b23c72d6fa0461e62d8b3fe3",
        invalid_address: "icp_not_valid",
        derivation_chain: Some("Internet Computer"),
        derivation_path: "m/44'/223'/0'/0/0",
    },
    ChainSpec {
        chain_key: "NEAR",
        chain_label: "NEAR",
        address_kind: "near",
        valid_address: "example.near",
        invalid_address: "-not-valid.near",
        derivation_chain: Some("NEAR"),
        derivation_path: "m/44'/397'/0'",
    },
    ChainSpec {
        chain_key: "Polkadot",
        chain_label: "Polkadot",
        address_kind: "polkadot",
        valid_address: "13DyfGHEWo6GF98AxoBbovBHy82rrr4H3LrWaxggtEpgku8o",
        invalid_address: "dot_not_valid",
        derivation_chain: Some("Polkadot"),
        derivation_path: "m/44'/354'/0'",
    },
    ChainSpec {
        chain_key: "Monero",
        chain_label: "Monero",
        address_kind: "monero",
        valid_address: "46pWvmHcWgbDZDhzkgqMN52rq4tJZGZv26qDTiZW4Jg21tqEyrDaQMjVVACuC59gc9Ma3LM9CqD44Cn8XVqjAnPxEnP1PrZ",
        invalid_address: "xmr_not_valid",
        derivation_chain: None,
        derivation_path: "",
    },
    ChainSpec {
        chain_key: "BNB Chain",
        chain_label: "BNB Chain",
        address_kind: "evm",
        valid_address: "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
        invalid_address: "0x_not_valid",
        derivation_chain: Some("Ethereum"),
        derivation_path: "m/44'/60'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Avalanche",
        chain_label: "Avalanche",
        address_kind: "evm",
        valid_address: "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
        invalid_address: "0x_not_valid",
        derivation_chain: Some("Avalanche"),
        derivation_path: "m/44'/60'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Ethereum Classic",
        chain_label: "Ethereum Classic",
        address_kind: "evm",
        valid_address: "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
        invalid_address: "0x_not_valid",
        derivation_chain: Some("Ethereum Classic"),
        derivation_path: "m/44'/61'/0'/0/0",
    },
    ChainSpec {
        chain_key: "Hyperliquid",
        chain_label: "Hyperliquid",
        address_kind: "evm",
        valid_address: "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
        invalid_address: "0x_not_valid",
        derivation_chain: Some("Hyperliquid"),
        derivation_path: "m/44'/60'/0'/0/0",
    },
];

fn validate(kind: &str, value: &str) -> bool {
    validate_address(AddressValidationRequest {
        kind: kind.to_string(),
        value: value.to_string(),
    })
    .is_valid
}

fn run_address_accepts(spec: &ChainSpec) -> ChainSelfTestResult {
    let passed = validate(spec.address_kind, spec.valid_address);
    ChainSelfTestResult {
        name: format!("{} Address Validation", spec.chain_label),
        passed,
        chain_label: spec.chain_label.to_string(),
        outcome: if passed {
            ChainSelfTestOutcome::ValidAddressAccepted
        } else {
            ChainSelfTestOutcome::ValidAddressRejected
        },
    }
}

fn run_address_rejects(spec: &ChainSpec) -> ChainSelfTestResult {
    let passed = !validate(spec.address_kind, spec.invalid_address);
    ChainSelfTestResult {
        name: format!("{} Address Rejects Invalid", spec.chain_label),
        passed,
        chain_label: spec.chain_label.to_string(),
        outcome: if passed {
            ChainSelfTestOutcome::InvalidAddressRejected
        } else {
            ChainSelfTestOutcome::InvalidAddressUnexpectedlyAccepted
        },
    }
}

/// Derive the address a chain's self-test compares against.
///
/// This was an eighteen-arm match — a third copy of the dispatch that
/// `derive_for_chain_name` has always been, and the smallest of the three, so
/// it silently returned `None` for the chains it had never been extended to
/// cover. The script types it forced are the ones the dispatcher derives from
/// each spec's path anyway: `m/84'` is P2WPKH, `m/44'` is P2PKH.
fn derive_one(chain_name: &str, path: &str) -> Option<String> {
    crate::derivation::dispatch::derive_for_chain_name(
        chain_name,
        CANONICAL_MNEMONIC,
        path,
        None,
        None,
        None,
        true,
        false,
        false,
    )
    .ok()?
    .address
}

fn run_derivation(spec: &ChainSpec) -> Option<ChainSelfTestResult> {
    let derivation_chain = spec.derivation_chain?;
    let derived = derive_one(derivation_chain, spec.derivation_path);
    let name = format!("{} Seed Derivation", spec.chain_label);
    let Some(address) = derived else {
        return Some(ChainSelfTestResult {
            name,
            passed: false,
            chain_label: spec.chain_label.to_string(),
            outcome: ChainSelfTestOutcome::DerivationFailed,
        });
    };
    let passed = validate(spec.address_kind, &address);
    Some(ChainSelfTestResult {
        name,
        passed,
        chain_label: spec.chain_label.to_string(),
        outcome: if passed {
            ChainSelfTestOutcome::DerivedAddressValid
        } else {
            ChainSelfTestOutcome::DerivedAddressInvalid
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
    let mainnet_passed = validate("dogecoin", valid_mainnet);
    let garbage_rejected = !validate("dogecoin", "not_a_real_address");
    let mutated = "DA7Q2K7f1k3wX6sVzP8fCBxNf31xHn3v7H";
    let checksum_rejected = !validate("dogecoin", mutated);
    vec![
        ChainSelfTestResult {
            name: "DOGE Address Mainnet Validation".to_string(),
            passed: mainnet_passed,
            chain_label: "Dogecoin".to_string(),
            outcome: if mainnet_passed {
                ChainSelfTestOutcome::ValidAddressAccepted
            } else {
                ChainSelfTestOutcome::ValidAddressRejected
            },
        },
        ChainSelfTestResult {
            name: "DOGE Address Rejects Invalid".to_string(),
            passed: garbage_rejected,
            chain_label: "Dogecoin".to_string(),
            outcome: if garbage_rejected {
                ChainSelfTestOutcome::InvalidAddressRejected
            } else {
                ChainSelfTestOutcome::InvalidAddressUnexpectedlyAccepted
            },
        },
        ChainSelfTestResult {
            name: "DOGE Address Rejects Bad Checksum".to_string(),
            passed: checksum_rejected,
            chain_label: "Dogecoin".to_string(),
            outcome: if checksum_rejected {
                ChainSelfTestOutcome::ChecksumMutationRejected
            } else {
                ChainSelfTestOutcome::ChecksumMutationAccepted
            },
        },
    ]
}

fn run_ethereum() -> Vec<ChainSelfTestResult> {
    let valid = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045";
    let valid_passed = validate("evm", valid);
    let garbage_rejected = !validate("evm", "0x_not_valid");
    let mixed_case = "0x52908400098527886E0F7030069857D2E4169EE7";
    let normalized_pass = validate_address(AddressValidationRequest {
        kind: "evm".to_string(),
        value: mixed_case.to_string(),
    })
    .normalized_value
    .map(|v| v == mixed_case.to_lowercase())
    .unwrap_or(false);
    let derived = derive_one("Ethereum", "m/44'/60'/0'/0/0");
    let derivation_passed = derived
        .as_deref()
        .map(|address| validate("evm", address))
        .unwrap_or(false);
    vec![
        ChainSelfTestResult {
            name: "ETH Address Validation".to_string(),
            passed: valid_passed,
            chain_label: "Ethereum".to_string(),
            outcome: if valid_passed {
                ChainSelfTestOutcome::ValidAddressAccepted
            } else {
                ChainSelfTestOutcome::ValidAddressRejected
            },
        },
        ChainSelfTestResult {
            name: "ETH Address Rejects Invalid".to_string(),
            passed: garbage_rejected,
            chain_label: "Ethereum".to_string(),
            outcome: if garbage_rejected {
                ChainSelfTestOutcome::InvalidAddressRejected
            } else {
                ChainSelfTestOutcome::InvalidAddressUnexpectedlyAccepted
            },
        },
        ChainSelfTestResult {
            name: "ETH Receive Address Normalization".to_string(),
            passed: normalized_pass,
            chain_label: "Ethereum".to_string(),
            outcome: if normalized_pass {
                ChainSelfTestOutcome::NormalizationSuccess
            } else {
                ChainSelfTestOutcome::NormalizationFailure
            },
        },
        ChainSelfTestResult {
            name: "ETH Seed Derivation".to_string(),
            passed: derivation_passed,
            chain_label: "Ethereum".to_string(),
            outcome: if derivation_passed {
                ChainSelfTestOutcome::DerivedAddressValid
            } else {
                ChainSelfTestOutcome::DerivedAddressInvalid
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

#[derive(Debug, Deserialize)]
struct EthRpcResponse {
    result: Option<String>,
}

async fn fetch_eth_rpc_hex(url: &str, method: &str, id: u32) -> Result<u64, String> {
    let body = format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":[]}}"#);
    let resp =
        crate::fetch::http::http_post_json(url.to_string(), body, std::collections::HashMap::new())
            .await
            .map_err(|e| format!("{e:?}"))?;
    let parsed: EthRpcResponse = serde_json::from_str(&resp.body).map_err(|e| e.to_string())?;
    let hex = parsed.result.unwrap_or_default();
    let trimmed = hex.strip_prefix("0x").unwrap_or(&hex);
    u64::from_str_radix(trimmed, 16).map_err(|e| e.to_string())
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn self_tests_run_ethereum_rpc(
    rpc_url: String,
    rpc_label: String,
) -> Vec<ChainSelfTestResult> {
    let chain_id_result = fetch_eth_rpc_hex(&rpc_url, "eth_chainId", 1).await;
    let block_result = fetch_eth_rpc_hex(&rpc_url, "eth_blockNumber", 2).await;
    match (chain_id_result, block_result) {
        (Ok(chain_id), Ok(latest_block)) => {
            let chain_pass = chain_id == 1;
            vec![
                ChainSelfTestResult {
                    name: "ETH RPC Chain ID".to_string(),
                    passed: chain_pass,
                    chain_label: "Ethereum".to_string(),
                    outcome: ChainSelfTestOutcome::Custom {
                        text: if chain_pass {
                            "RPC reports Ethereum mainnet (chain id 1).".to_string()
                        } else {
                            format!(
                                "RPC returned chain id {chain_id}. Configure an Ethereum mainnet endpoint."
                            )
                        },
                    },
                },
                ChainSelfTestResult {
                    name: "ETH RPC Latest Block".to_string(),
                    passed: latest_block > 0,
                    chain_label: "Ethereum".to_string(),
                    outcome: ChainSelfTestOutcome::Custom {
                        text: if latest_block > 0 {
                            format!("RPC latest block height: {latest_block} via {rpc_label}.")
                        } else {
                            "RPC returned an invalid latest block value.".to_string()
                        },
                    },
                },
            ]
        }
        (chain_id, block) => {
            let detail = chain_id.err().or_else(|| block.err()).unwrap_or_default();
            vec![ChainSelfTestResult {
                name: "ETH RPC Health".to_string(),
                passed: false,
                chain_label: "Ethereum".to_string(),
                outcome: ChainSelfTestOutcome::Custom {
                    text: format!("RPC health check failed for {rpc_label}: {detail}"),
                },
            }]
        }
    }
}

#[uniffi::export]
pub fn self_tests_run_chain(chain_key: String) -> Vec<ChainSelfTestResult> {
    run_for_chain(&chain_key)
}

#[uniffi::export]
pub fn self_tests_run_all() -> HashMap<String, Vec<ChainSelfTestResult>> {
    let mut all: Vec<(&str, Vec<ChainSelfTestResult>)> = Vec::new();
    all.push(("Dogecoin", run_dogecoin()));
    all.push(("Ethereum", run_ethereum()));
    for spec in CHAIN_SPECS {
        all.push((spec.chain_key, run_spec(spec)));
    }
    all.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_tests_cover_all_chains() {
        let map = self_tests_run_all();
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

#[cfg(test)]
mod fixtures_are_real_tests {
    use super::*;

    /// Every self-test passes.
    ///
    /// Seven did not, all of them "<chain> Address Validation", for Bitcoin,
    /// Bitcoin Cash, Litecoin, Monero, Polkadot, Stellar and Internet Computer.
    /// The validators were right; the *fixtures* were hand-typed strings that
    /// looked like addresses and had invalid checksums — Bitcoin's was the
    /// BIP-173 vector with the last seven characters wrong. The replacements
    /// are derived by core from the standard test mnemonic, so they are correct
    /// by construction rather than by typing.
    ///
    /// This test is the reason that cannot recur: the self-tests are now
    /// themselves tested, so a bad fixture fails the build instead of showing
    /// a red row on a diagnostics screen nobody reads.
    #[test]
    fn every_self_test_passes() {
        let failures: Vec<String> = self_tests_run_all()
            .into_iter()
            .flat_map(|(chain, results)| {
                results
                    .into_iter()
                    .filter(|result| !result.passed)
                    .map(move |result| format!("{chain}: {}", result.name))
            })
            .collect();
        assert!(failures.is_empty(), "failing self-tests: {failures:#?}");
    }

    /// A self-test suite with no checks is a chain nobody is checking.
    #[test]
    fn every_chain_with_a_suite_actually_checks_something() {
        for (chain, results) in self_tests_run_all() {
            assert!(!results.is_empty(), "{chain} has an empty self-test suite");
        }
    }
}
