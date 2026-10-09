//! Import and secret-store adapters.
use super::*;
use crate::derivation::import::{ImportedAddress, WalletImportKind};
use zeroize::Zeroizing;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Reveal a wallet's seed phrase.
    ///
    /// The password is passed as typed: core applies its one rule for
    /// passwords (surrounding whitespace is not part of one, and a blank one
    /// is refused; no password is `None`). The answer says why a phrase was
    /// not revealed, so a front end
    /// words the reason rather than guessing it from an error string.
    ///
    /// This is the reveal path. Derivation does not need it — core derives
    /// from the phrase without it leaving the crate.
    pub fn reveal_seed_phrase(
        &self,
        wallet_id: String,
        password: Option<String>,
    ) -> Result<SeedPhraseReveal, SpectraBridgeError> {
        use crate::store::wallet_secrets::WalletSecretError as E;
        let store = self.secrets()?;
        match crate::store::wallet_secrets::load_seed_phrase(
            &*store,
            &wallet_id,
            password.as_deref(),
        ) {
            Ok(phrase) if !phrase.trim().is_empty() => Ok(SeedPhraseReveal::Phrase {
                phrase: phrase.to_string(),
            }),
            Ok(_) | Err(E::NotSealed) => Ok(SeedPhraseReveal::NotStored),
            Err(E::PasswordRequired) => Ok(SeedPhraseReveal::PasswordRequired),
            Err(E::IncorrectPassword) => Ok(SeedPhraseReveal::IncorrectPassword),
            Err(E::PasswordNotRequired) => Ok(SeedPhraseReveal::PasswordNotRequired),
            Err(error) => Err(error.into()),
        }
    }

    /// What `import_wallets` would store for `commit`, without sealing or
    /// storing anything and without touching the network: the planning the
    /// import runs, so the two cannot disagree. The password is the import's
    /// to judge, not the preview's.
    pub async fn preview_wallet_import(
        &self,
        commit: crate::derivation::import::WalletImportCommit,
    ) -> Result<crate::derivation::import::WalletImportPreview, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let plan = plan_import(commit)?;
            let stored = this.wallet_state.read().await.wallets.clone();
            let placed = place_import(plan, &stored)?;
            Ok(crate::derivation::import::WalletImportPreview {
                addresses: placed
                    .plan
                    .wallets
                    .iter()
                    .map(preview_address)
                    .collect::<Result<_, _>>()?,
                rejected_addresses: placed.plan.rejected_addresses,
                upgrades_wallet: placed.upgrade.map(|wallet| wallet.name),
            })
        })
        .await
    }

    /// Import a wallet on one network: derive or validate its address, build
    /// it, and store it. A watch import of several addresses stores one wallet
    /// per address. A signing import whose address or account key a
    /// watch-only wallet on the network already holds gives that wallet its
    /// keys instead; every other duplicate is refused (`place_import`).
    ///
    /// Core seals secrets, then commits the wallets atomically, and removes
    /// the sealed secrets again if the commit fails.
    pub async fn import_wallets(
        &self,
        commit: crate::derivation::import::WalletImportCommit,
    ) -> Result<crate::derivation::import::WalletImportOutcome, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            // `None` is the choice of no password; a blank `Some` is a request
            // for one that has none. Refused before anything is planned or
            // stored, rather than left to the secret store after the wallets
            // were built.
            if commit
                .password
                .as_deref()
                .is_some_and(|p| p.trim().is_empty())
            {
                return Err(crate::store::wallet_secrets::WalletSecretError::EmptyPassword.into());
            }
            let plan = plan_import(commit)?;
            // A named account is held only once the network confirms it.
            if let Some((account, public_key)) = &plan.named_account_key {
                this.confirm_named_account(plan.commit.request.chain, account, public_key)
                    .await?;
            }
            let is_watch_only = plan.commit.request.kind.is_watch_only();
            this.write_persisted(move |service| async move {
                let database = service.bound_database().await?;
                let source = database.clone();
                let pending = tokio::task::spawn_blocking(move || {
                    crate::wallet_db::pending_secret_deletions(&source)
                })
                .await
                .map_err(SpectraBridgeError::failure)??;
                let mut snapshot = service.wallet_state.read().await.clone();
                // Placed against the state this write commits over, so a
                // duplicate cannot slip in between a check and the commit.
                let Placed { plan, upgrade } = place_import(plan, &snapshot.wallets)?;
                let ImportPlan {
                    mut commit,
                    mut wallets,
                    rejected_addresses,
                    scan_key,
                    ..
                } = plan;
                let seed = commit.seed_phrase.take().map(zeroize::Zeroizing::new);
                let private_key = commit.private_key.take().map(zeroize::Zeroizing::new);
                let password = commit.password.take().map(zeroize::Zeroizing::new);
                if wallets.iter().any(|wallet| pending.contains(&wallet.id)) {
                    return Err(SpectraBridgeError::failure(
                        "Import ID still has pending secret cleanup",
                    ));
                }
                if upgrade.is_none()
                    && wallets
                        .iter()
                        .any(|w| snapshot.wallets.iter().any(|old| old.id == w.id))
                {
                    return Err(SpectraBridgeError::failure("Import ID already exists"));
                }
                if upgrade.is_none() && commit.request.wallet_name.trim().is_empty() {
                    let mut used: std::collections::HashSet<String> = snapshot
                        .wallets
                        .iter()
                        .map(|wallet| wallet.name.clone())
                        .collect();
                    let mut index = 1u64;
                    for wallet in &mut wallets {
                        while used.contains(&format!("Wallet {index}")) {
                            index = index.checked_add(1).ok_or_else(|| {
                                SpectraBridgeError::failure("Wallet names exhausted")
                            })?;
                        }
                        wallet.name = format!("Wallet {index}");
                        used.insert(wallet.name.clone());
                    }
                }
                let previous = snapshot.clone();
                let states = match &upgrade {
                    Some(upgraded) => vec![upgraded.clone()],
                    None => wallets
                        .iter()
                        .map(|wallet| wallet.to_wallet_state())
                        .collect::<Result<Vec<_>, _>>()?,
                };
                for wallet in states {
                    reduce_state_in_place(&mut snapshot, StateCommand::UpsertWallet { wallet });
                }
                let changes =
                    crate::wallet_db::AppStateChanges::between(Some(&previous), &snapshot)?;
                let secrets = if is_watch_only && scan_key.is_none() {
                    None
                } else {
                    Some(service.secrets()?)
                };
                let result: Result<(), SpectraBridgeError> = async {
                    if let (Some(store), Some(view)) = (&secrets, &scan_key) {
                        for wallet in &wallets {
                            store.save_secret(
                                crate::store::secret_store::SecretClass::Generic,
                                format!("{}.scan-key", wallet.id),
                                view.to_string(),
                            )?;
                        }
                    }
                    if let Some(store) = secrets.as_ref().filter(|_| !is_watch_only) {
                        for wallet in &wallets {
                            let result = if commit.request.kind == WalletImportKind::PrivateKey {
                                crate::store::wallet_secrets::store_private_key(
                                    &**store,
                                    &wallet.id,
                                    private_key.as_ref().unwrap().as_str(),
                                    password.as_ref().map(|s| s.as_str()),
                                )
                            } else {
                                crate::store::wallet_secrets::store_seed_phrase(
                                    &**store,
                                    &wallet.id,
                                    seed.as_ref().unwrap().as_str(),
                                    password.as_ref().map(|s| s.as_str()),
                                )
                            };
                            result.map_err(SpectraBridgeError::failure)?;
                        }
                    }
                    tokio::task::spawn_blocking(move || changes.save(&database))
                        .await
                        .map_err(SpectraBridgeError::failure)??;
                    Ok(())
                }
                .await;
                if let Err(error) = result {
                    let mut cleanup_errors = Vec::new();
                    if let Some(store) = &secrets {
                        for wallet in &wallets {
                            if let Err(e) =
                                crate::store::wallet_secrets::delete(&**store, &wallet.id)
                            {
                                cleanup_errors.push(format!("{}: {e}", wallet.id));
                            }
                            // A watched Monero wallet the import would have
                            // upgraded keeps its view key.
                            if let (Some(_), Some(view)) = (&upgrade, &scan_key)
                                && let Err(e) = store.save_secret(
                                    crate::store::secret_store::SecretClass::Generic,
                                    format!("{}.scan-key", wallet.id),
                                    view.to_string(),
                                )
                            {
                                cleanup_errors.push(format!("{}: {e}", wallet.id));
                            }
                        }
                    }
                    return Err(SpectraBridgeError::failure(format!(
                        "{error}; import not committed; secret cleanup failures: {}",
                        cleanup_errors.join(", ")
                    )));
                }
                service.publish_state(snapshot).await;
                Ok(crate::derivation::import::WalletImportOutcome {
                    wallets,
                    rejected_addresses,
                    upgraded: upgrade.is_some(),
                })
            })
            .await
        })
        .await
    }
}

