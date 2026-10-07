use crate::derivation::error::DerivationError;

use serde::{Deserialize, Serialize};

use crate::registry::Chain;
use crate::validation::address::{AddressValidationRequest, validate_address};

/// How an import supplies its wallet: from a secret the commit carries, or by
/// watching addresses typed for the import's one chain.
///
/// One enum rather than a watch-only flag beside a private-key flag: the two
/// flags admitted an import that was both, which every caller then had to
/// refuse, and watched addresses travelled on every import whatever its kind.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum WalletImportKind {
    /// From the commit's `seed_phrase`, typed or created.
    Phrase,
    /// From the commit's `private_key`.
    PrivateKey,
    /// Addresses on the import's chain, one wallet each, in the order typed.
    WatchAddresses { addresses: Vec<String> },
    /// One account public key, watched as one wallet. Only on a chain that
    /// `accepts_account_xpub`.
    WatchAccountXpub { xpub: String },
}

impl WalletImportKind {
    pub fn is_watch_only(&self) -> bool {
        matches!(
            self,
            Self::WatchAddresses { .. } | Self::WatchAccountXpub { .. }
        )
    }
}

/// What a front end asks an import for: the one network it is on, the
/// wallet's name and how the wallet is supplied.
///
/// An import is on one network. A seed phrase across several chains used to
/// plan one wallet per chain, refused together when any one failed and named
/// "Name 1…N"; each wallet already belonged to one network for good, so the
/// batch bought nothing a second import does not.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletImportRequest {
    pub wallet_name: String,
    pub chain: Chain,
    pub kind: WalletImportKind,
}

/// Everything core needs to turn an import form into stored wallets.
///
/// The draft fields stay in Swift — they are an in-progress form. What crosses
/// is the resolved outcome: which chain, which secret or addresses, and the
/// derivation settings the wallet is created with.
#[derive(Debug, Clone, uniffi::Record)]
pub struct WalletImportCommit {
    pub password: Option<String>,
    pub request: WalletImportRequest,
    /// The path a phrase import derives along: a profile's
    /// (`derivation_profile_path`) or a custom one. `None` takes the chain's
    /// default profile at account 0. Any other kind, and a chain that derives
    /// without a path, refuses one.
    pub derivation_path: Option<String>,
    pub derivation_overrides: crate::store::wallet_domain::WalletDerivationOverrides,
    /// The phrase a `Phrase` import derives from. Any other kind refuses one.
    pub seed_phrase: Option<String>,
    /// The key a `PrivateKey` import derives from. Any other kind refuses one.
    ///
    /// A key that derives no address must not reach a sealed wallet, and that
    /// refusal belongs beside the derivation, not in each caller.
    pub private_key: Option<String>,
    /// Where a Monero wallet's scan starts: a typed height, or for a created
    /// wallet `monero_new_wallet_restore_height`. `None` reads a Polyseed's
    /// birthday, or scans a 25-word seed from the start. Any other chain
    /// refuses one.
    pub restore_height: Option<u64>,
    /// A NEAR named account (`alice.near`) the imported key controls. The
    /// wallet holds it instead of the key's implicit account once the import
    /// has confirmed on the network that the key is one of its full-access
    /// keys. Any other chain or kind refuses one.
    pub named_account: Option<String>,
    /// The wallet contract a TON key import holds its account under. `None`
    /// takes the default (W5); a watch import, and any other chain, refuses
    /// one. Nothing stores it: the address stored is the version's account,
    /// and a send reads the version back from that address and the key.
    pub ton_wallet_version: Option<crate::derivation::ton::TonWalletVersion>,
    /// The watched wallet a signing import is for, by id. The import must
    /// give that wallet its keys: a secret that holds another address is
    /// refused rather than added as a wallet of its own, and so is a watch.
    pub upgrade_wallet_id: Option<String>,
}

impl WalletImportCommit {
    /// What the imported wallets sign with. Only `None` stores the material
    /// unsealed; a blank password is refused before this is asked, which is
    /// the rule `store_seed_phrase` applies.
    pub fn signing(&self) -> crate::store::state::WalletSigning {
        use crate::store::state::WalletSigning;
        let password_protected = self.password.is_some();
        match self.request.kind {
            WalletImportKind::Phrase => WalletSigning::SeedPhrase { password_protected },
            WalletImportKind::PrivateKey => WalletSigning::PrivateKey { password_protected },
            WalletImportKind::WatchAddresses { .. } | WalletImportKind::WatchAccountXpub { .. } => {
                WalletSigning::WatchOnly
            }
        }
    }
}

