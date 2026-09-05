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

/// The bundled artwork an icon identifier names, or empty when none ships.
///
/// Resolution is by the coin's own symbol, because that is the only component
/// of an identifier that says what to *draw*. The chain component says where a
/// coin is held, and coins are routinely held away from home — USDC on Aptos,
/// ETH on Base — so artwork keyed on the chain drew a letter for thirty-one of
/// the wiki's sixty-six coins and for every per-chain breakdown row whose coin
/// is not its chain's own ticker.
#[uniffi::export]
pub fn core_icon_asset_name(identifier: String) -> String {
    let symbol = icon_identifier_symbol(&identifier).trim().to_uppercase();
    asset_name_by_symbol()
        .get(&symbol)
        .cloned()
        .unwrap_or_default()
}

/// The symbol an icon identifier carries: the third component of
/// `<namespace>:<chain>:<symbol>[:<contract>]`, or the last one of anything
/// shorter, so a bare symbol resolves as itself.
fn icon_identifier_symbol(identifier: &str) -> &str {
    let mut components = identifier.trim().split(':');
    match (components.next(), components.next(), components.next()) {
        (_, _, Some(symbol)) => symbol,
        (_, Some(symbol), None) => symbol,
        (Some(symbol), None, None) => symbol,
        _ => "",
    }
}

/// Coin symbol → the artwork that ships for it.
///
/// Three passes, most specific first. A chain's own ticker wins, so BASE draws
/// Base; then the token catalog, so UNI draws Uniswap; then the gas token of
/// the first chain in catalog order that pays fees in it, so OKB draws X
/// Layer's mark and ETH draws Ethereum's rather than one of its nine rollups'.
fn asset_name_by_symbol() -> &'static HashMap<String, String> {
    use std::sync::OnceLock;
    static LOOKUP: OnceLock<HashMap<String, String>> = OnceLock::new();
    LOOKUP.get_or_init(|| {
        let mut out: HashMap<String, String> = HashMap::new();
        for chain in crate::chains::catalog() {
            claim_artwork(&mut out, &chain.symbol, &chain.asset_name);
        }
        for token in crate::tokens::catalog() {
            claim_artwork(&mut out, &token.symbol, &token.asset_name);
        }
        for chain in crate::chains::catalog() {
            claim_artwork(&mut out, &chain.gas_token_symbol, &chain.asset_name);
        }
        out
    })
}

fn claim_artwork(out: &mut HashMap<String, String>, symbol: &str, asset_name: &str) {
    let key = symbol.trim().to_uppercase();
    let artwork = asset_name.trim();
    if key.is_empty() || artwork.is_empty() {
        return;
    }
    out.entry(key).or_insert_with(|| artwork.to_string());
}

#[cfg(test)]
mod artwork_follows_the_coin_not_the_chain {
    use super::core_icon_asset_name;

    /// A coin held away from home draws itself. Every one of these resolved to
    /// nothing before: the identifier says `native:<host chain>:<symbol>`, and
    /// the lookup in front of it was keyed on the host chain's own ticker.
    #[test]
    fn a_token_draws_its_own_mark_on_every_chain_it_lives_on() {
        for chain in ["ethereum", "base", "aptos", "solana"] {
            assert_eq!(
                core_icon_asset_name(format!("native:{chain}:usdc")),
                "circleusdc"
            );
        }
        assert_eq!(
            core_icon_asset_name("token:ethereum:usdc:0xa0b8".to_string()),
            "circleusdc"
        );
        assert_eq!(core_icon_asset_name("native:base:eth".to_string()), "ethereum");
        assert_eq!(core_icon_asset_name("native:ethereum:shib".to_string()), "shibainu");
        assert_eq!(core_icon_asset_name("native:base:dai".to_string()), "skydai");
    }

    /// A chain's own ticker draws the chain. Base's gas is ETH, so the two
    /// live side by side and neither may answer for the other.
    #[test]
    fn a_chain_ticker_draws_the_chain() {
        assert_eq!(core_icon_asset_name("native:base:base".to_string()), "base");
        assert_eq!(core_icon_asset_name("native:ethereum:eth".to_string()), "ethereum");
        assert_eq!(core_icon_asset_name("native:arbitrum:arb".to_string()), "arbitrum");
    }

    /// A gas token nothing else claims falls back to the chain that pays in
    /// it. OKB is X Layer's gas and no chain's ticker — X Layer's is the
    /// string "X Layer" — so this was the one native coin drawn as a letter.
    #[test]
    fn a_gas_token_falls_back_to_the_chain_that_pays_in_it() {
        assert_eq!(core_icon_asset_name("native:x-layer:okb".to_string()), "okb");
        assert_eq!(core_icon_asset_name("native:x-layer:x layer".to_string()), "okb");
    }

    /// Every coin the wiki lists has artwork, and the wiki is every coin the
    /// app can hold. This is the assertion the bug would have failed on 31 of
    /// 66 rows.
    #[test]
    fn every_wiki_coin_has_artwork() {
        for asset in crate::wiki::list_asset_wiki() {
            assert_eq!(
                core_icon_asset_name(asset.symbol.clone()),
                asset.asset_name,
                "{} draws the wrong mark",
                asset.symbol
            );
            assert!(!asset.asset_name.is_empty(), "{} has no mark", asset.symbol);
        }
    }

    /// Every mark a catalog names ships as a file. USDB named `usdb` and no
    /// such icon existed, so the one coin whose artwork was genuinely missing
    /// looked exactly like the thirty-one that were only looked up wrong.
    #[test]
    fn every_named_mark_ships_a_file() {
        let icons = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources/coinicon");
        for asset in crate::wiki::list_asset_wiki() {
            let file = icons.join(format!("{}.svg", asset.asset_name));
            assert!(
                file.is_file(),
                "{} names the mark {} and {} does not exist",
                asset.symbol,
                asset.asset_name,
                file.display()
            );
        }
    }

    /// A symbol nothing ships gets nothing, so the caller draws its letter
    /// rather than someone else's logo. The substring match this replaced
    /// would have handed `usdce` the USDC mark.
    #[test]
    fn an_unknown_symbol_resolves_to_nothing() {
        assert_eq!(core_icon_asset_name("token:ethereum:usdce:0x00".to_string()), "");
        assert_eq!(core_icon_asset_name("".to_string()), "");
        assert_eq!(core_icon_asset_name("Wallet name".to_string()), "");
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
