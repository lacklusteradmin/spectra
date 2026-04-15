// Pure state machine for send broadcast verification notices.
//
// Swift previously held 4 mutating functions to compute notice text + warning
// flag based on `CoreSendVerificationStatus` and the last sent transaction.
// This module centralizes the logic so all platforms derive the same notice.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum)]
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

#[uniffi::export]
pub fn verification_notice_for_status(
    status: CoreSendVerificationStatus,
    chain_name: String,
) -> SendVerificationNotice {
    match status {
        CoreSendVerificationStatus::Verified => SendVerificationNotice::default(),
        CoreSendVerificationStatus::Deferred => SendVerificationNotice {
            notice: Some(format!(
                "Broadcast succeeded, but {} network verification is still catching up. Status will update shortly.",
                chain_name
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct LastSentTransactionSnapshot {
    /// "send" or other kind strings.
    pub kind: String,
    /// "pending" | "confirmed" | "failed".
    pub status: String,
    pub chain_name: String,
    pub transaction_hash: Option<String>,
    pub failure_reason: Option<String>,
    pub transaction_history_source: Option<String>,
    pub receipt_block_number: Option<i64>,
    pub dogecoin_confirmations: Option<i64>,
}

/// Mirrors Swift's `updateSendVerificationNoticeForLastSentTransaction()`.
#[uniffi::export]
pub fn verification_notice_for_last_sent(
    snapshot: Option<LastSentTransactionSnapshot>,
) -> SendVerificationNotice {
    let Some(tx) = snapshot else {
        return SendVerificationNotice::default();
    };
    if tx.kind != "send" {
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
    if tx.status == "failed" {
        let message = tx
            .failure_reason
            .clone()
            .unwrap_or_else(|| "Broadcast was not confirmed by the network.".to_string());
        return verification_notice_for_status(
            CoreSendVerificationStatus::Failed { message },
            tx.chain_name.clone(),
        );
    }
    let observed_on_network = tx.status == "confirmed"
        || tx.transaction_history_source.is_some()
        || tx.receipt_block_number.is_some()
        || tx.dogecoin_confirmations.unwrap_or(0) > 0;
    if observed_on_network {
        return SendVerificationNotice::default();
    }
    verification_notice_for_status(
        CoreSendVerificationStatus::Deferred,
        tx.chain_name,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_clears_notice() {
        let n = verification_notice_for_status(
            CoreSendVerificationStatus::Verified,
            "Ethereum".into(),
        );
        assert!(n.notice.is_none());
        assert!(!n.is_warning);
    }

    #[test]
    fn deferred_mentions_chain_name() {
        let n = verification_notice_for_status(
            CoreSendVerificationStatus::Deferred,
            "Bitcoin".into(),
        );
        assert!(n.notice.unwrap().contains("Bitcoin"));
        assert!(!n.is_warning);
    }

    #[test]
    fn failed_includes_message_and_warning_flag() {
        let n = verification_notice_for_status(
            CoreSendVerificationStatus::Failed { message: "node down".into() },
            "Tron".into(),
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
            kind: "send".into(),
            status: "pending".into(),
            chain_name: "Ethereum".into(),
            transaction_hash: Some("   ".into()),
            ..Default::default()
        }));
        assert!(n.notice.is_none());
    }

    #[test]
    fn last_sent_confirmed_returns_clear() {
        let n = verification_notice_for_last_sent(Some(LastSentTransactionSnapshot {
            kind: "send".into(),
            status: "confirmed".into(),
            chain_name: "Ethereum".into(),
            transaction_hash: Some("0xabc".into()),
            ..Default::default()
        }));
        assert!(n.notice.is_none());
    }

    #[test]
    fn last_sent_failed_uses_fallback_reason() {
        let n = verification_notice_for_last_sent(Some(LastSentTransactionSnapshot {
            kind: "send".into(),
            status: "failed".into(),
            chain_name: "Ethereum".into(),
            transaction_hash: Some("0xabc".into()),
            failure_reason: None,
            ..Default::default()
        }));
        assert!(n.is_warning);
        assert!(n.notice.unwrap().contains("Broadcast was not confirmed"));
    }

    #[test]
    fn last_sent_pending_unobserved_returns_deferred() {
        let n = verification_notice_for_last_sent(Some(LastSentTransactionSnapshot {
            kind: "send".into(),
            status: "pending".into(),
            chain_name: "Solana".into(),
            transaction_hash: Some("0xabc".into()),
            ..Default::default()
        }));
        assert!(n.notice.unwrap().contains("Solana"));
        assert!(!n.is_warning);
    }

    #[test]
    fn last_sent_dogecoin_confirmed_via_counter_returns_clear() {
        let n = verification_notice_for_last_sent(Some(LastSentTransactionSnapshot {
            kind: "send".into(),
            status: "pending".into(),
            chain_name: "Dogecoin".into(),
            transaction_hash: Some("abc".into()),
            dogecoin_confirmations: Some(1),
            ..Default::default()
        }));
        assert!(n.notice.is_none());
    }
}
