//! The ways a wallet can be added on each network, and what each accepts.
//!
//! One answer for every front end: the app's network page lists these
//! methods, `spectra wallet methods` prints them, and `import_wallets` refuses
//! any import whose kind the chosen network does not offer. The per-network
//! facts are `registry::Chain`'s; this module only assembles them.

use serde::Serialize;

use crate::derivation::error::DerivationError;
use crate::derivation::import::WalletImportKind;
use crate::registry::Chain;

/// A way to add a wallet on a network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum WalletSetupMethod {
    /// Generate a new phrase and import it.
    CreatePhrase,
    /// Restore from an existing phrase.
    ImportPhrase,
    /// Add one account from its raw private key.
    ImportPrivateKey,
    /// Track addresses without keys, one wallet per address.
    WatchAddresses,
    /// Track a whole account from its extended public key.
    WatchAccountXpub,
}

impl WalletSetupMethod {
    /// The method an import of `kind` uses. A phrase import is the one
    /// operation behind both creating and restoring; whether the phrase was
    /// just generated is the front end's knowledge, not core's.
    pub fn for_kind(kind: &WalletImportKind) -> Self {
        match kind {
            WalletImportKind::Phrase => Self::ImportPhrase,
            WalletImportKind::PrivateKey => Self::ImportPrivateKey,
            WalletImportKind::WatchAddresses { .. } => Self::WatchAddresses,
            WalletImportKind::WatchAccountXpub { .. } => Self::WatchAccountXpub,
        }
    }
}

/// One encoding a setup method accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum WalletSecretFormat {
    /// A BIP-39 phrase of 12, 15, 18, 21 or 24 words.
    Bip39Phrase,
    /// Monero's 25-word seed, in any of Monero's wordlists.
    MoneroPhrase,
    /// A 16-word Polyseed, in any of its wordlists.
    Polyseed,
    /// TON's own 24-word mnemonic, from the BIP-39 English list but not
    /// BIP-39.
    TonMnemonic,
    /// A 32-byte secret as 64 hex digits, with or without `0x`.
    HexSecret32,
    /// Cardano's 64-byte extended key (kL ‖ kR) as 128 hex digits.
    CardanoExtendedKey,
    /// Wallet Import Format: Base58Check of the network's version byte, the
    /// key and the compressed-key flag.
    Wif,
    /// A 64-byte `secret ‖ public` keypair in base58, or as the Solana CLI's
    /// JSON byte array.
    SolanaKeypair,
    /// A Stellar `S…` secret seed.
    StellarSecretSeed,
    /// A Sui `suiprivkey1…` key.
    SuiPrivateKey,
    /// An AIP-80 `ed25519-priv-0x…` Aptos key.
    AptosPrivateKey,
    /// A NEAR `ed25519:…` key string.
    NearSecretKey,
    /// An address on the network.
    Address,
    /// A BIP-32 account public key: xpub, ypub or zpub, or tpub, upub or vpub
    /// on a test network.
    AccountXpub,
}

/// A value a method asks for beside its secret or addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum WalletSetupField {
    /// Where a Monero wallet's scan starts. Optional: a Polyseed carries its
    /// birthday, and a 25-word seed otherwise scans from the start.
    RestoreHeight,
    /// A named account the key controls (NEAR's `alice.near`), confirmed on
    /// the network at import. Optional: without it the wallet is the key's
    /// implicit account.
    NamedAccount,
    /// The wallet contract a TON key holds its account under. Optional: W5
    /// unless the wallet being restored is an older version.
    TonWalletVersion,
}

/// One method a network offers, with the formats it accepts there, the
/// derivation profiles a phrase can take on it and the values it asks for
/// beside them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletSetupOption {
    pub method: WalletSetupMethod,
    pub formats: Vec<WalletSecretFormat>,
    /// The phrase methods' derivation profiles, default first, each with an
    /// account index; empty for every other method and on a chain that
    /// derives without a path.
    pub profiles: Vec<crate::chains::DerivationProfile>,
    pub fields: Vec<WalletSetupField>,
}

/// Everything a front end needs to offer the ways of adding a wallet on one
/// network, in the order to offer them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletSetupDescriptor {
    pub chain: Chain,
    pub options: Vec<WalletSetupOption>,
}

impl WalletSetupDescriptor {
    pub fn option(&self, method: WalletSetupMethod) -> Option<&WalletSetupOption> {
        self.options.iter().find(|option| option.method == method)
    }
}