/// An import worked out without storing anything: the commit as core reads
/// it — the phrase canonical, the key in hex, the path resolved — the wallets
/// it builds and the typed addresses it refused. The preview and the import
/// both start here, so the address a page shows is the one the import stores.
pub(super) struct ImportPlan {
    commit: crate::derivation::import::WalletImportCommit,
    wallets: Vec<crate::store::wallet_domain::WalletView>,
    rejected_addresses: Vec<String>,
    /// The named account the wallet holds and the public key (hex) the
    /// network must list among its full-access keys before it is stored.
    named_account_key: Option<(String, String)>,
    /// A Monero wallet's private view key, hex, stored outside the password.
    scan_key: Option<Zeroizing<String>>,
}

pub(super) fn plan_import(
    mut commit: crate::derivation::import::WalletImportCommit,
) -> Result<ImportPlan, SpectraBridgeError> {
    // One validation rule for every address, derived or typed, applied
    // before anything is built so a malformed address cannot reach
    // storage.
    commit.request.check_shape()?;
    let chain = commit.request.chain;
    // Canonicalize before both derivation and storage, regardless of caller.
    commit.seed_phrase = commit.seed_phrase.map(|phrase| {
        phrase
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>()
            .join(" ")
    });
    // The secret the kind names, and no other: a watch import that
    // carried a phrase, or a phrase import that carried a key, would
    // seal or drop material the caller did not mean to hand over.
    let has_seed = commit
        .seed_phrase
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty());
    let has_key = commit
        .private_key
        .as_deref()
        .is_some_and(|k| !k.trim().is_empty());
    match (&commit.request.kind, has_seed, has_key) {
        (WalletImportKind::Phrase, true, false) | (WalletImportKind::PrivateKey, false, true) => {}
        (kind, false, false) if kind.is_watch_only() => {}
        (WalletImportKind::Phrase, false, _) => {
            return Err(SpectraBridgeError::invalid(
                "A phrase import requires a seed phrase.",
            ));
        }
        (WalletImportKind::PrivateKey, _, false) => {
            return Err(SpectraBridgeError::invalid(
                "A private-key import requires a private key.",
            ));
        }
        _ => {
            return Err(SpectraBridgeError::invalid(
                "An import carries only the secret its kind names.",
            ));
        }
    }
    // A key in any of the chain's own encodings, read into the hex the
    // key is sealed and derived as.
    if commit.request.kind == WalletImportKind::PrivateKey {
        let typed = Zeroizing::new(commit.private_key.take().unwrap_or_default());
        commit.private_key =
            Some(crate::derivation::key_formats::parse_private_key(chain, &typed)?.to_string());
    }
    if commit.request.kind != WalletImportKind::Phrase
        && (!commit.derivation_overrides.is_empty()
            || commit
                .derivation_path
                .as_deref()
                .is_some_and(|path| !path.trim().is_empty()))
    {
        return Err(SpectraBridgeError::failure(
            "Derivation overrides require a mnemonic wallet",
        ));
    }
    commit.derivation_overrides.validate_for_chain(chain)?;
    // A named account is a NEAR key holder's, by name; read before deriving
    // so a malformed one is refused before any key work.
    let named_account = match commit
        .named_account
        .take()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
    {
        None => None,
        Some(name) => {
            if !chain.supports_named_sender_accounts()
                || !matches!(
                    commit.request.kind,
                    WalletImportKind::Phrase | WalletImportKind::PrivateKey
                )
            {
                return Err(SpectraBridgeError::invalid(
                    "Only a NEAR key import takes a named account.",
                ));
            }
            let normalized = crate::derivation::import::normalized_import_address(chain, &name)
                .filter(|account| {
                    !(account.len() == 64 && account.bytes().all(|b| b.is_ascii_hexdigit()))
                })
                .ok_or_else(|| {
                    SpectraBridgeError::from(crate::derivation::error::DerivationError::refused(
                        "Not a NEAR named account: %@",
                        [&name],
                    ))
                })?;
            Some(normalized)
        }
    };
    let mut named_account_key = None;
    // A TON key holds one account per wallet contract version; the import
    // names which, W5 unless it says otherwise.
    let ton_wallet = if chain.has_wallet_versions()
        && matches!(
            commit.request.kind,
            WalletImportKind::Phrase | WalletImportKind::PrivateKey
        ) {
        Some(commit.ton_wallet_version.unwrap_or_default())
    } else if commit.ton_wallet_version.is_some() {
        return Err(SpectraBridgeError::invalid(
            "Only a TON key import takes a wallet version.",
        ));
    } else {
        None
    };
    // A phrase in a format the chain's own wallets do not write — BIP-39
    // on Monero or TON — restores nothing those wallets would, so it is
    // refused here rather than read another way.
    if let Some(seed) = commit
        .seed_phrase
        .as_deref()
        .filter(|_| commit.request.kind == WalletImportKind::Phrase)
    {
        crate::derivation::phrase::check_phrase(
            chain,
            seed,
            commit.derivation_overrides.passphrase.as_deref(),
        )?;
    }
    // The wallet's one path, resolved before deriving so the path
    // stored is the one the address came from. Only a phrase walks
    // one.
    commit.derivation_path = if commit.request.kind == WalletImportKind::Phrase {
        crate::derivation::path::import_derivation_path(chain, commit.derivation_path.as_deref())?
    } else {
        None
    };
    // A derived address is judged by the network it was derived for;
    // a typed one by the chain it was typed for. Typed addresses that
    // do not parse are reported, and the rest imported.
    let mut rejected_addresses = Vec::new();
    let imported = match &commit.request.kind {
        WalletImportKind::Phrase | WalletImportKind::PrivateKey => {
            // The secret was matched to its kind above.
            let derived = if let Some(version) = ton_wallet {
                crate::derivation::import::derive_ton_import_address(&commit, version)?
            } else if commit.request.kind == WalletImportKind::PrivateKey {
                crate::derivation::import::derive_private_key_import_address(
                    commit.private_key.as_deref().unwrap_or_default(),
                    chain,
                )?
            } else {
                crate::derivation::import::derive_import_address(
                    commit.seed_phrase.as_deref().unwrap_or_default(),
                    chain,
                    commit.derivation_path.as_deref().unwrap_or_default(),
                    &commit.derivation_overrides,
                )?
            };
            if let Some(account) = &named_account {
                // NEAR's implicit account is the public key in hex.
                named_account_key = Some((account.clone(), derived.to_ascii_lowercase()));
            }
            let derived = named_account.clone().unwrap_or(derived);
            let address = crate::derivation::import::normalized_import_address(chain, &derived)
                .ok_or_else(|| {
                    SpectraBridgeError::from(crate::derivation::error::DerivationError::refused(
                        "Could not derive a %@ address from this secret.",
                        [chain.chain_display_name()],
                    ))
                })?;
            vec![ImportedAddress::Address(address)]
        }
        WalletImportKind::WatchAddresses { addresses } => {
            let (kept, rejected) =
                crate::derivation::import::validated_watch_addresses(chain, addresses);
            rejected_addresses.extend(rejected);
            if kept.is_empty() {
                let message = "Enter at least one valid address to import.";
                return Err(if rejected_addresses.is_empty() {
                    SpectraBridgeError::invalid(message)
                } else {
                    SpectraBridgeError::InvalidInput {
                        message: format!("{message} Rejected: {}", rejected_addresses.join(", "))
                            .into(),
                    }
                });
            }
            kept.into_iter().map(ImportedAddress::Address).collect()
        }
        WalletImportKind::WatchAccountXpub { xpub } => {
            crate::derivation::account_key::parse(chain, xpub)?;
            vec![ImportedAddress::AccountXpub(xpub.trim().to_string())]
        }
        WalletImportKind::WatchViewKey { address, view_key } => {
            let keys = crate::derivation::monero::view_keys(chain, address, view_key)?;
            vec![ImportedAddress::Address(keys.address.to_string())]
        }
        WalletImportKind::WatchMultisig { policy } => {
            let policy = crate::derivation::multisig_policy::AccountPolicy::parse(chain, policy)?;
            vec![ImportedAddress::Multisig {
                address: policy.address(chain)?,
                descriptor: policy.canonical(),
            }]
        }
    };
    // A Monero wallet's private view key is stored beside it, outside the
    // password: its subaddresses are derived from it, and a view-only
    // wallet scans with it alone.
    let scan_key = match &commit.request.kind {
        _ if chain.mainnet_counterpart() != Chain::Monero => None,
        WalletImportKind::WatchViewKey { address, view_key } => {
            Some(crate::derivation::monero::view_keys(chain, address, view_key)?.view)
        }
        WalletImportKind::Phrase => {
            let derived = crate::derivation::dispatch::derive_for_chain(
                chain,
                commit.seed_phrase.as_deref().unwrap_or_default(),
                commit.derivation_path.as_deref().unwrap_or_default(),
                commit.derivation_overrides.passphrase.as_deref(),
                commit.derivation_overrides.hmac_key.as_deref(),
                None,
                false,
                false,
                true,
            )?;
            let private = Zeroizing::new(derived.private_key_hex.unwrap_or_default());
            Some(crate::derivation::monero::ViewKeys::from_private(chain, &private)?.view)
        }
        _ => None,
    }
    .map(|view| Zeroizing::new(hex::encode(*view)));
    // A Monero wallet, and a Zcash wallet's shielded pools, scan from its
    // restore height: the one typed or given for a created wallet, else a
    // Polyseed's birthday, else the chain's default (Monero's start,
    // Zcash's Sapling activation). A Zcash wallet without a phrase has no
    // shielded keys and scans nothing. No other chain scans.
    let scans = crate::restore_heights::takes_restore_height(chain)
        && (chain.mainnet_counterpart() == Chain::Monero || commit.seed_phrase.is_some());
    let restore_height = if scans {
        Some(match commit.restore_height {
            Some(height) => {
                crate::restore_heights::check_restore_height(chain, height)?;
                height
            }
            None => commit
                .seed_phrase
                .as_deref()
                .and_then(crate::derivation::phrase::polyseed_birthday)
                .map_or(
                    crate::restore_heights::default_restore_height(chain),
                    |birthday| crate::restore_heights::height_at_or_before(chain, birthday),
                ),
        })
    } else if commit.restore_height.is_some() {
        return Err(SpectraBridgeError::invalid(
            "Only Monero wallets and Zcash wallets restored from a phrase take a restore height.",
        ));
    } else {
        None
    };
    let mut wallets =
        crate::derivation::import::wallets_for_import(&commit, imported, restore_height);
    // A named account signs with the key its secret derives, whatever other
    // keys the account holds; the import confirms it is a full-access one.
    if let Some((_, key_hex)) = &named_account_key {
        let key: [u8; 32] = hex::decode(key_hex)?
            .try_into()
            .map_err(|_| SpectraBridgeError::failure("Invalid NEAR public key"))?;
        let key = format!("ed25519:{}", bs58::encode(key).into_string());
        for wallet in &mut wallets {
            wallet.near_account_key = Some(key.clone());
        }
    }
    if let Some(seed) = commit
        .seed_phrase
        .as_deref()
        .filter(|_| commit.request.kind == WalletImportKind::Phrase)
        && chain.uses_account_utxo()
    {
        let path = commit
            .derivation_path
            .as_deref()
            .ok_or_else(|| SpectraBridgeError::invalid("UTXO wallet has no derivation path"))?;
        let xpub = super::address_discovery::UtxoDerivation::account_xpub(
            chain,
            seed,
            path,
            &commit.derivation_overrides,
        )?;
        for wallet in &mut wallets {
            wallet.account_xpub = Some(xpub.clone());
        }
    }
    Ok(ImportPlan {
        commit,
        wallets,
        rejected_addresses,
        named_account_key,
        scan_key,
    })
}

