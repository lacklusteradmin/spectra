//! Built-in token registry.
//!
//! The source of truth is `core/data/tokens.toml` and, for the faucet coins,
//! `core/data/testnet-tokens.toml` — both embedded at compile time, and which
//! file a token sits in is checked against its networks. Call [`list_tokens`]
//! to get typed token entries for a given chain id string (or all chains when
//! the empty string `""` is passed).

use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

// Embedded at compile time — no bundle dependency at runtime.
static TOKENS_TOML: &str = include_str!("../data/tokens.toml");
static TESTNET_TOKENS_TOML: &str = include_str!("../data/testnet-tokens.toml");

/// The tokens one file spells, in file order.
fn parse_tokens(file: &'static str, source: &str) -> Vec<TomlToken> {
    let parsed: TomlFile = toml::from_str(file)
        .unwrap_or_else(|e| panic!("{source} is embedded at compile time and must be valid: {e}"));
    parsed.tokens
}

// ── Parsed TOML shape

#[derive(Debug, Deserialize)]
struct TomlFile {
    tokens: Vec<TomlToken>,
}

/// What a token is, and — nested under it — every network that carries it.
#[derive(Debug, Deserialize)]
struct TomlToken {
    id: String,
    symbol: String,
    name: String,
    coingecko_id: String,
    coinpaprika_id: String,
    color: crate::chains::CatalogColor,
    artwork_name: String,
    tags: Vec<String>,
    deployments: Vec<TomlDeployment>,
}

/// Where it lives, and what is true only there. Its catalog id is those facts
/// spelled one way, so the file does not carry one.
#[derive(Debug, Deserialize)]
struct TomlDeployment {
    network: String,
    kind: String,
    #[serde(default)]
    contract: String,
    decimals: u32,
    #[serde(default)]
    standard: String,
    enabled: bool,
}

