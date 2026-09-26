//! One module per noun. Each asks core for every decision it reports.

pub mod address;
pub mod address_pool;
pub mod alert;
pub mod chain;
pub mod diagnostics;
pub mod market;
pub mod network;
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
/// The display name for a chain id, for text a person reads.
pub fn chain_name(chain_id: &str) -> String {
    Chain::display_name_for_id(chain_id)
}

pub fn resolve_chain(needle: &str) -> CliResult<Chain> {
    let trimmed = needle.trim();
    Chain::from_display_name(trimmed)
        .or_else(|| Chain::from_str_id(&trimmed.to_lowercase().replace([' ', '_'], "-")))
        .ok_or_else(|| {
            CliError::usage(format!(
                "unknown chain {needle:?} — run `spectra chains` for the list"
            ))
        })
}

/// Refuse a seed phrase core would not accept, naming what is wrong with it.
///
/// The CLI reads phrases from a file or the environment, so it has no
/// language picker and no expected length: the phrase's own word count is
/// what it claims to be, and core checks that claim against every BIP-39
/// language.
pub fn reject_bad_seed_phrase(phrase: &str) -> CliResult<()> {
    use spectra_core::validation::{SeedPhraseCheck, check_seed_phrase};
    let words: Vec<String> = phrase.split_whitespace().map(str::to_string).collect();
    let verdict = check_seed_phrase(SeedPhraseCheck {
        expected_word_count: words.len() as u32,
        words,
        language: None,
    });
    if !verdict.invalid_words.is_empty() {
        return Err(CliError::rejected(format!(
            "not in any BIP-39 word list: {}",
            verdict.invalid_words.join(", ")
        )));
    }
    if !verdict.checksum_valid {
        return Err(CliError::rejected(verdict.error.unwrap_or_else(|| {
            "not a valid BIP-39 mnemonic (check the words and the count)".to_string()
        })));
    }
    Ok(())
}
