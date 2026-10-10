//! A TON multisig v2 account's sessions: an order transferring from the
//! multisig, proposed and approved on the network. Approvals are messages,
//! not signatures gathered as data: the first signer's wallet proposes the
//! order, which the multisig deploys as an Order contract at an address its
//! number derives, and each other signer's wallet approves it there; the
//! approval that meets the threshold has the multisig execute it, so a
//! session has nothing left to submit. The multisig is a watched wallet;
//! its signers and threshold are read from the contract, whose code must be
//! the v2 multisig's, and an order's approvals from the Order contract,
//! before every proposal and approval.
use super::multisig::{
    MultisigAccount, MultisigOutput, MultisigPermission, MultisigScheme, MultisigSigner,
    MultisigSpend, SessionBody, SessionReview, StoredSession,
};
use super::*;
use crate::derivation::ton::{cell_from_boc, friendly_address, parse_ton_address};
use crate::derivation::ton_cell::Cell;
use crate::send::ton_multisig::{
    self as ton, MultisigData, OrderState, TonAccount, TonTransferOrder,
};
use crate::store::state::WalletState;
use base64::Engine;

/// What a proposal attaches, as the multisig's own interface does: the
/// order's deployment, storage and execution, the rest staying with the
/// multisig.
const NEW_ORDER_NANOTONS: u64 = 200_000_000;
/// What an approval attaches; the order returns what it does not spend.
const APPROVE_NANOTONS: u64 = 100_000_000;
/// How long an order may gather approvals unless asked otherwise.
const DEFAULT_LIFETIME_SECS: u64 = 7 * 86_400;
const MAX_LIFETIME_SECS: u64 = 365 * 86_400;

/// A session's content: the order, and what the network last showed of its
/// signers and approvals.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TonSession {
    /// The order, a BOC in base64: one action, the transfer.
    pub order: String,
    /// When the order expires, in unix seconds.
    pub expiration: u64,
    /// The order's number, once proposed.
    pub seqno: Option<u64>,
    /// The signers it is approved by, in index order, raw: the multisig's
    /// until proposed, the order's own after.
    pub signers: Vec<String>,
    pub threshold: u8,
    /// The indices of the signers whose approvals the network shows, or
    /// that were sent from here since it last looked.
    pub approvals: Vec<u8>,
}

/// What one signer hands the next.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Shared {
    order: String,
    expiration: u64,
    seqno: Option<u64>,
}

fn engine() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn raw(account: &TonAccount) -> String {
    format!("{}:{}", account.0, hex::encode(account.1))
}

fn account_of(address: &str) -> Result<TonAccount, SpectraBridgeError> {
    let parsed = parse_ton_address(address.trim())?;
    Ok((parsed.workchain, parsed.account_id))
}

fn multisig_account(wallet: &WalletState) -> Result<TonAccount, SpectraBridgeError> {
    account_of(
        wallet
            .address_on(wallet.chain_id)
            .ok_or_else(|| SpectraBridgeError::invalid("This wallet has no TON address."))?,
    )
}

fn order_cell(text: &str) -> Result<Cell, SpectraBridgeError> {
    Ok(cell_from_boc(&engine().decode(text.trim()).map_err(
        |_| SpectraBridgeError::invalid("Not a TON order."),
    )?)?)
}

/// What a signature is given for: the order and when it expires.
fn digest(order: &Cell, expiration: u64) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(order.hash_depth().0);
    hasher.update(expiration.to_be_bytes());
    hex::encode(hasher.finalize())
}

fn members(
    signers: &[String],
    approvals: &[u8],
) -> Result<Vec<MultisigSigner>, SpectraBridgeError> {
    signers
        .iter()
        .enumerate()
        .map(|(index, signer)| {
            let account = account_of(signer)?;
            Ok(MultisigSigner {
                signer: friendly_address(account.0, &account.1, true),
                weight: 1,
                signed: approvals.contains(&(index as u8)),
                wallet_id: None,
            })
        })
        .collect()
}

