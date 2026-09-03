//! Built-in token registry.
//!
//! The source of truth is `core/data/tokens.toml`, embedded at compile time.
//! Call [`list_tokens`] to get typed token entries for a given chain id string
//! (or all chains when the empty string `""` is passed).

use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

// Embedded at compile time — no bundle dependency at runtime.
static TOKENS_TOML: &str = include_str!("../data/tokens.toml");

// ── Parsed TOML shape

#[derive(Debug, Deserialize)]
struct TomlFile {
    assets: Vec<TomlAsset>,
    deployments: Vec<TomlDeployment>,
}

/// What a token is — one row however many chains it ships on.
#[derive(Debug, Deserialize)]
struct TomlAsset {
    symbol: String,
    name: String,
    coingecko_id: String,
    color: String,
    asset_name: String,
    tags: Vec<String>,
}

/// Where it lives, and what is true only there.
#[derive(Debug, Deserialize)]
struct TomlDeployment {
    asset: String,
    chain: String,
    contract: String,
    decimals: u32,
    standard: String,
    enabled: bool,
}

// ── Public shape: one deployment, with its asset's facts joined in.

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TokenEntry {
    pub chain: String,
    pub name: String,
    pub symbol: String,
    pub token_standard: String,
    pub contract: String,
    pub coingecko_id: String,
    pub decimals: u32,
    pub tags: Vec<String>,
    pub color: String,
    pub asset_name: String,
    pub enabled: bool,
}

// ── Static catalog

static CATALOG: LazyLock<Vec<TokenEntry>> = LazyLock::new(|| {
    let parsed: TomlFile = toml::from_str(TOKENS_TOML)
        .expect("tokens.toml is embedded at compile time and must be valid TOML");
    let assets: std::collections::HashMap<&str, &TomlAsset> = parsed
        .assets
        .iter()
        .map(|a| (a.symbol.as_str(), a))
        .collect();
    parsed
        .deployments
        .iter()
        .map(|d| {
            // A deployment naming an asset the file does not define is a
            // build-time mistake, not a row to skip: the entry would carry a
            // symbol and nothing else.
            let a = assets.get(d.asset.as_str()).unwrap_or_else(|| {
                panic!("tokens.toml: deployment on {} names unknown asset {}", d.chain, d.asset)
            });
            TokenEntry {
                chain: d.chain.clone(),
                name: a.name.clone(),
                symbol: a.symbol.clone(),
                token_standard: d.standard.clone(),
                contract: d.contract.clone(),
                coingecko_id: a.coingecko_id.clone(),
                decimals: d.decimals,
                tags: a.tags.clone(),
                color: a.color.clone(),
                asset_name: a.asset_name.clone(),
                enabled: d.enabled,
            }
        })
        .collect()
});

// ── Public API

/// Return token entries for `chain_id`, or all chains when `chain_id` is `""`.
#[uniffi::export]
pub fn list_tokens(chain_id: String) -> Vec<TokenEntry> {
    if chain_id.is_empty() {
        CATALOG.clone()
    } else {
        CATALOG
            .iter()
            .filter(|t| t.chain == chain_id)
            .cloned()
            .collect()
    }
}

/// Return a reference to the static catalog slice.
pub fn catalog() -> &'static [TokenEntry] {
    &CATALOG
}

// ── Token-id + endpoint URL normalization helpers ─────────────────

// Pure token-identifier + endpoint normalization helpers (string munging,
// URL validation, CSV parsing). No mutable state — testable in isolation.

/// Strip leading zeros from a `0x…` hex string, keeping at least one digit.
/// Returns the value unchanged if it doesn't start with `0x`.
fn strip_hex_leading_zeros(value: &str) -> String {
    if !value.starts_with("0x") {
        return value.to_string();
    }
    let hex_part = &value[2..];
    let significant: String = hex_part.chars().skip_while(|c| *c == '0').collect();
    format!(
        "0x{}",
        if significant.is_empty() {
            "0"
        } else {
            &significant
        }
    )
}

/// Canonicalize a `0x…` hex string: strip leading zeroes, keep at least one.
/// Unchanged if the prefix is not `0x`.
/// Internal: `normalize_aptos_token_identifier` calls it. Exported until its
/// Swift forwarder turned out to have no caller.
pub(crate) fn canonical_aptos_hex_address(value: String) -> String {
    strip_hex_leading_zeros(&value)
}

