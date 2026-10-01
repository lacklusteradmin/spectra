//! BIP-32 derivation paths: parsing, formatting and the catalog defaults.

use crate::derivation::error::DerivationError;

use crate::registry::Chain;
use crate::store::wallet_domain::{CoreSeedDerivationPaths, CoreSeedDerivationPreset};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
pub struct DerivationPathSegment {
    pub value: u32,
    pub is_hardened: bool,
}

/// The derivation path a wallet on `chain` will use: the caller's, normalized,
/// or the chain's catalog default when the caller named none.
#[uniffi::export]
pub fn resolve_derivation_path(
    chain: Chain,
    derivation_path: String,
) -> Result<String, crate::SpectraBridgeError> {
    let default_path = default_path_from_catalog(chain)?;
    Ok(normalize_derivation_path(&derivation_path, &default_path))
}

#[uniffi::export]
pub fn derivation_paths_for_preset(
    preset: CoreSeedDerivationPreset,
) -> Result<CoreSeedDerivationPaths, crate::SpectraBridgeError> {
    Ok(seed_derivation_paths_for_account(preset.account_index())?)
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

/// Default derivation paths for every mainnet chain at `account`, driven off
/// `registry::Chain`.
///
/// Paths are keyed by concrete network. A missing template means that the
/// chain derives without a configurable BIP-32 path.
pub(crate) fn seed_derivation_paths_for_account(
    account: u32,
) -> Result<CoreSeedDerivationPaths, DerivationError> {
    let mut by_chain = std::collections::HashMap::new();
    for chain in Chain::all() {
        // Keyed by id rather than display name — ids are the stable key, and
        // `every_catalog_name_resolves` guarantees every name resolves back to
        // the id it belongs to.
        if let Some(template) = crate::chains::default_derivation_path_template(chain) {
            by_chain.insert(
                chain.str_id().to_string(),
                render_derivation_path_template(template, account),
            );
        }
    }
    if by_chain.is_empty() {
        return Err(DerivationError::Invalid(
            "Chain catalog produced no derivation paths.".into(),
        ));
    }
    Ok(CoreSeedDerivationPaths { by_chain })
}

fn render_derivation_path_template(template: &str, account: u32) -> String {
    template.replace("{account}", &account.to_string())
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
        Err(DerivationError::Invalid(format!(
            "Missing default derivation path for {chain}."
        )))
    } else {
        Ok(String::new())
    }
}

/// Extract a UTXO discovery index only when the path prefix matches the
/// chain's default path and the penultimate segment is the requested branch.
pub(crate) fn utxo_discovery_index(raw_path: &str, chain: Chain, branch: u32) -> Option<u32> {
    let default_path = default_path_from_catalog(chain).ok()?;
    let path = parse_derivation_path_str(raw_path)?;
    let mut candidate = parse_derivation_path_str(&default_path)?;
    if path.len() != candidate.len() || path.len() < 5 {
        return None;
    }
    let last = path.len() - 1;
    candidate[last - 1] = DerivationPathSegment {
        value: branch,
        is_hardened: false,
    };
    candidate[last] = DerivationPathSegment {
        value: path[last].value,
        is_hardened: false,
    };
    if format_derivation_path_segments(&candidate[..last])
        != format_derivation_path_segments(&path[..last])
    {
        return None;
    }
    if path[last - 1].value != branch {
        return None;
    }
    Some(path[last].value)
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

        // Monero is the only mainnet that says it, so a second one appearing
        // is a catalog edit to notice rather than a silent empty path.
        for chain in Chain::all().filter(|c| !c.is_testnet() && *c != Chain::Monero) {
            assert!(
                chain.uses_derivation_path(),
                "{} has no catalog derivation path",
                chain.str_id()
            );
        }
    }

    #[test]
    fn resolves_bitcoin_taproot_path() {
        let default_path = default_path_from_catalog(Chain::Bitcoin).expect("default path");
        let normalized = normalize_derivation_path("m/86'/0'/2'/0/0", &default_path);
        assert_eq!(normalized, "m/86'/0'/2'/0/0");
    }

    #[test]
    fn renders_catalog_default_paths_for_preset_accounts() {
        let paths = seed_derivation_paths_for_account(2).expect("paths");
        assert_eq!(paths.path_for(Chain::BitcoinSV), Some("m/44'/236'/2'/0/0"));
        assert_eq!(paths.path_for(Chain::Ethereum), Some("m/44'/60'/2'/0/0"));
        assert_eq!(paths.path_for(Chain::Solana), Some("m/44'/501'/2'/0'"));
    }

    /// Every concrete network with a catalog template gets its own path.
    #[test]
    fn derivation_paths_cover_the_catalog_and_resolve_testnets() {
        let paths = seed_derivation_paths_for_account(0).expect("paths");
        for chain in Chain::all() {
            let expected = crate::chains::default_derivation_path_template(chain).is_some();
            assert_eq!(
                paths.path_for(chain).is_some(),
                expected,
                "{} path presence disagrees with the catalog",
                chain.str_id()
            );
            assert_eq!(paths.by_chain.contains_key(chain.str_id()), expected);
        }

        // Monero derives its keys its own way and has `derivation_path = []`
        // in the catalog, so it is deliberately absent.
        assert_eq!(paths.path_for(Chain::Monero), None);

        // BNB Chain has a catalog template, so it gets an entry.
        assert!(paths.path_for(Chain::BnbChain).is_some());
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
}