/// Protocol identity is explicit; a missing contract never implies native.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Enum)]
pub enum TokenKind {
    Native,
    Protocol {
        standard: String,
        identifier: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TokenEntry {
    pub id: String,
    pub token_id: String,
    pub kind: TokenKind,
    pub chain: String,
    pub name: String,
    pub symbol: String,
    pub token_standard: String,
    pub contract: String,
    pub coingecko_id: String,
    pub decimals: u32,
    pub tags: Vec<String>,
    /// `None` for a token the user added: the catalog has no colour for it.
    pub color: Option<crate::chains::CatalogColor>,
    pub artwork_name: String,
    pub enabled: bool,
}

impl TokenEntry {
    /// A zero-balance holding of this token, in the shape a stored one has.
    ///
    /// `chain_name` is the chain's display name, which is what
    /// `AssetHolding::canonicalize` leaves behind and what
    /// `Chain::native_holding_template` has always produced. This copied
    /// `self.chain` instead — the catalog's `network`, a str id — so a template
    /// carried `"bitcoin"` where a stored holding carried `"Bitcoin"`. Nothing
    /// in core noticed, because every reader goes through `network()`, which
    /// tries both spellings; the app's exact-match lookup does not, so the id
    /// reached the screen. Templates are built on the read path and never
    /// persisted, which is how they slipped past the canonicalize that every
    /// stored holding passes through.
    pub fn holding_template(&self) -> crate::store::wallet_domain::AssetHolding {
        // The catalog writes a str id; a token the user added names its chain.
        let chain_name = crate::registry::Chain::from_str_id(&self.chain)
            .or_else(|| crate::registry::Chain::from_display_name(&self.chain))
            .map_or_else(
                || self.chain.clone(),
                |c| c.chain_display_name().to_string(),
            );
        crate::store::wallet_domain::AssetHolding {
            name: self.name.clone(),
            symbol: self.symbol.clone(),
            coin_gecko_id: self.coingecko_id.clone(),
            chain_name,
            token_standard: self.token_standard.clone(),
            contract_address: (!self.contract.is_empty()).then(|| self.contract.clone()),
            amount: 0.0,
            price_usd: 0.0,
        }
    }

    pub fn is_native(&self) -> bool {
        matches!(self.kind, TokenKind::Native)
    }
    pub fn matches_holding(&self, holding: &crate::store::wallet_domain::AssetHolding) -> bool {
        let network = crate::registry::Chain::from_str_id(&self.chain)
            .or_else(|| crate::registry::Chain::from_display_name(&self.chain));
        (self.is_native() || !self.contract.trim().is_empty())
            && network.is_some()
            && network == holding.network()
            && crate::tokens::normalize_token_identifier(
                Some(self.contract.clone()),
                network.unwrap().chain_display_name().into(),
            ) == holding.contract_address.clone().and_then(|c| {
                crate::tokens::normalize_token_identifier(Some(c), holding.chain_name.clone())
            })
            && self.is_native() == holding.is_native()
    }
}

// ── Static catalog

static CATALOG: LazyLock<Vec<TokenEntry>> = LazyLock::new(|| {
    let mainnet = parse_tokens(TOKENS_TOML, "tokens.toml");
    let testnet = parse_tokens(TESTNET_TOKENS_TOML, "testnet-tokens.toml");
    #[derive(Deserialize)]
    struct Networks {
        networks: Vec<Network>,
    }
    #[derive(Deserialize)]
    struct Network {
        id: String,
        environment: String,
        native_deployment: String,
        token_standard: String,
    }
    let networks: Networks =
        toml::from_str(include_str!("../data/chains.toml")).expect("valid networks");
    let mut identities = std::collections::HashSet::new();
    let mut token_ids = std::collections::HashSet::new();
    mainnet
        .iter()
        .map(|t| (t, "mainnet"))
        .chain(testnet.iter().map(|t| (t, "testnet")))
        .flat_map(|(t, environment)| {
            assert!(
                token_ids.insert(t.id.as_str()),
                "duplicate token id {}",
                t.id
            );
            // A token no network carries would reach a caller as a symbol and
            // nothing else, so the file may not express one.
            assert!(
                !t.deployments.is_empty(),
                "tokens.toml: token {} has no deployment",
                t.id
            );
            t.deployments.iter().map(move |d| (t, environment, d))
        })
        .map(|(t, environment, d)| {
            let network = networks
                .networks
                .iter()
                .find(|n| n.id == d.network)
                .expect("unknown deployment network");
            // Derived, not declared: an id written beside the facts it
            // restates can disagree with them, and the file spelled 268 of
            // them for the build to check character by character.
            let id = if d.kind == "native" {
                format!("{}:native", d.network)
            } else {
                format!("{}:{}:{}", d.network, d.standard.to_lowercase(), d.contract)
            };
            assert!(
                identities.insert(id.clone()),
                "duplicate deployment id {id}"
            );
            assert!(d.decimals <= 38, "unsupported deployment precision");
            // Which file a token is in is a claim about its networks, so the
            // networks are what settles it.
            assert_eq!(
                network.environment, environment,
                "{id} is in the wrong token file"
            );
            assert!(
                environment != "testnet"
                    || (t.coingecko_id.is_empty() && t.coinpaprika_id.is_empty()),
                "testnet token has market identity"
            );
            if d.kind == "native" {
                assert_eq!(
                    network.native_deployment, id,
                    "unreferenced native deployment"
                );
            } else {
                assert_eq!(
                    network.token_standard, d.standard,
                    "protocol differs from network"
                );
                let validator = match d.standard.as_str() {
                    "ERC-20" | "BEP-20" | "ARC-20" => "evm",
                    "SPL" => "solana",
                    "TRC-20" => "tron",
                    "TEP-74" => "ton",
                    "NEP-141" => "near",
                    "Sui Coin" => "suiCoinType",
                    "AIP-21" => "aptosTokenType",
                    other => panic!("unsupported token standard {other}"),
                };
                assert!(
                    crate::validation::address::validate_address(
                        crate::validation::address::AddressValidationRequest {
                            kind: validator.into(),
                            value: d.contract.clone(),
                        }
                    )
                    .is_valid,
                    "invalid deployment identifier {id}"
                );
            }
            TokenEntry {
                id,
                token_id: t.id.clone(),
                kind: match d.kind.as_str() {
                    "native" => {
                        assert!(
                            d.contract.is_empty() && d.standard.is_empty(),
                            "native deployment has protocol fields"
                        );
                        TokenKind::Native
                    }
                    "token" => {
                        assert!(
                            !d.contract.is_empty() && !d.standard.is_empty(),
                            "token requires protocol identity"
                        );
                        TokenKind::Protocol {
                            standard: d.standard.clone(),
                            identifier: d.contract.clone(),
                        }
                    }
                    other => panic!("unknown deployment kind {other}"),
                },
                chain: d.network.clone(),
                name: t.name.clone(),
                symbol: t.symbol.clone(),
                token_standard: if d.kind == "native" {
                    "Native".into()
                } else {
                    d.standard.clone()
                },
                contract: d.contract.clone(),
                coingecko_id: t.coingecko_id.clone(),
                decimals: d.decimals,
                tags: t.tags.clone(),
                color: Some(t.color),
                artwork_name: t.artwork_name.clone(),
                enabled: d.enabled,
            }
        })
        .collect()
});

/// Resolve an explicitly registered deployment, without guessing from a ticker.
pub fn deployment(id: &str) -> Option<&'static TokenEntry> {
    CATALOG.iter().find(|t| t.id == id)
}

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

/// The catalog's name for the token a chain's own feed calls `symbol`.
///
/// A history row names its asset by ticker, and a ticker is unique only within
/// a chain — which is why this takes one. `None` when the catalog does not
/// carry the token, and the caller shows the ticker itself.
///
/// Enabled or not does not enter into it: what a token is called is a fact
/// about the token, not about whether this wallet tracks it.
pub(crate) fn token_name_on_chain(chain_id: &str, symbol: &str) -> Option<&'static str> {
    let mut matches = CATALOG
        .iter()
        .filter(|t| t.chain == chain_id && t.symbol.eq_ignore_ascii_case(symbol));
    let token = matches.next()?;
    matches.next().is_none().then_some(token.name.as_str())
}