/// The address a private-key import stores on its chain.
///
/// A chain with no private-key derivation refuses here, before the key is
/// sealed, rather than storing a wallet that could never sign with it.
pub fn derive_private_key_import_address(
    private_key: &str,
    chain: Chain,
) -> Result<String, DerivationError> {
    crate::derivation::dispatch::derive_from_private_key(
        chain,
        private_key.trim().trim_start_matches("0x").to_string(),
        true,
        false,
    )
    .map_err(DerivationError::invalid)?
    .and_then(|result| result.address)
    .ok_or_else(|| {
        DerivationError::refused(
            "%@ cannot derive an address from a private key.",
            [chain.chain_display_name()],
        )
    })
}

/// The account a TON key import stores: the one `version` gives the key the
/// commit's mnemonic (with its password) or raw private key holds.
pub(crate) fn derive_ton_import_address(
    commit: &WalletImportCommit,
    version: crate::derivation::ton::TonWalletVersion,
) -> Result<String, DerivationError> {
    let public = if commit.request.kind == WalletImportKind::PrivateKey {
        let key = zeroize::Zeroizing::new(
            hex::decode(commit.private_key.as_deref().unwrap_or_default())
                .map_err(DerivationError::invalid)?,
        );
        let key: &[u8; 32] = key
            .as_slice()
            .try_into()
            .map_err(|_| DerivationError::invalid("Private key must be exactly 32 bytes"))?;
        ed25519_dalek::SigningKey::from_bytes(key)
            .verifying_key()
            .to_bytes()
    } else {
        crate::derivation::ton::ton_key_pair(
            commit.seed_phrase.as_deref().unwrap_or_default(),
            commit.derivation_overrides.passphrase.as_deref(),
        )?
        .1
    };
    version.address(&public, commit.request.chain)
}

/// The address a phrase import stores on its chain, along `path` (empty on a
/// chain that derives without one).
///
/// A phrase that derives nothing is a refusal, not an import: storing a wallet
/// with an empty address read to the user as "imported".
pub fn derive_import_address(
    seed_phrase: &str,
    chain: Chain,
    path: &str,
    overrides: &crate::store::wallet_domain::WalletDerivationOverrides,
) -> Result<String, DerivationError> {
    crate::derivation::dispatch::derive_for_chain(
        chain,
        seed_phrase,
        path,
        overrides.passphrase.as_deref(),
        overrides.hmac_key.as_deref(),
        None,
        true,
        false,
        false,
    )
    .ok()
    .and_then(|result| result.address)
    .ok_or_else(|| {
        DerivationError::refused(
            "Could not derive a %@ address from this secret.",
            [chain.chain_display_name()],
        )
    })
}

/// The committed wallets. SecretStore writes and rollback belong to core.
#[derive(Debug, Clone, uniffi::Record)]
pub struct WalletImportOutcome {
    pub wallets: Vec<crate::store::wallet_domain::WalletView>,
    /// Addresses the import refused, in the form they were supplied.
    ///
    /// Refusals are reported rather than silent. Dropping them quietly means a
    /// watch-only import of one bad address succeeds and stores a wallet with
    /// no address at all, which reads to the user as "imported" — the same
    /// mistake the address book already fixed with `addressBookRejected`.
    pub rejected_addresses: Vec<String>,
    /// Whether a signing import gave its keys to a watch-only wallet already
    /// stored on the network, keeping its id, name and history, rather than
    /// adding a wallet.
    pub upgraded: bool,
}

/// What an import would store, before anything is sealed or stored: the
/// address each wallet would hold — for a watched account, its key's first
/// receive address — and the typed addresses it would refuse.
#[derive(Debug, Clone, uniffi::Record)]
pub struct WalletImportPreview {
    pub addresses: Vec<String>,
    pub rejected_addresses: Vec<String>,
    /// The name of the watch-only wallet a signing import would give its
    /// keys to, rather than adding a wallet.
    pub upgrades_wallet: Option<String>,
}

/// A Bitcoin account public key carrying a recognized network and script
/// serialization prefix, trimmed, or `None`. Validate the checksum, full BIP32
/// payload and public key before storage; its textual prefix alone does not
/// establish that any addresses can be derived.
pub(crate) fn validated_account_xpub(xpub: &str) -> Option<String> {
    let trimmed = xpub.trim();
    let version = match trimmed.get(..4) {
        Some("xpub") => Some(super::bitcoin::XPUB_VERSION_MAINNET),
        Some("ypub") => Some([0x04, 0x9d, 0x7c, 0xb2]),
        Some("zpub") => Some([0x04, 0xb2, 0x47, 0x46]),
        Some("tpub") => Some(super::bitcoin::XPUB_VERSION_TESTNET),
        Some("upub") => Some([0x04, 0x4a, 0x52, 0x62]),
        Some("vpub") => Some([0x04, 0x5f, 0x1c, 0xf6]),
        _ => None,
    };
    version
        .is_some_and(|expected| {
            super::bitcoin::ExtendedPublicKey::from_xpub_string(trimmed)
                .is_ok_and(|(_, observed)| observed == expected)
        })
        .then(|| trimmed.to_string())
}

