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

/// One chain's self-test fixtures.
///
/// `chain_key` is the registry's display name, which is what a caller can type
/// and what the map this builds is keyed by. `chain_label` used to sit beside
/// it, identical in all twenty rows.
struct ChainSpec {
    chain_key: &'static str,
    valid_address: &'static str,
    invalid_address: &'static str,
}

const CHAIN_SPECS: &[ChainSpec] = &[
    ChainSpec {
        chain_key: "Bitcoin",
        valid_address: "bc1qgkju4yvvtuz0s8vqn837q396jezu2h8ex7gk98",
        invalid_address: "bc1_not_valid",
    },
    ChainSpec {
        chain_key: "Bitcoin Cash",
        valid_address: "19GmUu4QnfGirbAxnpDviczXKZ8LVCvvq8",
        invalid_address: "bitcoincash:not_valid",
    },
    ChainSpec {
        chain_key: "Bitcoin SV",
        valid_address: "1MirQ9bwyQcGVJPwKUgapu5ouK2E2Ey4gX",
        invalid_address: "bsv_not_valid",
    },
    ChainSpec {
        chain_key: "Litecoin",
        valid_address: "LZHamZCxNf71EmnHgUkztqMyaWyBc5nrkb",
        invalid_address: "ltc_not_valid",
    },
    ChainSpec {
        chain_key: "Cardano",
        valid_address: "addr1q9d6m0vxj4j6f0r2k6zk6n6w6r0v9x9k5n0d5u7r3q8v9w7c5m0h2g8t7u6k5a4s3d2f1g0h9j8k7l6m5n4p3q2r1s",
        invalid_address: "addr_not_valid",
    },
    ChainSpec {
        chain_key: "Solana",
        valid_address: "Vote111111111111111111111111111111111111111",
        invalid_address: "sol_not_valid",
    },
    ChainSpec {
        chain_key: "Stellar",
        valid_address: "GAFOIIMIXWLSN66RYL32JHCI7AMKWZ3TYYSZTOXSTLPHJWPMZDERMXUO",
        invalid_address: "stellar_not_valid",
    },
    ChainSpec {
        chain_key: "XRP Ledger",
        valid_address: "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh",
        invalid_address: "xrp_not_valid",
    },
    ChainSpec {
        chain_key: "Tron",
        valid_address: "TNPeeaaFB7K9cmo4uQpcU32zGK8G1NYqeL",
        invalid_address: "tron_not_valid",
    },
    ChainSpec {
        chain_key: "Sui",
        valid_address: "0x5f1e6bc4b4f4d7e4d4b5e7a6c3b2a1f0e9d8c7b6a5f4e3d2c1b0a9876543210f",
        invalid_address: "0xnotvalid",
    },
    ChainSpec {
        chain_key: "Aptos",
        valid_address: "0x1",
        invalid_address: "aptos_not_valid",
    },
    ChainSpec {
        chain_key: "TON",
        valid_address: "UQBm--PFwDv1yCeS-QTJ-L8oiUpqo9IT1BwgVptlSq3ts4DV",
        invalid_address: "ton_not_valid",
    },
    ChainSpec {
        chain_key: "Internet Computer",
        valid_address: "3d67a090082c446abb79b91cfa4937cb69256d23b23c72d6fa0461e62d8b3fe3",
        invalid_address: "icp_not_valid",
    },
    ChainSpec {
        chain_key: "NEAR",
        valid_address: "example.near",
        invalid_address: "-not-valid.near",
    },
    ChainSpec {
        chain_key: "Polkadot",
        valid_address: "13DyfGHEWo6GF98AxoBbovBHy82rrr4H3LrWaxggtEpgku8o",
        invalid_address: "dot_not_valid",
    },
    ChainSpec {
        chain_key: "Monero",
        valid_address: "46pWvmHcWgbDZDhzkgqMN52rq4tJZGZv26qDTiZW4Jg21tqEyrDaQMjVVACuC59gc9Ma3LM9CqD44Cn8XVqjAnPxEnP1PrZ",
        invalid_address: "xmr_not_valid",
    },
    ChainSpec {
        chain_key: "BNB Chain",
        valid_address: "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
        invalid_address: "0x_not_valid",
    },
    ChainSpec {
        chain_key: "Avalanche",
        valid_address: "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
        invalid_address: "0x_not_valid",
    },
    ChainSpec {
        chain_key: "Ethereum Classic",
        valid_address: "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
        invalid_address: "0x_not_valid",
    },
    ChainSpec {
        chain_key: "Hyperliquid",
        valid_address: "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
        invalid_address: "0x_not_valid",
    },
];