/// Each token's ids at the market-data providers, one row per token.
///
/// Kept out of [`TokenEntry`] the way the chain catalog keeps them out of
/// `ChainEntry`: no front end prices anything, so these would cross the FFI on
/// every `list_tokens` call for a caller that never reads them.
pub(crate) fn market_ids() -> &'static [crate::price::AssetMarketIds] {
    static IDS: LazyLock<Vec<crate::price::AssetMarketIds>> = LazyLock::new(|| {
        // Only the mainnet file: a testnet token with market ids does not
        // parse, so there is nothing to read out of the other one.
        parse_tokens(TOKENS_TOML, "tokens.toml")
            .iter()
            .filter(|t| !t.coingecko_id.is_empty())
            .map(|t| crate::price::AssetMarketIds {
                coingecko_id: t.coingecko_id.clone(),
                coinpaprika_id: t.coinpaprika_id.clone(),
            })
            .collect()
    });
    &IDS
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
    strip_hex_leading_zeros(&value.to_ascii_lowercase())
}

/// Normalize an Aptos coin-type / identifier string: preserve type case, rewrite
/// every `0x…` hex run in place with [`canonical_aptos_hex_address`].
/// Internal: `normalize_token_identifier` is the one entry point, and it
/// dispatches here by chain.
pub(crate) fn normalize_aptos_token_identifier(value: String) -> String {
    let trimmed = value.trim().to_string();
    // An identifier is ASCII, and everything below indexes it by byte. Handing
    // back anything else untouched is what keeps those indices sound — and it
    // is why the copy loop can move one byte at a time without re-encoding.
    // Copying bytes as `char` did the Latin-1 thing to any multi-byte sequence
    // that reached it, silently rewriting the identifier rather than refusing.
    if !trimmed.is_ascii() || trimmed.is_empty() {
        return trimmed;
    }
    let bytes = trimmed.as_bytes();
    let mut out = String::with_capacity(trimmed.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"0x") {
            let start = i;
            let mut end = i + 2;
            while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
                end += 1;
            }
            out.push_str(&canonical_aptos_hex_address(
                trimmed[start..end].to_string(),
            ));
            i = end;
        } else {
            // A slice, not a byte cast: correct by construction for the ASCII
            // this function has already established, and a compile error
            // rather than a corruption if that ever stops being true.
            out.push_str(&trimmed[i..i + 1]);
            i += 1;
        }
    }
    out
}

