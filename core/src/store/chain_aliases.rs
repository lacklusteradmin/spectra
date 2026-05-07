//! Canonical chain-name / icon-identifier helpers shared across UI planners.

use std::collections::HashMap;

fn known_chain_aliases() -> &'static [(&'static str, &'static str)] {
    &[
        ("bitcoin", "bitcoin"),
        ("bitcoin cash", "bitcoin-cash"),
        ("bitcoin sv", "bitcoin-sv"),
        ("litecoin", "litecoin"),
        ("dogecoin", "dogecoin"),
        ("ethereum", "ethereum"),
        ("ethereum classic", "ethereum-classic"),
        ("arbitrum", "arbitrum"),
        ("optimism", "optimism"),
        ("bnb chain", "bnb"),
        ("avalanche", "avalanche"),
        ("hyperliquid", "hyperliquid"),
        ("tron", "tron"),
        ("solana", "solana"),
        ("stellar", "stellar"),
        ("cardano", "cardano"),
        ("xrp ledger", "xrp"),
        ("monero", "monero"),
        ("sui", "sui"),
        ("aptos", "aptos"),
        ("ton", "ton"),
        ("internet computer", "internet-computer"),
        ("near", "near"),
        ("polkadot", "polkadot"),
        ("zcash", "zec"),
        ("bitcoin gold", "btg"),
        ("decred", "decred"),
        ("kaspa", "kaspa"),
        ("sei", "sei"),
        ("celo", "celo"),
        ("cronos", "cronos"),
        ("opbnb", "opbnb"),
        ("zksync era", "zksync"),
        ("sonic", "sonic"),
        ("berachain", "berachain"),
        ("unichain", "unichain"),
        ("ink", "ink"),
        ("dash", "dash"),
        ("x layer", "okb"),
        ("bittensor", "tao"),
    ]
}

fn native_symbol_chain_aliases() -> &'static [(&'static str, &'static str)] {
    &[
        ("BTC", "bitcoin"),
        ("BCH", "bitcoin-cash"),
        ("BSV", "bitcoin-sv"),
        ("LTC", "litecoin"),
        ("DOGE", "dogecoin"),
        ("ETH", "ethereum"),
        ("ETC", "ethereum-classic"),
        ("ARB", "arbitrum"),
        ("OP", "optimism"),
        ("BNB", "bnb"),
        ("AVAX", "avalanche"),
        ("HYPE", "hyperliquid"),
        ("TRX", "tron"),
        ("SOL", "solana"),
        ("XLM", "stellar"),
        ("ADA", "cardano"),
        ("XRP", "xrp"),
        ("XMR", "monero"),
        ("SUI", "sui"),
        ("APT", "aptos"),
        ("TON", "ton"),
        ("ICP", "internet-computer"),
        ("NEAR", "near"),
        ("DOT", "polkadot"),
        ("ZEC", "zec"),
        ("BTG", "btg"),
        ("DCR", "decred"),
        ("KAS", "kaspa"),
        ("SEI", "sei"),
        ("CELO", "celo"),
        ("CRO", "cronos"),
        ("BERA", "berachain"),
        ("DASH", "dash"),
        ("OKB", "okb"),
        ("TAO", "tao"),
    ]
}

fn chain_id_by_chain_name() -> &'static HashMap<String, String> {
    use std::sync::OnceLock;
    static LOOKUP: OnceLock<HashMap<String, String>> = OnceLock::new();
    LOOKUP.get_or_init(|| {
        crate::chains::catalog()
            .iter()
            .filter(|c| !c.name.is_empty())
            .map(|c| (c.name.trim().to_lowercase(), c.id.clone()))
            .collect()
    })
}

pub(super) fn canonical_chain_component_inner(chain_name: &str, symbol: &str) -> String {
    // Most lookups hit `known_chain_aliases` on the first pass, so defer the
    // String allocations as far as possible. Was: unconditional `.to_lowercase()`
    // on the chain name + `.to_uppercase()` on the symbol — two heap allocs
    // per call. Now: zero allocs in the hot path (alias hit), one in the
    // wiki-lookup path, none in the symbol-alias path.
    let trimmed_chain = chain_name.trim();
    let trimmed_symbol = symbol.trim();
    if let Some((_, alias)) = known_chain_aliases()
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(trimmed_chain))
    {
        return (*alias).to_string();
    }
    // HashMap<String, String> requires an owned key for lookup; allocate once.
    let normalized_chain_lower = trimmed_chain.to_lowercase();
    if let Some(id) = chain_id_by_chain_name().get(&normalized_chain_lower) {
        return id.clone();
    }
    if let Some((_, alias)) = native_symbol_chain_aliases()
        .iter()
        .find(|(sym, _)| sym.eq_ignore_ascii_case(trimmed_symbol))
    {
        return (*alias).to_string();
    }
    normalized_chain_lower.replace(' ', "-")
}

pub fn plan_canonical_chain_component(chain_name: String, symbol: String) -> String {
    canonical_chain_component_inner(&chain_name, &symbol)
}

pub fn plan_icon_identifier(
    symbol: String,
    chain_name: String,
    contract_address: Option<String>,
    token_standard: String,
) -> String {
    let normalized_symbol = symbol.to_lowercase();
    let trimmed_contract = contract_address.map(|c| c.trim().to_string()).unwrap_or_default();
    let normalized_chain = canonical_chain_component_inner(&chain_name, &symbol);
    if !trimmed_contract.is_empty() {
        return format!(
            "token:{}:{}:{}",
            normalized_chain,
            normalized_symbol,
            trimmed_contract.to_lowercase()
        );
    }
    let is_native_token =
        token_standard.eq_ignore_ascii_case("Native") || token_standard.is_empty();
    let namespace = if is_native_token { "native" } else { "asset" };
    format!("{namespace}:{normalized_chain}:{normalized_symbol}")
}

pub fn plan_normalized_icon_identifier(identifier: String) -> String {
    let trimmed_identifier = identifier.trim().to_string();
    let components: Vec<String> = trimmed_identifier.split(':').map(String::from).collect();
    if components.len() < 3 {
        return trimmed_identifier;
    }
    let namespace = &components[0];
    let chain_component = &components[1];
    let symbol_component = &components[2];
    match namespace.as_str() {
        "native" | "asset" | "token" => {
            let canonical_chain =
                canonical_chain_component_inner(chain_component, symbol_component);
            let mut normalized = components.clone();
            normalized[0] = namespace.clone();
            normalized[1] = canonical_chain;
            normalized[2] = symbol_component.to_lowercase();
            if normalized.len() >= 4 {
                normalized[3] = normalized[3].to_lowercase();
            }
            normalized.join(":")
        }
        _ => trimmed_identifier,
    }
}
