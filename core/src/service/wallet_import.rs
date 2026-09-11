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
    pub fn wallet_secret_state(&self, wallet_id: String) -> WalletSecretState {
        let Ok(store) = self.secrets() else {
            return WalletSecretState {
                has_signing_material: false,
                has_private_key: false,
                is_sealed: false,
            };
        };
        WalletSecretState {
            has_signing_material: crate::store::wallet_secrets::has_signing_material(
                &*store, &wallet_id,
            ),
            has_private_key: crate::store::wallet_secrets::is_private_key_backed(
                &*store, &wallet_id,
            ),
            is_sealed: crate::store::wallet_secrets::is_sealed(&*store, &wallet_id),
        }
    }

    /// Store a seed phrase, sealed under `password` when one is given.
    pub fn store_wallet_seed_phrase(
        &self,
        wallet_id: String,
        seed_phrase: String,
        password: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        let store = self.secrets()?;
        crate::store::wallet_secrets::store_seed_phrase(
            &*store,
            &wallet_id,
            &seed_phrase,
            password.as_deref(),
        )
        .map_err(|e| SpectraBridgeError::from(e.to_string()))
    }

    /// Store a raw private key, sealed under `password` when one is given.
    pub fn store_wallet_private_key(
        &self,
        wallet_id: String,
        private_key: String,
        password: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        let store = self.secrets()?;
        crate::store::wallet_secrets::store_private_key(
            &*store,
            &wallet_id,
            &private_key,
            password.as_deref(),
        )
        .map_err(|e| SpectraBridgeError::from(e.to_string()))
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

    /// Read a wallet's raw private key. Same password rule as
    /// [`Self::wallet_seed_phrase`].
    pub fn wallet_private_key(
        &self,
        wallet_id: String,
        password: Option<String>,
    ) -> Result<String, SpectraBridgeError> {
        let store = self.secrets()?;
        crate::store::wallet_secrets::load_private_key(&*store, &wallet_id, password.as_deref())
            .map(|key| key.to_string())
            .map_err(|e| SpectraBridgeError::from(e.to_string()))
    }

    /// Remove every blob this wallet has. Idempotent.
    pub fn delete_wallet_secrets(&self, wallet_id: String) -> Result<(), SpectraBridgeError> {
        let store = self.secrets()?;
        crate::store::wallet_secrets::delete(&*store, &wallet_id)
            .map_err(|e| SpectraBridgeError::from(e.to_string()))
    }

    /// Import wallets: plan them, build them, and store them.
    ///
    /// Replaces the old `core_plan_wallet_import` round trip, where core
    /// decided what to create and the caller constructed and stored it.
    /// Secrets are not touched here — `secret_instructions` in the outcome
    /// tells the platform what to write to its own keystore.
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
        // Derive here when the caller did not — from a seed phrase or from a
        // private key, whichever this import carries. Both front ends used to
        // derive first and hand the result over; the CLI could only do one
        // chain, so the multi-chain rule — every EVM chain derives from
        // Ethereum's path — existed on the iOS side alone.
        if commit.request.resolved_addresses.by_slot.is_empty()
            && !commit.request.is_watch_only_import
        {
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
                (None, None) => None,
            };
            if let Some(derived) = derived {
                commit.request.resolved_addresses.by_slot = derived
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
                if commit.request.resolved_addresses.by_slot.is_empty() {
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
        // `resolved_addresses` holds what the caller *derived*, and derivation
        // runs against the mainnet chain whatever network mode is selected — a
        // testnet wallet stores a mainnet-format address and re-derives the
        // testnet one for display. Judging it by the selected mode would drop
        // every address on a testnet import.
        //
        // `watch_only_entries` holds what the user *typed*, for the network
        // they are on, and `ImportDraft` has no testnet row to put it in — so
        // a testnet address arrives in the mainnet slot and only the mode says
        // how to read it.
        let typed_networks = crate::derivation::import::ImportNetworks {
            by_family: commit.network_chain_by_family.clone(),
        };
        let (validated, mut rejected_addresses) = crate::derivation::import::validated_addresses(
            &commit.request.resolved_addresses,
            &crate::derivation::import::ImportNetworks::default(),
        );
        commit.request.resolved_addresses = validated;
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
        let plan = match crate::derivation::import::plan_wallet_import(commit.request.clone()) {
            Ok(plan) => plan,
            Err(message) if !rejected_addresses.is_empty() => {
                return Err(SpectraBridgeError::InvalidInput {
                    message: format!("{message} Rejected: {}", rejected_addresses.join(", ")),
                })
            }
            Err(message) => return Err(SpectraBridgeError::from(message)),
        };
        let wallets = crate::derivation::import::wallets_for_import(&commit, &plan);
        let is_watch_only = commit.request.is_watch_only_import;
        for wallet in &wallets {
            self.apply_state_command(StateCommand::UpsertWallet {
                wallet: wallet.to_summary(is_watch_only),
            })
            .await?;
        }
        Ok(crate::derivation::import::WalletImportOutcome {
            secret_kind: plan.secret_kind,
            secret_instructions: plan.secret_instructions,
            wallets,
            rejected_addresses,
        })
    }
}