pub(super) fn review(
    wallet: &WalletState,
    session: &TonSession,
) -> Result<SessionReview, SpectraBridgeError> {
    let multisig = multisig_account(wallet)?;
    let order = order_cell(&session.order)?;
    let transfer = TonTransferOrder::decode(&order)?;
    let signers = members(&session.signers, &session.approvals)?;
    let approved = signers.iter().filter(|signer| signer.signed).count() as u64;
    Ok(SessionReview {
        transaction_id: match session.seqno {
            Some(seqno) => {
                let address = ton::order_address(&multisig, seqno)?;
                friendly_address(address.0, &address.1, true)
            }
            None => hex::encode(order.hash_depth().0),
        },
        digest: digest(&order, session.expiration),
        threshold: u64::from(session.threshold),
        signers,
        inputs: Vec::new(),
        outputs: vec![MultisigOutput {
            address: friendly_address(
                transfer.destination.0,
                &transfer.destination.1,
                transfer.bounce,
            ),
            value: transfer.nanotons.to_string(),
            is_change: false,
            data: None,
            asset: None,
            memo: transfer.comment,
        }],
        fee: "0".into(),
        sequence: session.seqno.map(|seqno| seqno.to_string()),
        expires_at: Some(session.expiration),
        expires_at_height: None,
        complete: approved >= u64::from(session.threshold),
        data: serde_json::to_string(&Shared {
            order: session.order.clone(),
            expiration: session.expiration,
            seqno: session.seqno,
        })?,
    })
}

/// Proposals and approvals are messages: there is no finished transaction
/// to hand over or submit.
pub(super) fn refuse_submission() -> SpectraBridgeError {
    SpectraBridgeError::invalid(
        "Each signer's approval goes to the network as it signs; the one that meets the threshold executes the order.",
    )
}

impl WalletService {
    fn ton_client(&self, endpoints: Arc<Vec<String>>) -> ToncenterV2Client {
        ToncenterV2Client::new(endpoints)
    }

    async fn ton_state(
        &self,
        chain: Chain,
        account: &TonAccount,
    ) -> Result<crate::api::toncenter_v2::TonAccountState, SpectraBridgeError> {
        Ok(self
            .ton_client(
                self.endpoints_for(chain, &[EndpointCapability::Verification])
                    .await,
            )
            .fetch_account_state(chain, &raw(account))
            .await?)
    }

    /// The multisig's data and balance, refused unless its code is the v2
    /// multisig's.
    async fn ton_multisig(
        &self,
        wallet: &WalletState,
    ) -> Result<(MultisigData, u64), SpectraBridgeError> {
        let state = self
            .ton_state(wallet.chain_id, &multisig_account(wallet)?)
            .await?;
        let code = (state.state == "active")
            .then(|| cell_from_boc(&state.code).ok())
            .flatten();
        if code.is_none_or(|code| hex::encode(code.hash_depth().0) != ton::MULTISIG_CODE_HASH) {
            return Err(SpectraBridgeError::invalid(
                "This wallet is not a TON multisig (v2) contract.",
            ));
        }
        Ok((
            MultisigData::parse(&cell_from_boc(&state.data)?)?,
            state.nanotons,
        ))
    }

    /// The multisig's order `seqno` as the network holds it, `None` until
    /// the multisig deploys it.
    async fn ton_order(
        &self,
        wallet: &WalletState,
        seqno: u64,
    ) -> Result<Option<OrderState>, SpectraBridgeError> {
        let multisig = multisig_account(wallet)?;
        let state = self
            .ton_state(wallet.chain_id, &ton::order_address(&multisig, seqno)?)
            .await?;
        if state.state != "active" {
            return Ok(None);
        }
        Ok(OrderState::parse(
            &cell_from_boc(&state.data)?,
            &multisig,
            seqno,
        )?)
    }

