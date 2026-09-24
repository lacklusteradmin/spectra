// Pure state machine for send-broadcast verification notices: given the last
// sent transaction and its verification status, decides the notice text and
// whether it should be shown as a warning.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub enum CoreSendVerificationStatus {
    Verified,
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

/// Not exported: the app only ever asked this with `Verified`, after a send it
/// had not verified. What a front end shows comes from the stored record,
/// through [`verification_notice_for_last_sent`].
pub fn verification_notice_for_status(
    status: CoreSendVerificationStatus,
    chain_id: String,
) -> SendVerificationNotice {
    match status {
        CoreSendVerificationStatus::Verified => SendVerificationNotice::default(),
        CoreSendVerificationStatus::Deferred => SendVerificationNotice {
            notice: Some(format!(
                "Broadcast succeeded, but {} network verification is still catching up. Status will update shortly.",
                crate::registry::Chain::display_name_for_id(&chain_id)
            )),
            is_warning: false,
        },
        CoreSendVerificationStatus::Failed { message } => SendVerificationNotice {
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
    pub kind: crate::store::wallet_domain::CoreTransactionKind,
    pub status: crate::store::wallet_domain::CoreTransactionStatus,
    pub chain_id: String,
    pub transaction_hash: Option<String>,
    pub failure_reason: Option<crate::store::persistence_models::TransactionFailure>,
    pub transaction_history_source: Option<String>,
    pub receipt_block_number: Option<i64>,
    pub confirmation_count: Option<i64>,
}

impl From<&crate::store::persistence_models::CorePersistedTransactionRecord>
    for LastSentTransactionSnapshot
{
    fn from(record: &crate::store::persistence_models::CorePersistedTransactionRecord) -> Self {
        Self {
            kind: record.kind,
            status: record.status,
            chain_id: record.chain_id.clone(),
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
/// record. The app used to rebuild this snapshot from its own copy of the
/// record, with the kind and status spelled as strings, and hand it back.
pub fn verification_notice_for_last_sent(
    snapshot: Option<LastSentTransactionSnapshot>,
) -> SendVerificationNotice {
    use crate::store::wallet_domain::{CoreTransactionKind, CoreTransactionStatus};
    let Some(tx) = snapshot else {
        return SendVerificationNotice::default();
    };
    if tx.kind != CoreTransactionKind::Send {
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
    if tx.status == CoreTransactionStatus::Failed {
        let message = tx
            .failure_reason
            .as_ref()
            .map(|failure| failure.english())
            .unwrap_or_else(|| "Broadcast was not confirmed by the network.".to_string());
        return verification_notice_for_status(
            CoreSendVerificationStatus::Failed { message },
            tx.chain_id.clone(),
        );
    }
    let observed_on_network = tx.status == CoreTransactionStatus::Confirmed
        || tx.transaction_history_source.is_some()
        || tx.receipt_block_number.is_some()
        || tx.confirmation_count.unwrap_or(0) > 0;
    if observed_on_network {
        return SendVerificationNotice::default();
    }
    verification_notice_for_status(CoreSendVerificationStatus::Deferred, tx.chain_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::wallet_domain::{CoreTransactionKind, CoreTransactionStatus};

    fn snapshot() -> LastSentTransactionSnapshot {
        LastSentTransactionSnapshot {
            kind: CoreTransactionKind::Send,
            status: CoreTransactionStatus::Pending,
            chain_id: String::new(),
            transaction_hash: None,
            failure_reason: None,
            transaction_history_source: None,
            receipt_block_number: None,
            confirmation_count: None,
        }
    }

    #[test]
    fn verified_clears_notice() {
        let n =
            verification_notice_for_status(CoreSendVerificationStatus::Verified, "ethereum".into());
        assert!(n.notice.is_none());
        assert!(!n.is_warning);
    }

    #[test]
    fn deferred_mentions_chain_name() {
        let n =
            verification_notice_for_status(CoreSendVerificationStatus::Deferred, "bitcoin".into());
        assert!(n.notice.unwrap().contains("Bitcoin"));
        assert!(!n.is_warning);
    }

    #[test]
    fn failed_includes_message_and_warning_flag() {
        let n = verification_notice_for_status(
            CoreSendVerificationStatus::Failed {
                message: "node down".into(),
            },
            "tron".into(),
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
            kind: CoreTransactionKind::Send,
            status: CoreTransactionStatus::Pending,
            chain_id: "ethereum".into(),
            transaction_hash: Some("   ".into()),
            ..snapshot()
        }));
        assert!(n.notice.is_none());
    }

    #[test]
    fn last_sent_confirmed_returns_clear() {
        let n = verification_notice_for_last_sent(Some(LastSentTransactionSnapshot {
            kind: CoreTransactionKind::Send,
            status: CoreTransactionStatus::Confirmed,
            chain_id: "ethereum".into(),
            transaction_hash: Some("0xabc".into()),
            ..snapshot()
        }));
        assert!(n.notice.is_none());
    }

    #[test]
    fn last_sent_failed_uses_fallback_reason() {
        let n = verification_notice_for_last_sent(Some(LastSentTransactionSnapshot {
            kind: CoreTransactionKind::Send,
            status: CoreTransactionStatus::Failed,
            chain_id: "ethereum".into(),
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
            kind: CoreTransactionKind::Send,
            status: CoreTransactionStatus::Pending,
            chain_id: "solana".into(),
            transaction_hash: Some("0xabc".into()),
            ..snapshot()
        }));
        assert!(n.notice.unwrap().contains("Solana"));
        assert!(!n.is_warning);
    }

    #[test]
    fn last_sent_dogecoin_confirmed_via_counter_returns_clear() {
        let n = verification_notice_for_last_sent(Some(LastSentTransactionSnapshot {
            kind: CoreTransactionKind::Send,
            status: CoreTransactionStatus::Pending,
            chain_id: "dogecoin".into(),
            transaction_hash: Some("abc".into()),
            confirmation_count: Some(1),
            ..snapshot()
        }));
        assert!(n.notice.is_none());
    }
}
