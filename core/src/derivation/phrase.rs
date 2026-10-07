//! The phrase formats each network restores from and creates.
//!
//! Most networks read BIP-39. Monero reads its own 25-word seed and Polyseed,
//! and TON its own 24-word mnemonic: a phrase in another format restores no
//! wallet those networks' own apps would, so it is refused rather than read
//! another way. The setup verdict, the import and creation all ask here, so
//! the three cannot disagree about what a phrase on a network is.

use zeroize::Zeroizing;

use crate::derivation::error::DerivationError;
use crate::derivation::monero_words::MoneroWordlist;
use crate::derivation::polyseed::{PolyseedProblem, PolyseedWordlist};
use crate::derivation::setup::WalletSecretFormat;
use crate::registry::Chain;

/// The word counts a phrase format has, shortest first.
pub(crate) fn word_counts(format: WalletSecretFormat) -> &'static [u32] {
    match format {
        WalletSecretFormat::Bip39Phrase => &crate::validation::STANDARD_SEED_PHRASE_WORD_COUNTS,
        WalletSecretFormat::MoneroPhrase => &[25],
        WalletSecretFormat::Polyseed => &[16],
        WalletSecretFormat::TonMnemonic => &[24],
        _ => &[],
    }
}

/// The secret a phrase of `word_count` words in `format` carries, in bits.
pub(crate) fn entropy_bits(format: WalletSecretFormat, word_count: u32) -> u32 {
    match format {
        // 32 bits of entropy per three words.
        WalletSecretFormat::Bip39Phrase => word_count / 3 * 32,
        WalletSecretFormat::MoneroPhrase => 256,
        WalletSecretFormat::Polyseed => 150,
        // 264 bits of words, less the 8 the TON check spends.
        WalletSecretFormat::TonMnemonic => 256,
        _ => 0,
    }
}

