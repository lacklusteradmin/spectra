//! A `sortedmulti` account's sessions on a UTXO network, judged against the
//! wallet's descriptor whenever shown, signed or broadcast: a P2WSH
//! account's (Bitcoin, Litecoin) a PSBT as its signatures have gathered
//! (`send::psbt`), a P2SH account's (Bitcoin Cash, Dogecoin) the unsigned
//! transaction with its inputs' amounts and signatures, passed between
//! signers in the network's own form (`send::p2sh_multisig`). The account
//! wallet signs as the cosigner whose phrase was added to it.
use super::multisig::{
    MultisigAccount, MultisigInput, MultisigOutput, MultisigPermission, MultisigScheme,
    MultisigSigner, MultisigSpend, SessionBody, SessionReview, StoredSession,
};
use super::*;
use crate::derivation::multisig::{MultisigPolicy, Place, UtxoMultisigScript};
use crate::send::p2sh_multisig::{self as p2sh, P2shSpend};
use crate::send::psbt;
use crate::send::stages::UtxoSendSource;
use crate::store::state::{WalletSigning, WalletState};

fn policy(wallet: &WalletState) -> Result<MultisigPolicy, SpectraBridgeError> {
    Ok(MultisigPolicy::parse(
        wallet.chain_id,
        wallet
            .multisig_policy
            .as_deref()
            .ok_or_else(|| SpectraBridgeError::invalid("Only a multisig wallet keeps PSBTs."))?,
    )?)
}

/// The cosigners, each named by its key fingerprint, `signed` when it
/// signed every input.
fn cosigners(policy: &MultisigPolicy, signed: &[usize]) -> Vec<MultisigSigner> {
    policy
        .cosigners
        .iter()
        .enumerate()
        .map(|(index, cosigner)| MultisigSigner {
            signer: cosigner.fingerprint.to_string(),
            weight: 1,
            signed: signed.contains(&index),
            wallet_id: None,
        })
        .collect()
}

pub(super) fn account(wallet: &WalletState) -> Result<MultisigAccount, SpectraBridgeError> {
    let policy = policy(wallet)?;
    Ok(MultisigAccount {
        wallet_id: wallet.id.clone(),
        chain: wallet.chain_id,
        scheme: MultisigScheme::SortedMulti,
        address: policy.address(wallet.chain_id, (0, 0))?,
        permissions: vec![MultisigPermission {
            name: "sortedmulti".into(),
            threshold: policy.threshold as u64,
            signers: cosigners(&policy, &[]),
            covers: Vec::new(),
        }],
        warnings: Vec::new(),
        submission: MultisigScheme::SortedMulti.submission(),
        signer_wallet_ids: Vec::new(),
    })
}

/// `encoded` reviewed against the wallet's descriptor.
pub(super) fn review(
    wallet: &WalletState,
    encoded: &str,
) -> Result<SessionReview, SpectraBridgeError> {
    let policy = policy(wallet)?;
    let decoded = psbt::decode(encoded)?;
    let review = psbt::review(&policy, wallet.chain_id, &decoded)?;
    Ok(session_review(&policy, review, encoded.to_string()))
}

/// A P2SH spend reviewed against the wallet's descriptor; what travels is
/// its network's form.
pub(super) fn review_p2sh(
    wallet: &WalletState,
    spend: &P2shSpend,
) -> Result<SessionReview, SpectraBridgeError> {
    let policy = policy(wallet)?;
    let review = p2sh::review(&policy, wallet.chain_id, spend)?;
    let data = p2sh::encode(&policy, wallet.chain_id, spend)?;
    Ok(session_review(&policy, review, data))
}

