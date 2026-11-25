//! Hedera: building, revalidating, signing and expiring an HBAR transfer,
//! and its send preview.
//!
//! Every read is made of one mirror node verified as the network's, so the
//! account, its key, the recipient, the exchange rate and the network time
//! come from the same place. The consensus nodes a transfer is signed for
//! come from the address book its submission will read.
use std::sync::Arc;

use super::protocols::StagedAmount;
use super::*;
use crate::api::hedera_mirror::{HederaAccount, HederaAccountKey, HederaMirrorClient};
use crate::derivation::hedera::{HederaAccountId, HederaPublicKey};
use crate::send::hedera::HederaTransferTerms;
use crate::send::payload::SignedPayload;
use crate::send::payment_memo::PaymentMemo;
use crate::send::preview_types::SendPreview;

/// `account` as an account a key of this wallet signs for alone: it exists,
/// is not deleted and holds one key, which is returned.
fn single_key(
    chain: Chain,
    id: HederaAccountId,
    account: Option<HederaAccount>,
) -> Result<HederaPublicKey, SpectraBridgeError> {
    let account = account.filter(|account| !account.deleted).ok_or_else(|| {
        SpectraBridgeError::failed(
            "%@ has no account %@.",
            [chain.chain_display_name().to_string(), id.to_string()],
        )
    })?;
    match account.key {
        HederaAccountKey::Single(key) => Ok(key),
        HederaAccountKey::Other => Err(SpectraBridgeError::failed(
            "%@ is held by a key list or threshold key, which one key does not sign for.",
            [id.to_string()],
        )),
    }
}

/// The recipient a transfer can pay: an account the network has, not
/// deleted, and that takes a transfer without signing it too.
async fn check_recipient(
    client: &HederaMirrorClient,
    chain: Chain,
    recipient: HederaAccountId,
) -> Result<(), SpectraBridgeError> {
    let account = client
        .fetch_account(recipient)
        .await?
        .filter(|account| !account.deleted)
        .ok_or_else(|| {
            SpectraBridgeError::failed(
                "%@ has no account %@: a transfer to it would fail and still pay its fee.",
                [
                    chain.chain_display_name().to_string(),
                    recipient.to_string(),
                ],
            )
        })?;
    if account.receiver_sig_required {
        return Err(SpectraBridgeError::failed(
            "%@ takes a transfer only with its own signature as well.",
            [recipient.to_string()],
        ));
    }
    Ok(())
}

/// The most a transfer may pay, in tinybars: the fee bound at the exchange
/// rate that buys the most HBAR for it, of the current and the next.
async fn fee_bound(client: &HederaMirrorClient) -> Result<u128, SpectraBridgeError> {
    client
        .fetch_exchange_rates()
        .await?
        .into_iter()
        .filter_map(|rate| {
            rate.tinybars_for_mills(crate::registry::HEDERA_TRANSFER_FEE_BOUND_MILLS)
        })
        .max()
        .ok_or_else(|| SpectraBridgeError::failure("Fee overflow"))
}

/// `count` distinct nodes of `nodes`, starting at a random one: the load of
/// many wallets spreads over the network.
fn pick_nodes(nodes: &[HederaAccountId], count: usize) -> Vec<HederaAccountId> {
    if nodes.is_empty() {
        return Vec::new();
    }
    let start = rand::random::<usize>() % nodes.len();
    nodes
        .iter()
        .cycle()
        .skip(start)
        .take(count.min(nodes.len()))
        .copied()
        .collect()
}

/// The transaction's valid start (seconds, nanoseconds), from the mirror
/// node's newest consensus time: at least a second before it, so it is never
/// ahead of a consensus node's clock, with random nanoseconds so two
/// signings from one account do not share a transaction id.
fn valid_start(network: crate::api::hedera_mirror::HederaTimestamp) -> (i64, i32) {
    (
        network.seconds.saturating_sub(1),
        (rand::random::<u32>() % 1_000_000_000) as i32,
    )
}

impl WalletService {
    /// Mirror nodes on `chain` with `needs`.
    async fn hedera_mirrors(&self, chain: Chain, needs: &[EndpointCapability]) -> Arc<Vec<String>> {
        self.endpoints_for(chain, needs).await
    }