/// One wordlist a phrase may be written in.
#[derive(Clone, Copy)]
pub(crate) enum PhraseWordlist {
    Bip39(bip39::Language),
    Monero(&'static MoneroWordlist),
    Polyseed(&'static PolyseedWordlist),
}

impl PhraseWordlist {
    pub fn holds(&self, word: &str) -> bool {
        match self {
            Self::Bip39(language) => language.find_word(word).is_some(),
            Self::Monero(list) => list.index_of(word).is_some(),
            Self::Polyseed(list) => list.index_of(word).is_some(),
        }
    }

    /// The code and English name a front end shows and localizes.
    pub fn code_and_name(&self) -> (&'static str, &'static str) {
        match self {
            Self::Bip39(language) => crate::validation::bip39_code_and_name(*language),
            Self::Monero(list) => (list.code, list.name),
            Self::Polyseed(list) => (list.code, list.name),
        }
    }
}

/// The wordlists phrases of `format` are written in, in the order a tie in
/// detection goes to.
pub(crate) fn wordlists(format: WalletSecretFormat) -> Vec<PhraseWordlist> {
    match format {
        WalletSecretFormat::Bip39Phrase => bip39::Language::ALL
            .iter()
            .map(|&language| PhraseWordlist::Bip39(language))
            .collect(),
        WalletSecretFormat::MoneroPhrase => crate::derivation::monero_words::wordlists()
            .iter()
            .map(PhraseWordlist::Monero)
            .collect(),
        WalletSecretFormat::Polyseed => crate::derivation::polyseed::wordlists()
            .iter()
            .map(PhraseWordlist::Polyseed)
            .collect(),
        WalletSecretFormat::TonMnemonic => vec![PhraseWordlist::Bip39(bip39::Language::English)],
        _ => Vec::new(),
    }
}

/// Why complete words in a list are not a phrase of their format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PhraseProblem {
    InvalidChecksum,
    AmbiguousLanguage,
    EncryptedPolyseed,
    UnsupportedPolyseed,
}

/// Whether `words`, every one held by `list`, are a phrase of `format`. A
/// password-protected TON mnemonic counts: its password is a derivation
/// option, checked when the wallet is derived.
pub(crate) fn reads(
    format: WalletSecretFormat,
    list: PhraseWordlist,
    words: &[&str],
) -> Result<(), PhraseProblem> {
    match (format, list) {
        (WalletSecretFormat::Bip39Phrase, PhraseWordlist::Bip39(language)) => {
            bip39::Mnemonic::parse_in(language, words.join(" "))
                .map(|_| ())
                .map_err(|_| PhraseProblem::InvalidChecksum)
        }
        (WalletSecretFormat::MoneroPhrase, PhraseWordlist::Monero(_)) => {
            match crate::derivation::monero_words::decode(&words.join(" ")) {
                Ok(_) => Ok(()),
                Err(_) => Err(PhraseProblem::InvalidChecksum),
            }
        }
        (WalletSecretFormat::Polyseed, PhraseWordlist::Polyseed(_)) => {
            crate::derivation::polyseed::read_words(words)
                .map(|_| ())
                .map_err(|(_, problem)| match problem {
                    PolyseedProblem::AmbiguousLanguage => PhraseProblem::AmbiguousLanguage,
                    PolyseedProblem::Encrypted => PhraseProblem::EncryptedPolyseed,
                    PolyseedProblem::UnsupportedFeatures => PhraseProblem::UnsupportedPolyseed,
                    PolyseedProblem::UnknownWords | PolyseedProblem::InvalidChecksum => {
                        PhraseProblem::InvalidChecksum
                    }
                })
        }
        (WalletSecretFormat::TonMnemonic, PhraseWordlist::Bip39(_)) => {
            match crate::derivation::ton::ton_mnemonic_kind(&words.join(" ")) {
                Ok(Some(_)) => Ok(()),
                _ => Err(PhraseProblem::InvalidChecksum),
            }
        }
        _ => Err(PhraseProblem::InvalidChecksum),
    }
}

/// The format `phrase` is on `chain`, refusing a phrase none of the chain's
/// formats reads. `password` is the derivation passphrase: TON's mnemonic
/// password, which must open a password-protected mnemonic and must be absent
/// for any other.
pub(crate) fn check_phrase(
    chain: Chain,
    phrase: &str,
    password: Option<&str>,
) -> Result<WalletSecretFormat, DerivationError> {
    let count = phrase.split_whitespace().count() as u32;
    let formats = chain.phrase_formats();
    let Some(&format) = formats
        .iter()
        .find(|format| word_counts(**format).contains(&count))
    else {
        let mut counts: Vec<u32> = formats
            .iter()
            .flat_map(|format| word_counts(*format).iter().copied())
            .collect();
        counts.sort_unstable();
        let counts: Vec<String> = counts.iter().map(u32::to_string).collect();
        return Err(DerivationError::refused(
            "A %@ phrase has %@ words, not %@.",
            [
                chain.chain_display_name().to_string(),
                counts.join(" or "),
                count.to_string(),
            ],
        ));
    };
    match format {
        WalletSecretFormat::Bip39Phrase => {
            crate::validation::parse_seed_phrase(phrase, None)?;
        }
        WalletSecretFormat::MoneroPhrase => {
            crate::derivation::monero_words::decode(phrase)?;
        }
        WalletSecretFormat::Polyseed => {
            crate::derivation::polyseed::decode(phrase)?;
        }
        WalletSecretFormat::TonMnemonic => {
            crate::derivation::ton::check_ton_mnemonic(phrase, password.unwrap_or(""))?;
        }
        _ => return Err(DerivationError::invalid("Not a phrase format.")),
    }
    Ok(format)
}

/// When a Polyseed was created, as a Unix timestamp no later than the actual
/// moment; `None` for any other phrase.
pub(crate) fn polyseed_birthday(phrase: &str) -> Option<u64> {
    (phrase.split_whitespace().count() == 16)
        .then(|| crate::derivation::polyseed::decode(phrase).ok())
        .flatten()
        .map(|seed| seed.birthday_unix())
}

/// A new phrase for a wallet on `chain`, in the format its own wallets
/// restore, of `word_count` words.
pub(crate) fn generate_phrase(
    chain: Chain,
    word_count: u32,
) -> Result<Zeroizing<String>, crate::SpectraBridgeError> {
    let format = chain.created_phrase_format();
    if !word_counts(format).contains(&word_count) {
        let counts: Vec<String> = word_counts(format).iter().map(u32::to_string).collect();
        return Err(crate::SpectraBridgeError::InvalidInput {
            message: crate::LocalizableMessage::new(
                "A new %@ phrase has %@ words, not %@.",
                [
                    chain.chain_display_name().to_string(),
                    counts.join(", "),
                    word_count.to_string(),
                ],
            ),
        });
    }
    match format {
        WalletSecretFormat::MoneroPhrase => Ok(crate::derivation::monero_words::generate()),
        WalletSecretFormat::TonMnemonic => Ok(crate::derivation::ton::generate_ton_mnemonic()?),
        _ => {
            use rand::RngCore;
            let mut entropy =
                Zeroizing::new(vec![0u8; entropy_bits(format, word_count) as usize / 8]);
            rand::thread_rng().fill_bytes(&mut entropy);
            let mnemonic = bip39::Mnemonic::from_entropy_in(bip39::Language::English, &entropy)
                .map_err(crate::SpectraBridgeError::failure)?;
            Ok(Zeroizing::new(mnemonic.to_string()))
        }
    }
}

#[cfg(test)]
#[path = "tests/phrase.rs"]
mod tests;

/// A fixture phrase on `chain` in its own format, for tests that walk every
/// chain: the BIP-39 "abandon … about" phrase, and for Monero the 25-word seed
/// of the same wallet that phrase used to read as (its spend key is
/// `sc_reduce32` of the BIP-39 seed's first half), so recorded Monero fixtures
/// keep their keys. TON's is a ton-crypto mnemonic from `ton-mnemonics.json`.
#[cfg(test)]
pub(crate) fn test_phrase(chain: Chain) -> &'static str {
    match chain.mainnet_counterpart() {
        Chain::Monero => {
            "syndrome portents apex vivid flippant dizzy bumper duplex enjoy deodorant bunch \
             pigment wolf muppet tuition wept ailments kiwi roles against today morsel eternal \
             excess wolf"
        }
        Chain::Ton => {
            "tribe trick matter citizen jealous turtle flee evidence tired milk wisdom eager \
             fancy mother gate worth fly wedding zero ski purchase evidence cycle public"
        }
        _ => {
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
              abandon about"
        }
    }
}
