//! Built-in token registry.
//!
//! The source of truth is `core/data/tokens.toml` and, for the faucet coins,
//! `core/data/testnet-tokens.toml` — both embedded at compile time, and which
//! file a token sits in is checked against its networks. Call [`list_token_deployments`]
//! to get typed token entries for a given chain id string (or all chains when
//! the empty string `""` is passed).

use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

// Embedded at compile time — no bundle dependency at runtime.
static TOKENS_TOML: &str = include_str!("../data/tokens.toml");
static TESTNET_TOKENS_TOML: &str = include_str!("../data/testnet-tokens.toml");

/// Parse both tables and refuse dangling or ambiguous token references.
fn parse_token_file(file: &str) -> Result<TomlFile, String> {
    let parsed: TomlFile = toml::from_str(file).map_err(|e| e.to_string())?;
    let mut token_ids = std::collections::HashSet::new();
    for token in &parsed.tokens {
        if token.id.is_empty() || !token_ids.insert(token.id.as_str()) {
            return Err(format!("empty or duplicate token id {:?}", token.id));
        }
    }
    let mut deployed = std::collections::HashSet::new();
    for deployment in &parsed.deployments {
        if !token_ids.contains(deployment.token_id.as_str()) {
            return Err(format!(
                "unknown deployment token_id {:?}",
                deployment.token_id
            ));
        }
        deployed.insert(deployment.token_id.as_str());
    }
    for token in &parsed.tokens {
        if !deployed.contains(token.id.as_str()) {
            return Err(format!("token {} has no deployment", token.id));
        }
    }
    Ok(parsed)
}

fn embedded_token_file(file: &str, source: &str) -> TomlFile {
    parse_token_file(file)
        .unwrap_or_else(|e| panic!("{source} is embedded at compile time and must be valid: {e}"))
}

// ── Parsed TOML shape

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlFile {
    tokens: Vec<TomlToken>,
    deployments: Vec<TomlDeployment>,
}

/// Asset identity, independent of where it is deployed.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlToken {
    id: String,
    symbol: String,
    name: String,
    coingecko_id: String,
    coinpaprika_id: String,
    color: crate::chains::CatalogColor,
    artwork_name: String,
    tags: Vec<String>,
}

/// Where it lives, and what is true only there. Its catalog id is those facts
/// spelled one way, so the file does not carry one.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlDeployment {
    token_id: String,
    chain_id: crate::registry::Chain,
    kind: String,
    #[serde(default)]
    contract: String,
    decimals: u32,
    #[serde(default)]
    standard: String,
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
pub struct TokenDeploymentEntry {
    pub deployment_id: String,
    pub token_id: String,
    pub kind: TokenKind,
    pub chain_id: crate::registry::Chain,
    pub name: String,
    pub symbol: String,
    pub token_standard: String,
    pub contract: String,
    pub coingecko_id: String,
    pub coinpaprika_id: String,
    pub decimals: u32,
    pub tags: Vec<String>,
    /// `None` for a token the user added: the catalog has no colour for it.
    pub color: Option<crate::chains::CatalogColor>,
    pub artwork_name: String,
}

impl TokenDeploymentEntry {
    /// A zero-balance holding of this deployment.
    pub fn holding_template(&self) -> crate::store::wallet_domain::AssetHolding {
        crate::store::wallet_domain::AssetHolding {
            id: String::new(),
            name: self.name.clone(),
            symbol: self.symbol.clone(),
            coingecko_id: self.coingecko_id.clone(),
            chain_id: self.chain_id,
            token_standard: self.token_standard.clone(),
            contract_address: (!self.contract.is_empty()).then(|| self.contract.clone()),
            amount: "0".to_string(),
        }
        .identified()
    }

    pub fn is_native(&self) -> bool {
        matches!(self.kind, TokenKind::Native)
    }
    pub fn matches_holding(&self, holding: &crate::store::wallet_domain::AssetHolding) -> bool {
        (self.is_native() || !self.contract.trim().is_empty())
            && self.chain_id == holding.chain_id
            && normalize_token_identifier(Some(self.contract.clone()), self.chain_id)
                == holding
                    .contract_address
                    .clone()
                    .and_then(|c| normalize_token_identifier(Some(c), holding.chain_id))
            && self.is_native() == holding.is_native()
            && self.token_standard == holding.token_standard
    }
}

// ── Static catalog

static CATALOG: LazyLock<Vec<TokenDeploymentEntry>> = LazyLock::new(|| {
    load_catalog(
        embedded_token_file(TOKENS_TOML, "tokens.toml"),
        embedded_token_file(TESTNET_TOKENS_TOML, "testnet-tokens.toml"),
    )
});