/// Canonicalize just a Sui package identifier: `0x…` with trimmed zeroes.
/// Internal: `normalize_sui_token_identifier` calls it.
pub(crate) fn normalize_sui_package_component(value: String) -> String {
    strip_hex_leading_zeros(&value.to_ascii_lowercase())
}

/// Normalize a Sui token identifier: preserve type case, split on `::`, canonicalize
/// the first (package) component, rejoin.
/// Internal: see `normalize_aptos_token_identifier`.
pub(crate) fn normalize_sui_token_identifier(value: String) -> String {
    let trimmed = value.trim().to_string();
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
pub fn normalize_token_identifier(
    contract_address: Option<String>,
    chain_name: String,
) -> Option<String> {
    let raw = contract_address?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    match crate::registry::Chain::from_display_name(&chain_name)
        .or_else(|| crate::registry::Chain::from_str_id(&chain_name))
        .map(|c| c.mainnet_counterpart())
    {
        Some(crate::registry::Chain::Sui) => {
            Some(normalize_sui_token_identifier(trimmed.to_string()))
        }
        Some(crate::registry::Chain::Aptos) => {
            Some(normalize_aptos_token_identifier(trimmed.to_string()))
        }
        Some(
            crate::registry::Chain::Ton
            | crate::registry::Chain::Solana
            | crate::registry::Chain::Tron,
        ) => Some(trimmed.to_string()),
        _ => Some(trimmed.to_lowercase()),
    }
}

// ---- Bitcoin Esplora endpoint parsing / validation ----

/// The custom Esplora bases a setting value names, in order.
///
/// Exported because the app split the same value itself, twice — once per
/// settings screen — with its own separators and trimming beside the
/// validation core already ran on it.
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

    /// The rule, not the symptom: a template is already what `canonicalize`
    /// would leave behind, so the two ways of getting an `AssetHolding` for a
    /// catalog token cannot drift apart again. `chain_name` is the field that
    /// did — the catalog's `network` is a str id and a stored holding's is a
    /// display name — and this checks every entry, so a catalog row spelled the
    /// other way fails here rather than on someone's screen.
    #[test]
    fn a_holding_template_is_already_canonical() {
        for token in catalog() {
            let template = token.holding_template();
            let mut canonical = template.clone();
            if canonical.canonicalize().is_err() {
                continue; // A chain the registry does not know; `network()` is None either way.
            }
            assert_eq!(
                template, canonical,
                "{} builds a template canonicalize would rewrite",
                token.id
            );
        }
    }

    /// The spelling itself, on the entry that reported it.
    #[test]
    fn a_native_template_names_its_chain_the_way_a_stored_holding_does() {
        let bitcoin = catalog()
            .iter()
            .find(|t| t.id == "bitcoin:native")
            .expect("the catalog lists bitcoin");
        assert_eq!(bitcoin.chain, "bitcoin", "the catalog stores a str id");
        assert_eq!(bitcoin.holding_template().chain_name, "Bitcoin");
        assert_eq!(
            bitcoin.holding_template().chain_name,
            crate::registry::Chain::Bitcoin
                .native_holding_template()
                .chain_name,
            "the two template builders agree"
        );
    }

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
            "0x1::coin::USDC"
        );
        assert_eq!(normalize_aptos_token_identifier("   ".into()), "");
    }

    #[test]
    fn normalize_sui_roundtrip() {
        assert_eq!(
            normalize_sui_token_identifier("0x0002::Foo::bar".into()),
            "0x2::Foo::bar"
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
            endpoint_validation_error(
                BitcoinEsploraList,
                "https://x.example,https://y.example".into()
            ),
            None
        );
        assert!(endpoint_validation_error(BitcoinEsploraList, "notaurl".into()).is_some());
        // An empty single-URL field is unset, not invalid; an empty list is too.
        assert_eq!(endpoint_validation_error(EvmRpc, "".into()), None);
        assert_eq!(
            endpoint_validation_error(BitcoinEsploraList, "".into()),
            None
        );
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
        for chain in [
            "Solana",
            "Solana Devnet",
            "Tron",
            "Tron Nile",
            "TON",
            "TON Testnet",
        ] {
            assert_eq!(
                normalize_token_identifier(Some(" AbCd ".into()), chain.into()),
                Some("AbCd".into()),
                "{chain}"
            );
        }

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
            Some("0x2::Foo::bar".into())
        );
        assert_eq!(
            normalize_token_identifier(Some("0x001::coin::USDC".into()), "Aptos".into()),
            Some("0x1::coin::USDC".into())
        );
        assert_eq!(
            normalize_token_identifier(Some("  EQAbC  ".into()), "TON".into()),
            Some("EQAbC".into()),
            "a jetton master address keeps its case"
        );
    }
}