    pub(super) async fn prepare_hedera(
        &self,
        chain: Chain,
        request: &mut crate::send::SendExecutionRequest,
        sender: &str,
        staged: StagedAmount,
    ) -> Result<PreparedPayload, SpectraBridgeError> {
        if request.contract_address.is_some() {
            return Err(SpectraBridgeError::failure("Hedera sends carry HBAR only"));
        }
        let payer = HederaAccountId::parse(chain, sender)?;
        let recipient = HederaAccountId::parse(chain, &request.to_address)?;
        let memo = PaymentMemo::text(request.memo.as_ref(), chain)?.to_string();
        let reviewed = request
            .fee_amount
            .as_deref()
            .map(|fee| crate::send::payload::fee_units(fee, u32::from(chain.native_decimals())))
            .transpose()?;
        let books = self
            .endpoints_for(chain, &[EndpointCapability::Broadcast])
            .await;
        let nodes = crate::api::hedera_hapi::HederaHapiClient::new(books)
            .fetch_nodes()
            .await?;
        let nodes: Vec<HederaAccountId> = nodes.into_iter().map(|node| node.account).collect();
        let nodes = pick_nodes(&nodes, crate::registry::HEDERA_SUBMISSION_NODES);
        let amount = staged.amount;
        let (key, max_fee) = crate::api::http::race(&staged.eps, |endpoint| {
            async move {
                let client = HederaMirrorClient::new(Arc::new(vec![endpoint]));
                client.verify_network(chain).await?;
                let key = single_key(chain, payer, client.fetch_account(payer).await?)?;
                check_recipient(&client, chain, recipient).await?;
                // A reviewed bound higher than the network asks for is kept:
                // it is what the person agreed to pay at most.
                let max_fee = fee_bound(&client)
                    .await?
                    .max(reviewed.map(u128::from).unwrap_or(0));
                let balance = client.fetch_balance(payer).await?;
                let required = amount
                    .checked_add(max_fee)
                    .ok_or_else(|| SpectraBridgeError::failure("Amount overflow"))?;
                if balance < required {
                    let decimals = u32::from(chain.native_decimals());
                    let symbol = chain.coin_symbol();
                    let required = crate::amount::decimal::from_units(required, decimals);
                    return Err(SpectraBridgeError::failed(
                        "Insufficient %@ for the amount plus the most the network fee can be (requires %@ %@).",
                        [symbol, required.as_str(), symbol],
                    ));
                }
                Ok::<_, SpectraBridgeError>((key, max_fee))
            }
        })
        .await?;
        let prepared = crate::send::hedera::prepare_transfer(
            chain,
            HederaTransferTerms {
                payer: &payer.to_string(),
                public_key: &key.der_hex(),
                recipient: &recipient.to_string(),
                amount,
                memo: &memo,
                max_fee,
                nodes: &nodes,
            },
        )?;
        request.fee_amount = Some(crate::amount::decimal::from_units(
            max_fee,
            u32::from(chain.native_decimals()),
        ));
        Ok(PreparedPayload::Hedera(prepared))
    }

