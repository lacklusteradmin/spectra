//! A Safe's sessions: a `SafeTx` and its owners' signatures, kept with the
//! policy (version, owners, threshold) the Safe held when the session was
//! built. The Safe is read again before an owner signs and before an owner
//! executes, and a session built under owners or a threshold the Safe no
//! longer has is refused rather than signed. Executing is an owner's own
//! transaction calling `execTransaction`, which that owner's wallet pays
//! the gas for.
use super::multisig::{
    MultisigAccount, MultisigOutput, MultisigPermission, MultisigScheme, MultisigSigner,
    MultisigSpend, SessionBody, SessionReview, StoredSession,
};
use super::*;
use crate::send::safe::{self, EvmAddress, SafePolicy, SafeSessionData, SafeState, SafeTx};
use crate::store::state::WalletState;
use zeroize::Zeroizing;

/// A session's content: the Safe's policy when it was built, and the
/// transaction with its signatures as owners hand it on.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SafeSession {
    pub policy: SafePolicy,
    pub data: SafeSessionData,
}

/// The Safe a wallet watches.
fn safe_address(wallet: &WalletState) -> Result<EvmAddress, SpectraBridgeError> {
    Ok(safe::parse_address(
        wallet
            .address_on(wallet.chain_id)
            .ok_or_else(|| SpectraBridgeError::failure("Wallet has no address on this network"))?,
    )?)
}

fn signers(policy: &SafePolicy, signed: &[EvmAddress]) -> Vec<MultisigSigner> {
    policy
        .owners
        .iter()
        .map(|owner| MultisigSigner {
            signer: owner.clone(),
            weight: 1,
            signed: signed
                .iter()
                .any(|signer| safe::address_text(signer) == *owner),
            wallet_id: None,
        })
        .collect()
}

pub(super) fn review(
    wallet: &WalletState,
    session: &SafeSession,
) -> Result<SessionReview, SpectraBridgeError> {
    let safe = safe_address(wallet)?;
    let chain_id = wallet.chain_id.evm_chain_id()?;
    let reviewed = safe::review(&session.policy, &safe, chain_id, &session.data)?;
    let signed: Vec<EvmAddress> = reviewed
        .signatures
        .iter()
        .map(|(signer, _)| *signer)
        .collect();
    let data = safe::session_data(
        &safe,
        chain_id,
        &session.policy,
        &reviewed.tx,
        &reviewed.signatures,
    );
    let digest = format!("0x{}", hex::encode(reviewed.hash));
    Ok(SessionReview {
        transaction_id: digest.clone(),
        digest,
        threshold: session.policy.threshold,
        signers: signers(&session.policy, &signed),
        inputs: Vec::new(),
        outputs: vec![MultisigOutput {
            address: safe::address_text(&reviewed.tx.to),
            value: reviewed.tx.value.to_string(),
            is_change: false,
            data: (!reviewed.tx.data.is_empty())
                .then(|| format!("0x{}", hex::encode(&reviewed.tx.data))),
            asset: None,
            memo: None,
        }],
        // The executing owner pays the gas, outside the Safe.
        fee: "0".into(),
        sequence: Some(reviewed.tx.nonce.to_string()),
        expires_at: None,
        expires_at_height: None,
        complete: reviewed.complete(&session.policy),
        data: serde_json::to_string(&data)?,
    })
}

/// The call that executes the session's transaction: `execTransaction`
/// on the Safe with the threshold's signatures in owner order, hex.
pub(super) fn finalize(
    wallet: &WalletState,
    session: &SafeSession,
) -> Result<String, SpectraBridgeError> {
    let reviewed = safe::review(
        &session.policy,
        &safe_address(wallet)?,
        wallet.chain_id.evm_chain_id()?,
        &session.data,
    )?;
    if !reviewed.complete(&session.policy) {
        return Err(SpectraBridgeError::invalid(
            "The session does not yet carry enough owners' signatures.",
        ));
    }
    Ok(format!(
        "0x{}",
        hex::encode(
            reviewed
                .tx
                .exec_calldata(&reviewed.signature_bytes(&session.policy))
        )
    ))
}