fn session_review(policy: &MultisigPolicy, review: psbt::Review, data: String) -> SessionReview {
    SessionReview {
        transaction_id: review.txid.clone(),
        digest: review.digest.clone(),
        threshold: policy.threshold as u64,
        signers: cosigners(policy, &review.signed_by()),
        inputs: review
            .inputs
            .iter()
            .map(|input| MultisigInput {
                outpoint: input.outpoint.to_string(),
                address: input.address.clone(),
                value: input.value.to_string(),
                signatures: input.signed_by.len() as u32,
            })
            .collect(),
        outputs: review
            .outputs
            .iter()
            .map(|output| MultisigOutput {
                address: output.address.clone(),
                value: output.value.to_string(),
                is_change: output.change.is_some(),
                data: None,
                asset: None,
                memo: None,
            })
            .collect(),
        fee: review.fee.to_string(),
        sequence: None,
        expires_at: None,
        expires_at_height: None,
        complete: review.complete,
        data,
    }
}

/// A PSBT another coordinator or cosigner wrote, refused unless it is the
/// wallet's. A copy of a transaction an open session holds joins it.
pub(super) fn import(
    wallet: &WalletState,
    open: Vec<StoredSession>,
    data: &str,
) -> Result<StoredSession, SpectraBridgeError> {
    let policy = policy(wallet)?;
    let chain = wallet.chain_id;
    let imported = psbt::decode(data)?;
    psbt::review(&policy, chain, &imported)?;
    let held = open.into_iter().find_map(|stored| match &stored.body {
        SessionBody::Psbt { psbt: held } => psbt::decode(held)
            .ok()
            .filter(|held| held.unsigned_tx == imported.unsigned_tx)
            .map(|held| (stored.clone(), held)),
        #[allow(unreachable_patterns)]
        _ => None,
    });
    Ok(match held {
        Some((mut stored, mut held)) => {
            psbt::combine(&policy, chain, &mut held, imported)?;
            stored.body = SessionBody::Psbt {
                psbt: psbt::encode(&held),
            };
            stored
        }
        None => StoredSession::new(
            wallet,
            SessionBody::Psbt {
                psbt: psbt::encode(&imported),
            },
        ),
    })
}

/// The finished transaction, hex.
pub(super) fn finalize(wallet: &WalletState, encoded: &str) -> Result<String, SpectraBridgeError> {
    let tx = psbt::finalize(&policy(wallet)?, wallet.chain_id, &psbt::decode(encoded)?)?;
    Ok(hex::encode(bitcoin::consensus::serialize(&tx)))
}

/// A P2SH spend's finished transaction, hex.
pub(super) fn finalize_p2sh(
    wallet: &WalletState,
    spend: &P2shSpend,
) -> Result<String, SpectraBridgeError> {
    let policy = policy(wallet)?;
    p2sh::review(&policy, wallet.chain_id, spend)?;
    Ok(hex::encode(bitcoin::consensus::serialize(&p2sh::finalize(
        &policy, spend,
    )?)))
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
        script_pubkey: policy.script_pubkey(place)?.into_bytes(),
    })
}

/// The outpoints an open session spends.
fn spent_by(stored: StoredSession) -> Vec<bitcoin::OutPoint> {
    match stored.body {
        SessionBody::Psbt { psbt } => psbt::decode(&psbt)
            .map(|held| {
                held.unsigned_tx
                    .input
                    .into_iter()
                    .map(|input| input.previous_output)
                    .collect()
            })
            .unwrap_or_default(),
        SessionBody::P2sh(spend) => spend
            .transaction
            .input
            .into_iter()
            .map(|input| input.previous_output)
            .collect(),
        _ => Vec::new(),
    }
}

/// A P2SH account's unspent outputs (amount and place) and its own
/// scripts' places.
type Holdings = (
    std::collections::HashMap<bitcoin::OutPoint, (u64, Place)>,
    std::collections::HashMap<Vec<u8>, Place>,
);

/// The smallest output the network relays to `script`.
fn dust(chain: Chain, script: &[u8]) -> Result<u64, SpectraBridgeError> {
    Ok(match chain.mainnet_counterpart() {
        Chain::Litecoin => crate::send::litecoin::litecoin_dust_threshold(chain, script)?,
        _ => super::send_stage_utxo::change_dust(chain)?,
    })
}

