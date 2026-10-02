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
pub fn chain_name(chain: Chain) -> String {
    chain.chain_display_name().to_string()
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
/// language picker and no expected length: core infers both from the words,
/// as the import page does.
pub fn reject_bad_seed_phrase(phrase: &str) -> CliResult<()> {
    use spectra_core::validation::{SeedPhraseCheck, check_seed_phrase};
    let verdict = check_seed_phrase(SeedPhraseCheck {
        words: phrase.split_whitespace().map(str::to_string).collect(),
        language: None,
        word_count: None,
    });
    if !verdict.invalid_words.is_empty() {
        return Err(CliError::rejected(match verdict.language {
            Some(language) => format!(
                "not in the {} BIP-39 word list: {}",
                language.name,
                verdict.invalid_words.join(", ")
            ),
            None => format!(
                "not in any BIP-39 word list: {}",
                verdict.invalid_words.join(", ")
            ),
        }));
    }
    if !verdict.is_complete {
        return Err(CliError::rejected(format!(
            "{} words; a seed phrase has 12, 15, 18, 21 or 24",
            verdict.words.len()
        )));
    }
    if !verdict.checksum_valid {
        return Err(CliError::rejected(verdict.problem.map_or_else(
            || "not a valid BIP-39 mnemonic (check the words and the count)".to_string(),
            seed_phrase_problem_text,
        )));
    }
    Ok(())
}

/// Core's seed-phrase problem, worded for the terminal.
pub fn seed_phrase_problem_text(problem: spectra_core::validation::SeedPhraseProblem) -> String {
    use spectra_core::validation::SeedPhraseProblem;
    match problem {
        SeedPhraseProblem::NonStandardLength { word_count } => {
            format!("{word_count} words; a seed phrase has 12, 15, 18, 21 or 24")
        }
        SeedPhraseProblem::WrongWordCount { expected } => {
            format!("seed phrase must be {expected} words")
        }
        SeedPhraseProblem::InvalidChecksum => {
            "invalid seed phrase checksum; check the words".to_string()
        }
    }
}
