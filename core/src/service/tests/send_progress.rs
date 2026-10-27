//! What a staged transaction allows next, from its stage, its submissions
//! and its history record: every combination the stages can reach.
use super::*;
use crate::service::TransactionActions;
use crate::staking::{StakingAction, StakingRequest};
use crate::store::persistence_models::TransactionRecord;
use serde_json::json;

fn artifact(stage: SendStage, outcomes: &[SubmissionOutcome]) -> SendArtifact {
    SendArtifact {
        id: "send".into(),
        revision: 0,
        stage,
        wallet_id: "wallet".into(),
        chain_id: Chain::Ethereum,
        sender: "sender".into(),
        recipient: "recipient".into(),
        memo: None,
        amount: "1".into(),
        asset: "ETH".into(),
        symbol: "ETH".into(),
        staking: None,
        operation: None,
        created_at: 1.0,
        review_digest: "digest".into(),
        review: SendArtifactReview::default(),
        prepared_details: String::new(),
        signing_payload_hex: String::new(),
        signed_payload: None,
        transaction_hash: None,
        attempts: outcomes
            .iter()
            .map(|outcome| BroadcastAttempt {
                endpoint: "https://node".into(),
                attempted_at: 1.0,
                outcome: outcome.clone(),
                transaction_hash: None,
                detail: String::new(),
            })
            .collect(),
        selected_endpoints: Vec::new(),
    }
}

fn record(id: &str, status: &str, rebroadcast: bool) -> TransactionRecord {
    let mut record: TransactionRecord = serde_json::from_value(json!({
        "id": id, "walletId": "wallet", "walletName": "Main", "kind": "send",
        "chainId": "ethereum", "symbol": "ETH", "assetDisplayName": "Ether",
        "status": status, "amount": "1", "address": "recipient", "createdAtUnix": 1.0,
    }))
    .unwrap();
    record.actions = TransactionActions {
        recheck_unavailable_reason: None,
        rebroadcast_unavailable_reason: (!rebroadcast).then(|| "not here".into()),
    };
    record
}

fn next(artifact: SendArtifact, record: Option<TransactionRecord>) -> SendNextStep {
    SendProgress::of(artifact, record).next
}

#[test]
fn each_stage_has_one_next_step() {
    use SendNextStep::*;
    use SubmissionOutcome::*;
    assert_eq!(next(artifact(SendStage::Prepared, &[]), None), Sign);
    assert_eq!(next(artifact(SendStage::Signed, &[]), None), Broadcast);
    let pending = || Some(record("send", "pending", true));
    assert_eq!(
        next(artifact(SendStage::Signed, &[Rejected]), pending()),
        Rebroadcast
    );
    assert_eq!(
        next(artifact(SendStage::Signed, &[Uncertain]), pending()),
        Rebroadcast
    );
    // One node's acceptance ends the stages, whatever the others said.
    assert_eq!(
        next(
            artifact(SendStage::Signed, &[Rejected, Accepted]),
            pending()
        ),
        Done
    );
    // A settled record ends them too, even with no acceptance on file.
    for status in ["confirmed", "failed"] {
        assert_eq!(
            next(
                artifact(SendStage::Signed, &[Uncertain]),
                Some(record("send", status, true))
            ),
            Done
        );
    }
}

/// A record speaks only for a submitted artifact: a repaired staking action
/// keeps its id, but is rebuilt and unsubmitted, whatever its record says.
#[test]
fn a_record_does_not_speak_for_an_unsubmitted_rebuild() {
    for (stage, next) in [
        (SendStage::Prepared, SendNextStep::Sign),
        (SendStage::Signed, SendNextStep::Broadcast),
    ] {
        let progress = SendProgress::of(artifact(stage, &[]), Some(record("send", "failed", true)));
        assert!(progress.transaction.is_none());
        assert_eq!(progress.next, next);
        assert!(!progress.can_rebroadcast);
    }
}

/// Another transaction's record says nothing about this one.
#[test]
fn only_the_artifacts_own_record_counts() {
    let progress = SendProgress::of(
        artifact(SendStage::Signed, &[SubmissionOutcome::Rejected]),
        Some(record("other", "confirmed", true)),
    );
    assert!(progress.transaction.is_none());
    assert_eq!(progress.next, SendNextStep::Rebroadcast);
    assert!(!progress.can_rebroadcast);
}

