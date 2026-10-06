// Pure state machine for send-broadcast verification notices: given the last
// sent transaction and its verification status, decides the notice text and
// whether it should be shown as a warning.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub enum SendVerificationStatus {
    Deferred,
    Failed { message: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendVerificationNotice {
    /// None means "clear any existing notice". Some(text) means display text.
    pub notice: Option<String>,
    pub is_warning: bool,
}

/// The notice for a send the network has not yet shown as settled. What a
/// front end shows comes from the stored record, through
/// [`verification_notice_for_last_sent`].
fn verification_notice_for_status(
    status: SendVerificationStatus,
    chain: crate::registry::Chain,
) -> SendVerificationNotice {
    match status {
        SendVerificationStatus::Deferred => SendVerificationNotice {
            notice: Some(format!(
                "Broadcast succeeded, but {} network verification is still catching up. Status will update shortly.",
                chain.chain_display_name()
            )),
            is_warning: false,
        },
        SendVerificationStatus::Failed { message } => SendVerificationNotice {
            notice: Some(format!(
                "Warning: Broadcast succeeded, but post-broadcast verification reported: {}",
                message
            )),
            is_warning: true,
        },
    }
}

/// The parts of a stored send record the notice reads.
#[derive(Debug, Clone)]
pub struct LastSentTransactionSnapshot {
    pub kind: crate::store::wallet_domain::TransactionKind,
    pub status: crate::store::wallet_domain::TransactionStatus,
    pub chain_id: crate::registry::Chain,
    pub transaction_hash: Option<String>,
    pub failure_reason: Option<crate::store::persistence_models::TransactionFailure>,
    pub transaction_history_source: Option<String>,
    pub receipt_block_number: Option<i64>,
    pub confirmation_count: Option<i64>,
}

impl From<&crate::store::persistence_models::TransactionRecord> for LastSentTransactionSnapshot {
    fn from(record: &crate::store::persistence_models::TransactionRecord) -> Self {
        Self {
            kind: record.kind,
            status: record.status,
            chain_id: record.chain_id,
            transaction_hash: record.transaction_hash.clone(),
            failure_reason: record.failure_reason.clone(),
            transaction_history_source: record.transaction_history_source.clone(),
            receipt_block_number: record.receipt_block_number,
            confirmation_count: record.confirmation_count,
        }
    }
}

/// Decides what to tell the user about their most recent send: whether the
/// broadcast has been seen by an indexer, is still unconfirmed, or failed.
/// Returns the default (no notice) for anything that isn't a hashed send.
///
/// Not exported: `WalletService::send_verification_notice` reads the stored
/// record.
pub fn verification_notice_for_last_sent(
    snapshot: Option<LastSentTransactionSnapshot>,
) -> SendVerificationNotice {
    use crate::store::wallet_domain::TransactionStatus;
    let Some(tx) = snapshot else {
        return SendVerificationNotice::default();
    };
    if !tx.kind.is_submitted() {
        return SendVerificationNotice::default();
    }
    let hash_trimmed = tx
        .transaction_hash
        .as_deref()
        .map(|h| h.trim())
        .unwrap_or("");
    if hash_trimmed.is_empty() {
        return SendVerificationNotice::default();
    }
    if tx.status == TransactionStatus::Failed {
        let message = tx
            .failure_reason
            .as_ref()
            .map(|failure| failure.english())
            .unwrap_or_else(|| "Broadcast was not confirmed by the network.".to_string());
        return verification_notice_for_status(
            SendVerificationStatus::Failed { message },
            tx.chain_id,
        );
    }
    let observed_on_network = tx.status == TransactionStatus::Confirmed
        || tx.transaction_history_source.is_some()
        || tx.receipt_block_number.is_some()
        || tx.confirmation_count.unwrap_or(0) > 0;
    if observed_on_network {
        return SendVerificationNotice::default();
    }
    verification_notice_for_status(SendVerificationStatus::Deferred, tx.chain_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::wallet_domain::{TransactionKind, TransactionStatus};

    fn snapshot() -> LastSentTransactionSnapshot {
        LastSentTransactionSnapshot {
            kind: TransactionKind::Send,
            status: TransactionStatus::Pending,
            chain_id: crate::registry::Chain::Bitcoin,
            transaction_hash: None,
            failure_reason: None,
            transaction_history_source: None,
            receipt_block_number: None,
            confirmation_count: None,
        }
    }

    #[test]
    fn deferred_mentions_chain_name() {
        let n = verification_notice_for_status(
            SendVerificationStatus::Deferred,
            crate::registry::Chain::Bitcoin,
        );
        assert!(n.notice.unwrap().contains("Bitcoin"));
        assert!(!n.is_warning);
    }

    #[test]
    fn failed_includes_message_and_warning_flag() {
        let n = verification_notice_for_status(
            SendVerificationStatus::Failed {
                message: "node down".into(),
            },
            crate::registry::Chain::Tron,
        );
        let text = n.notice.unwrap();
        assert!(text.contains("node down"));
        assert!(text.starts_with("Warning:"));
        assert!(n.is_warning);
    }

    #[test]
    fn last_sent_missing_returns_clear() {
        assert!(verification_notice_for_last_sent(None).notice.is_none());
    }

    #[test]
    fn last_sent_empty_hash_returns_clear() {
        let n = verification_notice_for_last_sent(Some(LastSentTransactionSnapshot {
            kind: TransactionKind::Send,
            status: TransactionStatus::Pending,
            chain_id: crate::registry::Chain::Ethereum,
            transaction_hash: Some("   ".into()),
            ..snapshot()
        }));
        assert!(n.notice.is_none());
    }

    #[test]
    fn last_sent_confirmed_returns_clear() {
        let n = verification_notice_for_last_sent(Some(LastSentTransactionSnapshot {
            kind: TransactionKind::Send,
            status: TransactionStatus::Confirmed,
            chain_id: crate::registry::Chain::Ethereum,
            transaction_hash: Some("0xabc".into()),
            ..snapshot()
        }));
        assert!(n.notice.is_none());
    }

    #[test]
    fn last_sent_failed_uses_fallback_reason() {
        let n = verification_notice_for_last_sent(Some(LastSentTransactionSnapshot {
            kind: TransactionKind::Send,
            status: TransactionStatus::Failed,
            chain_id: crate::registry::Chain::Ethereum,
            transaction_hash: Some("0xabc".into()),
            failure_reason: None,
            ..snapshot()
        }));
        assert!(n.is_warning);
        assert!(n.notice.unwrap().contains("Broadcast was not confirmed"));
    }

    #[test]
    fn last_sent_pending_unobserved_returns_deferred() {
        let n = verification_notice_for_last_sent(Some(LastSentTransactionSnapshot {
            kind: TransactionKind::Send,
            status: TransactionStatus::Pending,
            chain_id: crate::registry::Chain::Solana,
            transaction_hash: Some("0xabc".into()),
            ..snapshot()
        }));
        assert!(n.notice.unwrap().contains("Solana"));
        assert!(!n.is_warning);
    }

    #[test]
    fn last_sent_dogecoin_confirmed_via_counter_returns_clear() {
        let n = verification_notice_for_last_sent(Some(LastSentTransactionSnapshot {
            kind: TransactionKind::Send,
            status: TransactionStatus::Pending,
            chain_id: crate::registry::Chain::Dogecoin,
            transaction_hash: Some("abc".into()),
            confirmation_count: Some(1),
            ..snapshot()
        }));
        assert!(n.notice.is_none());
    }
}
