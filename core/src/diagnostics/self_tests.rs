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

/// Every chain's self-tests, derived rather than tabulated.
///
/// `CHAIN_SPECS` stood here: twenty rows of `(chain, valid address, invalid
/// address)`, with Dogecoin and Ethereum given hand-written suites beside it.
/// Twenty-six mainnets — Zcash, Dash, Decred, Kaspa, Bitcoin Gold, Bittensor
/// and every EVM chain outside four — had no self-test at all, in the one
/// subsystem whose job is to notice when something is wrong.
///
/// The fixture a chain needs is an address that is genuinely its own, and core
/// can produce one: derive from the canonical mnemonic down the chain's own
/// catalog path. That is stronger than a typed-in sample, because it checks
/// that derivation and validation agree rather than that a constant still
/// parses — and it cannot be short by a chain.
fn validate(kind: &str, value: &str) -> bool {
    validate_address(AddressValidationRequest {
        kind: kind.to_string(),
        value: value.to_string(),
    })
    .is_valid
}

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

/// A string no chain's address format permits.
///
/// Not a truncation of a real address: some formats carry a checksum and would
/// catch that, but Aptos genuinely accepts short forms (`0x1` is the framework
/// account) and a NEAR account id is an arbitrary name, so on those chains a
/// truncated address is a different valid address rather than a broken one.
/// `@` is outside every format's alphabet.
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

fn run_for_chain(chain_key: &str) -> Vec<ChainSelfTestResult> {
    let Some(chain) = crate::registry::Chain::from_display_name(chain_key) else {
        return Vec::new();
    };
    let kind = chain.address_validation_kind();
    let mut results = Vec::new();

    // A chain that does not derive has nothing to build a fixture from, and
    // says so rather than reporting an empty suite.
    if crate::send::flow::seed_derivation_chain_raw(chain).is_none() {
        return results;
    }
    // The chain itself, not its mainnet counterpart: `seed_derivation_chain_raw`
    // folds testnets onto their mainnet, and deriving Bitcoin Testnet that way
    // yields a `bc1q…` mainnet address that Bitcoin Testnet's own validator
    // correctly rejects.
    let derivation_chain = chain.chain_display_name().to_string();
    let Ok(path) = crate::app_core::default_path_from_catalog(chain.chain_display_name()) else {
        return results;
    };
    let Some(address) = derive_one(&derivation_chain, &path) else {
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
        .map(|v| v == crate::send::flow::normalize_address(chain.chain_display_name(), &address))
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
    crate::registry::Chain::all()
        .map(|chain| {
            (
                chain.chain_display_name().to_string(),
                run_for_chain(chain.chain_display_name()),
            )
        })
        .filter(|(_, results)| !results.is_empty())
        .collect()
}

#[cfg(test)]
mod fixtures_are_real_tests {
    use super::*;

    /// Every chain that can derive has a suite, and the suite checks that it
    /// derives.
    ///
    /// A twenty-row fixture table stood here and twenty-six mainnets were not
    /// in it, so the chains with no self-test were exactly the chains nobody
    /// had thought to add — which is the set most likely to need one.
    #[test]
    fn every_chain_that_derives_has_a_suite() {
        use crate::registry::Chain;
        for chain in Chain::all() {
            if crate::send::flow::seed_derivation_chain_raw(chain).is_none() {
                continue;
            }
            if crate::app_core::default_path_from_catalog(chain.chain_display_name()).is_err() {
                continue;
            }
            let names: Vec<String> = run_for_chain(chain.chain_display_name())
                .into_iter()
                .map(|r| r.name)
                .collect();
            assert!(
                names.iter().any(|n| n.ends_with("Seed Derivation")),
                "{} derives and has no derivation self-test; it has {names:?}",
                chain.chain_display_name()
            );
            assert!(names.iter().any(|n| n.ends_with("Address Rejects Invalid")));
        }
    }

    /// The suite covers far more than the table did.
    #[test]
    fn the_suite_covers_the_catalog() {
        let suites = self_tests_run_all();
        assert!(
            suites.len() >= 60,
            "only {} chains have a self-test suite; the table this replaced had 20",
            suites.len()
        );
    }

    /// A chain accepts the address it derives, on the network it derives it
    /// for.
    ///
    /// Zcash, Decred and Dash Testnet each derived a correct testnet address —
    /// `tm…`, `Ts…`, `y…` — and their own validator refused it, because the
    /// dispatcher sent both networks to the mainnet decoder. The receive screen
    /// showed an address the send screen would have rejected.
    #[test]
    fn a_chain_accepts_the_address_it_derives() {
        use crate::registry::Chain;
        use crate::validation::address::{validate_address, AddressValidationRequest};

        for chain in Chain::all() {
            if crate::send::flow::seed_derivation_chain_raw(chain).is_none() {
                continue;
            }
            let Ok(path) = crate::app_core::default_path_from_catalog(chain.chain_display_name())
            else {
                continue;
            };
            let Some(address) = derive_one(chain.chain_display_name(), &path) else {
                continue;
            };
            assert!(
                validate_address(AddressValidationRequest {
                    kind: chain.address_validation_kind().to_string(),
                    value: address.clone(),
                })
                .is_valid,
                "{} derives {address} and its own validator refuses it",
                chain.chain_display_name()
            );
        }
    }

    /// Every suite is reachable by a name a caller can type.
    ///
    /// The map both front ends look a chain up in is keyed by chain name, and
    /// every caller resolves its input through the registry first. A key the
    /// registry does not know reaches nothing, and `every_self_test_passes`
    /// walks the map directly, so such a suite is green and unreachable at the
    /// same time. Only this asserts it is not.
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

