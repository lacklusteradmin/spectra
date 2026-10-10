//! A Substrate multisig account's sessions (`pallet-multisig`, on Polkadot
//! Asset Hub and Bittensor): a transfer the account makes once its
//! signatories approve it on chain. Approvals are transactions here, not
//! signatures gathered as data: each signatory signs as a wallet of its
//! own, which submits its approval and pays its fee, and the first reserves
//! the pallet's deposit until the operation ends. The approval that meets
//! the threshold carries the transfer, and the runtime executes it, so a
//! session has nothing left to submit. What travels between signatories is
//! the transfer's call; who approved it is read from the network before
//! every approval.
use super::multisig::{
    MultisigAccount, MultisigOutput, MultisigPermission, MultisigScheme, MultisigSigner,
    MultisigSpend, SessionBody, SessionReview, StoredSession,
};
use super::*;
use crate::api::substrate_json_rpc::multisig::PendingMultisig;
use crate::derivation::substrate_multisig::SubstrateMultisig;
use crate::send::polkadot::PreparedPolkadotTransaction;
use crate::send::substrate_multisig::{self as substrate, MultisigTransfer, Timepoint};
use crate::store::state::WalletState;

/// A session's content: the transfer and what the network last showed of
/// its approvals.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SubstrateSession {
    /// The transfer the account makes, hex, as its runtime encodes it.
    pub call: String,
    /// The runtime's `Balances` pallet and `transfer_keep_alive` indices the
    /// call was encoded under.
    pub indices: (u8, u8),
    /// The operation's timepoint, once its first approval is on chain.
    pub timepoint: Option<Timepoint>,
    /// The signatories whose approvals are on chain, or were submitted from
    /// this device since it last looked; addresses.
    pub approvals: Vec<String>,
}

fn policy(wallet: &WalletState) -> Result<SubstrateMultisig, SpectraBridgeError> {
    Ok(SubstrateMultisig::parse(
        wallet.chain_id,
        wallet.multisig_policy.as_deref().ok_or_else(|| {
            SpectraBridgeError::invalid("This wallet is not a Substrate multisig account.")
        })?,
    )?)
}

fn members(policy: &SubstrateMultisig, approvals: &[String]) -> Vec<MultisigSigner> {
    policy
        .signatories
        .iter()
        .map(|key| {
            let address = policy.address_of(key);
            MultisigSigner {
                signed: approvals.contains(&address),
                signer: address,
                weight: 1,
                wallet_id: None,
            }
        })
        .collect()
}

fn decode_call(text: &str) -> Result<Vec<u8>, SpectraBridgeError> {
    hex::decode(text.trim().trim_start_matches("0x"))
        .map_err(|_| SpectraBridgeError::invalid("Not a Substrate call."))
}

/// The session's policy, call and transfer.
fn read(
    wallet: &WalletState,
    session: &SubstrateSession,
) -> Result<(SubstrateMultisig, Vec<u8>, MultisigTransfer), SpectraBridgeError> {
    let policy = policy(wallet)?;
    let call = decode_call(&session.call)?;
    let transfer = substrate::decode_transfer(&call, session.indices)?;
    Ok((policy, call, transfer))
}

/// The approvals `pending` holds, as addresses.
fn approvals_of(policy: &SubstrateMultisig, pending: &PendingMultisig) -> Vec<String> {
    pending
        .approvals
        .iter()
        .map(|key| policy.address_of(key))
        .collect()
}

pub(super) fn review(
    wallet: &WalletState,
    session: &SubstrateSession,
) -> Result<SessionReview, SpectraBridgeError> {
    let (policy, call, transfer) = read(wallet, session)?;
    let hash = substrate::call_hash(&call);
    let signers = members(&policy, &session.approvals);
    let approved = signers.iter().filter(|signer| signer.signed).count() as u64;
    Ok(SessionReview {
        transaction_id: format!("0x{}", hex::encode(hash)),
        digest: hex::encode(hash),
        threshold: u64::from(policy.threshold),
        signers,
        inputs: Vec::new(),
        outputs: vec![MultisigOutput {
            address: policy.address_of(&transfer.recipient),
            value: transfer.amount.to_string(),
            is_change: false,
            data: None,
            asset: None,
            memo: None,
        }],
        fee: "0".into(),
        sequence: session
            .timepoint
            .map(|(height, index)| format!("{height}-{index}")),
        expires_at: None,
        expires_at_height: None,
        complete: approved >= u64::from(policy.threshold),
        data: format!("0x{}", hex::encode(&call)),
    })
}

