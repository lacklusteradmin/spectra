//! A multisig wallet's PSBT sessions: a spend from the account, built here
//! or read from another coordinator, kept until it is broadcast or
//! discarded. Each holds the PSBT as its signatures have gathered; what a
//! session shows is reviewed again from it against the wallet's policy, so
//! nothing stored stands in for that review.
use super::*;
use crate::derivation::multisig::{MultisigPolicy, Place};
use crate::send::psbt;
use crate::send::stages::UtxoSendSource;
use crate::store::state::{WalletSigning, WalletState};

#[derive(Debug, Clone, Serialize, uniffi::Record)]
pub struct PsbtInputView {
    /// `txid:vout`.
    pub outpoint: String,
    pub address: String,
    pub value_sat: u64,
    /// How many cosigners' valid signatures the input carries.
    pub signatures: u32,
}

#[derive(Debug, Clone, Serialize, uniffi::Record)]
pub struct PsbtOutputView {
    pub address: String,
    pub value_sat: u64,
    /// Change back to the wallet, at the place its key origins name.
    pub is_change: bool,
}

/// A PSBT session as a cosigner reviews it.
#[derive(Debug, Clone, Serialize, uniffi::Record)]
pub struct PsbtSession {
    pub id: String,
    pub wallet_id: String,
    pub txid: String,
    /// What a signature is given for: the unsigned transaction and the
    /// amounts it spends. Signing names it, and a PSBT that no longer has
    /// it is refused.
    pub review_digest: String,
    pub threshold: u32,
    pub inputs: Vec<PsbtInputView>,
    pub outputs: Vec<PsbtOutputView>,
    pub fee_sat: u64,
    /// The fingerprints of the cosigners who signed every input.
    pub signed_by: Vec<String>,
    /// Every input carries the threshold's signatures.
    pub complete: bool,
    /// The PSBT, base64, to hand to the next cosigner.
    pub psbt: String,
    pub broadcast_txid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredPsbt {
    id: String,
    wallet_id: String,
    chain: Chain,
    psbt: String,
    created_at: f64,
    broadcast_txid: Option<String>,
}

/// The account's address at `place` as a source of coins.
fn multisig_source(
    chain: Chain,
    policy: &MultisigPolicy,
    place: Place,
) -> Result<UtxoSendSource, SpectraBridgeError> {
    Ok(UtxoSendSource {
        address: policy.address(chain, place)?,
        derivation_path: None,
        script_pubkey: bitcoin::ScriptBuf::new_p2wsh(&policy.witness_script(place)?.wscript_hash())
            .into_bytes(),
    })
}

impl WalletService {
    /// The wallet's policy and network; refused for a single-key wallet.
    async fn multisig_policy(
        &self,
        wallet_id: &str,
    ) -> Result<(WalletState, MultisigPolicy), SpectraBridgeError> {
        let wallet = self.stored_wallet(wallet_id).await?;
        let policy = MultisigPolicy::parse(
            wallet.chain_id,
            wallet.multisig_descriptor.as_deref().ok_or_else(|| {
                SpectraBridgeError::invalid("Only a multisig wallet keeps PSBTs.")
            })?,
        )?;
        Ok((wallet, policy))
    }

    async fn psbt_load(&self, id: &str) -> Result<StoredPsbt, SpectraBridgeError> {
        let db = self.bound_database().await?;
        let id = id.to_string();
        let payload =
            tokio::task::spawn_blocking(move || crate::wallet_db::psbt_session_load(&db, &id))
                .await
                .map_err(SpectraBridgeError::failure)??
                .ok_or_else(|| SpectraBridgeError::invalid("No such PSBT session"))?;
        Ok(serde_json::from_str(&payload)?)
    }