/// The ways a wallet can be added on `chain`. Every network offers a phrase,
/// created or restored; the rest are offered where the network supports them.
#[uniffi::export]
pub fn wallet_setup_descriptor(chain: Chain) -> WalletSetupDescriptor {
    // A scanning chain asks where a restored wallet's scan starts; a created
    // one starts near now without asking.
    let restore_fields = if crate::restore_heights::takes_restore_height(chain) {
        vec![WalletSetupField::RestoreHeight]
    } else {
        Vec::new()
    };
    // A key may control a named account as well as its implicit one, and a
    // TON key one account per wallet version.
    let key_fields = if chain.supports_named_sender_accounts() {
        vec![WalletSetupField::NamedAccount]
    } else if chain.has_wallet_versions() {
        vec![WalletSetupField::TonWalletVersion]
    } else {
        Vec::new()
    };
    let mut options = vec![
        WalletSetupOption {
            method: WalletSetupMethod::CreatePhrase,
            formats: vec![chain.created_phrase_format()],
            profiles: chain.derivation_profiles(),
            fields: Vec::new(),
        },
        WalletSetupOption {
            method: WalletSetupMethod::ImportPhrase,
            formats: chain.phrase_formats(),
            profiles: chain.derivation_profiles(),
            fields: [restore_fields, key_fields.clone()].concat(),
        },
    ];
    let key_formats = chain.private_key_formats();
    if !key_formats.is_empty() {
        options.push(WalletSetupOption {
            method: WalletSetupMethod::ImportPrivateKey,
            formats: key_formats,
            profiles: Vec::new(),
            fields: key_fields,
        });
    }
    if chain.supports_watch_only_import() {
        options.push(WalletSetupOption {
            method: WalletSetupMethod::WatchAddresses,
            formats: vec![WalletSecretFormat::Address],
            profiles: Vec::new(),
            fields: Vec::new(),
        });
    }
    if chain.accepts_account_xpub() {
        options.push(WalletSetupOption {
            method: WalletSetupMethod::WatchAccountXpub,
            formats: vec![WalletSecretFormat::AccountXpub],
            profiles: Vec::new(),
            fields: Vec::new(),
        });
    }
    WalletSetupDescriptor { chain, options }
}