pub(super) fn account(wallet: &WalletState) -> Result<MultisigAccount, SpectraBridgeError> {
    let policy = policy(wallet)?;
    Ok(MultisigAccount {
        wallet_id: wallet.id.clone(),
        chain: wallet.chain_id,
        scheme: MultisigScheme::SubstrateMultisig,
        address: policy.address(),
        permissions: vec![MultisigPermission {
            name: "multisig".into(),
            threshold: u64::from(policy.threshold),
            signers: members(&policy, &[]),
            covers: Vec::new(),
        }],
        warnings: Vec::new(),
        submission: MultisigScheme::SubstrateMultisig.submission(),
        signer_wallet_ids: Vec::new(),
    })
}

/// Approvals are transactions: there is no finished transaction to hand
/// over or submit.
pub(super) fn refuse_submission() -> SpectraBridgeError {
    SpectraBridgeError::invalid(
        "Each signatory's approval goes to the network as it signs; the one that meets the threshold executes the transfer.",
    )
}

impl WalletService {
    async fn substrate_endpoints(&self, chain: Chain) -> Arc<Vec<String>> {
        self.endpoints_for(chain, &[EndpointCapability::Verification])
            .await
    }

    /// A transfer from the account the runtime would accept now: within
    /// what the account can spend and keep its existential deposit, leaving
    /// the recipient at least that deposit.
    pub(super) async fn create_substrate(
        &self,
        wallet: &WalletState,
        spend: &MultisigSpend,
        open: &[StoredSession],
    ) -> Result<SessionBody, SpectraBridgeError> {
        let chain = wallet.chain_id;
        let policy = policy(wallet)?;
        let to = spend.to_address.trim();
        let recipient = crate::derivation::primitives::decode_ss58(to, chain.ss58_prefix())
            .map(|(_, key)| key)
            .map_err(|_| {
                SpectraBridgeError::refused(
                    "Not an address on %@: %@",
                    [chain.chain_display_name(), to],
                )
            })?;
        let amount =
            crate::decimal::to_units(spend.amount.trim(), u32::from(chain.native_decimals()))
                .filter(|units| *units > 0)
                .filter(|units| {
                    chain.substrate_balance_bytes() != Some(8) || *units <= u128::from(u64::MAX)
                })
                .ok_or_else(|| SpectraBridgeError::invalid("Invalid amount"))?;
        let account = policy.account_id();
        let (policy, account) = (&policy, &account);
        let body = crate::api::http::race(
            &self.substrate_endpoints(chain).await,
            |endpoint| async move {
                let client = SubstrateClient::new(Arc::new(vec![endpoint]));
                let indices = {
                    let context = client.polkadot_context(chain).await?;
                    (
                        context.runtime.transfer_pallet,
                        context.runtime.transfer_call,
                    )
                };
                let transfer = MultisigTransfer { recipient, amount };
                let call = substrate::transfer_call(indices, &transfer);
                let snapshot = client
                    .multisig_snapshot(chain, account, &substrate::call_hash(&call))
                    .await?;
                if (
                    snapshot.context.runtime.transfer_pallet,
                    snapshot.context.runtime.transfer_call,
                ) != indices
                {
                    return Err(SpectraBridgeError::invalid(
                        "The runtime changed while the transfer was built; build it again.",
                    ));
                }
                if snapshot.pallet.max_signatories < policy.signatories.len() as u32 {
                    return Err(SpectraBridgeError::invalid(
                        "The runtime takes fewer signatories than this account has.",
                    ));
                }
                let existential = snapshot.context.runtime.existential_deposit;
                if amount > snapshot.balance.keep_alive_spendable(existential) {
                    return Err(SpectraBridgeError::invalid(
                        "The account cannot pay this and keep its existential deposit.",
                    ));
                }
                let destination = client
                    .fetch_balance_at(chain, &recipient, &snapshot.context.block_hash)
                    .await?;
                if destination
                    .free
                    .checked_add(amount)
                    .is_none_or(|total| total < existential)
                {
                    return Err(SpectraBridgeError::invalid(
                        "The recipient would hold less than the existential deposit.",
                    ));
                }
                Ok(SubstrateSession {
                    call: format!("0x{}", hex::encode(&call)),
                    indices,
                    timepoint: snapshot.pending.as_ref().map(|p| (p.height, p.index)),
                    approvals: snapshot
                        .pending
                        .as_ref()
                        .map(|pending| approvals_of(policy, pending))
                        .unwrap_or_default(),
                })
            },
        )
        .await?;
        if open.iter().any(
            |stored| matches!(&stored.body, SessionBody::Substrate(held) if held.call == body.call),
        ) {
            return Err(SpectraBridgeError::invalid(
                "An open session already makes this transfer.",
            ));
        }
        Ok(SessionBody::Substrate(body))
    }