    pub(super) async fn ton_account(
        &self,
        wallet: &WalletState,
    ) -> Result<MultisigAccount, SpectraBridgeError> {
        let (data, _) = self.ton_multisig(wallet).await?;
        let signers: Vec<String> = data.signers.iter().map(raw).collect();
        let mut warnings = Vec::new();
        if data.proposers > 0 {
            warnings.push(
                "Proposers can put orders to the signers; only the signers' approvals execute one."
                    .into(),
            );
        }
        if data.allow_arbitrary_seqno {
            warnings.push(
                "This multisig takes orders under any number; Spectra proposes none for it.".into(),
            );
        }
        Ok(MultisigAccount {
            wallet_id: wallet.id.clone(),
            chain: wallet.chain_id,
            scheme: MultisigScheme::TonMultisig,
            address: wallet
                .address_on(wallet.chain_id)
                .unwrap_or_default()
                .to_string(),
            permissions: vec![MultisigPermission {
                name: "multisig".into(),
                threshold: u64::from(data.threshold),
                signers: members(&signers, &[])?,
                covers: Vec::new(),
            }],
            warnings,
            submission: MultisigScheme::TonMultisig.submission(),
            signer_wallet_ids: Vec::new(),
        })
    }

    /// An order transferring from the multisig, under its current signers.
    pub(super) async fn create_ton(
        &self,
        wallet: &WalletState,
        spend: &MultisigSpend,
    ) -> Result<SessionBody, SpectraBridgeError> {
        let chain = wallet.chain_id;
        let to = spend.to_address.trim();
        let destination = parse_ton_address(to)
            .and_then(|address| address.for_network(chain.is_testnet()))
            .map_err(|_| {
                SpectraBridgeError::refused(
                    "Not an address on %@: %@",
                    [chain.chain_display_name(), to],
                )
            })?;
        let nanotons =
            crate::decimal::to_units(spend.amount.trim(), u32::from(chain.native_decimals()))
                .and_then(|units| u64::try_from(units).ok())
                .filter(|units| *units > 0)
                .ok_or_else(|| SpectraBridgeError::invalid("Invalid amount"))?;
        let lifetime = spend.expires_in_secs.unwrap_or(DEFAULT_LIFETIME_SECS);
        if !(600..=MAX_LIFETIME_SECS).contains(&lifetime) {
            return Err(SpectraBridgeError::invalid(
                "An order gathers approvals for ten minutes to a year.",
            ));
        }
        let (data, balance) = self.ton_multisig(wallet).await?;
        if data.allow_arbitrary_seqno {
            return Err(SpectraBridgeError::invalid(
                "This multisig takes orders under any number; Spectra proposes none for it.",
            ));
        }
        if nanotons > balance {
            return Err(SpectraBridgeError::invalid("The multisig cannot pay this."));
        }
        let order = TonTransferOrder {
            destination: (destination.workchain, destination.account_id),
            bounce: destination.bounceable,
            nanotons,
            comment: None,
        }
        .order()?;
        Ok(SessionBody::Ton(TonSession {
            order: engine().encode(order.to_boc()?),
            expiration: crate::store::now_unix() as u64 + lifetime,
            seqno: None,
            signers: data.signers.iter().map(raw).collect(),
            threshold: data.threshold,
            approvals: Vec::new(),
        }))
    }

    /// The session as the network shows it now: the order's own signers
    /// and approvals once deployed, else the multisig's signers.
    async fn ton_refreshed(
        &self,
        wallet: &WalletState,
        session: &TonSession,
    ) -> Result<(TonSession, Option<OrderState>), SpectraBridgeError> {
        let order = order_cell(&session.order)?;
        let mut refreshed = session.clone();
        let state = match session.seqno {
            Some(seqno) => self.ton_order(wallet, seqno).await?,
            None => None,
        };
        match &state {
            Some(state) => {
                if state.order.hash_depth().0 != order.hash_depth().0
                    || state.expiration != session.expiration
                {
                    return Err(SpectraBridgeError::invalid(
                        "The order under this number is another one; build the transfer again.",
                    ));
                }
                refreshed.signers = state.signers.iter().map(raw).collect();
                refreshed.threshold = state.threshold;
                refreshed.approvals = state.approvals.clone();
            }
            None if session.seqno.is_none() => {
                let (data, _) = self.ton_multisig(wallet).await?;
                refreshed.signers = data.signers.iter().map(raw).collect();
                refreshed.threshold = data.threshold;
            }
            None => {}
        }
        Ok((refreshed, state))
    }