    async fn psbt_save(&self, stored: &StoredPsbt) -> Result<(), SpectraBridgeError> {
        let db = self.bound_database().await?;
        let payload = serde_json::to_string(stored)?;
        let (id, wallet_id) = (stored.id.clone(), stored.wallet_id.clone());
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::psbt_session_save(&db, &id, &wallet_id, &payload)
        })
        .await
        .map_err(SpectraBridgeError::failure)??;
        Ok(())
    }

    async fn psbt_stored_for_wallet(
        &self,
        wallet_id: &str,
    ) -> Result<Vec<StoredPsbt>, SpectraBridgeError> {
        let db = self.bound_database().await?;
        let wallet_id = wallet_id.to_string();
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::psbt_sessions_for_wallet(&db, &wallet_id)
        })
        .await
        .map_err(SpectraBridgeError::failure)??
        .iter()
        .map(|payload| serde_json::from_str(payload).map_err(SpectraBridgeError::from))
        .collect()
    }

    /// The account's addresses that may hold coins, each at its place:
    /// the first receive address and every one discovery found.
    async fn multisig_sources(
        &self,
        wallet: &WalletState,
        policy: &MultisigPolicy,
    ) -> Result<Vec<(UtxoSendSource, Place)>, SpectraBridgeError> {
        let chain = wallet.chain_id;
        let mut places = vec![(0, 0)];
        places.extend(
            self.keypool
                .read()
                .await
                .owned_on(chain)
                .iter()
                .filter(|row| row.wallet_id == wallet.id)
                .filter_map(|row| {
                    let branch = match row.branch.as_deref()? {
                        "external" => 0,
                        "change" => 1,
                        _ => return None,
                    };
                    Some((branch, u32::try_from(row.branch_index?).ok()?))
                }),
        );
        places.sort_unstable();
        places.dedup();
        places
            .into_iter()
            .map(|place| Ok((multisig_source(chain, policy, place)?, place)))
            .collect()
    }

    fn psbt_view(
        stored: &StoredPsbt,
        policy: &MultisigPolicy,
    ) -> Result<PsbtSession, SpectraBridgeError> {
        let decoded = psbt::decode(&stored.psbt)?;
        let review = psbt::review(policy, stored.chain, &decoded)?;
        Ok(PsbtSession {
            id: stored.id.clone(),
            wallet_id: stored.wallet_id.clone(),
            txid: review.txid.clone(),
            review_digest: review.digest.clone(),
            threshold: policy.threshold as u32,
            inputs: review
                .inputs
                .iter()
                .map(|input| PsbtInputView {
                    outpoint: input.outpoint.to_string(),
                    address: input.address.clone(),
                    value_sat: input.value,
                    signatures: input.signed_by.len() as u32,
                })
                .collect(),
            outputs: review
                .outputs
                .iter()
                .map(|output| PsbtOutputView {
                    address: output.address.clone(),
                    value_sat: output.value,
                    is_change: output.change.is_some(),
                })
                .collect(),
            fee_sat: review.fee,
            signed_by: review
                .signed_by()
                .into_iter()
                .map(|cosigner| policy.cosigners[cosigner].fingerprint.to_string())
                .collect(),
            complete: review.complete,
            psbt: stored.psbt.clone(),
            broadcast_txid: stored.broadcast_txid.clone(),
        })
    }

    /// Every input still a confirmed output of its address, of the amount
    /// reviewed: what an indexer says, not what the PSBT claims.
    async fn psbt_inputs_unspent(
        &self,
        wallet: &WalletState,
        policy: &MultisigPolicy,
        review: &psbt::Review,
    ) -> Result<(), SpectraBridgeError> {
        let mut sources: Vec<UtxoSendSource> = Vec::new();
        for input in &review.inputs {
            if sources.iter().all(|source| source.address != input.address) {
                sources.push(multisig_source(wallet.chain_id, policy, input.place)?);
            }
        }
        let unspent: std::collections::HashSet<(String, u32, u64)> = self
            .inputs_paying(wallet.chain_id, sources)
            .await?
            .into_iter()
            .map(|input| {
                (
                    input.utxo.0.to_ascii_lowercase(),
                    input.utxo.1,
                    input.utxo.2,
                )
            })
            .collect();
        for input in &review.inputs {
            if !unspent.contains(&(
                input.outpoint.txid.to_string(),
                input.outpoint.vout,
                input.value,
            )) {
                return Err(SpectraBridgeError::invalid(
                    "An input is spent, unconfirmed or not of the amount the PSBT says; build it again.",
                ));
            }
        }
        Ok(())
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// A PSBT paying `amount` (in BTC) to `to_address` from the multisig
    /// wallet's confirmed outputs, largest first and none another open
    /// session spends, its change to the account's next change address, at
    /// `fee_rate_svb` or the network's rate.
    pub async fn create_psbt(
        &self,
        wallet_id: String,
        to_address: String,
        amount: String,
        fee_rate_svb: Option<String>,
    ) -> Result<PsbtSession, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let (wallet, policy) = this.multisig_policy(&wallet_id).await?;
            let chain = wallet.chain_id;
            let recipient = bitcoin::ScriptBuf::from(crate::send::account_utxo::recipient_script(
                chain,
                to_address.trim(),
            )?);
            let amount = crate::decimal::to_units(amount.trim(), 8)
                .and_then(|units| u64::try_from(units).ok())
                .filter(|units| *units > 0)
                .ok_or_else(|| SpectraBridgeError::invalid("Invalid amount"))?;
            let dust = super::send_stage_utxo::change_dust(chain)?;
            if amount < dust {
                return Err(SpectraBridgeError::invalid(
                    "The amount is below the network's dust threshold",
                ));
            }
            let rate = match fee_rate_svb {
                Some(rate) => rate.trim().parse::<f64>().ok(),
                None => Some(this.bitcoin_fee_rate(chain).await?.sats_per_vbyte.ceil()),
            }
            .filter(|rate| rate.is_finite() && *rate > 0.0)
            .ok_or_else(|| SpectraBridgeError::invalid("Invalid fee rate"))?;
            let fee_for = |inputs: usize, change: bool| {
                let mut scripts = vec![recipient.len()];
                if change {
                    scripts.push(34);
                }
                (psbt::estimate_vsize(&policy, inputs, scripts) as f64 * rate).ceil() as u64
            };
            let pending: std::collections::HashSet<bitcoin::OutPoint> = this
                .psbt_stored_for_wallet(&wallet_id)
                .await?
                .into_iter()
                .filter(|stored| stored.broadcast_txid.is_none())
                .filter_map(|stored| psbt::decode(&stored.psbt).ok())
                .flat_map(|stored| {
                    stored
                        .unsigned_tx
                        .input
                        .into_iter()
                        .map(|input| input.previous_output)
                })
                .collect();
            let sources = this.multisig_sources(&wallet, &policy).await?;
            let places: std::collections::HashMap<String, Place> = sources
                .iter()
                .map(|(source, place)| (source.address.clone(), *place))
                .collect();
            let mut candidates: Vec<(bitcoin::OutPoint, u64, Place)> = this
                .inputs_paying(
                    chain,
                    sources.into_iter().map(|(source, _)| source).collect(),
                )
                .await?
                .into_iter()
                .map(|input| {
                    let outpoint = bitcoin::OutPoint {
                        txid: input.utxo.0.parse().map_err(SpectraBridgeError::failure)?,
                        vout: input.utxo.1,
                    };
                    Ok((outpoint, input.utxo.2, places[&input.source.address]))
                })
                .collect::<Result<_, SpectraBridgeError>>()?;
            candidates.retain(|(outpoint, _, _)| !pending.contains(outpoint));
            candidates.sort_by_key(|(_, value, _)| std::cmp::Reverse(*value));
            let mut selected = Vec::new();
            let mut total = 0u64;
            let mut plan = None;
            for candidate in candidates {
                total = total
                    .checked_add(candidate.1)
                    .ok_or_else(|| SpectraBridgeError::invalid("Input total overflow"))?;
                selected.push(candidate);
                let with_change = fee_for(selected.len(), true);
                if let Some(change) = total
                    .checked_sub(amount)
                    .and_then(|rest| rest.checked_sub(with_change))
                    .filter(|change| *change >= dust)
                {
                    plan = Some(Some(change));
                    break;
                }
                if total >= amount.saturating_add(fee_for(selected.len(), false)) {
                    plan = Some(None);
                    break;
                }
            }
            let change = plan.ok_or_else(|| {
                SpectraBridgeError::from(crate::send::error::SendError::insufficient_funds())
            })?;
            let change_place = (
                1,
                u32::try_from(
                    this.keypool_state(wallet_id.clone(), chain)
                        .await?
                        .next_change_index,
                )
                .map_err(|_| SpectraBridgeError::failure("change index is out of range"))?,
            );
            let built = psbt::build(
                &policy,
                &selected,
                &[(recipient, amount)],
                change.map(|value| (change_place, value)),
            )?;
            let stored = StoredPsbt {
                id: crate::store::new_event_id(),
                wallet_id: wallet_id.clone(),
                chain,
                psbt: psbt::encode(&built),
                created_at: crate::wallet_db::now_secs() as f64,
                broadcast_txid: None,
            };
            let view = Self::psbt_view(&stored, &policy)?;
            this.psbt_save(&stored).await?;
            Ok(view)
        })
        .await
    }

    /// Read a PSBT another coordinator or cosigner wrote, refused unless it
    /// is the wallet's (`send::psbt::review`). A copy of a transaction an
    /// open session holds joins it, its signatures with the session's.
    pub async fn import_psbt(
        &self,
        wallet_id: String,
        psbt_base64: String,
    ) -> Result<PsbtSession, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let (wallet, policy) = this.multisig_policy(&wallet_id).await?;
            let chain = wallet.chain_id;
            let imported = psbt::decode(&psbt_base64)?;
            psbt::review(&policy, chain, &imported)?;
            let open = this
                .psbt_stored_for_wallet(&wallet_id)
                .await?
                .into_iter()
                .filter(|stored| stored.broadcast_txid.is_none())
                .find(|stored| {
                    psbt::decode(&stored.psbt)
                        .is_ok_and(|held| held.unsigned_tx == imported.unsigned_tx)
                });
            let stored = match open {
                Some(mut stored) => {
                    let mut held = psbt::decode(&stored.psbt)?;
                    psbt::combine(&policy, chain, &mut held, imported)?;
                    stored.psbt = psbt::encode(&held);
                    stored
                }
                None => StoredPsbt {
                    id: crate::store::new_event_id(),
                    wallet_id: wallet_id.clone(),
                    chain,
                    psbt: psbt::encode(&imported),
                    created_at: crate::wallet_db::now_secs() as f64,
                    broadcast_txid: None,
                },
            };
            let view = Self::psbt_view(&stored, &policy)?;
            this.psbt_save(&stored).await?;
            Ok(view)
        })
        .await
    }

    /// Sign session `session_id` as the wallet's cosigner, once the PSBT is
    /// still the one reviewed (`review_digest`) and every input still one of
    /// the wallet's unspent outputs of the amount reviewed.
    pub async fn sign_psbt(
        &self,
        session_id: String,
        review_digest: String,
        password: Option<String>,
    ) -> Result<PsbtSession, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let mut stored = this.psbt_load(&session_id).await?;
            let (wallet, policy) = this.multisig_policy(&stored.wallet_id).await?;
            if !matches!(wallet.signing, WalletSigning::SeedPhrase { .. }) {
                return Err(SpectraBridgeError::invalid(
                    "This wallet holds no cosigner's key: import a cosigner's phrase into it to sign.",
                ));
            }
            let mut held = psbt::decode(&stored.psbt)?;
            let review = psbt::review(&policy, stored.chain, &held)?;
            if review.digest != review_digest.trim() {
                return Err(SpectraBridgeError::invalid(
                    "The PSBT is not the one reviewed; review it again.",
                ));
            }
            this.psbt_inputs_unspent(&wallet, &policy, &review).await?;
            let phrase = crate::store::wallet_secrets::load_seed_phrase(
                &*this.secrets()?,
                &wallet.id,
                password.as_deref(),
            )?;
            let (_, account) = policy.cosigner_of_phrase(
                stored.chain,
                &phrase,
                wallet
                    .derivation_overrides
                    .passphrase
                    .as_deref()
                    .unwrap_or_default(),
            )?;
            psbt::sign(&policy, stored.chain, &mut held, &account)?;
            stored.psbt = psbt::encode(&held);
            let view = Self::psbt_view(&stored, &policy)?;
            this.psbt_save(&stored).await?;
            Ok(view)
        })
        .await
    }

    pub async fn psbt_session(
        &self,
        session_id: String,
    ) -> Result<PsbtSession, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let stored = this.psbt_load(&session_id).await?;
            let (_, policy) = this.multisig_policy(&stored.wallet_id).await?;
            Self::psbt_view(&stored, &policy)
        })
        .await
    }

    /// The wallet's sessions, oldest first.
    pub async fn psbt_sessions(
        &self,
        wallet_id: String,
    ) -> Result<Vec<PsbtSession>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (_, policy) = this.multisig_policy(&wallet_id).await?;
            this.psbt_stored_for_wallet(&wallet_id)
                .await?
                .iter()
                .map(|stored| Self::psbt_view(stored, &policy))
                .collect()
        })
        .await
    }

    /// The finished transaction, hex, once the threshold has signed every
    /// input. Nothing is broadcast.
    pub async fn finalize_psbt(&self, session_id: String) -> Result<String, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let stored = this.psbt_load(&session_id).await?;
            let (_, policy) = this.multisig_policy(&stored.wallet_id).await?;
            let tx = psbt::finalize(&policy, stored.chain, &psbt::decode(&stored.psbt)?)?;
            Ok(hex::encode(bitcoin::consensus::serialize(&tx)))
        })
        .await
    }

    /// Finalize the session's transaction and broadcast it, the inputs
    /// first checked unspent.
    pub async fn broadcast_psbt(
        &self,
        session_id: String,
    ) -> Result<PsbtSession, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let mut stored = this.psbt_load(&session_id).await?;
            let (wallet, policy) = this.multisig_policy(&stored.wallet_id).await?;
            let held = psbt::decode(&stored.psbt)?;
            let review = psbt::review(&policy, stored.chain, &held)?;
            this.psbt_inputs_unspent(&wallet, &policy, &review).await?;
            let tx = psbt::finalize(&policy, stored.chain, &held)?;
            let txid = this
                .broadcast_raw_extract(
                    stored.chain,
                    hex::encode(bitcoin::consensus::serialize(&tx)),
                    "txid".into(),
                )
                .await?;
            if !txid.is_empty() && !txid.eq_ignore_ascii_case(&review.txid) {
                return Err(SpectraBridgeError::failure(
                    "The network accepted another transaction id than the PSBT's",
                ));
            }
            stored.broadcast_txid = Some(review.txid.clone());
            let view = Self::psbt_view(&stored, &policy)?;
            this.psbt_save(&stored).await?;
            Ok(view)
        })
        .await
    }

    /// Forget a session; its inputs are free for another.
    pub async fn discard_psbt(&self, session_id: String) -> Result<(), SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let db = this.bound_database().await?;
            tokio::task::spawn_blocking(move || {
                crate::wallet_db::psbt_session_delete(&db, &session_id)
            })
            .await
            .map_err(SpectraBridgeError::failure)??;
            Ok(())
        })
        .await
    }
}