    /// A transfer another signatory sent: its call, read under the runtime
    /// the network runs, with the approvals the network shows. A call an
    /// open session holds joins it.
    pub(super) async fn import_substrate(
        &self,
        wallet: &WalletState,
        open: Vec<StoredSession>,
        data: &str,
    ) -> Result<StoredSession, SpectraBridgeError> {
        let chain = wallet.chain_id;
        let policy = policy(wallet)?;
        let call = decode_call(data)?;
        let account = policy.account_id();
        let (policy, account, call) = (&policy, &account, &call);
        let session = crate::api::http::race(
            &self.substrate_endpoints(chain).await,
            |endpoint| async move {
                let client = SubstrateClient::new(Arc::new(vec![endpoint]));
                let snapshot = client
                    .multisig_snapshot(chain, account, &substrate::call_hash(call))
                    .await?;
                let indices = (
                    snapshot.context.runtime.transfer_pallet,
                    snapshot.context.runtime.transfer_call,
                );
                substrate::decode_transfer(call, indices)?;
                Ok::<_, SpectraBridgeError>(SubstrateSession {
                    call: format!("0x{}", hex::encode(call)),
                    indices,
                    timepoint: snapshot.pending.as_ref().map(|p| (p.height, p.index)),
                    approvals: snapshot
                        .pending
                        .as_ref()
                        .map(|pending| approvals_of(policy, pending))
                        .unwrap_or_default(),
                })
            },
        )
        .await?;
        for mut stored in open {
            let SessionBody::Substrate(held) = &stored.body else {
                continue;
            };
            if held.call != session.call {
                continue;
            }
            let mut merged = session.clone();
            // Approvals sent from here may not have reached a block yet.
            if merged.timepoint.is_none() {
                for approval in &held.approvals {
                    if !merged.approvals.contains(approval) {
                        merged.approvals.push(approval.clone());
                    }
                }
            }
            stored.body = SessionBody::Substrate(merged);
            return Ok(stored);
        }
        Ok(StoredSession::new(wallet, SessionBody::Substrate(session)))
    }