fn load_catalog(mainnet: TomlFile, testnet: TomlFile) -> Vec<TokenDeploymentEntry> {
    let mut identities = std::collections::HashSet::new();
    let mut protocol_identifiers = std::collections::HashSet::new();
    let files = [(mainnet, "mainnet"), (testnet, "testnet")];
    let mut tokens_by_id = std::collections::HashMap::new();
    for (file, _) in &files {
        for token in &file.tokens {
            assert!(
                tokens_by_id.insert(token.id.as_str(), token).is_none(),
                "duplicate token id {}",
                token.id
            );
        }
    }
    files
        .iter()
        .flat_map(|(file, environment)| file.deployments.iter().map(move |d| (*environment, d)))
        .map(|(environment, d)| {
            let t = tokens_by_id[d.token_id.as_str()];
            let network = crate::chains::declared(d.chain_id);
            let contract = if d.kind == "native" {
                String::new()
            } else {
                validate_protocol_identifier(d.chain_id, &d.standard, &d.contract)
                    .unwrap_or_else(|e| panic!("invalid deployment {}: {e}", d.token_id))
            };
            // Derived, not declared: an id written beside the facts it
            // restates can disagree with them, and the file spelled 268 of
            // them for the build to check character by character.
            let id = if d.kind == "native" {
                format!("{}:native", d.chain_id)
            } else {
                format!("{}:{}:{}", d.chain_id, d.standard.to_lowercase(), contract)
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
            assert!(
                environment != "testnet" || t.name.starts_with("Test "),
                "testnet token {} is not named as a test coin",
                t.id
            );
            if d.kind != "native" {
                validate_protocol_identifier(d.chain_id, &d.standard, &d.contract)
                    .unwrap_or_else(|e| panic!("invalid deployment {id}: {e}"));
                assert!(
                    protocol_identifiers.insert((
                        d.chain_id,
                        normalize_token_identifier(Some(d.contract.clone()), d.chain_id)
                    )),
                    "duplicate protocol identifier on {}: {}",
                    d.chain_id,
                    d.contract
                );
            }
            TokenDeploymentEntry {
                deployment_id: id,
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
                            identifier: contract.clone(),
                        }
                    }
                    other => panic!("unknown deployment kind {other}"),
                },
                chain_id: d.chain_id,
                name: t.name.clone(),
                symbol: t.symbol.clone(),
                token_standard: if d.kind == "native" {
                    "Native".into()
                } else {
                    d.standard.clone()
                },
                contract,
                coingecko_id: t.coingecko_id.clone(),
                coinpaprika_id: t.coinpaprika_id.clone(),
                decimals: d.decimals,
                tags: t.tags.clone(),
                color: Some(t.color),
                artwork_name: t.artwork_name.clone(),
            }
        })
        .collect()
}

/// Resolve an explicitly registered deployment, without guessing from a ticker.
pub fn deployment(id: &str) -> Option<&'static TokenDeploymentEntry> {
    CATALOG.iter().find(|t| t.deployment_id == id)
}

// ── Public API

/// Token entries on `chain`, or on every chain for `None`.
#[uniffi::export]
pub fn list_token_deployments(chain: Option<crate::registry::Chain>) -> Vec<TokenDeploymentEntry> {
    CATALOG
        .iter()
        .filter(|t| chain.is_none_or(|chain| t.chain_id == chain))
        .cloned()
        .collect()
}

/// Return a reference to the static catalog slice.
pub fn catalog() -> &'static [TokenDeploymentEntry] {
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
    strip_hex_leading_zeros(&value.to_ascii_lowercase())
}

