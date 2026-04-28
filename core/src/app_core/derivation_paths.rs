use super::{
    AppCoreCatalog, AppCoreRequestCompilationPreset, AppCoreScriptPolicy, AppCoreScriptType,
    DerivationPathSegment,
};
use crate::store::wallet_domain::CoreSeedDerivationPaths;

pub(crate) fn parse_derivation_path(raw_path: &str) -> Option<Vec<DerivationPathSegment>> {
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
                .map(|value| DerivationPathSegment { value, is_hardened })
        })
        .collect()
}

pub(crate) fn normalize_derivation_path(raw_path: &str, fallback: &str) -> String {
    parse_derivation_path(raw_path)
        .map(|segments| derivation_path_string(&segments))
        .unwrap_or_else(|| fallback.to_string())
}

pub(crate) fn derivation_path_string(segments: &[DerivationPathSegment]) -> String {
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

pub(crate) fn derivation_path_segment_value(path: &str, index: usize) -> Option<u32> {
    parse_derivation_path(path).and_then(|segments| segments.get(index).map(|s| s.value))
}

pub(crate) fn compile_script_type(
    preset: &AppCoreRequestCompilationPreset,
    derivation_path: Option<&str>,
) -> Result<AppCoreScriptType, String> {
    match preset.script_policy {
        AppCoreScriptPolicy::BitcoinPurpose => {
            let purpose = derivation_path
                .and_then(|path| derivation_path_segment_value(path, 0))
                .ok_or_else(|| {
                    "Unable to compile Bitcoin script type from derivation path.".to_string()
                })?;
            let map = preset.bitcoin_purpose_script_map.as_ref().ok_or_else(|| {
                "Bitcoin purpose script policy requires bitcoinPurposeScriptMap.".to_string()
            })?;
            map.get(&purpose.to_string())
                .copied()
                .ok_or_else(|| format!("Unsupported Bitcoin derivation purpose {purpose}."))
        }
        AppCoreScriptPolicy::Fixed => preset
            .fixed_script_type
            .ok_or_else(|| "Fixed script policy requires fixedScriptType.".to_string()),
    }
}

pub(super) fn resolved_account_index(chain_name: &str, normalized_path: &str) -> u32 {
    match chain_name {
        "Bitcoin" if normalized_path == "m/0'/0" || normalized_path == "m/0'/0/0" => 0,
        "Bitcoin Cash" | "Bitcoin SV" if normalized_path == "m/0" => 0,
        _ => derivation_path_segment_value(normalized_path, 2).unwrap_or(0),
    }
}

pub(super) fn resolved_flavor(chain_name: &str, normalized_path: &str) -> &'static str {
    match chain_name {
        "Bitcoin" => match normalized_path {
            p if p.starts_with("m/86'") => "taproot",
            p if p.starts_with("m/84'") => "nativeSegWit",
            p if p.starts_with("m/49'") => "nestedSegWit",
            "m/0'/0" | "m/0'/0/0" => "electrumLegacy",
            p if p.starts_with("m/44'") => "legacy",
            _ => "standard",
        },
        "Litecoin" => match normalized_path {
            p if p.starts_with("m/84'/2'") => "nativeSegWit",
            p if p.starts_with("m/49'/2'") => "nestedSegWit",
            p if p.starts_with("m/44'/2'") => "legacy",
            _ => "standard",
        },
        "Bitcoin Cash" => match normalized_path {
            "m/0" => "electrumLegacy",
            p if p.starts_with("m/44'/0'") || p.starts_with("m/44'/145'") => "legacy",
            _ => "standard",
        },
        "Solana" if normalized_path == "m/44'/501'/0'" => "legacy",
        "Cardano" if normalized_path.starts_with("m/44'/1815'") => "legacy",
        "Tron"
            if normalized_path == "m/44'/195'/0'"
                || normalized_path.starts_with("m/44'/60'") =>
        {
            "legacy"
        }
        "XRP Ledger" if normalized_path == "m/44'/144'/0'" => "legacy",
        _ => "standard",
    }
}

