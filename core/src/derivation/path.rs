//! BIP-32 derivation paths: parsing, formatting and the catalog defaults.

use crate::derivation::error::DerivationError;

use crate::chains::DerivationProfile;
use crate::registry::Chain;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
pub struct DerivationPathSegment {
    pub value: u32,
    pub is_hardened: bool,
}

/// The derivation path a wallet on `chain` will use: the caller's, normalized,
/// or the chain's catalog default when the caller named none. A path that
/// does not parse for the chain is refused rather than replaced by the
/// default, which would sign with a key the wallet never named.
pub(crate) fn resolve_derivation_path(
    chain: Chain,
    derivation_path: String,
) -> Result<String, crate::SpectraBridgeError> {
    Ok(import_derivation_path(chain, Some(&derivation_path))?.unwrap_or_default())
}

/// A chain's derivation profiles, default first, each with the template its
/// account index fills in. Empty where the chain derives without a path.
#[uniffi::export]
pub fn derivation_profiles(chain: Chain) -> Vec<DerivationProfileOption> {
    chain
        .derivation_profiles()
        .into_iter()
        .map(|profile| DerivationProfileOption {
            profile,
            path: chain
                .derivation_profile_path(profile, 0)
                .expect("a listed profile has a path"),
        })
        .collect()
}

/// One profile a chain offers, with its path at account 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct DerivationProfileOption {
    pub profile: DerivationProfile,
    pub path: String,
}

/// The path `profile` derives at `account` on `chain`. Refuses a profile the
/// chain does not offer and an index past the hardened range, rather than
/// substituting the default.
#[uniffi::export]
pub fn derivation_profile_path(
    chain: Chain,
    profile: DerivationProfile,
    account: u32,
) -> Result<String, crate::SpectraBridgeError> {
    chain
        .derivation_profile_path(profile, account)
        .ok_or_else(|| {
            DerivationError::refused(
                "%@ has no such derivation profile or account.",
                [chain.chain_display_name()],
            )
            .into()
        })
}

/// Which profile and account `path` is on `chain`, or `None` for a path no
/// profile derives: a custom one.
#[uniffi::export]
pub fn derivation_profile_of_path(chain: Chain, path: String) -> Option<DerivationProfileChoice> {
    let segments = parse_derivation_path_str(&path)?;
    chain.derivation_profiles().into_iter().find_map(|profile| {
        let (template, account_at) = profile_template(chain, profile)?;
        let matches =
            segments.len() == template.len()
                && segments[account_at].is_hardened
                && segments.iter().zip(&template).enumerate().all(
                    |(position, (segment, expected))| position == account_at || segment == expected,
                );
        matches.then(|| DerivationProfileChoice {
            profile,
            account: segments[account_at].value,
        })
    })
}

/// A profile and the account index on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct DerivationProfileChoice {
    pub profile: DerivationProfile,
    pub account: u32,
}

/// `profile`'s path at account 0 on `chain`, and where its account index
/// sits: the one segment that differs between accounts 0 and 1.
fn profile_template(
    chain: Chain,
    profile: DerivationProfile,
) -> Option<(Vec<DerivationPathSegment>, usize)> {
    let zero = parse_derivation_path_str(&chain.derivation_profile_path(profile, 0)?)?;
    let one = parse_derivation_path_str(&chain.derivation_profile_path(profile, 1)?)?;
    let mut differing = zero
        .iter()
        .zip(&one)
        .enumerate()
        .filter(|(_, (a, b))| a != b);
    let (position, _) = differing.next()?;
    differing.next().is_none().then_some((zero, position))
}

#[uniffi::export]
pub fn parse_derivation_path(raw_path: String) -> Option<Vec<DerivationPathSegment>> {
    parse_derivation_path_str(&raw_path)
}

#[uniffi::export]
pub fn format_derivation_path(segments: Vec<DerivationPathSegment>) -> String {
    format_derivation_path_segments(&segments)
}

