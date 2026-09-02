//! Canonical chain-name / icon-identifier helpers shared across UI planners.

use std::collections::HashMap;

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

/// The registry id a chain name or native symbol stands for.
///
/// Two hand-written tables sat in front of this — forty name pairs and
/// thirty-five symbol pairs. Thirty-five of the forty were the registry id
/// spelled again; the other five (`zcash → zec`, `bitcoin gold → btg`,
/// `zksync era → zksync`, `x layer → okb`, `bittensor → tao`) were compared
/// against `NativeChainIconDescriptor.registryID`, which *is* the registry id,
/// so they matched nothing and fell through to the chain-name comparison that
/// would have answered anyway. Both tables covered forty of seventy-eight
/// chains; the registry covers all of them.
pub(super) fn canonical_chain_component_inner(chain_name: &str, symbol: &str) -> String {
    // HashMap<String, String> requires an owned key for lookup; allocate once.
    let normalized_chain_lower = chain_name.trim().to_lowercase();
    if let Some(id) = chain_id_by_chain_name().get(&normalized_chain_lower) {
        return id.clone();
    }
    let trimmed_symbol = symbol.trim();
    if !trimmed_symbol.is_empty() {
        if let Some(id) = chain_id_by_native_symbol().get(&trimmed_symbol.to_uppercase()) {
            return id.clone();
        }
    }
    normalized_chain_lower.replace(' ', "-")
}

/// Native gas symbol → the id of the first chain in catalog order that pays
/// its fees in it. The EVM family shares ETH, and Ethereum comes first.
fn chain_id_by_native_symbol() -> &'static HashMap<String, String> {
    use std::sync::OnceLock;
    static LOOKUP: OnceLock<HashMap<String, String>> = OnceLock::new();
    LOOKUP.get_or_init(|| {
        let mut out = HashMap::new();
        for c in crate::chains::catalog() {
            if c.gas_token_symbol.is_empty() {
                continue;
            }
            out.entry(c.gas_token_symbol.trim().to_uppercase())
                .or_insert_with(|| c.id.clone());
        }
        out
    })
}

#[uniffi::export]
pub fn core_canonical_chain_component(chain_name: String, symbol: String) -> String {
    canonical_chain_component_inner(&chain_name, &symbol)
}

#[uniffi::export]
pub fn core_icon_identifier(
    symbol: String,
    chain_name: String,
    contract_address: Option<String>,
    token_standard: String,
) -> String {
    let normalized_symbol = symbol.to_lowercase();
    let trimmed_contract = contract_address
        .map(|c| c.trim().to_string())
        .unwrap_or_default();
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

#[uniffi::export]
pub fn core_normalized_icon_identifier(identifier: String) -> String {
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


#[cfg(test)]
mod canonical_component_covers_the_catalog {
    use super::canonical_chain_component_inner;
    use crate::registry::Chain;

    /// Every chain resolves to its own registry id, by name and by native
    /// symbol. The tables this replaced covered forty of seventy-eight.
    #[test]
    fn every_chain_name_resolves_to_its_registry_id() {
        for chain in Chain::all() {
            assert_eq!(
                canonical_chain_component_inner(chain.chain_display_name(), ""),
                chain.str_id(),
                "{} did not resolve to its own id",
                chain.chain_display_name()
            );
        }
    }

    /// A symbol with no chain name still finds a chain, and where several
    /// chains share a symbol it is the first in catalog order.
    #[test]
    fn a_bare_native_symbol_resolves() {
        assert_eq!(canonical_chain_component_inner("", "BTC"), "bitcoin");
        assert_eq!(canonical_chain_component_inner("", "ETH"), "ethereum");
        assert_eq!(canonical_chain_component_inner("", "ZEC"), "zcash");
        assert_eq!(canonical_chain_component_inner("", "TAO"), "bittensor");
    }

    /// Something the registry has never heard of is slugged, not dropped.
    #[test]
    fn an_unknown_name_falls_back_to_a_slug() {
        assert_eq!(
            canonical_chain_component_inner("Some New Chain", ""),
            "some-new-chain"
        );
    }
}
