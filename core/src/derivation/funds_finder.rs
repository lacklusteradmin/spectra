//! Funds Finder — derive the addresses a phrase could have used, to locate
//! hidden or lost funds.
//!
//! The candidates are the registry's derivation profiles at the first
//! accounts: the same profiles a network's setup page offers, so a funded
//! candidate is a wallet that page can import. Candidate generation derives
//! addresses without network requests; the core FundsScan session reads them.

use crate::SpectraBridgeError;
use crate::chains::DerivationProfile;
use crate::registry::Chain;

/// How many accounts of each profile a scan derives: 0, 1 and 2.
pub const SCANNED_ACCOUNTS: u32 = 3;

/// Input to the candidate generation step.
#[derive(Debug, Clone, uniffi::Record)]
pub struct FundsFinderRequest {
    pub seed_phrase: String,
    pub passphrase: Option<String>,
}

/// A single (chain, profile, account, address) derived from the phrase.
/// The balance is checked by the core FundsScan session.
#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct FundsFinderCandidate {
    /// The network the candidate was derived for.
    pub chain_id: Chain,
    /// The profile the path is, or `None` on a chain that derives without a
    /// path.
    pub profile: Option<DerivationProfile>,
    /// The account index on the profile; 0 on a pathless chain.
    pub account: u32,
    /// The wallet contract the address is the account of, on a chain whose
    /// key holds one per version (TON); `None` elsewhere.
    pub ton_wallet_version: Option<crate::derivation::ton::TonWalletVersion>,
    /// The full path derived (e.g. `m/84'/0'/1'/0/0`); empty on a pathless
    /// chain.
    pub derivation_path: String,
    /// The derived address to check.
    pub address: String,
}

/// The candidates across every mainnet that restores a BIP-39 phrase: each
/// profile at the first [`SCANNED_ACCOUNTS`] accounts, in registry order.
/// Pure computation — no network calls.
pub fn generate_funds_finder_candidates(
    request: FundsFinderRequest,
) -> Result<Vec<FundsFinderCandidate>, SpectraBridgeError> {
    Ok(Chain::all()
        .filter(|chain| {
            !chain.is_testnet()
                && chain
                    .phrase_formats()
                    .contains(&crate::derivation::setup::WalletSecretFormat::Bip39Phrase)
        })
        .flat_map(|chain| {
            chain_candidates(
                chain,
                &request.seed_phrase,
                request.passphrase.as_deref(),
                SCANNED_ACCOUNTS,
            )
        })
        .collect())
}

/// `chain`'s candidates: each of its profiles at accounts `0..accounts`, the
/// default profile first; on TON each wallet version's account, the default
/// first; or the one pathless address. A path the deriver refuses is skipped
/// rather than aborting the rest.
pub(crate) fn chain_candidates(
    chain: Chain,
    seed_phrase: &str,
    passphrase: Option<&str>,
    accounts: u32,
) -> Vec<FundsFinderCandidate> {
    let passphrase = passphrase.filter(|value| !value.is_empty());
    if chain.has_wallet_versions() {
        let Ok((_, public)) = crate::derivation::ton::ton_key_pair(seed_phrase, passphrase) else {
            return Vec::new();
        };
        return crate::derivation::ton::TonWalletVersion::ALL
            .into_iter()
            .filter_map(|version| {
                Some(FundsFinderCandidate {
                    chain_id: chain,
                    profile: None,
                    account: 0,
                    ton_wallet_version: Some(version),
                    derivation_path: String::new(),
                    address: version.address(&public, chain).ok()?,
                })
            })
            .collect();
    }
    let profiles = chain.derivation_profiles();
    let paths: Vec<(Option<DerivationProfile>, u32, String)> = if profiles.is_empty() {
        vec![(None, 0, String::new())]
    } else {
        profiles
            .into_iter()
            .flat_map(|profile| {
                (0..accounts).filter_map(move |account| {
                    chain
                        .derivation_profile_path(profile, account)
                        .map(|path| (Some(profile), account, path))
                })
            })
            .collect()
    };
    paths
        .into_iter()
        .filter_map(|(profile, account, path)| {
            let address = crate::derivation::dispatch::derive_for_chain(
                chain,
                seed_phrase,
                &path,
                passphrase,
                None,
                None,
                true,
                false,
                false,
            )
            .ok()?
            .address?;
            Some(FundsFinderCandidate {
                chain_id: chain,
                profile,
                account,
                ton_wallet_version: None,
                derivation_path: path,
                address,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    fn request() -> FundsFinderRequest {
        FundsFinderRequest {
            seed_phrase: PHRASE.into(),
            passphrase: None,
        }
    }

    /// The scan is the registry's profiles, nothing of its own: every
    /// profile of every BIP-39 mainnet at the first accounts, and the one
    /// address of a pathless chain.
    #[test]
    fn candidates_are_the_registrys_profiles_at_the_first_accounts() {
        let candidates = generate_funds_finder_candidates(request()).unwrap();
        for chain in Chain::all() {
            let ours: Vec<_> = candidates.iter().filter(|c| c.chain_id == chain).collect();
            let reads_bip39 = chain
                .phrase_formats()
                .contains(&crate::derivation::setup::WalletSecretFormat::Bip39Phrase);
            if chain.is_testnet() || !reads_bip39 {
                assert!(ours.is_empty(), "{chain}");
                continue;
            }
            let profiles = chain.derivation_profiles();
            let expected = if profiles.is_empty() {
                1
            } else {
                profiles.len() * SCANNED_ACCOUNTS as usize
            };
            assert_eq!(ours.len(), expected, "{chain}");
            for candidate in ours {
                if let Some(profile) = candidate.profile {
                    assert_eq!(
                        crate::derivation::path::derivation_profile_of_path(
                            chain,
                            candidate.derivation_path.clone()
                        ),
                        Some(crate::derivation::path::DerivationProfileChoice {
                            profile,
                            account: candidate.account
                        })
                    );
                }
            }
        }
        // Monero and TON read their own phrases, not this one.
        assert!(
            !candidates
                .iter()
                .any(|c| matches!(c.chain_id, Chain::Monero | Chain::Ton))
        );
    }

    /// Each account is its own wallet, so no two candidates on a chain share
    /// an address.
    #[test]
    fn accounts_and_profiles_derive_distinct_addresses() {
        for chain in Chain::all().filter(|c| c.uses_derivation_path()) {
            let candidates = chain_candidates(chain, PHRASE, None, SCANNED_ACCOUNTS);
            let unique: std::collections::HashSet<_> =
                candidates.iter().map(|c| &c.address).collect();
            assert_eq!(unique.len(), candidates.len(), "{chain}");
        }
    }
}