/// Validate one address as `chain`'s, returning the normalized form to store,
/// or `None` when it does not parse for that chain.
pub(crate) fn normalized_import_address(chain: Chain, address: &str) -> Option<String> {
    let result = validate_address(AddressValidationRequest {
        kind: chain.address_validation_kind().to_string(),
        value: address.to_string(),
    });
    result.is_valid.then(|| {
        result
            .normalized_value
            .unwrap_or_else(|| address.to_string())
    })
}

/// Whether a watch-only import on `chain` would keep `address`: the rule
/// `validated_watch_addresses` applies, for a form to judge each line by.
#[uniffi::export]
pub fn is_valid_watch_only_address(chain: Chain, address: String) -> bool {
    chain.supports_watch_only_import() && normalized_import_address(chain, address.trim()).is_some()
}

/// Split typed watch addresses into the normalized ones to store and the
/// refused ones, as typed. Blank lines are neither.
///
/// A signing import's address is derived by core and valid by construction; a
/// watch-only import's is typed by the user, and it is the only address the
/// wallet will ever have.
pub(crate) fn validated_watch_addresses(
    chain: Chain,
    addresses: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut kept = Vec::new();
    let mut rejected = Vec::new();
    for address in addresses {
        let trimmed = address.trim();
        if trimmed.is_empty() {
            continue;
        }
        match normalized_import_address(chain, trimmed) {
            Some(normalized) => kept.push(normalized),
            None => rejected.push(trimmed.to_string()),
        }
    }
    (kept, rejected)
}

impl WalletImportRequest {
    /// Refuse a request whose parts do not belong together, before anything
    /// is derived or sealed.
    pub fn check_shape(&self) -> Result<(), DerivationError> {
        let chain = self.chain;
        // The network's setup descriptor is the one answer to which kinds it
        // takes; the app offers from it and the import refuses from it.
        super::setup::check_offered(chain, &self.kind)?;
        match &self.kind {
            WalletImportKind::Phrase
            | WalletImportKind::PrivateKey
            | WalletImportKind::WatchAddresses { .. } => Ok(()),
            WalletImportKind::WatchAccountXpub { xpub } => {
                let (_, _, network) = super::xpub_walker::normalize_xpub(xpub.trim())?;
                let testnet = network == super::xpub_walker::HdNetwork::Testnet;
                if testnet != chain.is_testnet() {
                    return Err(DerivationError::invalid(
                        "Account public key belongs to a different network.",
                    ));
                }
                Ok(())
            }
        }
    }
}

/// One wallet an import stores: its address, or for a watched account its
/// public key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImportedAddress {
    Address(String),
    AccountXpub(String),
}

/// Build the imported wallets without storing them, minting an id for each.
/// Each is on the import's network for good: there is no switching it later.
/// A name is the one asked for, numbered when a watch import made several; a
/// blank one is filled in from the stored wallets when the import commits.
/// An ICP signing import's principal, from its key: the identity the
/// account identifier it stores derives from, which ICRC tokens and the NNS
/// address. `None` for every other import.
fn icp_principal_for(commit: &WalletImportCommit) -> Option<String> {
    let chain = commit.request.chain;
    if chain.mainnet_counterpart() != Chain::Icp {
        return None;
    }
    let public_hex = match commit.request.kind {
        WalletImportKind::PrivateKey => {
            crate::derivation::dispatch::derive_from_private_key(
                chain,
                commit.private_key.clone()?,
                false,
                true,
            )
            .ok()??
            .public_key_hex
        }
        WalletImportKind::Phrase => {
            crate::derivation::dispatch::derive_for_chain(
                chain,
                commit.seed_phrase.as_deref()?,
                commit.derivation_path.as_deref().unwrap_or_default(),
                commit.derivation_overrides.passphrase.as_deref(),
                commit.derivation_overrides.hmac_key.as_deref(),
                None,
                false,
                true,
                false,
            )
            .ok()?
            .public_key_hex
        }
        _ => None,
    }?;
    let public: [u8; 32] = hex::decode(public_hex).ok()?.try_into().ok()?;
    Some(candid::Principal::from_slice(&crate::derivation::icp::principal(&public)).to_text())
}