/// Refuse an import whose kind `chain` does not offer, naming the network.
pub(crate) fn check_offered(chain: Chain, kind: &WalletImportKind) -> Result<(), DerivationError> {
    let method = WalletSetupMethod::for_kind(kind);
    if wallet_setup_descriptor(chain).option(method).is_some() {
        return Ok(());
    }
    let template = match method {
        WalletSetupMethod::ImportPrivateKey => "%@ cannot derive an address from a private key.",
        WalletSetupMethod::WatchAddresses => "%@ cannot be imported as watch-only.",
        WalletSetupMethod::WatchAccountXpub => "%@ does not take an account xpub.",
        // Every network offers a phrase; the descriptor test holds it.
        WalletSetupMethod::CreatePhrase | WalletSetupMethod::ImportPhrase => return Ok(()),
    };
    Err(DerivationError::refused(
        template,
        [chain.chain_display_name()],
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::derivation::import::{WalletImportCommit, WalletImportRequest};
    use crate::service::WalletService;

    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    fn commit(chain: Chain, kind: WalletImportKind) -> WalletImportCommit {
        WalletImportCommit {
            password: None,
            request: WalletImportRequest {
                wallet_name: String::new(),
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

    pub(crate) async fn service() -> (std::sync::Arc<WalletService>, std::path::PathBuf) {
        let directory = std::env::temp_dir().join(crate::store::new_event_id());
        std::fs::create_dir_all(&directory).unwrap();
        let service = WalletService::new(vec![]).unwrap();
        service.set_secret_store(std::sync::Arc::new(
            crate::store::secret_backends::InMemorySecretStore::new(),
        ));
        service
            .open_state(directory.join("state.db").to_string_lossy().into_owned())
            .await
            .unwrap();
        (service, directory)
    }

    /// A phrase in the chain's own format: BIP-39 for most, a documented
    /// Monero seed, a ton-crypto mnemonic.
    pub(crate) fn phrase(chain: Chain) -> &'static str {
        match chain.mainnet_counterpart() {
            Chain::Monero => {
                "tissue raking haunted huts afraid volcano howls liar egotistic befit rounded \
                 older bluntly imbalance pivot exotic tuxedo amaze mostly lukewarm macro vocal \
                 hounded biplane rounded"
            }
            Chain::Ton => {
                "tribe trick matter citizen jealous turtle flee evidence tired milk wisdom eager \
                 fancy mother gate worth fly wedding zero ski purchase evidence cycle public"
            }
            _ => PHRASE,
        }
    }

    /// The fixture for each method on `chain`, derived from one phrase so
    /// every network has one: the phrase itself, the key and address it
    /// derives, and for an account-xpub network the account's public key.
    pub(crate) fn fixture(chain: Chain, method: WalletSetupMethod) -> WalletImportCommit {
        let path = crate::derivation::path::default_path_from_catalog(chain).unwrap();
        let derived = crate::derivation::dispatch::derive_for_chain(
            chain,
            phrase(chain),
            &path,
            None,
            None,
            None,
            true,
            false,
            true,
        )
        .unwrap();
        match method {
            WalletSetupMethod::CreatePhrase | WalletSetupMethod::ImportPhrase => {
                let mut commit = commit(chain, WalletImportKind::Phrase);
                commit.seed_phrase = Some(if method == WalletSetupMethod::CreatePhrase {
                    let length = crate::validation::seed_phrase_lengths(Some(chain))
                        .into_iter()
                        .find(|length| length.format == chain.created_phrase_format())
                        .unwrap();
                    crate::service::generate_seed_phrase(chain, length.word_count).unwrap()
                } else {
                    phrase(chain).to_string()
                });
                commit
            }
            WalletSetupMethod::ImportPrivateKey => {
                let mut commit = commit(chain, WalletImportKind::PrivateKey);
                commit.private_key =
                    Some(derived.private_key_hex.unwrap_or_else(|| "4c".repeat(32)));
                commit
            }
            WalletSetupMethod::WatchAddresses => commit(
                chain,
                WalletImportKind::WatchAddresses {
                    addresses: vec![derived.address.unwrap()],
                },
            ),
            WalletSetupMethod::WatchAccountXpub => {
                // A Bitcoin-family account path; a chain that takes no xpub
                // is offered a mainnet Bitcoin one, which it must refuse.
                let xpub_chain = if chain.accepts_account_xpub() {
                    chain
                } else {
                    Chain::Bitcoin
                };
                let account =
                    crate::derivation::path::default_path_from_catalog(xpub_chain).unwrap();
                let xpub = crate::service::address_discovery::UtxoDerivation::account_xpub(
                    xpub_chain,
                    PHRASE,
                    &account,
                    &Default::default(),
                )
                .unwrap();
                commit(chain, WalletImportKind::WatchAccountXpub { xpub })
            }
        }
    }

    const METHODS: [WalletSetupMethod; 5] = [
        WalletSetupMethod::CreatePhrase,
        WalletSetupMethod::ImportPhrase,
        WalletSetupMethod::ImportPrivateKey,
        WalletSetupMethod::WatchAddresses,
        WalletSetupMethod::WatchAccountXpub,
    ];

    /// The descriptor is honest: on every network, each method it offers
    /// imports a wallet, and each method it does not offer is refused before
    /// anything is stored.
    #[tokio::test]
    async fn every_offered_method_imports_and_every_other_is_refused() {
        for chain in Chain::all() {
            let descriptor = wallet_setup_descriptor(chain);
            let (service, directory) = service().await;
            for method in METHODS {
                let offered = descriptor.option(method).is_some();
                let before = service.app_state().await.wallets.len();
                let outcome = service.import_wallets(fixture(chain, method)).await;
                let after = service.app_state().await.wallets.len();
                if offered {
                    let outcome = outcome
                        .unwrap_or_else(|error| panic!("{chain} {method:?} refused: {error}"));
                    assert_eq!(outcome.wallets.len(), 1, "{chain} {method:?}");
                    assert_eq!(outcome.wallets[0].chain_id, chain);
                    assert_eq!(after, before + 1, "{chain} {method:?}");
                    // The fixtures share one phrase's address, so each method
                    // starts from an empty store rather than as a duplicate.
                    service
                        .apply_state_command(crate::store::state::StateCommand::RemoveWallet {
                            wallet_id: outcome.wallets[0].id.clone(),
                        })
                        .await
                        .unwrap();
                } else {
                    assert!(outcome.is_err(), "{chain} accepted {method:?}");
                    assert_eq!(after, before, "{chain} stored a refused {method:?}");
                }
            }
            drop(service);
            std::fs::remove_dir_all(directory).unwrap();
        }
    }

    /// Every profile a phrase method offers imports at a second account and
    /// stores that path — including on the account-UTXO chains, whose signer
    /// spends only the profiles it can rediscover — and a path on a chain
    /// that derives without one is refused rather than ignored.
    #[tokio::test]
    async fn every_offered_profile_imports_at_its_path() {
        for chain in Chain::all() {
            let descriptor = wallet_setup_descriptor(chain);
            let option = descriptor.option(WalletSetupMethod::ImportPhrase).unwrap();
            let (service, directory) = service().await;
            for profile in &option.profiles {
                let path = chain.derivation_profile_path(*profile, 1).unwrap();
                let mut commit = commit(chain, WalletImportKind::Phrase);
                commit.seed_phrase = Some(phrase(chain).to_string());
                commit.derivation_path = Some(path.clone());
                let outcome = service
                    .import_wallets(commit)
                    .await
                    .unwrap_or_else(|error| panic!("{chain} {profile:?} refused: {error}"));
                let wallet = &outcome.wallets[0];
                assert_eq!(wallet.derivation_path.as_deref(), Some(path.as_str()));
            }
            if option.profiles.is_empty() {
                let mut commit = commit(chain, WalletImportKind::Phrase);
                commit.seed_phrase = Some(phrase(chain).to_string());
                commit.derivation_path = Some("m/44'/0'/1'".into());
                assert!(service.import_wallets(commit).await.is_err(), "{chain}");
                assert!(service.app_state().await.wallets.is_empty(), "{chain}");
            }
            drop(service);
            std::fs::remove_dir_all(directory).unwrap();
        }
    }

    /// On every network and method, the preview shows the address the import
    /// then stores — for a watched account its key's first receive address —
    /// and previewing stores and seals nothing.
    #[tokio::test]
    async fn the_preview_is_the_address_the_import_stores() {
        for chain in Chain::all() {
            let descriptor = wallet_setup_descriptor(chain);
            let (service, directory) = service().await;
            for option in &descriptor.options {
                let preview = service
                    .preview_wallet_import(fixture(chain, option.method))
                    .await
                    .unwrap_or_else(|error| panic!("{chain} {:?}: {error}", option.method));
                assert!(service.app_state().await.wallets.is_empty(), "{chain}");
                if option.method == WalletSetupMethod::CreatePhrase {
                    // A created phrase is new each time; its preview is
                    // checked against itself below through ImportPhrase.
                    continue;
                }
                let outcome = service
                    .import_wallets(fixture(chain, option.method))
                    .await
                    .unwrap();
                let stored: Vec<String> = outcome
                    .wallets
                    .iter()
                    .map(|wallet| match wallet.primary_address() {
                        Some(address) => address.to_string(),
                        None => crate::derivation::xpub_walker::derive_children(
                            wallet.account_xpub.as_deref().unwrap(),
                            0,
                            0,
                            1,
                        )
                        .unwrap()[0]
                            .address
                            .clone(),
                    })
                    .collect();
                assert_eq!(preview.addresses, stored, "{chain} {:?}", option.method);
                // Start each method from an empty store.
                for wallet in outcome.wallets {
                    service
                        .apply_state_command(crate::store::state::StateCommand::RemoveWallet {
                            wallet_id: wallet.id,
                        })
                        .await
                        .unwrap();
                }
            }
            drop(service);
            std::fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn every_network_offers_a_phrase_and_lists_formats_for_every_method() {
        for chain in Chain::all() {
            let descriptor = wallet_setup_descriptor(chain);
            assert!(descriptor.option(WalletSetupMethod::CreatePhrase).is_some());
            assert!(descriptor.option(WalletSetupMethod::ImportPhrase).is_some());
            for option in &descriptor.options {
                assert!(!option.formats.is_empty(), "{chain} {:?}", option.method);
            }
        }
    }

    #[test]
    fn a_testnet_offers_what_its_mainnet_offers() {
        for chain in Chain::all().filter(|chain| chain.is_testnet()) {
            let methods = |chain| {
                wallet_setup_descriptor(chain)
                    .options
                    .into_iter()
                    .map(|option| option.method)
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                methods(chain),
                methods(chain.mainnet_counterpart()),
                "{chain}"
            );
        }
    }
}