impl WalletService {
    /// The Safe's state, read from the first endpoint on the right network
    /// that answers.
    async fn safe_state(
        &self,
        chain: Chain,
        safe: &EvmAddress,
    ) -> Result<SafeState, SpectraBridgeError> {
        let endpoints = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        let safe = safe::address_text(safe);
        crate::api::http::race(&endpoints, |endpoint| {
            let safe = &safe;
            async move {
                self.validate_endpoint_network(chain, &endpoint).await?;
                Ok::<_, SpectraBridgeError>(
                    EvmClient::new(Arc::new(vec![endpoint]), chain.evm_chain_id()?)
                        .fetch_safe(safe)
                        .await?,
                )
            }
        })
        .await
    }

    /// The Safe's state and policy now, refused unless it is still the
    /// policy `session` was built under.
    async fn safe_unchanged(
        &self,
        wallet: &WalletState,
        session: &SafeSession,
    ) -> Result<SafeState, SpectraBridgeError> {
        let state = self
            .safe_state(wallet.chain_id, &safe_address(wallet)?)
            .await?;
        if state.policy()? != session.policy {
            return Err(SpectraBridgeError::invalid(
                "The Safe's owners, threshold or version changed since this session was built; build it again.",
            ));
        }
        Ok(state)
    }

    pub(super) async fn safe_account(
        &self,
        wallet: &WalletState,
    ) -> Result<MultisigAccount, SpectraBridgeError> {
        let safe = safe_address(wallet)?;
        let state = self.safe_state(wallet.chain_id, &safe).await?;
        let policy = state.policy()?;
        Ok(MultisigAccount {
            wallet_id: wallet.id.clone(),
            chain: wallet.chain_id,
            scheme: MultisigScheme::Safe,
            address: safe::address_text(&safe),
            permissions: vec![MultisigPermission {
                name: format!("Safe {}", policy.version),
                threshold: policy.threshold,
                signers: signers(&policy, &[]),
                covers: Vec::new(),
            }],
            warnings: state.warnings(),
            submission: MultisigScheme::Safe.submission(),
            signer_wallet_ids: Vec::new(),
        })
    }

    /// A Safe transaction paying `spend` from the Safe at the next nonce no
    /// open session holds.
    pub(super) async fn create_safe(
        &self,
        wallet: &WalletState,
        spend: &MultisigSpend,
    ) -> Result<SessionBody, SpectraBridgeError> {
        let chain = wallet.chain_id;
        let safe = safe_address(wallet)?;
        let to = spend.to_address.trim();
        if !crate::send::flow::is_valid_send_address(chain, to.to_string()) {
            return Err(SpectraBridgeError::refused(
                "Not an address on %@: %@",
                [chain.chain_display_name(), to],
            ));
        }
        let value =
            crate::decimal::to_units(spend.amount.trim(), u32::from(chain.native_decimals()))
                .filter(|value| *value > 0)
                .ok_or_else(|| SpectraBridgeError::invalid("Invalid amount"))?;
        let state = self.safe_state(chain, &safe).await?;
        let policy = state.policy()?;
        let open = self.open_safe_sessions(wallet).await?;
        let committed: u128 = open
            .iter()
            .filter_map(|session| session.data.transaction.parse().ok())
            .filter(|tx| tx.nonce >= state.nonce)
            .filter_map(|tx| u128::try_from(&tx.value).ok())
            .fold(0, u128::saturating_add);
        if state.balance_wei < value.saturating_add(committed) {
            return Err(crate::send::error::SendError::insufficient_funds().into());
        }
        let nonce = open
            .iter()
            .filter_map(|session| session.data.transaction.parse().ok())
            .map(|tx| tx.nonce + 1)
            .fold(state.nonce, u64::max);
        let tx = SafeTx::call(safe::parse_address(to)?, value, Vec::new(), nonce);
        Ok(SessionBody::Safe(Box::new(SafeSession {
            data: safe::session_data(&safe, chain.evm_chain_id()?, &policy, &tx, &[]),
            policy,
        })))
    }

    async fn open_safe_sessions(
        &self,
        wallet: &WalletState,
    ) -> Result<Vec<SafeSession>, SpectraBridgeError> {
        Ok(self
            .multisig_open_sessions(&wallet.id)
            .await?
            .into_iter()
            .filter_map(|stored| match stored.body {
                SessionBody::Safe(session) => Some(*session),
                _ => None,
            })
            .collect())
    }