pub(super) fn seed_derivation_paths_for_account(
    catalog: &AppCoreCatalog,
    account: u32,
) -> Result<CoreSeedDerivationPaths, String> {
    // SLIP-44 standard `m/44'/coin'/account'/0/0` is the most common shape;
    // a few chains diverge (Solana, Stellar, NEAR, Polkadot, Sui, Aptos).
    let evm = slip44(60, account);
    Ok(CoreSeedDerivationPaths {
        is_custom_enabled: false,
        bitcoin: format!("m/84'/0'/{account}'/0/0"),
        bitcoin_cash: format!("m/44'/145'/{account}'/0/0"),
        bitcoin_sv: default_path_from_catalog(catalog, "Bitcoin SV")?,
        litecoin: slip44(2, account),
        dogecoin: slip44(3, account),
        ethereum: evm.clone(),
        ethereum_classic: slip44(61, account),
        arbitrum: evm.clone(),
        optimism: evm.clone(),
        avalanche: evm.clone(),
        hyperliquid: evm.clone(),
        polygon: evm.clone(),
        base: evm.clone(),
        linea: evm.clone(),
        scroll: evm.clone(),
        blast: evm.clone(),
        mantle: evm.clone(),
        tron: slip44(195, account),
        solana: format!("m/44'/501'/{account}'/0'"),
        stellar: format!("m/44'/148'/{account}'"),
        xrp: slip44(144, account),
        cardano: format!("m/1852'/1815'/{account}'/0/0"),
        sui: format!("m/44'/784'/{account}'/0'/0'"),
        aptos: format!("m/44'/637'/{account}'/0'/0'"),
        ton: slip44(607, account),
        internet_computer: slip44(223, account),
        near: format!("m/44'/397'/{account}'"),
        polkadot: format!("m/44'/354'/{account}'"),
        zcash: slip44(133, account),
        bitcoin_gold: slip44(156, account),
        // EVM L1/L2s share the EVM derivation path (SLIP-44 60).
        sei: evm.clone(),
        celo: evm.clone(),
        cronos: evm.clone(),
        op_bnb: evm.clone(),
        zksync_era: evm.clone(),
        sonic: evm.clone(),
        berachain: evm.clone(),
        unichain: evm.clone(),
        ink: evm,
        decred: slip44(42, account),
        // Kaspa SLIP-44 coin type 111111.
        kaspa: format!("m/44'/111111'/{account}'/0/0"),
        dash: slip44(5, account),
        // X Layer is an EVM L2 — uses the standard EVM derivation path.
        x_layer: slip44(60, account),
        // Bittensor uses SLIP-44 1005 (Polkadot.js convention for substrate
        // chains; the substrate-bip39 expansion ignores BIP-32 path nodes
        // but we include the canonical path for downstream display).
        bittensor: format!("m/44'/1005'/{account}'/0'/0'"),
    })
}

fn slip44(coin_type: u32, account: u32) -> String {
    format!("m/44'/{coin_type}'/{account}'/0/0")
}

pub(super) fn default_path_from_catalog(
    catalog: &AppCoreCatalog,
    chain_name: &str,
) -> Result<String, String> {
    catalog
        .chain_presets
        .iter()
        .find(|p| p.chain == chain_name)
        .and_then(|p| {
            p.derivation_paths
                .iter()
                .find(|path| path.is_default)
                .or_else(|| p.derivation_paths.first())
        })
        .map(|p| p.derivation_path.clone())
        .ok_or_else(|| format!("Missing default derivation path for {chain_name}."))
}

#[cfg(test)]
pub(super) fn default_path_for_chain(chain_name: &str) -> Result<String, String> {
    default_path_from_catalog(super::app_core_catalog()?, chain_name)
}