    /// An order another signer sent: refused unless it is a transfer as
    /// Spectra writes one, and, once proposed, the one the network holds
    /// under its number. An order an open session holds joins it.
    pub(super) async fn import_ton(
        &self,
        wallet: &WalletState,
        open: Vec<StoredSession>,
        data: &str,
    ) -> Result<StoredSession, SpectraBridgeError> {
        let shared: Shared = serde_json::from_str(data)
            .map_err(|_| SpectraBridgeError::invalid("Not a TON multisig order."))?;
        let order = order_cell(&shared.order)?;
        TonTransferOrder::decode(&order)?;
        let incoming = TonSession {
            order: engine().encode(order.to_boc()?),
            expiration: shared.expiration,
            seqno: shared.seqno,
            signers: Vec::new(),
            threshold: 0,
            approvals: Vec::new(),
        };
        let (incoming, state) = self.ton_refreshed(wallet, &incoming).await?;
        for mut stored in open {
            let SessionBody::Ton(held) = &stored.body else {
                continue;
            };
            if order_cell(&held.order)?.hash_depth().0 != order.hash_depth().0
                || held.expiration != incoming.expiration
                || (held.seqno.is_some()
                    && incoming.seqno.is_some()
                    && held.seqno != incoming.seqno)
            {
                continue;
            }
            let mut merged = incoming.clone();
            merged.seqno = incoming.seqno.or(held.seqno);
            // Approvals sent from here may not have reached the order yet.
            if state.is_none() {
                for index in &held.approvals {
                    if !merged.approvals.contains(index) {
                        merged.approvals.push(*index);
                    }
                }
            }
            stored.body = SessionBody::Ton(merged);
            return Ok(stored);
        }
        Ok(StoredSession::new(wallet, SessionBody::Ton(incoming)))
    }