fn validate(kind: &str, value: &str) -> bool {
    validate_address(AddressValidationRequest {
        kind: kind.to_string(),
        value: value.to_string(),
    })
    .is_valid
}

fn spec_chain(spec: &ChainSpec) -> Option<crate::registry::Chain> {
    crate::registry::Chain::from_display_name(spec.chain_key)
}

fn spec_address_kind(spec: &ChainSpec) -> &'static str {
    spec_chain(spec)
        .map(|chain| chain.address_validation_kind())
        .unwrap_or("")
}

fn run_address_accepts(spec: &ChainSpec) -> ChainSelfTestResult {
    let passed = validate(spec_address_kind(spec), spec.valid_address);
    ChainSelfTestResult {
        name: format!("{} Address Validation", spec.chain_key),
        passed,
        chain_label: spec.chain_key.to_string(),
        outcome: if passed {
            ChainSelfTestOutcome::ValidAddressAccepted
        } else {
            ChainSelfTestOutcome::ValidAddressRejected
        },
    }
}

fn run_address_rejects(spec: &ChainSpec) -> ChainSelfTestResult {
    let passed = !validate(spec_address_kind(spec), spec.invalid_address);
    ChainSelfTestResult {
        name: format!("{} Address Rejects Invalid", spec.chain_key),
        passed,
        chain_label: spec.chain_key.to_string(),
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
    let chain = spec_chain(spec)?;
    let derivation_chain = crate::send::flow::seed_derivation_chain_raw(chain)?;
    let derivation_path = crate::app_core::default_path_from_catalog(spec.chain_key).ok()?;
    let derived = derive_one(&derivation_chain, &derivation_path);
    let name = format!("{} Seed Derivation", spec.chain_key);
    let Some(address) = derived else {
        return Some(ChainSelfTestResult {
            name,
            passed: false,
            chain_label: spec.chain_key.to_string(),
            outcome: ChainSelfTestOutcome::DerivationFailed,
        });
    };
    let passed = validate(spec_address_kind(spec), &address);
    Some(ChainSelfTestResult {
        name,
        passed,
        chain_label: spec.chain_key.to_string(),
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
mod fixtures_are_real_tests {
    use super::*;

    /// A suite for a chain that derives includes the derivation check.
    ///
    /// The three columns this test replaces — `address_kind`,
    /// `derivation_chain` and `derivation_path` — were the row restating facts
    /// the registry holds. Nineteen of twenty agreed; Monero's
    /// `derivation_chain: None` said "this chain does not derive", which was
    /// true when it was written and stopped being true when
    /// `uses_derivation_path` landed. So Monero ran two checks where every
    /// other chain ran three, and nothing said so. Reading the registry means
    /// the stale answer cannot survive the fact changing.
    #[test]
    fn a_chain_that_derives_has_a_derivation_self_test() {
        for spec in CHAIN_SPECS {
            let chain = crate::registry::Chain::from_display_name(spec.chain_key)
                .expect("keyed by a name the registry knows");
            if crate::send::flow::seed_derivation_chain_raw(chain).is_none() {
                continue;
            }
            let names: Vec<String> = run_spec(spec).into_iter().map(|r| r.name).collect();
            assert!(
                names.iter().any(|n| n.ends_with("Seed Derivation")),
                "{} derives and has no derivation self-test; it has {names:?}",
                spec.chain_key
            );
        }
    }

    /// Every suite is reachable by a name a caller can type.
    ///
    /// `CHAIN_SPECS` is keyed by chain name and the map it builds is what both
    /// front ends look a chain up in — and every caller resolves its input
    /// through the registry first. One row was keyed `"XRP"`, which the registry
    /// spells `"XRP Ledger"`, so `spectra diagnostics self-test --chain "XRP
    /// Ledger"` answered *"XRP Ledger has no self-tests"* and no spelling
    /// reached the suite. `every_self_test_passes` walks the map directly, so it
    /// ran those tests and passed — a suite can be green and unreachable at the
    /// same time, and only this asserts it is not.
    #[test]
    fn every_self_test_suite_is_keyed_by_a_name_the_registry_knows() {
        for chain_key in self_tests_run_all().keys() {
            assert!(
                crate::registry::Chain::from_display_name(chain_key).is_some(),
                "{chain_key} keys a self-test suite and is not a chain the registry knows, \
                 so nothing can ask for it"
            );
        }
    }

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
