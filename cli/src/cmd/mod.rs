//! One module per noun. Each asks core for every decision it reports.

pub mod address;
pub mod address_pool;
pub mod alert;
pub mod chain;
pub mod diagnostics;
pub mod market;
pub mod psbt;
pub mod refresh;
pub mod rescan;
pub mod settings;
pub mod staking;
pub mod token;
pub mod tx;
pub mod wallet;

use crate::error::{CliError, CliResult};
use spectra_core::registry::Chain;

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
/// language picker and no expected length: core infers both from the words
/// in the chain's own formats, as the import page does; BIP-39 for a phrase
/// on no chain, which is what Funds Finder scans.
pub fn reject_bad_seed_phrase(chain: Option<Chain>, phrase: &str) -> CliResult<()> {
    use spectra_core::validation::{SeedPhraseCheck, check_seed_phrase};
    let verdict = check_seed_phrase(SeedPhraseCheck {
        words: phrase.split_whitespace().map(str::to_string).collect(),
        language: None,
        word_count: None,
        chain,
    });
    let kind = chain.map_or("BIP-39", |chain| chain.chain_display_name());
    if !verdict.invalid_words.is_empty() {
        return Err(CliError::rejected(match verdict.language {
            Some(language) => format!(
                "not in the {} word list: {}",
                language.name,
                verdict.invalid_words.join(", ")
            ),
            None => format!(
                "not {kind} phrase words: {}",
                verdict.invalid_words.join(", ")
            ),
        }));
    }
    if !verdict.is_valid {
        return Err(CliError::rejected(verdict.problem.map_or_else(
            || format!("not a {kind} phrase (check the words and the count)"),
            seed_phrase_problem_text,
        )));
    }
    Ok(())
}

/// Core's seed-phrase problem, worded for the terminal.
pub fn seed_phrase_problem_text(problem: spectra_core::validation::SeedPhraseProblem) -> String {
    use spectra_core::validation::SeedPhraseProblem;
    match problem {
        SeedPhraseProblem::NonStandardLength {
            word_count,
            allowed,
        } => {
            let allowed: Vec<String> = allowed.iter().map(u32::to_string).collect();
            format!(
                "{word_count} words; a phrase here has {}",
                allowed.join(", ")
            )
        }
        SeedPhraseProblem::WrongWordCount { expected } => {
            format!("seed phrase must be {expected} words")
        }
        SeedPhraseProblem::InvalidChecksum => {
            "invalid seed phrase checksum; check the words".to_string()
        }
        SeedPhraseProblem::AmbiguousLanguage => {
            "the words read as different phrases in two languages; type them in full".to_string()
        }
        SeedPhraseProblem::EncryptedPolyseed => {
            "this Polyseed is encrypted with a password, which Spectra cannot take".to_string()
        }
        SeedPhraseProblem::UnsupportedPolyseed => {
            "this Polyseed uses features Spectra does not support".to_string()
        }
    }
}