impl WalletService {
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

    /// A spend another coordinator or cosigner wrote, in the network's
    /// form, refused unless it is the wallet's; a Dogecoin transaction's
    /// inputs must be the wallet's unspent outputs, whose amounts it does
    /// not carry. A copy of a transaction an open session holds joins it.
    pub(super) async fn import_p2sh(
        &self,
        wallet: &WalletState,
        open: Vec<StoredSession>,
        data: &str,
    ) -> Result<StoredSession, SpectraBridgeError> {
        let policy = policy(wallet)?;
        let chain = wallet.chain_id;
        let mut held_outputs = std::collections::HashMap::new();
        let mut own_scripts = std::collections::HashMap::new();
        if chain.sighash_fork_id().is_err() {
            // An input at an address this device has not seen used yet is
            // found by discovering the account's addresses first.
            let spends: Vec<bitcoin::OutPoint> = hex::decode(data.trim())
                .ok()
                .and_then(|bytes| {
                    bitcoin::consensus::deserialize::<bitcoin::Transaction>(&bytes).ok()
                })
                .map(|tx| tx.input.iter().map(|input| input.previous_output).collect())
                .unwrap_or_default();
            let (held, scripts) = self.p2sh_holdings(wallet, &policy).await?;
            (held_outputs, own_scripts) =
                if spends.iter().all(|outpoint| held.contains_key(outpoint)) {
                    (held, scripts)
                } else {
                    self.discover_utxo_addresses(wallet.id.clone(), chain)
                        .await?;
                    self.p2sh_holdings(wallet, &policy).await?
                };
        }
        let unspent = |outpoint: &bitcoin::OutPoint| held_outputs.get(outpoint).copied();
        let change = |script: &bitcoin::ScriptBuf| own_scripts.get(script.as_bytes()).copied();
        let imported = p2sh::decode(
            &policy,
            chain,
            data,
            &p2sh::Holdings {
                unspent: &unspent,
                change: &change,
            },
        )?;
        for mut stored in open {
            let SessionBody::P2sh(held) = &stored.body else {
                continue;
            };
            if held.transaction != imported.transaction {
                continue;
            }
            let mut held = held.clone();
            p2sh::combine(&policy, chain, &mut held, imported)?;
            stored.body = SessionBody::P2sh(held);
            return Ok(stored);
        }
        Ok(StoredSession::new(wallet, SessionBody::P2sh(imported)))
    }