    /// Recheck a prepared transfer against the network: the payer still
    /// holds the key it was built for alone, the recipient still takes it,
    /// and the balance still pays the amount and the most the fee can be.
    pub(super) async fn validate_hedera_state(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<(), SpectraBridgeError> {
        let PreparedPayload::Hedera(prepared) = &stored.prepared else {
            return Ok(());
        };
        let payer = HederaAccountId::parse(chain, &prepared.payer)?;
        let recipient = HederaAccountId::parse(chain, &prepared.recipient)?;
        let eps = self
            .hedera_mirrors(
                chain,
                &[
                    EndpointCapability::Verification,
                    EndpointCapability::Balance,
                ],
            )
            .await;
        crate::api::http::race(&eps, |endpoint| async move {
            let client = HederaMirrorClient::new(Arc::new(vec![endpoint]));
            client.verify_network(chain).await?;
            let key = single_key(chain, payer, client.fetch_account(payer).await?)?;
            if key.der_hex() != prepared.public_key {
                return Err(SpectraBridgeError::failure(
                    "Hedera account's key changed; build and review again",
                ));
            }
            check_recipient(&client, chain, recipient).await?;
            let required = prepared
                .amount
                .checked_add(prepared.max_fee)
                .ok_or_else(|| SpectraBridgeError::failure("Amount overflow"))?;
            if client.fetch_balance(payer).await? < required {
                return Err(SpectraBridgeError::failed(
                    "Insufficient %@ for the amount plus the most the network fee can be.",
                    [chain.coin_symbol()],
                ));
            }
            Ok(())
        })
        .await
    }

    /// Sign the transfer for each of its nodes, its valid start read from a
    /// verified mirror node now: the signed bodies stay valid for the
    /// transaction's 120 seconds from there.
    pub(super) async fn sign_staged_hedera(
        &self,
        chain: Chain,
        stored: &StoredSend,
        signer: &crate::service::send_identity::ResolvedSendIdentity,
    ) -> Result<(SignedPayload, Option<String>), SpectraBridgeError> {
        let PreparedPayload::Hedera(prepared) = &stored.prepared else {
            return Err(SpectraBridgeError::failure("Not a Hedera transaction"));
        };
        let key = crate::derivation::hedera::stored_key(&signer.private_key_hex)?;
        let eps = self
            .hedera_mirrors(chain, &[EndpointCapability::Verification])
            .await;
        let now = crate::api::http::race(&eps, |endpoint| async move {
            let client = HederaMirrorClient::new(Arc::new(vec![endpoint]));
            client.verify_network(chain).await?;
            client.fetch_network_time().await
        })
        .await?;
        let signed = prepared.sign(chain, &key, valid_start(now))?;
        Ok((
            SignedPayload::Hex(hex::encode(signed.transaction_list)),
            Some(signed.transaction_id),
        ))
    }

    /// Whether a signed Hedera transfer can no longer land: the network's
    /// time is past its valid start and duration.
    pub(super) async fn hedera_signature_expired(
        &self,
        chain: Chain,
        stored: &StoredSend,
    ) -> Result<bool, SpectraBridgeError> {
        let Some(transaction_id) = stored
            .submission
            .as_ref()
            .and_then(|submission| submission.transaction_hash.as_deref())
        else {
            return Ok(false);
        };
        let start: i64 = transaction_id
            .split('-')
            .nth(1)
            .and_then(|seconds| seconds.parse().ok())
            .ok_or_else(|| SpectraBridgeError::failure("Invalid Hedera transaction id"))?;
        let eps = self
            .hedera_mirrors(chain, &[EndpointCapability::Verification])
            .await;
        let now = crate::api::http::race(&eps, |endpoint| async move {
            let client = HederaMirrorClient::new(Arc::new(vec![endpoint]));
            client.verify_network(chain).await?;
            client.fetch_network_time().await
        })
        .await?;
        Ok(now.seconds >= start.saturating_add(crate::registry::HEDERA_TRANSACTION_VALID_SECS))
    }

    /// A Hedera send's quote: the most a transfer's fee can be at the
    /// network's exchange rate, and what the account holds. An account the
    /// network does not have holds nothing.
    pub(in crate::service) async fn preview_hedera_send(
        &self,
        chain: Chain,
        address: &str,
    ) -> Result<SendPreview, SpectraBridgeError> {
        let account = HederaAccountId::parse(chain, address)?;
        let eps = self
            .hedera_mirrors(
                chain,
                &[
                    EndpointCapability::Fee,
                    EndpointCapability::Balance,
                    EndpointCapability::Verification,
                ],
            )
            .await;
        let (fee, balance) = crate::api::http::race(&eps, |endpoint| async move {
            let client = HederaMirrorClient::new(Arc::new(vec![endpoint]));
            client.verify_network(chain).await?;
            Ok::<_, SpectraBridgeError>((
                fee_bound(&client).await?,
                client.fetch_balance(account).await?,
            ))
        })
        .await?;
        let units = |amount: u128| {
            crate::amount::decimal::from_units(amount, u32::from(chain.native_decimals()))
        };
        Ok(SendPreview {
            estimated_network_fee: units(fee),
            spendable_balance: units(balance),
            max_sendable: units(balance.saturating_sub(fee)),
            fee_rate_description: Some(
                "The most the fee can be, at the network's exchange rate; it charges what the transfer costs".into(),
            ),
            ..Default::default()
        })
    }
}

#[cfg(test)]
#[path = "../tests/send_stage_hedera.rs"]
mod tests;