#[cfg(test)]
mod a_token_and_the_deployments_under_it {
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
            let first = seen.entry(entry.token_id.as_str()).or_insert(entry);
            for (field, a, b) in [
                ("name", &first.name, &entry.name),
                ("coingecko_id", &first.coingecko_id, &entry.coingecko_id),
                ("artwork_name", &first.artwork_name, &entry.artwork_name),
            ] {
                assert_eq!(
                    a, b,
                    "{}'s {field} differs between {} and {}",
                    entry.symbol, first.chain, entry.chain
                );
            }
            assert_eq!(first.color, entry.color, "{}'s color differs", entry.symbol);
            assert_eq!(first.tags, entry.tags, "{}'s tags differ", entry.symbol);
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

    /// The contract is the id now, so it has to be spelled the way every
    /// lookup spells it: `history_deployment` builds the same string from a
    /// normalized address, and a row that disagrees is a row nothing resolves.
    #[test]
    fn every_catalog_contract_is_already_normalized() {
        for entry in CATALOG.iter().filter(|e| !e.is_native()) {
            let chain = crate::registry::Chain::from_str_id(&entry.chain)
                .expect("a deployment network is a registry chain");
            assert_eq!(
                normalize_token_identifier(
                    Some(entry.contract.clone()),
                    chain.chain_display_name().into()
                )
                .as_deref(),
                Some(entry.contract.as_str()),
                "{} is not in normalized form",
                entry.id
            );
        }
    }

    /// Nesting makes an orphan deployment unspellable, so what is left to
    /// check is the other direction: no token sits there carried by nothing,
    /// and the catalog holds every row the file spells.
    #[test]
    fn every_token_is_deployed_somewhere() {
        let files = [
            parse_tokens(TOKENS_TOML, "tokens.toml"),
            parse_tokens(TESTNET_TOKENS_TOML, "testnet-tokens.toml"),
        ];
        for token in files.iter().flatten() {
            assert!(
                !token.deployments.is_empty(),
                "{} is a token with no deployment",
                token.symbol
            );
        }
        assert_eq!(
            CATALOG.len(),
            files
                .iter()
                .flatten()
                .map(|t| t.deployments.len())
                .sum::<usize>()
        );
    }
}

/// Display precision resolved by deployment; unknown history has no native assumption.
#[uniffi::export]
pub fn token_display_decimals(deployment_id: Option<String>, custom_decimals: Option<u32>) -> u32 {
    deployment_id
        .as_deref()
        .and_then(deployment)
        .map(|t| t.decimals)
        .or(custom_decimals)
        .unwrap_or(18)
        .min(38)
}

/// Canonical deployment identity for a transfer on its actual network.
pub(crate) fn history_deployment(
    chain: crate::registry::Chain,
    contract: Option<&str>,
) -> Option<String> {
    match contract {
        None => Some(chain.entry().native_deployment_id.clone()),
        Some(contract) => {
            normalize_token_identifier(Some(contract.into()), chain.chain_display_name().into())
                .map(|id| {
                    format!(
                        "{}:{}:{}",
                        chain.str_id(),
                        chain
                            .mainnet_counterpart()
                            .entry()
                            .token_standard
                            .to_lowercase(),
                        id
                    )
                })
        }
    }
}
