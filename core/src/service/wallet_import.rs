//! Import and secret-store adapters.
use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// What material this wallet has, and whether it is behind a password.
    ///
    /// Three Swift methods asked this three ways —
    /// `walletRequiresSeedPhrasePassword`, `walletHasSigningMaterial` and
    /// `isPrivateKeyBackedWallet` — each one reaching into the Keychain under
    /// a key it built itself.
    ///
    /// A store that cannot be read is an error, not three `false`s. Answering
    /// "not sealed" for a sealed wallet whose verifier read failed is what let
    /// the reveal path read the envelope as if it were the phrase.
    pub fn wallet_secret_state(
        &self,
        wallet_id: String,
    ) -> Result<WalletSecretState, SpectraBridgeError> {
        use crate::store::wallet_secrets::{
            has_signing_material, is_private_key_backed, is_sealed,
        };
        let store = self.secrets()?;
        let read = |e: crate::store::wallet_secrets::WalletSecretError| {
            SpectraBridgeError::from(e.to_string())
        };
        Ok(WalletSecretState {
            has_signing_material: has_signing_material(&*store, &wallet_id).map_err(read)?,
            has_private_key: is_private_key_backed(&*store, &wallet_id).map_err(read)?,
            is_sealed: is_sealed(&*store, &wallet_id).map_err(read)?,
        })
    }

    /// Read a wallet's seed phrase. `password` is required exactly when
    /// `wallet_secret_state().is_sealed`.
    ///
    /// This is the reveal path. Derivation does not need it — core derives
    /// from the phrase without it leaving the crate.
    pub fn wallet_seed_phrase(
        &self,
        wallet_id: String,
        password: Option<String>,
    ) -> Result<String, SpectraBridgeError> {
        let store = self.secrets()?;
        crate::store::wallet_secrets::load_seed_phrase(&*store, &wallet_id, password.as_deref())
            .map(|phrase| phrase.to_string())
            .map_err(|e| SpectraBridgeError::from(e.to_string()))
    }

    /// Import wallets: plan them, build them, and store them.
    ///
    /// Core stores secrets before atomically committing all wallets.
    pub async fn import_wallets(
        &self,
        commit: crate::derivation::import::WalletImportCommit,
    ) -> Result<crate::derivation::import::WalletImportOutcome, SpectraBridgeError> {
        // One validation rule for every chain, applied before planning so a
        // malformed address cannot reach storage. Both inputs carry addresses:
        // `resolved_addresses` for a signing import, `watch_only_entries` for a
        // watch-only one. Validating only the first covered the path whose
        // address core derived itself and skipped the path where the user
        // typed it.
        let mut commit = commit;
        // Canonicalize before both derivation and storage, regardless of caller.
        commit.seed_phrase = commit.seed_phrase.map(|phrase| {
            phrase
                .split_whitespace()
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
                .join(" ")
        });
        if commit.request.is_private_key_import {
            commit.private_key = Some(
                super::standalone::private_key_hex(commit.private_key.take().unwrap_or_default())
                    .ok_or("Enter a valid 32-byte hex key.")?,
            );
        }
        if (commit.request.is_watch_only_import || commit.request.is_private_key_import)
            && !commit.derivation_overrides.is_empty()
        {
            return Err("Derivation overrides require a mnemonic wallet".into());
        }
        for name in &commit.request.selected_chain_names {
            let chain =
                crate::registry::Chain::from_display_name(name).ok_or("Unknown import chain")?;
            commit.derivation_overrides.validate_for_chain(chain)?;
        }
        // Complete explicit overrides with network-local defaults before
        // deriving, so those same paths are persisted with the addresses.
        let mut paths = crate::derivation_paths_for_preset(commit.seed_derivation_preset)?;
        paths.is_custom_enabled = commit.seed_derivation_paths.is_custom_enabled;
        paths
            .by_chain
            .extend(std::mem::take(&mut commit.seed_derivation_paths.by_chain));
        commit.seed_derivation_paths = paths;
        let mut resolved_addresses = crate::derivation::import::WalletImportAddresses::default();
        // Derive here when the caller did not — from a seed phrase or from a
        // private key, whichever this import carries. Both front ends used to
        // derive first and hand the result over; the CLI could only do one
        // chain, so the multi-chain rule — every EVM chain derives from
        // Ethereum's derivation path — existed on the iOS side alone.
        if !commit.request.is_watch_only_import {
            let key = commit
                .private_key
                .clone()
                .filter(|k| !k.trim().is_empty())
                .filter(|_| commit.request.is_private_key_import);
            let seed = commit
                .seed_phrase
                .clone()
                .filter(|s| !s.trim().is_empty())
                .filter(|_| !commit.request.is_private_key_import);
            let derived = match (&key, &seed) {
                (Some(key), _) => Some(
                    crate::derivation::import::derive_private_key_import_address(
                        key,
                        &commit.request.selected_chain_names,
                    )
                    .map_err(|message| SpectraBridgeError::InvalidInput { message })?,
                ),
                (None, Some(seed)) => Some(crate::derivation::import::derive_import_addresses(
                    seed,
                    &commit.request.selected_chain_names,
                    &commit.seed_derivation_paths,
                    &commit.derivation_overrides,
                )),
                (None, None) => {
                    return Err("Signing import requires a seed phrase or private key".into())
                }
            };
            if let Some(derived) = derived {
                resolved_addresses.by_slot = derived
                    .into_iter()
                    .filter_map(|(chain_name, address)| {
                        crate::registry::Chain::from_display_name(&chain_name)
                            .map(|chain| (chain.address_slot().to_string(), address))
                    })
                    .collect();
                // Deriving nothing is a refusal, not an import. A secret the
                // deriver cannot read — the wrong wordlist, an override that
                // does not apply — produced a stored wallet with an empty
                // address that read to the user as "imported", which is the
                // mistake watch-only imports already refuse to make.
                if resolved_addresses.by_slot.is_empty() {
                    return Err(SpectraBridgeError::InvalidInput {
                        message: "Could not derive an address from this secret for any \
                                  selected chain."
                            .to_string(),
                    });
                }
            }
        }
        // The two inputs carry addresses of different provenance, so they are
        // judged against different networks.
        //
        // `resolved_addresses` contains core-derived addresses for each
        // concrete network, stored under that network's validated slot.
        // Validate each slot as its own network, independently of the current
        // UI selection.
        //
        // `watch_only_entries` holds what the user *typed*, for the network
        // they are on, and `ImportDraft` has no testnet row to put it in — so
        // a testnet address arrives in the mainnet slot and only the mode says
        // how to read it.
        //
        // The selection is core's setting, read here. The app sent its copy of
        // it on the commit.
        let typed_networks = crate::derivation::import::ImportNetworks {
            by_family: self.app_state().await.settings.selected_chain_by_family,
        };
        let (validated, mut rejected_addresses) = crate::derivation::import::validated_addresses(
            &resolved_addresses,
            &crate::derivation::import::ImportNetworks::default(),
        );
        let (validated_watch_only, rejected_watch_only) =
            crate::derivation::import::validated_watch_only_entries(
                &commit.request.watch_only_entries,
                &typed_networks,
            );
        commit.request.watch_only_entries = validated_watch_only;
        rejected_addresses.extend(rejected_watch_only);

        // A plan that fails *because* validation emptied the input is a refusal
        // of what the caller supplied, not an internal failure — say which
        // address was refused, and classify it so a caller can tell the two
        // apart without reading the message.
        let plan_request = crate::derivation::import::WalletImportPlanRequest::new(
            commit.request.clone(),
            validated,
            commit.password.is_some(),
        );
        let plan = match crate::derivation::import::plan_wallet_import(plan_request) {
            Ok(plan) => plan,
            Err(message) if !rejected_addresses.is_empty() => {
                return Err(SpectraBridgeError::InvalidInput {
                    message: format!("{message} Rejected: {}", rejected_addresses.join(", ")),
                })
            }
            Err(message) => return Err(SpectraBridgeError::from(message)),
        };
        let mut wallets =
            crate::derivation::import::wallets_for_import(&commit, &plan, &typed_networks);
        let is_watch_only = commit.request.is_watch_only_import;
        let seed = commit.seed_phrase.take().map(zeroize::Zeroizing::new);
        let private_key = commit.private_key.take().map(zeroize::Zeroizing::new);
        let password = commit.password.take().map(zeroize::Zeroizing::new);
        self.write_persisted(move |service| async move {
            let database = service.bound_database().await?;
            let mut snapshot = service.wallet_state.read().await.clone();
            if wallets
                .iter()
                .any(|w| snapshot.wallets.iter().any(|old| old.id == w.id))
            {
                return Err("Import ID already exists".into());
            }
            if commit.request.wallet_name.trim().is_empty() {
                let mut used: std::collections::HashSet<String> = snapshot
                    .wallets
                    .iter()
                    .map(|wallet| wallet.name.clone())
                    .collect();
                let mut index = 1u64;
                for wallet in &mut wallets {
                    while used.contains(&format!("Wallet {index}")) {
                        index = index
                            .checked_add(1)
                            .ok_or_else(|| SpectraBridgeError::from("Wallet names exhausted"))?;
                    }
                    wallet.name = format!("Wallet {index}");
                    used.insert(wallet.name.clone());
                }
            }
            let previous = snapshot.clone();
            for wallet in &wallets {
                reduce_state_in_place(
                    &mut snapshot,
                    StateCommand::UpsertWallet {
                        wallet: wallet.to_wallet_state(is_watch_only)?,
                    },
                );
            }
            let changes = crate::wallet_db::AppStateChanges::between(Some(&previous), &snapshot)?;
            let secrets = if is_watch_only {
                None
            } else {
                Some(service.secrets()?)
            };
            let result: Result<(), SpectraBridgeError> = async {
                if let Some(store) = &secrets {
                    for wallet in &wallets {
                        let result = if commit.request.is_private_key_import {
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
                        result.map_err(|e| SpectraBridgeError::from(e.to_string()))?;
                    }
                }
                tokio::task::spawn_blocking(move || changes.save(&database))
                    .await
                    .map_err(|e| SpectraBridgeError::from(e.to_string()))??;
                Ok(())
            }
            .await;
            if let Err(error) = result {
                let mut cleanup_errors = Vec::new();
                if let Some(store) = &secrets {
                    for wallet in &wallets {
                        if let Err(e) = crate::store::wallet_secrets::delete(&**store, &wallet.id) {
                            cleanup_errors.push(format!("{}: {e}", wallet.id));
                        }
                    }
                }
                return Err(format!(
                    "{error}; import not committed; secret cleanup failures: {}",
                    cleanup_errors.join(", ")
                )
                .into());
            }
            service.publish_state(snapshot).await;
            Ok(crate::derivation::import::WalletImportOutcome {
                secret_kind: plan.secret_kind,
                wallets,
                rejected_addresses,
            })
        })
        .await
    }
}