    /// Approve the session's transfer as `signer_wallet_id`, a wallet at one
    /// of the account's signatories: by its hash while approvals are short
    /// of the threshold, and with the transfer itself, which the runtime
    /// then executes, once this approval meets it. The approval is
    /// submitted from the signer's own account.
    pub(super) async fn sign_substrate(
        &self,
        wallet: &WalletState,
        stored: &mut StoredSession,
        signer_wallet_id: Option<String>,
        password: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        let SessionBody::Substrate(session) = &stored.body else {
            return Err(SpectraBridgeError::failure("not a Substrate session"));
        };
        let chain = wallet.chain_id;
        let signer_wallet_id = signer_wallet_id
            .ok_or_else(|| SpectraBridgeError::invalid("Name the wallet that signs."))?;
        let signer = self.stored_wallet(&signer_wallet_id).await?;
        let (policy, call, transfer) = read(wallet, session)?;
        let signatory = signer
            .address_on(chain)
            .filter(|_| signer.chain_id == chain && !signer.is_watch_only())
            .and_then(|address| {
                crate::derivation::primitives::decode_ss58(address, chain.ss58_prefix()).ok()
            })
            .map(|(_, key)| key)
            .filter(|key| policy.signatories.contains(key))
            .ok_or_else(|| {
                SpectraBridgeError::refused(
                    "%@ is not one of the account's signatories.",
                    [signer.name.as_str()],
                )
            })?;
        let sender = policy.address_of(&signatory);
        let identity = self
            .resolve_send_identity(chain, &signer_wallet_id, password.as_deref())
            .await?;
        let hash = substrate::call_hash(&call);
        let account = policy.account_id();
        let threshold = usize::from(policy.threshold);
        let others = policy.others(&signatory);
        let (session_ref, policy_ref, call_ref, account_ref, others_ref, sender_ref) =
            (session, &policy, &call, &account, &others, &sender);
        let (prepared, pending) =
            crate::api::http::race(&self.substrate_endpoints(chain).await, |endpoint| async move {
                let client = SubstrateClient::new(Arc::new(vec![endpoint]));
                let snapshot = client.multisig_snapshot(chain, account_ref, &hash).await?;
                let context = snapshot.context;
                if (context.runtime.transfer_pallet, context.runtime.transfer_call)
                    != session_ref.indices
                {
                    return Err(SpectraBridgeError::invalid(
                        "The runtime changed since the transfer was built; build it again.",
                    ));
                }
                if snapshot.pallet.max_signatories < policy_ref.signatories.len() as u32 {
                    return Err(SpectraBridgeError::invalid(
                        "The runtime takes fewer signatories than this account has.",
                    ));
                }
                let (timepoint, approved) = match &snapshot.pending {
                    Some(pending) => (
                        Some((pending.height, pending.index)),
                        pending.approvals.clone(),
                    ),
                    // Approved before, yet nothing pending: the first
                    // approval is not in a block, or the operation ended.
                    None if session_ref.timepoint.is_some() || !session_ref.approvals.is_empty() => {
                        return Err(SpectraBridgeError::invalid(
                            "No approval of this transfer is pending: the first is not in a block yet, or the transfer was executed or cancelled.",
                        ));
                    }
                    None => (None, Vec::new()),
                };
                let already = approved.contains(&signatory);
                if already && approved.len() < threshold {
                    return Err(SpectraBridgeError::invalid(
                        "This signatory already approved the transfer.",
                    ));
                }
                let executes = approved.len() + usize::from(!already) >= threshold;
                let (approval, reserve) = if executes {
                    if transfer.amount
                        > snapshot
                            .balance
                            .keep_alive_spendable(context.runtime.existential_deposit)
                    {
                        return Err(SpectraBridgeError::invalid(
                            "The account cannot pay this and keep its existential deposit.",
                        ));
                    }
                    let dispatch = PreparedPolkadotTransaction {
                        runtime: context.runtime.clone(),
                        sender: *account_ref,
                        call_data: call_ref.clone(),
                        nonce: 0,
                        amount: 0,
                        fee: 0,
                        finalized_number: context.finalized_number,
                    };
                    let weight = client
                        .query_weight(&dispatch.fee_extrinsic()?, &context.block_hash)
                        .await?;
                    (
                        substrate::as_multi(
                            &snapshot.pallet,
                            policy_ref.threshold,
                            others_ref,
                            timepoint,
                            call_ref,
                            weight,
                        ),
                        0,
                    )
                } else {
                    (
                        substrate::approve_as_multi(
                            &snapshot.pallet,
                            policy_ref.threshold,
                            others_ref,
                            timepoint,
                            &hash,
                        ),
                        if timepoint.is_none() {
                            snapshot
                                .pallet
                                .deposit(policy_ref.threshold)
                                .ok_or_else(|| SpectraBridgeError::failure("Deposit overflow"))?
                        } else {
                            0
                        },
                    )
                };
                let prepared = crate::send::polkadot::prepare_call(
                    &client, chain, sender_ref, context, approval, reserve,
                )
                .await?;
                Ok::<_, SpectraBridgeError>((prepared, (timepoint, approved, executes)))
            })
            .await?;
        let key = zeroize::Zeroizing::new(hex::decode(identity.private_key_hex.as_str())?);
        let raw = prepared.sign(&key, &signatory)?;
        let sent = SubstrateClient::new(
            self.endpoints_for(chain, &[EndpointCapability::Broadcast])
                .await,
        )
        .submit_extrinsic_hex(&format!("0x{}", hex::encode(raw)))
        .await?;
        let (timepoint, approved, executes) = pending;
        let mut approvals: Vec<String> =
            approved.iter().map(|key| policy.address_of(key)).collect();
        if !approvals.contains(&sender) {
            approvals.push(sender);
        }
        stored.body = SessionBody::Substrate(SubstrateSession {
            call: session.call.clone(),
            indices: session.indices,
            timepoint,
            approvals,
        });
        if executes {
            stored.submitted_txid = Some(sent.txid);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/multisig_substrate.rs"]
mod multisig_substrate_tests;