    /// A Safe transaction another owner wrote, refused unless it is this
    /// Safe's under its policy now, its nonce not yet used. A copy of a
    /// transaction an open session holds joins it.
    pub(super) async fn import_safe(
        &self,
        wallet: &WalletState,
        open: Vec<StoredSession>,
        data: &str,
    ) -> Result<StoredSession, SpectraBridgeError> {
        let chain = wallet.chain_id;
        let safe = safe_address(wallet)?;
        let chain_id = chain.evm_chain_id()?;
        let imported: SafeSessionData = serde_json::from_str(data)
            .map_err(|_| SpectraBridgeError::invalid("Not a Safe transaction."))?;
        let state = self.safe_state(chain, &safe).await?;
        let policy = state.policy()?;
        let reviewed = safe::review(&policy, &safe, chain_id, &imported)?;
        if reviewed.tx.nonce < state.nonce {
            return Err(SpectraBridgeError::invalid(
                "This Safe transaction's nonce was already used.",
            ));
        }
        for mut stored in open {
            let SessionBody::Safe(session) = &stored.body else {
                continue;
            };
            let held = safe::review(&session.policy, &safe, chain_id, &session.data)?;
            if held.hash != reviewed.hash {
                continue;
            }
            let mut signatures = held.signatures;
            for (signer, signature) in reviewed.signatures {
                if signatures.iter().all(|(known, _)| *known != signer) {
                    signatures.push((signer, signature));
                }
            }
            signatures.sort_by_key(|(owner, _)| *owner);
            stored.body = SessionBody::Safe(Box::new(SafeSession {
                data: safe::session_data(&safe, chain_id, &policy, &held.tx, &signatures),
                policy,
            }));
            return Ok(stored);
        }
        Ok(StoredSession::new(
            wallet,
            SessionBody::Safe(Box::new(SafeSession {
                data: safe::session_data(
                    &safe,
                    chain_id,
                    &policy,
                    &reviewed.tx,
                    &reviewed.signatures,
                ),
                policy,
            })),
        ))
    }

    /// An owner wallet on the Safe's network and its key: refused for a
    /// wallet that is no owner, holds no key, or is on another network.
    async fn safe_owner_key(
        &self,
        wallet: &WalletState,
        policy: &SafePolicy,
        owner_wallet_id: &str,
        password: Option<&str>,
    ) -> Result<(EvmAddress, Zeroizing<Vec<u8>>), SpectraBridgeError> {
        let owner = self.stored_wallet(owner_wallet_id).await?;
        if owner.chain_id != wallet.chain_id {
            return Err(SpectraBridgeError::refused(
                "%@ is not on %@.",
                [owner.name.as_str(), wallet.chain_id.chain_display_name()],
            ));
        }
        let identity = self
            .resolve_send_identity(wallet.chain_id, owner_wallet_id, password)
            .await?;
        let address = safe::parse_address(&identity.from_address)?;
        if !policy.owners.contains(&safe::address_text(&address)) {
            return Err(SpectraBridgeError::refused(
                "%@ is not one of the Safe's owners.",
                [owner.name.as_str()],
            ));
        }
        Ok((
            address,
            Zeroizing::new(hex::decode(identity.private_key_hex.as_str())?),
        ))
    }

    /// Sign the session's transaction as the owner `signer_wallet_id`, the
    /// Safe read again first: its policy unchanged and the nonce not yet
    /// used.
    pub(super) async fn sign_safe(
        &self,
        wallet: &WalletState,
        stored: &mut StoredSession,
        signer_wallet_id: Option<String>,
        password: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        let SessionBody::Safe(session) = &stored.body else {
            return Err(SpectraBridgeError::failure("not a Safe session"));
        };
        let signer_wallet_id = signer_wallet_id
            .ok_or_else(|| SpectraBridgeError::invalid("Name the owner wallet that signs."))?;
        let safe = safe_address(wallet)?;
        let chain_id = wallet.chain_id.evm_chain_id()?;
        let state = self.safe_unchanged(wallet, session).await?;
        let reviewed = safe::review(&session.policy, &safe, chain_id, &session.data)?;
        if reviewed.tx.nonce < state.nonce {
            return Err(SpectraBridgeError::invalid(
                "This Safe transaction's nonce was already used.",
            ));
        }
        let (owner, key) = self
            .safe_owner_key(
                wallet,
                &session.policy,
                &signer_wallet_id,
                password.as_deref(),
            )
            .await?;
        if reviewed
            .signatures
            .iter()
            .any(|(signer, _)| *signer == owner)
        {
            return Err(SpectraBridgeError::invalid(
                "This owner already signed the session.",
            ));
        }
        let signature = safe::sign(&reviewed.hash, &key)?;
        if safe::recover_signer(&reviewed.hash, &signature)? != owner {
            return Err(SpectraBridgeError::failure("signature does not recover"));
        }
        let mut signatures = reviewed.signatures;
        signatures.push((owner, signature));
        signatures.sort_by_key(|(owner, _)| *owner);
        stored.body = SessionBody::Safe(Box::new(SafeSession {
            data: safe::session_data(&safe, chain_id, &session.policy, &reviewed.tx, &signatures),
            policy: session.policy.clone(),
        }));
        Ok(())
    }