pub(crate) fn wallets_for_import(
    commit: &WalletImportCommit,
    addresses: Vec<ImportedAddress>,
    restore_height: Option<u64>,
) -> Vec<crate::store::wallet_domain::WalletView> {
    let icp_principal = icp_principal_for(commit);
    let chain = commit.request.chain;
    let name = commit.request.wallet_name.trim();
    let count = addresses.len();
    addresses
        .into_iter()
        .enumerate()
        .map(|(index, imported)| {
            let (address, account_xpub) = match imported {
                ImportedAddress::Address(address) => (Some(address), None),
                ImportedAddress::AccountXpub(xpub) => (None, Some(xpub)),
            };
            crate::store::wallet_domain::WalletView {
                id: crate::store::new_transaction_id(),
                name: if count > 1 && !name.is_empty() {
                    format!("{name} {}", index + 1)
                } else {
                    name.to_string()
                },
                chain_id: chain,
                addresses: address
                    .map(|address| (chain.address_slot().to_string(), address))
                    .into_iter()
                    .collect(),
                account_xpub,
                derivation_path: commit.derivation_path.clone(),
                derivation_overrides: commit.derivation_overrides.clone(),
                holdings: vec![chain.native_holding_template()],
                include_in_portfolio_total: true,
                signing: commit.signing(),
                restore_height,
                hidden_holdings: Vec::new(),
                icp_principal: icp_principal.clone(),
                // A named NEAR account's key is set where the import
                // confirmed it.
                near_account_key: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The principal and account @dfinity/identity give the key of 32 0x01
    /// bytes (`icp-staking-vectors.json`); a phrase records its own; a watch
    /// records none.
    #[test]
    fn an_icp_key_import_records_the_principal_its_account_derives_from() {
        let fixtures: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/icp-staking-vectors.json"
        ))
        .unwrap();
        let commit = |kind| WalletImportCommit {
            password: None,
            request: WalletImportRequest {
                wallet_name: String::new(),
                chain: Chain::Icp,
                kind,
            },
            derivation_path: None,
            derivation_overrides: Default::default(),
            seed_phrase: None,
            private_key: Some("01".repeat(32)),
            restore_height: None,
            named_account: None,
            ton_wallet_version: None,
            upgrade_wallet_id: None,
        };
        let key = commit(WalletImportKind::PrivateKey);
        assert_eq!(
            icp_principal_for(&key).as_deref(),
            fixtures["controller"].as_str()
        );
        assert_eq!(
            derive_private_key_import_address(&"01".repeat(32), Chain::Icp).unwrap(),
            fixtures["owner"].as_str().unwrap()
        );
        let mut phrase = commit(WalletImportKind::Phrase);
        phrase.private_key = None;
        phrase.seed_phrase = Some(crate::derivation::phrase::test_phrase(Chain::Icp).into());
        phrase.derivation_path =
            crate::derivation::path::import_derivation_path(Chain::Icp, None).unwrap();
        let principal = icp_principal_for(&phrase).unwrap();
        let account = derive_import_address(
            phrase.seed_phrase.as_deref().unwrap(),
            Chain::Icp,
            phrase.derivation_path.as_deref().unwrap(),
            &Default::default(),
        )
        .unwrap();
        let principal_bytes = candid::Principal::from_text(&principal).unwrap();
        assert_eq!(
            hex::encode(crate::derivation::icp::account_from_principal(
                principal_bytes.as_slice()
            )),
            account
        );
        let mut watch = commit(WalletImportKind::WatchAddresses {
            addresses: vec![account],
        });
        watch.private_key = None;
        assert_eq!(icp_principal_for(&watch), None);
    }

    fn request(chain: Chain, kind: WalletImportKind) -> WalletImportRequest {
        WalletImportRequest {
            wallet_name: "W".to_string(),
            chain,
            kind,
        }
    }

    fn commit(chain: Chain, name: &str, kind: WalletImportKind) -> WalletImportCommit {
        WalletImportCommit {
            password: None,
            request: WalletImportRequest {
                wallet_name: name.to_string(),
                chain,
                kind,
            },
            derivation_path: None,
            derivation_overrides: Default::default(),
            seed_phrase: None,
            private_key: None,
            restore_height: None,
            named_account: None,
            ton_wallet_version: None,
            upgrade_wallet_id: None,
        }
    }

    /// A testnet is never offered on its own — the importer picks a mainnet and
    /// the network selector decides which network it lands on.
    #[test]
    fn the_private_key_picker_offers_mainnets_only() {
        for chain in Chain::all().filter(|c| c.derives_from_private_key()) {
            assert!(
                !chain.is_testnet() || chain.mainnet_counterpart() != chain,
                "{} is a testnet and would be offered",
                chain.str_id()
            );
        }
    }

    /// Every EVM chain stores its one address in the shared EVM slot.
    #[test]
    fn evm_wallets_store_their_address_in_the_shared_slot() {
        for chain in [
            Chain::Ethereum,
            Chain::Arbitrum,
            Chain::Base,
            Chain::Polygon,
            Chain::EthereumClassic,
            Chain::XLayer,
        ] {
            let wallets = wallets_for_import(
                &commit(chain, "W", WalletImportKind::Phrase),
                vec![ImportedAddress::Address("0xabc".into())],
                None,
            );
            assert_eq!(wallets.len(), 1);
            assert_eq!(wallets[0].chain_id, chain);
            assert_eq!(
                wallets[0].addresses.get("ethereum").map(String::as_str),
                Some("0xabc"),
                "{chain}"
            );
            assert_eq!(wallets[0].addresses.len(), 1);
        }
    }

    /// One wallet per watched address, numbered after the name asked for; a
    /// single wallet keeps the name as typed.
    #[test]
    fn a_watch_import_makes_one_numbered_wallet_per_address() {
        let kind = WalletImportKind::WatchAddresses {
            addresses: vec!["a".into(), "b".into()],
        };
        let wallets = wallets_for_import(
            &commit(Chain::Solana, "Watch", kind.clone()),
            vec![
                ImportedAddress::Address("addr1".into()),
                ImportedAddress::Address("addr2".into()),
            ],
            None,
        );
        assert_eq!(
            wallets.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(),
            ["Watch 1", "Watch 2"]
        );
        assert_ne!(wallets[0].id, wallets[1].id);
        assert_eq!(
            wallets[1].addresses.get("solana").map(String::as_str),
            Some("addr2")
        );
        assert!(
            wallets
                .iter()
                .all(|w| w.signing == crate::store::state::WalletSigning::WatchOnly)
        );
        let single = wallets_for_import(
            &commit(Chain::Solana, "Watch", kind),
            vec![ImportedAddress::Address("addr1".into())],
            None,
        );
        assert_eq!(single[0].name, "Watch");
    }

    /// Core mints each id, parseable as a UUID like every other id that
    /// crosses the boundary.
    #[test]
    fn imported_wallet_ids_are_minted_here() {
        let wallets = wallets_for_import(
            &commit(Chain::Bitcoin, "", WalletImportKind::Phrase),
            vec![ImportedAddress::Address("bc1q".into())],
            None,
        );
        let id = &wallets[0].id;
        assert_eq!(id.len(), 36, "{id}");
        assert_eq!(id.chars().filter(|c| *c == '-').count(), 4, "{id}");
    }

    #[test]
    fn a_watched_account_xpub_is_one_wallet_with_no_address() {
        let wallets = wallets_for_import(
            &commit(
                Chain::Bitcoin,
                "",
                WalletImportKind::WatchAccountXpub {
                    xpub: "xpub123".into(),
                },
            ),
            vec![ImportedAddress::AccountXpub("xpub123".into())],
            None,
        );
        assert_eq!(wallets.len(), 1);
        assert_eq!(wallets[0].account_xpub.as_deref(), Some("xpub123"));
        assert!(wallets[0].addresses.is_empty());
    }

    #[test]
    fn watch_only_refuses_chains_that_need_more_than_an_address() {
        let refused = request(
            Chain::Monero,
            WalletImportKind::WatchAddresses {
                addresses: vec!["4addr".into()],
            },
        )
        .check_shape()
        .unwrap_err();
        // Monero watch-only needs a view key, so an address alone is refused.
        assert!(
            refused
                .to_string()
                .contains("cannot be imported as watch-only")
        );
    }

    #[test]
    fn an_account_xpub_needs_a_chain_that_takes_one() {
        let xpub = WalletImportKind::WatchAccountXpub {
            xpub: "xpub123".to_string(),
        };
        assert!(
            request(Chain::Litecoin, xpub)
                .check_shape()
                .unwrap_err()
                .to_string()
                .contains("account xpub")
        );
    }

    #[test]
    fn testnet_chains_do_not_borrow_their_mainnet_slot() {
        // A Bitcoin testnet address is not a Bitcoin address, so a testnet
        // wallet must not store into the mainnet slot.
        assert_ne!(
            Chain::BitcoinTestnet.address_slot(),
            Chain::Bitcoin.address_slot()
        );
    }
}
