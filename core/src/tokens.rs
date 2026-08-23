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
    tokens: Vec<TomlToken>,
}

#[derive(Debug, Deserialize)]
struct TomlToken {
    chain: String,
    name: String,
    symbol: String,
    standard: String,
    contract: String,
    coingecko_id: String,
    decimals: u32,
    display_decimals: Option<u32>,
    tags: Vec<String>,
    comment: String,
    color: String,
    asset_name: String,
    enabled: bool,
}

// ── Public serialized shape (mirrors ChainTokenRegistryEntry in Swift)

#[derive(Debug, Clone, Serialize, uniffi::Record)]
pub struct TokenEntry {
    pub chain: String,
    pub name: String,
    pub symbol: String,
    pub token_standard: String,
    pub contract: String,
    pub coingecko_id: String,
    pub decimals: u32,
    pub display_decimals: Option<u32>,
    pub tags: Vec<String>,
    pub comment: String,
    pub color: String,
    pub asset_name: String,
    pub enabled: bool,
}

// ── Static catalog

static CATALOG: LazyLock<Vec<TokenEntry>> = LazyLock::new(|| {
    let parsed: TomlFile = toml::from_str(TOKENS_TOML)
        .expect("tokens.toml is embedded at compile time and must be valid TOML");
    parsed
        .tokens
        .into_iter()
        .map(|t| TokenEntry {
            chain: t.chain,
            name: t.name,
            symbol: t.symbol,
            token_standard: t.standard,
            contract: t.contract,
            coingecko_id: t.coingecko_id,
            decimals: t.decimals,
            display_decimals: t.display_decimals,
            tags: t.tags,
            comment: t.comment,
            color: t.color,
            asset_name: t.asset_name,
            enabled: t.enabled,
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
/// This existed twice. `normalizedTrackedTokenIdentifier` in `AppState` had its
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
    EthereumRpc,
    MoneroBackend,
}

/// `None` when the value is usable, otherwise the message to show under it.
#[uniffi::export]
pub fn endpoint_validation_error(field: EndpointField, raw: String) -> Option<String> {
    let invalid = match field {
        EndpointField::BitcoinEsploraList => parse_bitcoin_esplora_endpoints(raw)
            .iter()
            .any(|endpoint| !is_valid_http_url(endpoint)),
        EndpointField::EthereumRpc | EndpointField::MoneroBackend => {
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
            EndpointField::EthereumRpc => "Enter a valid http or https RPC URL.",
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
        assert_eq!(endpoint_validation_error(EthereumRpc, "".into()), None);
        assert_eq!(endpoint_validation_error(BitcoinEsploraList, "".into()), None);
        assert!(endpoint_validation_error(EthereumRpc, "ftp://x".into()).is_some());
        assert_eq!(
            endpoint_validation_error(EthereumRpc, "https://rpc.example/abc".into()),
            None
        );
        // Same check, different name in the message.
        assert_ne!(
            endpoint_validation_error(EthereumRpc, "ftp://x".into()),
            endpoint_validation_error(MoneroBackend, "ftp://x".into())
        );
    }

    /// One normalizer, keyed by the chain.
    ///
    /// TON is the arm worth stating: a jetton master address is
    /// case-significant base64, so the lowercase default would produce an
    /// address that does not resolve. `normalizedTrackedTokenIdentifier` in
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