/// Normalize an Aptos coin-type / identifier string: preserve type case, rewrite
/// every `0x…` hex run in place with [`canonical_aptos_hex_address`].
/// Internal: `normalize_token_identifier` is the one entry point, and it
/// dispatches here by chain.
pub(crate) fn normalize_aptos_token_identifier(value: String) -> String {
    let trimmed = value.trim().to_string();
    let first = trimmed.split("::").next().unwrap_or_default();
    let body = first
        .strip_prefix("0x")
        .or_else(|| first.strip_prefix("0X"))
        .unwrap_or(first);
    let trimmed =
        if !body.is_empty() && body.len() <= 64 && body.bytes().all(|b| b.is_ascii_hexdigit()) {
            format!(
                "{}{}",
                canonical_aptos_hex_address(format!("0x{body}")),
                &trimmed[first.len()..]
            )
        } else {
            trimmed
        };
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

/// Validate the deployment's own standard and identifier together. A chain's
/// protocol set is a capability, never a replacement for protocol identity.
pub fn validate_protocol_identifier(
    chain: crate::registry::Chain,
    standard: &str,
    identifier: &str,
) -> Result<String, crate::SpectraBridgeError> {
    use crate::SpectraBridgeError as E;
    if !chain.allows_token_standard(standard) {
        return Err(E::invalid("token protocol does not belong to network"));
    }
    let identifier = normalize_token_identifier(Some(identifier.into()), chain)
        .ok_or_else(|| E::invalid("protocol token requires an identifier"))?;
    let kind = match standard {
        "ERC-20" | "BEP-20" | "ARC-20" => "evm",
        "SPL" => "solana",
        "TRC-10" => {
            let id = identifier
                .parse::<i64>()
                .ok()
                .filter(|id| *id > 0 && id.to_string() == identifier)
                .ok_or_else(|| E::invalid("invalid TRC-10 token ID"))?;
            return Ok(id.to_string());
        }
        "TRC-20" => "tron",
        "TEP-74" => chain.address_validation_kind(),
        "NEP-141" => "near",
        "Sui Coin" => "suiCoinType",
        "AIP-21" => "aptos",
        "Aptos Coin" => {
            let components = identifier.split("::").collect::<Vec<_>>();
            let move_name = |s: &str| {
                !s.is_empty()
                    && s.bytes()
                        .next()
                        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                    && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            };
            if components.len() != 3 || !move_name(components[1]) || !move_name(components[2]) {
                return Err(E::invalid("invalid Aptos coin type"));
            }
            "aptosTokenType"
        }
        _ => return Err(E::invalid("invalid token identifier for protocol")),
    };
    let result = crate::validation::address::validate_address(
        crate::validation::address::AddressValidationRequest {
            kind: kind.into(),
            value: identifier.clone(),
        },
    );
    if !result.is_valid {
        return Err(E::invalid("invalid token identifier for protocol"));
    }
    // Move type names and base58 identifiers are case-significant. Their
    // validator may case-fold only for checking; identity keeps their case.
    if standard == "AIP-21" {
        return normalize_token_identifier(result.normalized_value, chain)
            .ok_or_else(|| E::invalid("invalid token identifier for protocol"));
    }
    Ok(identifier)
}

/// Identity of a protocol deployment, with its actual standard validated.
pub fn protocol_deployment_id(
    chain: crate::registry::Chain,
    standard: &str,
    identifier: &str,
) -> Option<String> {
    let identifier = validate_protocol_identifier(chain, standard, identifier).ok()?;
    Some(format!(
        "{}:{}:{}",
        chain.str_id(),
        standard.to_lowercase(),
        identifier
    ))
}

/// The canonical form of a token's contract address or identifier on a chain,
/// for grouping and equality.
///
/// Sui and Aptos have structured identifiers (`package::module::type`) with
/// their own canonicalisation; everything else is the trimmed value lowercased.
/// TON masters use the raw account address, independent of friendly routing
/// flags. The network flag is checked before it is discarded.
pub fn normalize_token_identifier(
    contract_address: Option<String>,
    chain: crate::registry::Chain,
) -> Option<String> {
    let raw = contract_address?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    use crate::registry::Chain;
    match chain {
        Chain::Sui | Chain::SuiTestnet => Some(normalize_sui_token_identifier(trimmed.to_string())),
        Chain::Aptos | Chain::AptosTestnet => {
            Some(normalize_aptos_token_identifier(trimmed.to_string()))
        }
        Chain::Ton | Chain::TonTestnet => {
            let address = crate::derivation::ton::parse_ton_address(trimmed)
                .ok()?
                .for_network(chain.is_testnet())
                .ok()?;
            Some(format!(
                "{}:{}",
                address.workchain,
                hex::encode(address.account_id)
            ))
        }
        Chain::Solana | Chain::SolanaDevnet | Chain::Tron | Chain::TronNile => {
            Some(trimmed.to_string())
        }
        _ => Some(trimmed.to_lowercase()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn display_decimals_use_known_deployment_or_custom_precision() {
        assert_eq!(
            super::token_display_decimals(Some("ethereum:native".into()), None),
            18
        );
        assert_eq!(
            super::token_display_decimals(Some("custom:unlisted".into()), Some(6)),
            6
        );
    }

    use super::*;

    /// The rule, not the symptom: a template is already what `canonicalize`
    /// would leave behind, so the two ways of getting an `AssetHolding` for a
    /// catalog token cannot drift apart again. `chain_id` is the field that
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
                token.deployment_id
            );
        }
    }

    /// The spelling itself, on the entry that reported it.
    #[test]
    fn a_native_template_names_its_chain_the_way_a_stored_holding_does() {
        let bitcoin = catalog()
            .iter()
            .find(|t| t.deployment_id == "bitcoin:native")
            .expect("the catalog lists bitcoin");
        assert_eq!(
            bitcoin.chain_id,
            crate::registry::Chain::Bitcoin,
            "the catalog stores a str id"
        );
        assert_eq!(
            bitcoin.holding_template().chain_id,
            crate::registry::Chain::Bitcoin
        );
        assert_eq!(
            bitcoin.holding_template().chain_id,
            crate::registry::Chain::Bitcoin
                .native_holding_template()
                .chain_id,
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

    /// One normalizer, keyed by the chain.
    ///
    /// TON is the arm worth stating: a jetton master address is
    /// case-significant base64, so the lowercase default would produce an
    /// address that does not resolve.
    #[test]
    fn token_identifier_normalisation_is_per_chain() {
        for chain in [
            crate::registry::Chain::Solana,
            crate::registry::Chain::SolanaDevnet,
            crate::registry::Chain::Tron,
            crate::registry::Chain::TronNile,
        ] {
            assert_eq!(
                normalize_token_identifier(Some(" AbCd ".into()), chain),
                Some("AbCd".into()),
                "{chain}"
            );
        }

        assert_eq!(
            normalize_token_identifier(Some("  ".into()), crate::registry::Chain::Ethereum),
            None
        );
        assert_eq!(
            normalize_token_identifier(None, crate::registry::Chain::Ethereum),
            None
        );
        assert_eq!(
            normalize_token_identifier(Some("0xABCDEF".into()), crate::registry::Chain::Ethereum),
            Some("0xabcdef".into())
        );
        assert_eq!(
            normalize_token_identifier(
                Some("0x0002::Foo::bar".into()),
                crate::registry::Chain::Sui
            ),
            Some("0x2::Foo::bar".into())
        );
        assert_eq!(
            normalize_token_identifier(
                Some("0x001::coin::USDC".into()),
                crate::registry::Chain::Aptos
            ),
            Some("0x1::coin::USDC".into())
        );
        assert_eq!(
            normalize_token_identifier(Some("  EQAbC  ".into()), crate::registry::Chain::Ton),
            None,
            "an invalid TON address has no identity"
        );
    }
}

#[cfg(test)]
mod tokens_and_deployments {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn ton_aliases_share_catalog_custom_and_stored_identity_after_network_validation() {
        use crate::registry::Chain;
        use base64::Engine;
        let friendly = |tag| {
            let mut bytes = vec![tag, 0];
            bytes.extend([0x22; 32]);
            let checksum = crate::derivation::ton::crc16_xmodem(&bytes).to_be_bytes();
            bytes.extend(checksum);
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
        };
        let raw = format!("0:{}", "22".repeat(32));
        let id = protocol_deployment_id(Chain::Ton, "TEP-74", &raw).unwrap();
        for alias in [raw.clone(), friendly(0x11), friendly(0x51)] {
            assert_eq!(
                validate_protocol_identifier(Chain::Ton, "TEP-74", &alias).unwrap(),
                raw
            );
            assert_eq!(
                protocol_deployment_id(Chain::Ton, "TEP-74", &alias).unwrap(),
                id
            );
            let mut holding = crate::store::wallet_domain::AssetHolding {
                id: String::new(),
                name: "Jetton".into(),
                symbol: "J".into(),
                coingecko_id: String::new(),
                chain_id: Chain::Ton,
                token_standard: "TEP-74".into(),
                contract_address: Some(alias),
                amount: "1".into(),
            };
            holding.canonicalize().unwrap();
            assert_eq!(holding.contract_address, Some(raw.clone()));
            assert_eq!(holding.deployment_id(), id);
        }
        assert!(validate_protocol_identifier(Chain::Ton, "TEP-74", &friendly(0x91)).is_err());
        assert_eq!(
            validate_protocol_identifier(Chain::TonTestnet, "TEP-74", &friendly(0x91)).unwrap(),
            raw
        );
    }

    // References can precede the definitions and need not follow token order.
    const SAMPLE: &str = r#"
[[deployments]]
token_id = "ether"
chain_id = "ethereum"
kind = "native"
decimals = 18

[[deployments]]
token_id = "usdc"
chain_id = "ethereum"
kind = "token"
contract = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
standard = "ERC-20"
decimals = 6

[[tokens]]
id = "usdc"
symbol = "USDC"
name = "USD Coin"
coingecko_id = "usd-coin"
coinpaprika_id = "usdc-usd-coin"
color = "blue"
artwork_name = "usdc"
tags = []

[[tokens]]
id = "ether"
symbol = "ETH"
name = "Ether"
coingecko_id = "ethereum"
coinpaprika_id = "eth-ethereum"
color = "purple"
artwork_name = "ethereum"
tags = []
"#;

    fn empty_file() -> TomlFile {
        parse_token_file("tokens = []\ndeployments = []").unwrap()
    }

    #[test]
    fn protocol_deployments_on_one_network_own_their_standards() {
        use crate::registry::Chain;
        let mut file = parse_token_file(SAMPLE).unwrap();
        file.deployments[1].chain_id = Chain::Tron;
        file.deployments[1].standard = "TRC-20".into();
        file.deployments[1].contract = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t".into();
        let entries = load_catalog(file, empty_file());
        assert_eq!(entries[1].token_standard, "TRC-20");
        assert!(Chain::Tron.allows_token_standard("TRC-10"));
        assert_eq!(
            validate_protocol_identifier(Chain::Tron, "TRC-10", "1002000").unwrap(),
            "1002000"
        );
        assert!(validate_protocol_identifier(Chain::Tron, "TRC-20", "1002000").is_err());
    }

    #[test]
    fn bittorrent_legacy_and_redenominated_deployments_have_distinct_identities() {
        let old = deployment("tron:trc-10:1002000").unwrap();
        let current = catalog()
            .iter()
            .find(|token| token.token_id == "bittorrent")
            .unwrap();
        assert_eq!(old.token_id, "bittorrent-old");
        assert_eq!((old.token_standard.as_str(), old.decimals), ("TRC-10", 6));
        assert_eq!(
            (current.token_standard.as_str(), current.decimals),
            ("TRC-20", 18)
        );
        assert_ne!(old.token_id, current.token_id);
        assert!(old.coingecko_id.is_empty() && old.coinpaprika_id.is_empty());
    }

    #[test]
    fn trc10_identifier_is_an_exact_positive_decimal_i64() {
        use crate::registry::Chain;
        for valid in ["1", "1002000", "9223372036854775807"] {
            for chain in [Chain::Tron, Chain::TronNile] {
                assert_eq!(
                    validate_protocol_identifier(chain, "TRC-10", valid).unwrap(),
                    valid
                );
                assert_eq!(chain.token_standard_for_identifier(valid), "TRC-10");
                assert_eq!(
                    protocol_deployment_id(chain, "TRC-10", valid).unwrap(),
                    format!("{}:trc-10:{valid}", chain.str_id())
                );
            }
        }
        for invalid in [
            "0",
            "01",
            "-1",
            "+1",
            "1.0",
            "1e6",
            "１００２０００",
            "9223372036854775808",
        ] {
            assert!(
                validate_protocol_identifier(Chain::Tron, "TRC-10", invalid).is_err(),
                "{invalid}"
            );
        }
        assert!(validate_protocol_identifier(Chain::Ethereum, "TRC-10", "1002000").is_err());
    }

    #[test]
    fn alternate_evm_standard_stays_on_the_deployment_and_matching_requires_it() {
        use crate::registry::Chain;
        let mut file = parse_token_file(SAMPLE).unwrap();
        file.deployments[1].chain_id = Chain::BnbChain;
        let entries = load_catalog(file, empty_file());
        let token = &entries[1];
        assert_eq!(Chain::BnbChain.token_standards(), ["ERC-20", "BEP-20"]);
        assert_eq!(token.token_standard, "ERC-20");
        let mut holding = token.holding_template();
        holding.canonicalize().unwrap();
        assert_eq!(holding.token_standard, "ERC-20");
        assert!(token.matches_holding(&holding));
        holding.token_standard = "BEP-20".into();
        assert!(!token.matches_holding(&holding));
    }

    #[test]
    #[should_panic(expected = "duplicate protocol identifier")]
    fn changing_a_standard_alias_cannot_duplicate_a_catalog_asset() {
        use crate::registry::Chain;
        let mut file = parse_token_file(SAMPLE).unwrap();
        file.deployments[1].chain_id = Chain::BnbChain;
        file.deployments.push(TomlDeployment {
            token_id: "usdc".into(),
            chain_id: Chain::BnbChain,
            kind: "token".into(),
            standard: "BEP-20".into(),
            contract: file.deployments[1].contract.clone(),
            decimals: 6,
        });
        load_catalog(file, empty_file());
    }

    #[test]
    fn protocol_validation_rejects_unknown_standards_wrong_networks_and_wrong_identifier_shapes() {
        use crate::registry::Chain;
        let address = "0x1111111111111111111111111111111111111111";
        for (chain, standard, identifier) in [
            (Chain::Solana, "ERC-20", address),
            (Chain::Ethereum, "SPL", "11111111111111111111111111111111"),
            (Chain::Tron, "unknown", "1002000"),
            (Chain::Tron, "TRC-20", "1002000"),
            (Chain::Tron, "TRC-10", "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"),
            (Chain::Tron, "TRC-10", "001002000"),
            (Chain::Tron, "TRC-10", "0"),
            (Chain::Aptos, "AIP-21", "0x1::coin::T"),
            (Chain::Aptos, "Aptos Coin", "0x1"),
            (Chain::Aptos, "Aptos Coin", "0x1::coin"),
        ] {
            assert!(
                validate_protocol_identifier(chain, standard, identifier).is_err(),
                "{chain} {standard} {identifier}"
            );
        }
        assert_eq!(
            validate_protocol_identifier(Chain::Aptos, "Aptos Coin", "0x001::coin::T").unwrap(),
            "0x1::coin::T"
        );
        assert_eq!(
            Chain::Aptos.token_standard_for_identifier("0x1::coin::T"),
            "Aptos Coin"
        );
        assert_eq!(
            validate_protocol_identifier(Chain::Aptos, "Aptos Coin", "0X001::coin::T").unwrap(),
            "0x1::coin::T"
        );
        assert_eq!(
            validate_protocol_identifier(Chain::Aptos, "Aptos Coin", "ABC::coin::T").unwrap(),
            "0xabc::coin::T"
        );
    }

    #[test]
    fn deployment_references_do_not_depend_on_table_order() {
        let mut file = parse_token_file(SAMPLE).unwrap();
        file.tokens.reverse();
        let entries = load_catalog(file, empty_file());
        assert_eq!(entries[0].token_id, "ether");
        assert_eq!(entries[0].symbol, "ETH");
        assert_eq!(entries[0].deployment_id, "ethereum:native");
        assert_eq!(entries[1].symbol, "USDC");
        assert_eq!(entries[1].decimals, 6);
        assert_eq!(
            entries,
            load_catalog(parse_token_file(SAMPLE).unwrap(), empty_file())
        );
    }

    #[test]
    fn broken_token_references_and_legacy_shapes_are_refused() {
        for (file, reason) in [
            (
                SAMPLE.replace("token_id = \"ether\"", "token_id = \"unknown\""),
                "unknown deployment token_id",
            ),
            (
                SAMPLE.replace("\nid = \"ether\"", "\nid = \"usdc\""),
                "duplicate token id",
            ),
            (
                SAMPLE.replace("token_id = \"ether\"", "token_id = \"usdc\""),
                "has no deployment",
            ),
            (
                SAMPLE.replace("token_id = \"ether\"\n", ""),
                "missing field `token_id`",
            ),
            (
                SAMPLE.replace("chain_id =", "network ="),
                "unknown field `network`",
            ),
            (
                format!("{SAMPLE}\n[[tokens.deployments]]\nnetwork = \"ethereum\""),
                "unknown field `deployments`",
            ),
        ] {
            let error = parse_token_file(&file).unwrap_err();
            assert!(error.contains(reason), "{error}");
        }
    }

    #[test]
    fn flat_deployments_still_enforce_network_and_asset_integrity() {
        for file in [
            SAMPLE.replace("chain_id = \"ethereum\"", "chain_id = \"unknown\""),
            SAMPLE.replace("chain_id = \"ethereum\"", "chain_id = \"ethereum-sepolia\""),
            SAMPLE.replace("decimals = 6", "decimals = 39"),
            SAMPLE.replace("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48", "bad-contract"),
        ] {
            assert!(
                std::panic::catch_unwind(|| {
                    load_catalog(parse_token_file(&file).unwrap(), empty_file())
                })
                .is_err()
            );
        }
        let mut duplicate = parse_token_file(SAMPLE).unwrap();
        duplicate
            .deployments
            .push(parse_token_file(SAMPLE).unwrap().deployments.remove(0));
        assert!(std::panic::catch_unwind(|| load_catalog(duplicate, empty_file())).is_err());
        assert!(
            std::panic::catch_unwind(|| {
                load_catalog(
                    parse_token_file(SAMPLE).unwrap(),
                    parse_token_file(SAMPLE).unwrap(),
                )
            })
            .is_err()
        );
        // Testnet assets still cannot borrow a mainnet market identity.
        let testnet = SAMPLE.replace("chain_id = \"ethereum\"", "chain_id = \"ethereum-sepolia\"");
        assert!(
            std::panic::catch_unwind(|| {
                load_catalog(empty_file(), parse_token_file(&testnet).unwrap())
            })
            .is_err()
        );
    }

    /// A token's non-deployment facts are one fact, whatever it is deployed on.
    ///
    /// They were columns on every deployment row, so a token on ten chains
    /// carried ten names and ten colours — and DAI's had already come apart:
    /// "Dai" in orange on Ethereum, "Dai Stablecoin" in yellow on Base and
    /// Polygon. The join makes that unrepresentable; this asserts it.
    #[test]
    fn every_deployment_of_a_token_agrees_about_the_token() {
        let mut seen: HashMap<&str, &TokenDeploymentEntry> = HashMap::new();
        for entry in CATALOG.iter() {
            let first = seen.entry(entry.token_id.as_str()).or_insert(entry);
            for (field, a, b) in [
                ("name", &first.name, &entry.name),
                ("coingecko_id", &first.coingecko_id, &entry.coingecko_id),
                (
                    "coinpaprika_id",
                    &first.coinpaprika_id,
                    &entry.coinpaprika_id,
                ),
                ("artwork_name", &first.artwork_name, &entry.artwork_name),
            ] {
                assert_eq!(
                    a, b,
                    "{}'s {field} differs between {} and {}",
                    entry.symbol, first.chain_id, entry.chain_id
                );
            }
            assert_eq!(first.color, entry.color, "{}'s color differs", entry.symbol);
            assert_eq!(first.tags, entry.tags, "{}'s tags differ", entry.symbol);
        }
    }

    /// Asset identity does not determine deployment precision.
    #[test]
    fn deployment_precision_is_independent_of_asset_identity() {
        let mut file = parse_token_file(SAMPLE).unwrap();
        let mut second = parse_token_file(SAMPLE).unwrap().deployments.remove(1);
        second.chain_id = crate::registry::Chain::Arbitrum;
        second.decimals = 9;
        file.deployments.push(second);
        let entries = load_catalog(file, empty_file());
        assert_eq!(entries[1].token_id, entries[2].token_id);
        assert_eq!(entries[1].decimals, 6);
        assert_eq!(entries[2].decimals, 9);
    }

    /// The contract is the id now, so it has to be spelled the way every
    /// lookup spells it: `deployment_id_for` builds the same string from a
    /// normalized address, and a row that disagrees is a row nothing resolves.
    #[test]
    fn every_catalog_contract_is_already_normalized() {
        for entry in CATALOG.iter().filter(|e| !e.is_native()) {
            let chain = entry.chain_id;
            assert_eq!(
                normalize_token_identifier(Some(entry.contract.clone()), chain).as_deref(),
                Some(entry.contract.as_str()),
                "{} is not in normalized form",
                entry.deployment_id
            );
        }
    }

    #[test]
    fn every_declared_deployment_reaches_the_catalog() {
        let files = [
            embedded_token_file(TOKENS_TOML, "tokens.toml"),
            embedded_token_file(TESTNET_TOKENS_TOML, "testnet-tokens.toml"),
        ];
        assert_eq!(
            CATALOG.len(),
            files.iter().map(|f| f.deployments.len()).sum::<usize>()
        );
    }
}

/// Display precision resolved by deployment; unknown history has no native assumption.
pub(crate) fn token_display_decimals(
    deployment_id: Option<String>,
    custom_decimals: Option<u32>,
) -> u32 {
    deployment_id
        .as_deref()
        .and_then(deployment)
        .map(|t| t.decimals)
        .or(custom_decimals)
        .unwrap_or(18)
        .min(38)
}

/// Resolve known deployments by their actual standard. Unknown identifiers
/// use the network's identifier shape; explicit deployments use
/// `protocol_deployment_id` and never lose their own protocol.
pub fn deployment_id_for(chain: crate::registry::Chain, contract: Option<&str>) -> Option<String> {
    let Some(contract) = contract else {
        return Some(format!("{}:native", chain.str_id()));
    };
    let normalized = normalize_token_identifier(Some(contract.into()), chain)?;
    if let Some(token) = catalog()
        .iter()
        .find(|t| t.chain_id == chain && t.contract == normalized && !t.is_native())
    {
        return Some(token.deployment_id.clone());
    }
    protocol_deployment_id(
        chain,
        chain.token_standard_for_identifier(&normalized),
        &normalized,
    )
}

#[cfg(test)]
mod provider_identity_tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn each_provider_listing_belongs_to_one_catalog_token() {
        let catalog = parse_token_file(TOKENS_TOML).unwrap();
        for provider in ["CoinGecko", "CoinPaprika"] {
            let mut owners = HashMap::new();
            for token in &catalog.tokens {
                let id = match provider {
                    "CoinGecko" => &token.coingecko_id,
                    _ => &token.coinpaprika_id,
                };
                if !id.is_empty() {
                    assert!(
                        owners.insert(id, &token.id).is_none(),
                        "{provider} listing {id} is claimed by multiple token identities"
                    );
                }
            }
        }
    }

    #[test]
    fn both_provider_ids_are_lowercase_and_trimmed() {
        for file in [TOKENS_TOML, TESTNET_TOKENS_TOML] {
            for token in parse_token_file(file).unwrap().tokens {
                for id in [token.coingecko_id, token.coinpaprika_id] {
                    assert_eq!(id.trim().to_lowercase(), id);
                }
            }
        }
    }
}

#[cfg(test)]
mod pending_catalog_tests {
    use super::*;
    use std::collections::HashSet;

    fn pending_records(source: &str) -> toml::Value {
        let mut pending = false;
        let mut records = String::new();
        for line in source.lines() {
            if line.starts_with("# TODO(verify-token):") {
                pending = true;
            } else if pending {
                if let Some(record) = line.strip_prefix("# ") {
                    records.push_str(record);
                    records.push('\n');
                } else if line == "#" {
                    records.push('\n');
                } else {
                    pending = false;
                }
            }
        }
        toml::from_str(&records).expect("pending catalog comments remain complete TOML records")
    }

    #[test]
    fn researched_deployments_follow_recorded_catalog_decisions() {
        let archive: serde_json::Value = serde_json::from_str(include_str!(
            "../../docs/audits/chain-support-2026-10-04/removed-token-deployments.json"
        ))
        .unwrap();
        let audit: serde_json::Value = serde_json::from_str(include_str!(
            "../../docs/audits/token-verification-2026-10-04/verification.json"
        ))
        .unwrap();
        let decisions = audit["deployments"].as_array().unwrap();
        assert_eq!(
            decisions.len(),
            archive["deployments"].as_array().unwrap().len()
        );
        let pending = pending_records(TOKENS_TOML);
        let active: toml::Value = toml::from_str(TOKENS_TOML).unwrap();
        let active_deployments = serde_json::to_value(&active["deployments"]).unwrap();
        let pending_deployments = pending
            .get("deployments")
            .map(|rows| serde_json::to_value(rows).unwrap())
            .unwrap_or_else(|| serde_json::json!([]));
        for record in archive["deployments"].as_array().unwrap() {
            let matching_decisions: Vec<_> = decisions
                .iter()
                .filter(|entry| entry["original_deployment"] == *record)
                .collect();
            assert_eq!(
                matching_decisions.len(),
                1,
                "one catalog decision per original: {record}"
            );
            let expected_counts = match matching_decisions[0]["catalog_decision"].as_str().unwrap()
            {
                "active" => (1, 0),
                "pending" => (0, 1),
                "removed_at_user_request" => (0, 0),
                decision => panic!("unknown catalog decision: {decision}"),
            };
            let active_count = active_deployments
                .as_array()
                .unwrap()
                .iter()
                .filter(|row| *row == record)
                .count();
            let pending_count = pending_deployments
                .as_array()
                .unwrap()
                .iter()
                .filter(|row| *row == record)
                .count();
            assert_eq!(
                (active_count, pending_count),
                expected_counts,
                "catalog must match its recorded decision with exact original fields: {record}"
            );
        }
        let pending_ids: HashSet<_> = pending
            .get("tokens")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .map(|token| token["id"].as_str().unwrap())
            .collect();
        let active = parse_token_file(TOKENS_TOML).unwrap();
        assert!(
            active
                .tokens
                .iter()
                .all(|token| !pending_ids.contains(token.id.as_str()))
        );
        let wiki = pending_records(include_str!("../data/crypto-wiki.toml"));
        let wiki_ids: HashSet<_> = wiki
            .get("assets")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .map(|asset| asset["token_id"].as_str().unwrap())
            .collect();
        assert_eq!(
            pending_ids, wiki_ids,
            "pending identities retain their original wiki descriptions"
        );
    }
}