    /// The account's unspent outputs with their amounts and places, and
    /// the scripts of its known addresses and next change address: what a
    /// Dogecoin transaction does not carry.
    async fn p2sh_holdings(
        &self,
        wallet: &WalletState,
        policy: &MultisigPolicy,
    ) -> Result<Holdings, SpectraBridgeError> {
        let chain = wallet.chain_id;
        let sources = self.multisig_sources(wallet, policy).await?;
        let mut own_scripts: std::collections::HashMap<Vec<u8>, Place> = sources
            .iter()
            .map(|(source, place)| (source.script_pubkey.clone(), *place))
            .collect();
        let next_change = (
            1,
            u32::try_from(
                self.keypool_state(wallet.id.clone(), chain)
                    .await?
                    .next_change_index,
            )
            .map_err(|_| SpectraBridgeError::failure("change index is out of range"))?,
        );
        own_scripts.insert(policy.script_pubkey(next_change)?.into_bytes(), next_change);
        let places: std::collections::HashMap<String, Place> = sources
            .iter()
            .map(|(source, place)| (source.address.clone(), *place))
            .collect();
        let mut held_outputs = std::collections::HashMap::new();
        for input in self
            .inputs_paying(
                chain,
                sources.into_iter().map(|(source, _)| source).collect(),
            )
            .await?
        {
            let outpoint = bitcoin::OutPoint {
                txid: input.utxo.0.parse().map_err(SpectraBridgeError::failure)?,
                vout: input.utxo.1,
            };
            held_outputs.insert(outpoint, (input.utxo.2, places[&input.source.address]));
        }
        Ok((held_outputs, own_scripts))
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

    /// A PSBT paying `spend` from the wallet's confirmed outputs, largest
    /// first and none another open session spends, its change to the
    /// account's next change address, at the spend's fee rate or the
    /// network's.
    pub(super) async fn create_psbt(
        &self,
        wallet: &WalletState,
        spend: &MultisigSpend,
    ) -> Result<SessionBody, SpectraBridgeError> {
        let policy = policy(wallet)?;
        let chain = wallet.chain_id;
        let to_address = spend.to_address.trim();
        if chain.mainnet_counterpart() == Chain::Litecoin
            && crate::send::litecoin_mweb::keys::StealthAddress::decode(chain, to_address).is_some()
        {
            return Err(SpectraBridgeError::invalid(
                "A PSBT cannot pay an MWEB address; pay its owner's transparent address.",
            ));
        }
        let recipient = bitcoin::ScriptBuf::from(crate::send::account_utxo::recipient_script(
            chain, to_address,
        )?);
        let amount = crate::decimal::to_units(spend.amount.trim(), 8)
            .and_then(|units| u64::try_from(units).ok())
            .filter(|units| *units > 0)
            .ok_or_else(|| SpectraBridgeError::invalid("Invalid amount"))?;
        if amount < dust(chain, recipient.as_bytes())? {
            return Err(SpectraBridgeError::invalid(
                "The amount is below the network's dust threshold",
            ));
        }
        let rate = match (&spend.fee_rate, policy.script) {
            (Some(rate), _) => rate.trim().parse::<f64>().ok(),
            (None, UtxoMultisigScript::Wsh) => {
                Some(self.bitcoin_fee_rate(chain).await?.sats_per_vbyte.ceil())
            }
            (None, UtxoMultisigScript::Sh) => Some(1.0),
        }
        .filter(|rate| rate.is_finite() && *rate > 0.0)
        .ok_or_else(|| SpectraBridgeError::invalid("Invalid fee rate"))?;
        let change_script = policy.script_pubkey((1, 0))?;
        let change_dust = dust(chain, change_script.as_bytes())?;
        // A P2SH network without a requested rate pays its static fee per
        // started kilobyte, as its ordinary sends do.
        let per_kilobyte = match (policy.script, &spend.fee_rate) {
            (UtxoMultisigScript::Sh, None) => Some(super::helpers::fee_or_static(chain, None)?),
            _ => None,
        };
        let fee_for = |inputs: usize, change: bool| {
            let mut scripts = vec![recipient.len()];
            if change {
                scripts.push(change_script.len());
            }
            match (policy.script, per_kilobyte) {
                (UtxoMultisigScript::Wsh, _) => {
                    (psbt::estimate_vsize(&policy, inputs, scripts) as f64 * rate).ceil() as u64
                }
                (UtxoMultisigScript::Sh, Some(per_kilobyte)) => {
                    (p2sh::estimate_size(&policy, inputs, scripts) as u64).div_ceil(1_000)
                        * per_kilobyte
                }
                (UtxoMultisigScript::Sh, None) => {
                    (p2sh::estimate_size(&policy, inputs, scripts) as f64 * rate).ceil() as u64
                }
            }
        };
        let pending: std::collections::HashSet<bitcoin::OutPoint> = self
            .multisig_open_sessions(&wallet.id)
            .await?
            .into_iter()
            .flat_map(spent_by)
            .collect();
        let sources = self.multisig_sources(wallet, &policy).await?;
        let places: std::collections::HashMap<String, Place> = sources
            .iter()
            .map(|(source, place)| (source.address.clone(), *place))
            .collect();
        let mut candidates: Vec<(bitcoin::OutPoint, u64, Place)> = self
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
                .filter(|change| *change >= change_dust)
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
                self.keypool_state(wallet.id.clone(), chain)
                    .await?
                    .next_change_index,
            )
            .map_err(|_| SpectraBridgeError::failure("change index is out of range"))?,
        );
        let change = change.map(|value| (change_place, value));
        Ok(match policy.script {
            UtxoMultisigScript::Wsh => SessionBody::Psbt {
                psbt: psbt::encode(&psbt::build(
                    &policy,
                    &selected,
                    &[(recipient, amount)],
                    change,
                )?),
            },
            UtxoMultisigScript::Sh => SessionBody::P2sh(p2sh::build(
                &policy,
                chain,
                &selected,
                &[(recipient, amount)],
                change,
            )?),
        })
    }

    /// Sign every input as the cosigner whose phrase the wallet holds,
    /// once every input is still one of the wallet's unspent outputs of
    /// the amount reviewed.
    pub(super) async fn sign_psbt(
        &self,
        wallet: &WalletState,
        stored: &mut StoredSession,
        password: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        let policy = policy(wallet)?;
        if !matches!(wallet.signing, WalletSigning::SeedPhrase { .. }) {
            return Err(SpectraBridgeError::invalid(
                "This wallet holds no cosigner's key: import a cosigner's phrase into it to sign.",
            ));
        }
        let review = match &stored.body {
            SessionBody::Psbt { psbt: encoded } => {
                psbt::review(&policy, stored.chain, &psbt::decode(encoded)?)?
            }
            SessionBody::P2sh(spend) => p2sh::review(&policy, stored.chain, spend)?,
            _ => return Err(SpectraBridgeError::failure("not a UTXO multisig session")),
        };
        self.psbt_inputs_unspent(wallet, &policy, &review).await?;
        let phrase = crate::store::wallet_secrets::load_seed_phrase(
            &*self.secrets()?,
            &wallet.id,
            password.as_deref(),
        )?;
        let (cosigner, account) = policy.cosigner_of_phrase(
            stored.chain,
            &phrase,
            wallet
                .derivation_overrides
                .passphrase
                .as_deref()
                .unwrap_or_default(),
        )?;
        stored.body = match &stored.body {
            SessionBody::Psbt { psbt: encoded } => {
                let mut held = psbt::decode(encoded)?;
                psbt::sign(&policy, stored.chain, &mut held, &account)?;
                SessionBody::Psbt {
                    psbt: psbt::encode(&held),
                }
            }
            SessionBody::P2sh(spend) => {
                let mut spend = spend.clone();
                p2sh::sign(&policy, stored.chain, &mut spend, cosigner, &account)?;
                SessionBody::P2sh(spend)
            }
            _ => return Err(SpectraBridgeError::failure("not a UTXO multisig session")),
        };
        Ok(())
    }

    /// Finalize the PSBT and broadcast it, the inputs first checked
    /// unspent; the network's transaction id.
    pub(super) async fn broadcast_psbt(
        &self,
        wallet: &WalletState,
        body: &SessionBody,
    ) -> Result<String, SpectraBridgeError> {
        let policy = policy(wallet)?;
        let chain = wallet.chain_id;
        let (review, tx) = match body {
            SessionBody::Psbt { psbt: encoded } => {
                let held = psbt::decode(encoded)?;
                (
                    psbt::review(&policy, chain, &held)?,
                    psbt::finalize(&policy, chain, &held)?,
                )
            }
            SessionBody::P2sh(spend) => (
                p2sh::review(&policy, chain, spend)?,
                p2sh::finalize(&policy, spend)?,
            ),
            _ => return Err(SpectraBridgeError::failure("not a UTXO multisig session")),
        };
        self.psbt_inputs_unspent(wallet, &policy, &review).await?;
        let txid = self
            .broadcast_raw_extract(
                chain,
                hex::encode(bitcoin::consensus::serialize(&tx)),
                "txid".into(),
            )
            .await?;
        if !txid.is_empty() && !txid.eq_ignore_ascii_case(&review.txid) {
            return Err(SpectraBridgeError::failure(
                "The network accepted another transaction id than the session's",
            ));
        }
        Ok(review.txid)
    }
}