pub(crate) fn parse_derivation_path_str(raw_path: &str) -> Option<Vec<DerivationPathSegment>> {
    let trimmed = raw_path.trim();
    let mut components = trimmed.split('/');
    let head = components.next()?;
    if !head.eq_ignore_ascii_case("m") {
        return None;
    }
    components
        .map(|component| {
            let is_hardened = component.ends_with('\'');
            let value_string = if is_hardened {
                &component[..component.len().saturating_sub(1)]
            } else {
                component
            };
            value_string
                .parse::<u32>()
                .ok()
                .filter(|value| *value < (1 << 31))
                .map(|value| DerivationPathSegment { value, is_hardened })
        })
        .collect()
}

pub(crate) fn normalize_derivation_path(raw_path: &str, fallback: &str) -> String {
    parse_derivation_path_str(raw_path)
        .map(|segments| format_derivation_path_segments(&segments))
        .unwrap_or_else(|| fallback.to_string())
}

pub(crate) fn format_derivation_path_segments(segments: &[DerivationPathSegment]) -> String {
    let suffix = segments
        .iter()
        .map(|segment| {
            format!(
                "{}{}",
                segment.value,
                if segment.is_hardened { "'" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join("/");
    if suffix.is_empty() {
        "m".to_string()
    } else {
        format!("m/{suffix}")
    }
}

fn render_derivation_path_template(template: &str, account: u32) -> String {
    template.replace("{account}", &account.to_string())
}

/// The path an import stores on `chain`: the one chosen, normalized, or the
/// chain's default profile at account 0 when none was. A Substrate chain
/// takes junctions (`substrate_path`), none being its root key. A chain that
/// derives without a path refuses one rather than ignoring it, and a path
/// that does not parse is refused rather than replaced by the default.
pub(crate) fn import_derivation_path(
    chain: Chain,
    requested: Option<&str>,
) -> Result<Option<String>, DerivationError> {
    let requested = requested.map(str::trim).filter(|path| !path.is_empty());
    if chain.derives_along_junctions() {
        return requested.map_or(Ok(None), super::substrate_path::normalized);
    }
    if !chain.uses_derivation_path() {
        return match requested {
            None => Ok(None),
            Some(_) => Err(DerivationError::refused(
                "%@ derives without a derivation path.",
                [chain.chain_display_name()],
            )),
        };
    }
    match requested {
        None => default_path_from_catalog(chain).map(Some),
        Some(path) => parse_derivation_path_str(path)
            .map(|segments| Some(format_derivation_path_segments(&segments)))
            .ok_or_else(|| DerivationError::refused("Not a derivation path: %@", [path])),
    }
}

pub(crate) fn default_path_from_catalog(chain: Chain) -> Result<String, DerivationError> {
    if let Some(template) = crate::chains::default_derivation_path_template(chain) {
        return Ok(render_derivation_path_template(template, 0));
    }
    // A chain the registry knows and the catalog gives no path for derives
    // without one — that is what `derivation_path = []` says, and Monero is
    // the mainnet that says it. That is an answer, not a broken catalog row,
    // so it is not an error.
    if chain.uses_derivation_path() {
        Err(DerivationError::Invalid(
            format!("Missing default derivation path for {chain}.").into(),
        ))
    } else {
        Ok(String::new())
    }
}

/// Extract a UTXO discovery index from a catalog-supported path and account.
/// Receive/change indices must be non-hardened and belong to the requested branch.
pub(crate) fn utxo_discovery_index(raw_path: &str, chain: Chain, branch: u32) -> Option<u32> {
    let path = parse_derivation_path_str(raw_path)?;
    if path.len() < 5 || branch > 1 {
        return None;
    }
    let last = path.len() - 1;
    if path[last - 1].is_hardened || path[last].is_hardened || path[last - 1].value != branch {
        return None;
    }
    chain.entry().derivation_path.iter().find_map(|template| {
        let candidate = parse_derivation_path_str(&render_derivation_path_template(
            &template.path,
            path[2].value,
        ))?;
        (path.len() == candidate.len() && candidate[..last - 1] == path[..last - 1])
            .then_some(path[last].value)
    })
}

/// A discovery path: the chain's default path with its last two segments
/// replaced by branch and index.
pub(crate) fn derivation_path_replacing_last_two(
    raw_path: String,
    branch: u32,
    index: u32,
    fallback: String,
) -> String {
    let normalized = normalize_derivation_path(&raw_path, &fallback);
    let Some(mut segments) = parse_derivation_path_str(&normalized) else {
        return fallback;
    };
    if segments.len() < 2 {
        return fallback;
    }
    let len = segments.len();
    segments[len - 2] = DerivationPathSegment {
        value: branch,
        is_hardened: false,
    };
    segments[len - 1] = DerivationPathSegment {
        value: index,
        is_hardened: false,
    };
    format_derivation_path_segments(&segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "This chain has no derivation path" is an answer, not a failure.
    #[test]
    fn a_chain_with_no_catalog_path_derives_without_one() {
        assert!(!Chain::Monero.uses_derivation_path());
        assert_eq!(
            default_path_from_catalog(Chain::Monero).expect("an answer"),
            ""
        );

        // These derive without a path by their own schemes — Monero's keys,
        // TON's mnemonic, Substrate's mini-secret — so a catalog path there
        // would be one the deriver ignores. Any other mainnet without one is
        // a catalog edit to notice rather than a silent empty path.
        let pathless = [Chain::Monero, Chain::Ton, Chain::Polkadot, Chain::Bittensor];
        for chain in Chain::all().filter(|c| !c.is_testnet()) {
            assert_eq!(
                chain.uses_derivation_path(),
                !pathless.contains(&chain),
                "{}",
                chain.str_id()
            );
            assert_eq!(
                chain.derivation_profiles().is_empty(),
                !chain.uses_derivation_path()
            );
        }
    }

    /// A profile's account index moves exactly one hardened segment, and the
    /// path reads back as the profile and account it came from.
    #[test]
    fn every_profile_path_reads_back_as_its_profile_and_account() {
        for chain in Chain::all() {
            for profile in chain.derivation_profiles() {
                for account in [0, 1, 7] {
                    let path = derivation_profile_path(chain, profile, account).unwrap();
                    assert_eq!(
                        derivation_profile_of_path(chain, path.clone()),
                        Some(DerivationProfileChoice { profile, account }),
                        "{chain} {path}"
                    );
                }
            }
            assert!(derivation_profile_of_path(chain, "m/1'/2'/3'".into()).is_none());
        }
        assert_eq!(
            derivation_profile_path(Chain::Bitcoin, DerivationProfile::Taproot, 2).unwrap(),
            "m/86'/0'/2'/0/0"
        );
        assert_eq!(
            derivation_profile_path(Chain::Solana, DerivationProfile::Legacy, 1).unwrap(),
            "m/44'/501'/1'"
        );
        // Refused, not substituted: a profile the chain lacks, and an
        // account past the hardened range.
        assert!(derivation_profile_path(Chain::Ethereum, DerivationProfile::Taproot, 0).is_err());
        assert!(
            derivation_profile_path(Chain::Bitcoin, DerivationProfile::Legacy, 1 << 31).is_err()
        );
        assert!(derivation_profiles(Chain::Monero).is_empty());
        assert_eq!(
            derivation_profiles(Chain::Bitcoin)[0].profile,
            DerivationProfile::NativeSegWit
        );
    }

    /// Every profile the registry lists derives, at accounts 0 and 1, the
    /// address an independent implementation of the chain's wallets does
    /// (`derivation-profiles.json`, from
    /// scripts/generate-derivation-profile-vectors.cjs) — and the fixture
    /// names no profile the registry lacks.
    #[test]
    fn every_profile_matches_independent_vectors() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/derivation-profiles.json"
        ))
        .unwrap();
        let phrase = fixture["phrase"].as_str().unwrap();
        let vectors = fixture["vectors"].as_array().unwrap();
        let mut listed = std::collections::HashSet::new();
        for chain in Chain::all() {
            for profile in chain.derivation_profiles() {
                for account in [0, 1] {
                    let path = derivation_profile_path(chain, profile, account).unwrap();
                    let vector = vectors
                        .iter()
                        .find(|v| {
                            v["chain"] == chain.str_id()
                                && v["profile"] == serde_json::to_value(profile).unwrap()
                                && v["account"] == account
                        })
                        .unwrap_or_else(|| panic!("no vector for {chain} {profile:?} {account}"));
                    assert_eq!(vector["path"], path.as_str(), "{chain} {profile:?}");
                    let derived = crate::derivation::dispatch::derive_for_chain(
                        chain, phrase, &path, None, None, None, true, false, false,
                    )
                    .unwrap()
                    .address
                    .unwrap();
                    // The address as an import stores it.
                    let stored =
                        crate::derivation::import::normalized_import_address(chain, &derived)
                            .unwrap();
                    // EVM addresses are stored lowercase; the vector carries
                    // EIP-55 casing.
                    let expected = vector["address"].as_str().unwrap();
                    let expected = if chain.is_evm() {
                        expected.to_ascii_lowercase()
                    } else {
                        expected.to_string()
                    };
                    assert_eq!(expected, stored, "{chain} {path}");
                    listed.insert((chain.str_id(), account, path));
                }
            }
        }
        assert_eq!(
            listed.len(),
            vectors.len(),
            "the fixture lists profiles the registry does not"
        );
    }

    #[test]
    fn resolves_bitcoin_taproot_path() {
        let default_path = default_path_from_catalog(Chain::Bitcoin).expect("default path");
        let normalized = normalize_derivation_path("m/86'/0'/2'/0/0", &default_path);
        assert_eq!(normalized, "m/86'/0'/2'/0/0");
    }

    /// Every network resolves independently, including pathless derivation.
    #[test]
    fn every_testnet_resolves_its_own_catalog_path() {
        for chain in Chain::all().filter(|c| c.is_testnet()) {
            let resolved = resolve_derivation_path(chain, String::new());
            assert!(
                resolved.is_ok(),
                "{} failed to resolve: {:?}",
                chain.str_id(),
                resolved.err()
            );
        }
    }

    #[test]
    fn bitcoin_testnet_uses_coin_type_one() {
        let testnet =
            resolve_derivation_path(Chain::BitcoinTestnet4, String::new()).expect("testnet4");
        let mainnet = resolve_derivation_path(Chain::Bitcoin, String::new()).expect("bitcoin");
        assert_eq!(testnet, "m/84'/1'/0'/0/0");
        assert_eq!(mainnet, "m/84'/0'/0'/0/0");
    }

    #[test]
    fn utxo_indices_cover_catalog_script_paths_and_accounts() {
        for (chain, coin) in [(Chain::Litecoin, 2), (Chain::LitecoinTestnet, 1)] {
            for purpose in [44, 49, 84] {
                for account in [0, 2] {
                    for branch in [0, 1] {
                        assert_eq!(
                            utxo_discovery_index(
                                &format!("m/{purpose}'/{coin}'/{account}'/{branch}/17"),
                                chain,
                                branch
                            ),
                            Some(17)
                        );
                    }
                }
            }
        }
        for path in [
            "m/84'/0'/0'/0/17",
            "m/86'/2'/0'/0/17",
            "m/84'/2'/0'/0'/17",
            "m/84'/2'/0'/0/17'",
            "m/84'/2'/0'/1/17",
        ] {
            assert_eq!(
                utxo_discovery_index(path, Chain::Litecoin, 0),
                None,
                "{path}"
            );
        }
    }
}
