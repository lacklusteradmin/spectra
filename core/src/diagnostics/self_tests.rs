use crate::api::error::{ApiError, OrDecode};
use crate::registry::Chain;
use crate::validation::address::{AddressValidationRequest, validate_address};
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

fn validate(kind: &str, value: &str) -> bool {
    validate_address(AddressValidationRequest {
        kind: kind.to_string(),
        value: value.to_string(),
    })
    .is_valid
}

/// Derive the fixture from the network's catalog path so derivation and
/// validation are checked against the same address.
fn derive_one(chain: crate::registry::Chain, path: &str) -> Option<String> {
    crate::derivation::dispatch::derive_for_chain(
        chain,
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

/// `@` is outside every address alphabet. Truncation is insufficient because
/// short Aptos addresses and NEAR account names can still be valid.
const IMPOSSIBLE_ADDRESS: &str = "@@not-an-address@@";

fn result(
    chain: crate::registry::Chain,
    suffix: &str,
    passed: bool,
    yes: ChainSelfTestOutcome,
    no: ChainSelfTestOutcome,
) -> ChainSelfTestResult {
    ChainSelfTestResult {
        name: format!("{} {suffix}", chain.chain_display_name()),
        passed,
        chain_label: chain.chain_display_name().to_string(),
        outcome: if passed { yes } else { no },
    }
}

fn run_for_chain(chain: crate::registry::Chain) -> Vec<ChainSelfTestResult> {
    let kind = chain.address_validation_kind();
    let mut results = Vec::new();

    if crate::send::flow::seed_derivation_chain_raw(chain).is_none() {
        return results;
    }
    // Derive for the exact network, using its own path and address format.
    let Ok(path) = crate::derivation::path::default_path_from_catalog(chain) else {
        return results;
    };
    let Some(address) = derive_one(chain, &path) else {
        results.push(result(
            chain,
            "Seed Derivation",
            false,
            ChainSelfTestOutcome::DerivationFailed,
            ChainSelfTestOutcome::DerivationFailed,
        ));
        return results;
    };

    let accepted = validate(kind, &address);
    results.push(result(
        chain,
        "Seed Derivation",
        accepted,
        ChainSelfTestOutcome::DerivedAddressValid,
        ChainSelfTestOutcome::DerivedAddressInvalid,
    ));
    results.push(result(
        chain,
        "Address Validation",
        accepted,
        ChainSelfTestOutcome::ValidAddressAccepted,
        ChainSelfTestOutcome::ValidAddressRejected,
    ));

    let rejected = !validate(kind, IMPOSSIBLE_ADDRESS);
    results.push(result(
        chain,
        "Address Rejects Invalid",
        rejected,
        ChainSelfTestOutcome::InvalidAddressRejected,
        ChainSelfTestOutcome::InvalidAddressUnexpectedlyAccepted,
    ));

    // Where a chain folds addresses to a canonical form, the derived address
    // must already be in it — otherwise a receive address and the same address
    // typed back in are two different strings.
    if chain.address_normalization() != crate::registry::AddressNormalization::None {
        let normalized = validate_address(AddressValidationRequest {
            kind: kind.to_string(),
            value: address.clone(),
        })
        .normalized_value
        .map(|v| v == crate::send::flow::normalize_address(chain, &address))
        .unwrap_or(false);
        results.push(result(
            chain,
            "Receive Address Normalization",
            normalized,
            ChainSelfTestOutcome::NormalizationSuccess,
            ChainSelfTestOutcome::NormalizationFailure,
        ));
    }
    results
}

async fn fetch_eth_rpc_hex(url: &str, method: &str) -> Result<u64, ApiError> {
    let client = crate::api::evm_json_rpc::EvmClient::new(std::sync::Arc::new(vec![url.into()]), 0);
    let result = client.call(method, serde_json::json!([])).await?;
    crate::api::evm_json_rpc::parse_hex_u64(result.as_str().or_decode("expected a hex quantity")?)
}

/// Check that an endpoint serves the chain it was configured for, and that it
/// has a block height.
///
/// The id to expect is `Chain::evm_chain_id`, so every EVM chain can be asked
/// whether the node it is pointed at is the node it means.
pub(crate) async fn self_tests_run_evm_rpc(
    chain_id: String,
    rpc_url: String,
    rpc_label: String,
) -> Vec<ChainSelfTestResult> {
    let Some(chain) = crate::registry::Chain::from_str_id(&chain_id).filter(|c| c.is_evm()) else {
        return vec![ChainSelfTestResult {
            name: "RPC Chain ID".to_string(),
            passed: false,
            chain_label: chain_id.clone(),
            outcome: ChainSelfTestOutcome::Custom {
                text: format!("{chain_id} is not an EVM chain, so it has no JSON-RPC chain id."),
            },
        }];
    };
    let label = chain.chain_display_name();
    let expected = chain.evm_chain_id().expect("EVM chain filtered above");
    let reported = fetch_eth_rpc_hex(&rpc_url, "eth_chainId").await;
    let block = fetch_eth_rpc_hex(&rpc_url, "eth_blockNumber").await;
    match (reported, block) {
        (Ok(reported), Ok(latest_block)) => vec![
            ChainSelfTestResult {
                name: "RPC Chain ID".to_string(),
                passed: reported == expected,
                chain_label: label.to_string(),
                outcome: ChainSelfTestOutcome::Custom {
                    text: if reported == expected {
                        format!("RPC reports {label} (chain id {expected}).")
                    } else {
                        format!(
                            "RPC returned chain id {reported}, not {label}'s {expected}. Configure a {label} endpoint."
                        )
                    },
                },
            },
            ChainSelfTestResult {
                name: "RPC Latest Block".to_string(),
                passed: latest_block > 0,
                chain_label: label.to_string(),
                outcome: ChainSelfTestOutcome::Custom {
                    text: if latest_block > 0 {
                        format!("RPC latest block height: {latest_block} via {rpc_label}.")
                    } else {
                        "RPC returned an invalid latest block value.".to_string()
                    },
                },
            },
        ],
        (reported, block) => {
            let detail = reported
                .err()
                .or_else(|| block.err())
                .map(|e| e.to_string())
                .unwrap_or_default();
            vec![ChainSelfTestResult {
                name: "RPC Health".to_string(),
                passed: false,
                chain_label: label.to_string(),
                outcome: ChainSelfTestOutcome::Custom {
                    text: format!("RPC health check failed for {rpc_label}: {detail}"),
                },
            }]
        }
    }
}

pub fn self_tests_run_chain(chain: crate::registry::Chain) -> Vec<ChainSelfTestResult> {
    run_for_chain(chain)
}

#[uniffi::export]
pub fn self_tests_run_all() -> HashMap<Chain, Vec<ChainSelfTestResult>> {
    Chain::all()
        .map(|chain| (chain, run_for_chain(chain)))
        .filter(|(_, results)| !results.is_empty())
        .collect()
}

#[cfg(test)]
mod fixtures_are_real_tests {
    use super::*;
    use std::collections::HashSet;

    /// A non-EVM chain is refused before any network request.
    #[tokio::test]
    async fn only_an_evm_chain_has_a_json_rpc_id_to_check() {
        let refused = self_tests_run_evm_rpc(
            "bitcoin".into(),
            "http://127.0.0.1:1/".into(),
            "unused".into(),
        )
        .await;
        assert_eq!(refused.len(), 1);
        assert!(!refused[0].passed);
        assert!(matches!(
            &refused[0].outcome,
            ChainSelfTestOutcome::Custom { text } if text.contains("not an EVM chain")
        ));
        // Every EVM chain has one, which is what the caller passes.
        for chain in crate::registry::Chain::all().filter(|c| c.is_evm()) {
            assert!(
                chain.evm_chain_id().is_ok_and(|id| id > 0),
                "{}",
                chain.str_id()
            );
        }
    }

    #[test]
    fn public_suites_cover_every_derivable_chain() {
        let expected: HashSet<_> = Chain::all()
            .filter(|&chain| {
                crate::send::flow::seed_derivation_chain_raw(chain).is_some()
                    && crate::derivation::path::default_path_from_catalog(chain).is_ok()
            })
            .collect();
        let suites = self_tests_run_all();
        assert_eq!(suites.keys().copied().collect::<HashSet<_>>(), expected);
        for (chain, results) in suites {
            assert!(
                results.iter().any(|result| matches!(
                    result.outcome,
                    ChainSelfTestOutcome::DerivedAddressValid
                        | ChainSelfTestOutcome::DerivedAddressInvalid
                        | ChainSelfTestOutcome::DerivationFailed
                )),
                "{chain} has no seed derivation self-test"
            );
            assert!(
                results.iter().any(|result| matches!(
                    result.outcome,
                    ChainSelfTestOutcome::InvalidAddressRejected
                        | ChainSelfTestOutcome::InvalidAddressUnexpectedlyAccepted
                )),
                "{chain} has no invalid-address self-test"
            );
        }
    }

    /// Mainnet and testnet derivation must dispatch to their own validators.
    #[test]
    fn a_chain_accepts_the_address_it_derives() {
        use crate::registry::Chain;
        use crate::validation::address::{AddressValidationRequest, validate_address};

        for chain in Chain::all() {
            if crate::send::flow::seed_derivation_chain_raw(chain).is_none() {
                continue;
            }
            let Ok(path) = crate::derivation::path::default_path_from_catalog(chain) else {
                continue;
            };
            let Some(address) = derive_one(chain, &path) else {
                continue;
            };
            assert!(
                validate_address(AddressValidationRequest {
                    kind: chain.address_validation_kind().to_string(),
                    value: address.clone(),
                })
                .is_valid,
                "{} derives {address} and its own validator refuses it",
                chain.str_id()
            );
        }
    }

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
}
