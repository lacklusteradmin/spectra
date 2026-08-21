//! One module per noun. Each asks core for every decision it reports.

pub mod address;
pub mod alert;
pub mod address_pool;
pub mod chain;
pub mod diagnostics;
pub mod market;
pub mod refresh;
pub mod rescan;
pub mod settings;
pub mod staking;
pub mod token;
pub mod tx;
pub mod wallet;

use crate::error::{CliError, CliResult};
use spectra_core::registry::Chain;

/// One lookup, so `bitcoin`, `Bitcoin` and `BTC` behave the same everywhere.
/// The previous CLI had three near-identical resolvers and they disagreed.
pub fn resolve_chain(needle: &str) -> CliResult<Chain> {
    let trimmed = needle.trim();
    Chain::from_display_name(trimmed)
        .or_else(|| Chain::from_str_id(&trimmed.to_lowercase().replace([' ', '_'], "-")))
        .or_else(|| {
            Chain::all().find(|chain| {
                chain.coin_name().eq_ignore_ascii_case(trimmed)
                    || chain.coin_symbol().eq_ignore_ascii_case(trimmed)
            })
        })
        .ok_or_else(|| {
            CliError::usage(format!(
                "unknown chain {needle:?} — run `spectra chains` for the list"
            ))
        })
}