    /// Execute the session's transaction: `execTransaction` with the
    /// threshold's signatures in owner order, sent and paid for by the owner
    /// `executor_wallet_id`, once the Safe is read again and the
    /// transaction's nonce is the Safe's next. The executing transaction's
    /// hash.
    pub(super) async fn execute_safe(
        &self,
        wallet: &WalletState,
        session: &SafeSession,
        executor_wallet_id: Option<String>,
        password: Option<String>,
    ) -> Result<String, SpectraBridgeError> {
        let chain = wallet.chain_id;
        let executor_wallet_id = executor_wallet_id.ok_or_else(|| {
            SpectraBridgeError::invalid("Name the owner wallet that executes and pays the gas.")
        })?;
        let safe = safe_address(wallet)?;
        let chain_id = chain.evm_chain_id()?;
        let state = self.safe_unchanged(wallet, session).await?;
        let reviewed = safe::review(&session.policy, &safe, chain_id, &session.data)?;
        if !reviewed.complete(&session.policy) {
            return Err(SpectraBridgeError::invalid(
                "The session does not yet carry enough owners' signatures.",
            ));
        }
        if reviewed.tx.nonce < state.nonce {
            return Err(SpectraBridgeError::invalid(
                "This Safe transaction's nonce was already used.",
            ));
        }
        if reviewed.tx.nonce > state.nonce {
            return Err(SpectraBridgeError::refused(
                "The Safe runs nonce %@ next; execute its earlier transactions first.",
                [state.nonce],
            ));
        }
        let (executor, key) = self
            .safe_owner_key(
                wallet,
                &session.policy,
                &executor_wallet_id,
                password.as_deref(),
            )
            .await?;
        let executor = safe::address_text(&executor);
        let calldata = reviewed
            .tx
            .exec_calldata(&reviewed.signature_bytes(&session.policy));
        let overrides = crate::send::evm::EvmSendOverrides {
            nonce: Some(self.next_send_nonce(chain, &executor).await?),
            ..Default::default()
        };
        let endpoints = self.endpoints_for(chain, &[EndpointCapability::Fee]).await;
        let safe_text = safe::address_text(&safe);
        let prepared = crate::api::http::race(&endpoints, |endpoint| {
            let executor = &executor;
            let safe_text = &safe_text;
            let calldata = &calldata;
            let overrides = &overrides;
            async move {
                self.validate_endpoint_network(chain, &endpoint).await?;
                Ok::<_, SpectraBridgeError>(
                    crate::send::evm::prepare_transfer(
                        &EvmClient::new(Arc::new(vec![endpoint]), chain_id),
                        executor,
                        safe_text,
                        0,
                        calldata,
                        overrides,
                    )
                    .await?,
                )
            }
        })
        .await?;
        self.validate_evm_funds(chain, &executor, &prepared).await?;
        let raw = prepared.sign(&key)?;
        let hash = format!("0x{}", hex::encode(crate::derivation::evm::keccak256(&raw)));
        let accepted = self
            .broadcast_raw_extract(chain, format!("0x{}", hex::encode(&raw)), "txid".into())
            .await?;
        if !accepted.is_empty() && !accepted.eq_ignore_ascii_case(&hash) {
            return Err(SpectraBridgeError::failure(
                "The network accepted another transaction hash than the one signed",
            ));
        }
        Ok(hash)
    }
}