    /// Propose the order, or approve it once proposed, as
    /// `signer_wallet_id`, a TON wallet at one of the signers' addresses,
    /// which attaches what the contract spends on it.
    pub(super) async fn sign_ton(
        &self,
        wallet: &WalletState,
        stored: &mut StoredSession,
        signer_wallet_id: Option<String>,
        password: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        let SessionBody::Ton(session) = &stored.body else {
            return Err(SpectraBridgeError::failure("not a TON session"));
        };
        let chain = wallet.chain_id;
        let signer_wallet_id = signer_wallet_id
            .ok_or_else(|| SpectraBridgeError::invalid("Name the wallet that signs."))?;
        let signer = self.stored_wallet(&signer_wallet_id).await?;
        let multisig = multisig_account(wallet)?;
        let order = order_cell(&session.order)?;
        let transfer = TonTransferOrder::decode(&order)?;
        let (mut refreshed, state) = self.ton_refreshed(wallet, session).await?;
        let sender = signer
            .address_on(chain)
            .filter(|_| signer.chain_id == chain && !signer.is_watch_only())
            .ok_or_else(|| {
                SpectraBridgeError::refused(
                    "%@ is not one of the multisig's signers.",
                    [signer.name.as_str()],
                )
            })?
            .to_string();
        let index = refreshed
            .signers
            .iter()
            .position(|signer| account_of(signer).ok() == account_of(&sender).ok())
            .ok_or_else(|| {
                SpectraBridgeError::refused(
                    "%@ is not one of the multisig's signers.",
                    [signer.name.as_str()],
                )
            })? as u8;
        let now = crate::store::now_unix() as u64;
        if session.expiration <= now + 60 {
            return Err(SpectraBridgeError::invalid("The order expired."));
        }
        let (target, attached, body, seqno) = match (session.seqno, &state) {
            (None, _) => {
                let (data, balance) = self.ton_multisig(wallet).await?;
                if data.allow_arbitrary_seqno {
                    return Err(SpectraBridgeError::invalid(
                        "This multisig takes orders under any number; Spectra proposes none for it.",
                    ));
                }
                if refreshed.signers != session.signers || refreshed.threshold != session.threshold
                {
                    return Err(SpectraBridgeError::invalid(
                        "The multisig's signers changed since the order was built; build it again.",
                    ));
                }
                if refreshed.threshold == 1 && transfer.nanotons > balance {
                    return Err(SpectraBridgeError::invalid("The multisig cannot pay this."));
                }
                let seqno = data.next_order_seqno;
                (
                    multisig,
                    NEW_ORDER_NANOTONS,
                    ton::new_order(seqno, seqno, index, session.expiration, order.clone())?,
                    seqno,
                )
            }
            (Some(_), None) => {
                return Err(SpectraBridgeError::invalid(
                    "The order is not on the network yet; sign again once its proposal is processed.",
                ));
            }
            (Some(seqno), Some(state)) => {
                if state.sent_for_execution {
                    return Err(SpectraBridgeError::invalid(
                        "The order was already executed.",
                    ));
                }
                if state.approvals.contains(&index) {
                    return Err(SpectraBridgeError::invalid(
                        "This signer already approved the order.",
                    ));
                }
                if state.approvals.len() + 1 >= usize::from(state.threshold) {
                    let (_, balance) = self.ton_multisig(wallet).await?;
                    if transfer.nanotons > balance {
                        return Err(SpectraBridgeError::invalid("The multisig cannot pay this."));
                    }
                }
                (
                    ton::order_address(&multisig, seqno)?,
                    APPROVE_NANOTONS,
                    ton::approve(seqno, index)?,
                    seqno,
                )
            }
        };
        let client = self.ton_client(
            self.endpoints_for(chain, &[EndpointCapability::Verification])
                .await,
        );
        let fee = chain
            .static_fee_units()
            .ok_or_else(|| SpectraBridgeError::failure("TON fee unavailable"))?;
        if u128::from(client.fetch_balance(&sender).await?.nanotons) < u128::from(attached) + fee {
            return Err(SpectraBridgeError::refused(
                "%@ cannot pay what its approval attaches and its fee.",
                [signer.name.as_str()],
            ));
        }
        let identity = self
            .resolve_send_identity(chain, &signer_wallet_id, password.as_deref())
            .await?;
        let key = decode_secret_array::<32>(&identity.private_key_hex)?;
        let public =
            crate::send::keys::Ed25519Seed::from_hex(&identity.private_key_hex)?.public_key();
        let wallet_signer = crate::send::ton::TonSigner::for_sender(chain, &sender, &key, &public)?;
        let wallet_seqno = client.fetch_seqno(&sender).await?;
        let valid_until = u32::try_from(now + 60)
            .map_err(|_| SpectraBridgeError::failure("TON expiry overflow"))?;
        let raw = crate::send::ton::build_transfer_with_body(
            &wallet_signer,
            crate::derivation::ton::TonAddress {
                workchain: target.0,
                account_id: target.1,
                bounceable: true,
                test_only: false,
            },
            attached,
            wallet_seqno,
            Some(body),
            valid_until,
            3,
        )?;
        let sent = self
            .ton_client(
                self.endpoints_for(chain, &[EndpointCapability::Broadcast])
                    .await,
            )
            .send_boc(&engine().encode(raw))
            .await?;
        let executes = refreshed.approvals.len() + 1 >= usize::from(refreshed.threshold);
        refreshed.seqno = Some(seqno);
        if !refreshed.approvals.contains(&index) {
            refreshed.approvals.push(index);
        }
        stored.body = SessionBody::Ton(refreshed);
        if executes {
            stored.submitted_txid = Some(sent.message_hash);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/multisig_ton.rs"]
mod multisig_ton_tests;