/// Normalize an Aptos coin-type / identifier string: lowercase, then rewrite
/// every `0x…` hex run in place with [`canonical_aptos_hex_address`].
/// Internal: `normalize_token_identifier` is the one entry point, and it
/// dispatches here by chain.
pub(crate) fn normalize_aptos_token_identifier(value: String) -> String {
    let lowercased = value.trim().to_lowercase();
    if lowercased.is_empty() {
        return String::new();
    }
    let bytes = lowercased.as_bytes();
    let mut out = String::with_capacity(lowercased.len());
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && &bytes[i..i + 2] == b"0x" {
            let start = i;
            let mut end = i + 2;
            while end < bytes.len() && (bytes[end] as char).is_ascii_hexdigit() {
                end += 1;
            }
            out.push_str(&canonical_aptos_hex_address(
                lowercased[start..end].to_string(),
            ));
            i = end;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Canonicalize just a Sui package identifier: `0x…` with trimmed zeroes.
/// Internal: `normalize_sui_token_identifier` calls it.
pub(crate) fn normalize_sui_package_component(value: String) -> String {
    strip_hex_leading_zeros(&value)
}

/// Normalize a Sui token identifier: lowercase, split on `::`, canonicalize
/// the first (package) component, rejoin.
/// Internal: see `normalize_aptos_token_identifier`.
pub(crate) fn normalize_sui_token_identifier(value: String) -> String {
    let trimmed = value.trim().to_lowercase();
    if trimmed.is_empty() {
        return String::new();
    }
    let parts: Vec<&str> = trimmed.split("::").collect();
    let first = match parts.first() {
        Some(p) => *p,
        None => return trimmed,
    };
    let normalized_package = normalize_sui_package_component(first.to_string());
    if parts.len() <= 1 {
        return normalized_package;
    }
    let mut out = normalized_package;
    for rest in &parts[1..] {
        out.push_str("::");
        out.push_str(rest);
    }
    out
}

/// Normalize a dashboard asset's contract address for grouping/equality.
/// The canonical form of a token's contract address or identifier on a chain.
///
/// Sui and Aptos have structured identifiers (`package::module::type`) with
/// their own canonicalisation; everything else is the trimmed value lowercased.
/// TON is the exception in the other direction: a jetton master address is
/// case-significant base64, so lowercasing it produces an address that does not
/// resolve.
///
/// This existed twice. `normalizedKnownTokenIdentifier` in `AppState` had its
/// own copy — a twelve-name EVM arm, then Aptos, Sui, TON and a lowercase
/// default — and the two disagreed about TON, which is the one chain where
/// disagreeing changes the answer. One function, keyed by the chain, with the
/// TON rule stated where the others are.
#[uniffi::export]
pub fn normalize_token_identifier(
    contract_address: Option<String>,
    chain_name: String,
) -> Option<String> {
    let raw = contract_address?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    match crate::registry::Chain::from_display_name(&chain_name) {
        Some(crate::registry::Chain::Sui) => {
            Some(normalize_sui_token_identifier(trimmed.to_string()))
        }
        Some(crate::registry::Chain::Aptos) => {
            Some(normalize_aptos_token_identifier(trimmed.to_string()))
        }
        Some(crate::registry::Chain::Ton) => Some(trimmed.to_string()),
        _ => Some(trimmed.to_lowercase()),
    }
}

// ---- Bitcoin Esplora endpoint parsing / validation ----

#[uniffi::export]
pub fn parse_bitcoin_esplora_endpoints(raw: String) -> Vec<String> {
    raw.split([',', '\n', ';'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Which endpoint setting a value came from.
///
/// Decides both how the value is parsed — one URL, or a comma-separated list —
/// and which message names it. There were three exports for this and two of
/// them were byte-identical but for their string.
#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum EndpointField {
    /// A comma, semicolon or newline separated list.
    BitcoinEsploraList,
    /// Any EVM chain's custom RPC. The rule is "a valid http(s) URL" and was
    /// never Ethereum-specific; the name was.
    EvmRpc,
    MoneroBackend,
}

/// `None` when the value is usable, otherwise the message to show under it.
#[uniffi::export]
pub fn endpoint_validation_error(field: EndpointField, raw: String) -> Option<String> {
    let invalid = match field {
        EndpointField::BitcoinEsploraList => parse_bitcoin_esplora_endpoints(raw)
            .iter()
            .any(|endpoint| !is_valid_http_url(endpoint)),
        EndpointField::EvmRpc | EndpointField::MoneroBackend => {
            let trimmed = raw.trim();
            !trimmed.is_empty() && !is_valid_http_url(trimmed)
        }
    };
    if !invalid {
        return None;
    }
    Some(
        match field {
            EndpointField::BitcoinEsploraList => {
                "Bitcoin Esplora endpoints must be valid http(s) URLs separated by commas."
            }
            EndpointField::EvmRpc => "Enter a valid http or https RPC URL.",
            EndpointField::MoneroBackend => "Enter a valid http or https Monero backend URL.",
        }
        .to_string(),
    )
}

fn is_valid_http_url(s: &str) -> bool {
    // Minimal-but-correct parser matching the semantics the Swift code needed:
    // scheme in {http, https} and a non-empty host.
    let Some(scheme_end) = s.find("://") else {
        return false;
    };
    let scheme = &s[..scheme_end].to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return false;
    }
    let after = &s[scheme_end + 3..];
    if after.is_empty() {
        return false;
    }
    // Host ends at '/', '?', '#', or end. Strip any userinfo ('@').
    let host_end = after.find(['/', '?', '#']).unwrap_or(after.len());
    let authority = &after[..host_end];
    let host_part = match authority.rsplit_once('@') {
        Some((_, h)) => h,
        None => authority,
    };
    // Strip port if present.
    let host = match host_part.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => h,
        Some(_) => return false,
        None => host_part,
    };
    !host.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_hex_strips_leading_zeros() {
        assert_eq!(canonical_aptos_hex_address("0x0000abcd".into()), "0xabcd");
        assert_eq!(canonical_aptos_hex_address("0x0".into()), "0x0");
        assert_eq!(canonical_aptos_hex_address("0x00000".into()), "0x0");
        assert_eq!(canonical_aptos_hex_address("nohex".into()), "nohex");
    }

    #[test]
    fn normalize_aptos_rewrites_embedded_hex() {
        assert_eq!(
            normalize_aptos_token_identifier("0x001::coin::USDC".into()),
            "0x1::coin::usdc"
        );
        assert_eq!(normalize_aptos_token_identifier("   ".into()), "");
    }

    #[test]
    fn normalize_sui_roundtrip() {
        assert_eq!(
            normalize_sui_token_identifier("0x0002::Foo::bar".into()),
            "0x2::foo::bar"
        );
        assert_eq!(
            normalize_sui_token_identifier("plaintext".into()),
            "plaintext"
        );
    }

    #[test]
    fn parse_endpoints_splits_and_trims() {
        assert_eq!(
            parse_bitcoin_esplora_endpoints("a, b ;c\nd,,".into()),
            vec!["a", "b", "c", "d"]
        );
    }

    /// The field decides how the value is parsed and which message names it.
    #[test]
    fn endpoint_validation_is_per_field() {
        use EndpointField::*;
        assert_eq!(
            endpoint_validation_error(BitcoinEsploraList, "https://x.example,https://y.example".into()),
            None
        );
        assert!(endpoint_validation_error(BitcoinEsploraList, "notaurl".into()).is_some());
        // An empty single-URL field is unset, not invalid; an empty list is too.
        assert_eq!(endpoint_validation_error(EvmRpc, "".into()), None);
        assert_eq!(endpoint_validation_error(BitcoinEsploraList, "".into()), None);
        assert!(endpoint_validation_error(EvmRpc, "ftp://x".into()).is_some());
        assert_eq!(
            endpoint_validation_error(EvmRpc, "https://rpc.example/abc".into()),
            None
        );
        // Same check, different name in the message.
        assert_ne!(
            endpoint_validation_error(EvmRpc, "ftp://x".into()),
            endpoint_validation_error(MoneroBackend, "ftp://x".into())
        );
    }

    /// One normalizer, keyed by the chain.
    ///
    /// TON is the arm worth stating: a jetton master address is
    /// case-significant base64, so the lowercase default would produce an
    /// address that does not resolve. `normalizedKnownTokenIdentifier` in
    /// Swift knew that and this function did not, until they became one.
    #[test]
    fn token_identifier_normalisation_is_per_chain() {
        assert_eq!(
            normalize_token_identifier(Some("  ".into()), "Ethereum".into()),
            None
        );
        assert_eq!(normalize_token_identifier(None, "Ethereum".into()), None);
        assert_eq!(
            normalize_token_identifier(Some("0xABCDEF".into()), "Ethereum".into()),
            Some("0xabcdef".into())
        );
        assert_eq!(
            normalize_token_identifier(Some("0x0002::Foo::bar".into()), "Sui".into()),
            Some("0x2::foo::bar".into())
        );
        assert_eq!(
            normalize_token_identifier(Some("0x001::coin::USDC".into()), "Aptos".into()),
            Some("0x1::coin::usdc".into())
        );
        assert_eq!(
            normalize_token_identifier(Some("  EQAbC  ".into()), "TON".into()),
            Some("EQAbC".into()),
            "a jetton master address keeps its case"
        );
    }

}


#[cfg(test)]
mod the_catalog_is_two_tables {
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// A token's non-deployment facts are one fact, whatever it is deployed on.
    ///
    /// They were columns on every deployment row, so a token on ten chains
    /// carried ten names and ten colours — and DAI's had already come apart:
    /// "Dai" in orange on Ethereum, "Dai Stablecoin" in yellow on Base and
    /// Polygon. The join makes that unrepresentable; this asserts it.
    #[test]
    fn every_deployment_of_a_token_agrees_about_the_token() {
        let mut seen: HashMap<&str, &TokenEntry> = HashMap::new();
        for entry in CATALOG.iter() {
            let first = seen.entry(entry.symbol.as_str()).or_insert(entry);
            for (field, a, b) in [
                ("name", &first.name, &entry.name),
                ("coingecko_id", &first.coingecko_id, &entry.coingecko_id),
                ("color", &first.color, &entry.color),
                ("asset_name", &first.asset_name, &entry.asset_name),
            ] {
                assert_eq!(
                    a, b,
                    "{}'s {field} differs between {} and {}",
                    entry.symbol, first.chain, entry.chain
                );
            }
            assert_eq!(first.tags, entry.tags, "{}'s tags differ", entry.symbol);
        }
    }

    /// One symbol, one market-data id — across *both* catalogs.
    ///
    /// CRO was two: `chains.toml` called the native coin `crypto-com-chain`
    /// and `tokens.toml` called the ERC-20 at CoinGecko's own contract for
    /// that coin `cronos`. Both ids answer `simple/price`, which is why
    /// nothing looked broken — `cronos` returned $0.00010, `crypto-com-chain`
    /// $0.054, a factor of five hundred on the same holding.
    ///
    /// Since a dashboard row is keyed by coingecko id, two ids for one coin is
    /// also two rows for it. This is the invariant that makes both impossible.
    #[test]
    fn a_symbol_has_one_market_data_id_across_both_catalogs() {
        let mut by_symbol: HashMap<&str, (&str, &str)> = HashMap::new();
        for chain in crate::chains::catalog() {
            if chain.native_coingecko_id.is_empty() {
                continue;
            }
            // `gas_token_symbol`, not `symbol`: on eleven chains they differ,
            // because `symbol` is the chain's ticker and the native coin is
            // something else — Arbitrum is ARB and runs on ETH.
            let (id, source) = by_symbol
                .entry(chain.gas_token_symbol.as_str())
                .or_insert((chain.native_coingecko_id.as_str(), chain.name.as_str()));
            assert_eq!(
                *id,
                chain.native_coingecko_id.as_str(),
                "{} is {id} as {source} and {} as {}",
                chain.gas_token_symbol,
                chain.native_coingecko_id,
                chain.name
            );
        }
        for entry in CATALOG.iter() {
            if entry.coingecko_id.is_empty() {
                continue;
            }
            let (id, source) = by_symbol
                .entry(entry.symbol.as_str())
                .or_insert((entry.coingecko_id.as_str(), entry.chain.as_str()));
            assert_eq!(
                *id, entry.coingecko_id,
                "{} is {id} as {source} and {} in the token catalog",
                entry.symbol, entry.coingecko_id
            );
        }
    }

    /// Decimals stay per deployment, and the catalog still says so.
    ///
    /// This is the field the split must *not* fold up: a bridged token really
    /// does differ by chain, and folding it would silently mis-scale a balance.
    #[test]
    fn decimals_are_allowed_to_differ_by_chain() {
        let mut by_symbol: HashMap<&str, HashSet<u32>> = HashMap::new();
        for entry in CATALOG.iter() {
            by_symbol
                .entry(entry.symbol.as_str())
                .or_default()
                .insert(entry.decimals);
        }
        let differing: Vec<&str> = by_symbol
            .iter()
            .filter(|(_, d)| d.len() > 1)
            .map(|(s, _)| *s)
            .collect();
        assert!(
            differing.len() >= 5,
            "expected several bridged tokens to differ in decimals, found {differing:?}"
        );
        // LINK is 18 on Ethereum and 8 on Solana; if that ever reads the same
        // on both, the split folded a field it should not have.
        let link: HashSet<u32> = CATALOG
            .iter()
            .filter(|e| e.symbol == "LINK")
            .map(|e| e.decimals)
            .collect();
        assert!(link.len() > 1, "LINK's decimals collapsed to one value");
    }

    /// Every deployment resolves to an asset, and every asset is deployed
    /// somewhere. A row on either side with no partner is dead data.
    #[test]
    fn the_two_tables_cover_each_other() {
        let parsed: TomlFile = toml::from_str(TOKENS_TOML).expect("valid TOML");
        let deployed: HashSet<&str> = parsed.deployments.iter().map(|d| d.asset.as_str()).collect();
        for asset in &parsed.assets {
            assert!(
                deployed.contains(asset.symbol.as_str()),
                "{} is an asset with no deployment",
                asset.symbol
            );
        }
        assert_eq!(CATALOG.len(), parsed.deployments.len());
    }
}