/// The address a planned wallet shows: its own, or for a watched account the
/// first receive address of the key.
fn preview_address(
    wallet: &crate::store::wallet_domain::WalletView,
) -> Result<String, SpectraBridgeError> {
    if let Some(address) = wallet.primary_address() {
        return Ok(address.to_string());
    }
    let xpub = wallet
        .account_xpub
        .as_deref()
        .ok_or_else(|| SpectraBridgeError::failure("a planned wallet has no address"))?;
    Ok(crate::derivation::account_key::first_receive_address(
        wallet.chain_id,
        xpub,
    )?)
}

/// A plan placed among the wallets already stored on its network.
struct Placed {
    plan: ImportPlan,
    /// The watch-only wallet a signing import gives its keys to, as it will
    /// be stored; `plan.wallets` is then that one wallet.
    upgrade: Option<crate::store::state::WalletState>,
}

/// Where an import meets the stored wallets. A wallet holds its address,
/// and a watched account holds its key and the key's first receive address;
/// no two wallets on one network hold the same one.
///
/// - A signing import (a phrase or a key) whose address or account key a
///   watch-only wallet holds gives that wallet its keys: the wallet keeps its
///   id, name, settings and balances — so its history and labels, which hang
///   off the id — and is otherwise what the import would have stored.
/// - One a wallet already signs for is refused, naming that wallet.
/// - A watched line already held, or typed twice, joins the refused lines;
///   a watch with no line left, and an account key watched again, is refused
///   naming the wallet that holds it.
fn place_import(
    mut plan: ImportPlan,
    stored: &[crate::store::state::WalletState],
) -> Result<Placed, SpectraBridgeError> {
    use crate::derivation::error::DerivationError;
    let chain = plan.commit.request.chain;
    // What each stored wallet on the network holds, worked out once.
    let held: Vec<_> = stored
        .iter()
        .filter(|wallet| wallet.chain_id == chain)
        .map(|wallet| {
            let mut addresses: Vec<String> =
                wallet.addresses.iter().map(|a| a.address.clone()).collect();
            // A watched account's first address; a phrase wallet's stored
            // key is its own address's account, already listed.
            addresses.extend(
                wallet
                    .xpub
                    .as_deref()
                    .filter(|_| wallet.is_watch_only())
                    .and_then(|xpub| {
                        crate::derivation::account_key::first_receive_address(chain, xpub).ok()
                    }),
            );
            (wallet, addresses, wallet.xpub.clone())
        })
        .collect();
    // A phrase on a network that watches account keys holds the key of the
    // account it derives along, though only the account-UTXO networks store
    // it on the wallet; a watched account key meets it here either way.
    let phrase_account_key = (plan.commit.request.kind == WalletImportKind::Phrase
        && chain.accepts_account_xpub())
    .then(|| {
        super::address_discovery::UtxoDerivation::account_xpub(
            chain,
            plan.commit.seed_phrase.as_deref()?,
            plan.commit.derivation_path.as_deref()?,
            &plan.commit.derivation_overrides,
        )
        .ok()
    })
    .flatten();
    let holder = |wallet: &crate::store::wallet_domain::WalletView| {
        let address = preview_address(wallet).ok();
        let key = wallet
            .account_xpub
            .as_deref()
            .or(phrase_account_key.as_deref());
        held.iter()
            .find(|(_, addresses, stored_key)| {
                address.as_ref().is_some_and(|a| addresses.contains(a))
                    || key.zip(stored_key.as_deref()).is_some_and(|(key, stored)| {
                        crate::derivation::account_key::same_account(key, stored)
                    })
            })
            .map(|(wallet, _, _)| *wallet)
    };
    let refused = |held_by: &crate::store::state::WalletState| -> SpectraBridgeError {
        DerivationError::refused("This is already in the wallet “%@”.", [&held_by.name]).into()
    };
    // An import bound to a watched wallet gives that wallet its keys or is
    // refused; it never becomes a wallet of its own.
    let bound = match plan.commit.upgrade_wallet_id.as_deref() {
        None => None,
        Some(id) => {
            let wallet = stored
                .iter()
                .find(|wallet| wallet.id == id)
                .ok_or_else(|| SpectraBridgeError::failure("Wallet removed"))?;
            if !wallet.is_watch_only() {
                return Err(DerivationError::refused(
                    "The wallet “%@” already has its keys.",
                    [&wallet.name],
                )
                .into());
            }
            Some(wallet)
        }
    };
    // A cosigner's phrase gives a watched multisig account that cosigner's
    // signature: the wallet keeps its policy and addresses, and signs
    // with the key at the cosigner's origin.
    if let Some(bound) = bound
        && let Some(descriptor) = bound.multisig_policy.as_deref()
    {
        let policy = crate::derivation::multisig::MultisigPolicy::parse(chain, descriptor)?;
        let seed = plan
            .commit
            .seed_phrase
            .as_deref()
            .filter(|_| plan.commit.request.kind == WalletImportKind::Phrase)
            .ok_or_else(|| {
                SpectraBridgeError::invalid("A multisig wallet takes a cosigner's phrase.")
            })?;
        if plan.commit.derivation_overrides.hmac_key.is_some() {
            return Err(SpectraBridgeError::invalid(
                "A multisig cosigner derives with BIP-32's own master key.",
            ));
        }
        let (cosigner, _) = policy.cosigner_of_phrase(
            chain,
            seed,
            plan.commit
                .derivation_overrides
                .passphrase
                .as_deref()
                .unwrap_or_default(),
        )?;
        let mut upgraded = bound.clone();
        upgraded.signing = plan.wallets[0].signing;
        upgraded.derivation_path = Some(format!("m/{}", policy.cosigners[cosigner].origin));
        upgraded.derivation_overrides = plan.commit.derivation_overrides.clone();
        upgraded.xpub = None;
        plan.wallets = vec![upgraded.to_wallet_view()];
        return Ok(Placed {
            plan,
            upgrade: Some(upgraded),
        });
    }
    match plan.commit.request.kind {
        WalletImportKind::Phrase | WalletImportKind::PrivateKey => {
            let holds_bound = |existing: Option<&crate::store::state::WalletState>| {
                bound.is_none_or(|bound| existing.is_some_and(|e| e.id == bound.id))
            };
            let existing = holder(&plan.wallets[0]);
            if !holds_bound(existing) {
                let bound = bound.map(|bound| bound.name.clone()).unwrap_or_default();
                return Err(DerivationError::refused(
                    "This secret does not hold the address the wallet “%@” watches.",
                    [&bound],
                )
                .into());
            }
            let Some(existing) = existing else {
                return Ok(Placed {
                    plan,
                    upgrade: None,
                });
            };
            if !existing.is_watch_only() {
                return Err(refused(existing));
            }
            let mut upgraded = plan.wallets[0].to_wallet_state()?;
            upgraded.id = existing.id.clone();
            upgraded.name = existing.name.clone();
            upgraded.include_in_portfolio_total = existing.include_in_portfolio_total;
            upgraded.holdings = existing.holdings.clone();
            upgraded.hidden_holdings = existing.hidden_holdings.clone();
            plan.wallets = vec![upgraded.to_wallet_view()];
            Ok(Placed {
                plan,
                upgrade: Some(upgraded),
            })
        }
        WalletImportKind::WatchAddresses { .. }
        | WalletImportKind::WatchAccountXpub { .. }
        | WalletImportKind::WatchViewKey { .. }
        | WalletImportKind::WatchMultisig { .. }
            if bound.is_some() =>
        {
            Err(SpectraBridgeError::invalid(
                "Only a phrase or a key gives a watched wallet its keys.",
            ))
        }
        WalletImportKind::WatchAddresses { .. } => {
            let mut kept: Vec<crate::store::wallet_domain::WalletView> = Vec::new();
            let mut first_holder = None;
            for wallet in std::mem::take(&mut plan.wallets) {
                let address = wallet.primary_address().unwrap_or_default().to_string();
                let repeated = kept
                    .iter()
                    .any(|k| k.primary_address() == Some(address.as_str()));
                match holder(&wallet) {
                    Some(held_by) => {
                        first_holder.get_or_insert(held_by);
                        plan.rejected_addresses.push(address);
                    }
                    None if repeated => plan.rejected_addresses.push(address),
                    None => kept.push(wallet),
                }
            }
            if kept.is_empty() {
                return Err(match first_holder {
                    Some(held_by) => refused(held_by),
                    None => {
                        SpectraBridgeError::invalid("Enter at least one valid address to import.")
                    }
                });
            }
            // Numbered names follow the wallets that remain.
            let name = plan.commit.request.wallet_name.trim();
            let count = kept.len();
            for (index, wallet) in kept.iter_mut().enumerate() {
                wallet.name = if count > 1 && !name.is_empty() {
                    format!("{name} {}", index + 1)
                } else {
                    name.to_string()
                };
            }
            plan.wallets = kept;
            Ok(Placed {
                plan,
                upgrade: None,
            })
        }
        WalletImportKind::WatchAccountXpub { .. }
        | WalletImportKind::WatchViewKey { .. }
        | WalletImportKind::WatchMultisig { .. } => match holder(&plan.wallets[0]) {
            Some(held_by) => Err(refused(held_by)),
            None => Ok(Placed {
                plan,
                upgrade: None,
            }),
        },
    }
}

impl WalletService {
    /// Confirm on the network that `account` lists `public_key_hex` among
    /// its full-access keys, the key a send from it will sign with.
    async fn confirm_named_account(
        &self,
        chain: Chain,
        account: &str,
        public_key_hex: &str,
    ) -> Result<(), SpectraBridgeError> {
        let client = crate::api::near_json_rpc::NearClient::new(
            self.endpoints_for(chain, &[EndpointCapability::Verification])
                .await,
        );
        client.verify_network(chain).await?;
        let key = hex::decode(public_key_hex)?;
        match client
            .fetch_full_access_key_nonce(account, &bs58::encode(key).into_string())
            .await
        {
            Ok(_) => Ok(()),
            Err(
                crate::api::error::ApiError::Rejected(_)
                | crate::api::error::ApiError::InvalidInput(_),
            ) => Err(crate::derivation::error::DerivationError::refused(
                "“%@” does not hold this key as a full-access key.",
                [account],
            )
            .into()),
            Err(error) => Err(error.into()),
        }
    }
}