#[test]
fn a_rebroadcast_needs_a_submission_and_its_records_consent() {
    use SubmissionOutcome::*;
    let can = |stage, outcomes: &[SubmissionOutcome], record| {
        SendProgress::of(artifact(stage, outcomes), record).can_rebroadcast
    };
    assert!(can(
        SendStage::Signed,
        &[Rejected],
        Some(record("send", "pending", true))
    ));
    assert!(can(
        SendStage::Signed,
        &[Accepted],
        Some(record("send", "pending", true))
    ));
    assert!(!can(
        SendStage::Signed,
        &[Rejected],
        Some(record("send", "pending", false))
    ));
    assert!(!can(SendStage::Signed, &[Rejected], None));
    assert!(!can(
        SendStage::Signed,
        &[],
        Some(record("send", "pending", true))
    ));
    assert!(!can(
        SendStage::Prepared,
        &[Rejected],
        Some(record("send", "pending", true))
    ));
}

fn staking(chain: Chain, action: StakingAction, outcomes: &[SubmissionOutcome]) -> SendArtifact {
    SendArtifact {
        chain_id: chain,
        staking: Some(StakingRequest {
            wallet_id: "wallet".into(),
            chain_id: chain,
            action,
            validator_id: None,
            position_id: None,
            amount: None,
            lockup_seconds: None,
        }),
        ..artifact(SendStage::Signed, outcomes)
    }
}

/// Only a submitted new ICP neuron that has not confirmed is rebuilt.
#[test]
fn repair_is_for_an_interrupted_icp_stake() {
    use SubmissionOutcome::*;
    let can = |artifact, record| SendProgress::of(artifact, record).can_repair;
    let pending = || Some(record("send", "pending", false));
    assert!(can(
        staking(Chain::Icp, StakingAction::Stake, &[Uncertain]),
        pending()
    ));
    assert!(can(
        staking(Chain::Icp, StakingAction::Stake, &[Accepted]),
        None
    ));
    assert!(can(
        staking(Chain::Icp, StakingAction::Stake, &[Accepted]),
        Some(record("send", "failed", false))
    ));
    assert!(!can(
        staking(Chain::Icp, StakingAction::Stake, &[Accepted]),
        Some(record("send", "confirmed", false))
    ));
    assert!(!can(staking(Chain::Icp, StakingAction::Stake, &[]), None));
    assert!(!can(
        staking(Chain::Icp, StakingAction::Withdraw, &[Accepted]),
        pending()
    ));
    assert!(!can(
        staking(Chain::Solana, StakingAction::Stake, &[Accepted]),
        pending()
    ));
    assert!(!can(artifact(SendStage::Signed, &[Accepted]), pending()));
}

/// A submitted staking action's status is read again until it settles.
#[test]
fn recheck_is_for_an_unsettled_staking_submission() {
    use SubmissionOutcome::*;
    let can = |artifact, record| SendProgress::of(artifact, record).can_recheck;
    let stake =
        |outcomes: &[SubmissionOutcome]| staking(Chain::Solana, StakingAction::Stake, outcomes);
    assert!(can(stake(&[Accepted]), None));
    assert!(can(
        stake(&[Accepted]),
        Some(record("send", "pending", false))
    ));
    assert!(!can(
        stake(&[Accepted]),
        Some(record("send", "confirmed", false))
    ));
    assert!(!can(
        stake(&[Accepted]),
        Some(record("send", "failed", false))
    ));
    assert!(!can(stake(&[]), None));
    assert!(!can(artifact(SendStage::Signed, &[Accepted]), None));
}

/// What the asset's rules do is said in the review, in the terms' order,
/// after the transfer's own warnings.
#[test]
fn transfer_terms_join_the_review_warnings() {
    let mut review = SendArtifactReview::of_transfer(
        vec![crate::send::flow::HighRiskSendWarning::NewAddress],
        Vec::new(),
        false,
    );
    review.adopt_transfer_terms(Some(AssetTransferTerms {
        debited: "10".into(),
        received: "9.9".into(),
        fee: "0.1".into(),
        hook_program: Some("Hook111".into()),
        carried_native: Some("1.2".into()),
        recipient_registration: Some("0.00125".into()),
    }));
    assert_eq!(
        review.warnings,
        vec![
            ReviewWarning::Risk {
                warning: crate::send::flow::HighRiskSendWarning::NewAddress
            },
            ReviewWarning::TokenFee {
                fee: "0.1".into(),
                received: "9.9".into()
            },
            ReviewWarning::CarriesNative {
                amount: "1.2".into()
            },
            ReviewWarning::RegistersRecipient {
                deposit: "0.00125".into()
            },
            ReviewWarning::TransferHook {
                program: "Hook111".into()
            },
        ]
    );
    // A token that takes nothing says nothing of a fee.
    let mut free = SendArtifactReview::of_transfer(Vec::new(), Vec::new(), true);
    free.adopt_transfer_terms(Some(AssetTransferTerms {
        debited: "1".into(),
        received: "1".into(),
        fee: "0".into(),
        hook_program: None,
        carried_native: None,
        recipient_registration: None,
    }));
    assert_eq!(free.warnings, vec![ReviewWarning::OwnAddress]);
    assert!(free.transfer_terms.is_some());
}
