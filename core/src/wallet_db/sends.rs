use super::*;
use crate::send::stages::StoredSend;
use crate::wallet_db::error::DbError;
use rusqlite::OptionalExtension;

pub(crate) fn send_load(database: &WalletDatabase, id: &str) -> Result<StoredSend, DbError> {
    with_conn(database, |conn| {
        let payload: String = conn
            .query_row(
                "SELECT payload FROM send_artifacts WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(|e| DbError::Invalid(format!("Transaction artifact not found: {e}")))?;
        let stored: StoredSend = serde_json::from_str(&payload).map_err(DbError::from)?;
        stored
            .validate()
            .map_err(|e| DbError::Corrupt(e.to_string()))?;
        Ok(stored)
    })
}

/// Compare-and-swap protects against another process signing the same artifact.
/// Resource reservations and the signed bytes commit together, before broadcast.
pub(crate) fn send_save(
    database: &WalletDatabase,
    stored: &StoredSend,
    resources: &[String],
) -> Result<(), DbError> {
    stored
        .validate()
        .map_err(|e| DbError::Invalid(e.to_string()))?;
    with_conn(database, |conn| {
        let tx = conn.unchecked_transaction().map_err(DbError::from)?;
        let mut stored = stored.clone();
        let previous: Option<String> = tx
            .query_row(
                "SELECT payload FROM send_artifacts WHERE id=?1",
                [&stored.view.id],
                |r| r.get(0),
            )
            .optional()
            .map_err(DbError::from)?;
        if let Some(previous) = previous {
            let previous: StoredSend = serde_json::from_str(&previous).map_err(DbError::from)?;
            merge_receipts(
                &mut stored.icp_staking_receipts,
                &previous.icp_staking_receipts,
            )?;
        }
        stored
            .validate()
            .map_err(|e| DbError::Invalid(e.to_string()))?;
        let payload = serde_json::to_string(&stored).map_err(DbError::from)?;
        let changed = if stored.view.revision == 0 {
            tx.execute(
                "INSERT INTO send_artifacts(id,revision,payload) VALUES(?1,0,?2)",
                params![stored.view.id, payload],
            )
        } else {
            tx.execute(
                "UPDATE send_artifacts SET revision=?2,payload=?3 WHERE id=?1 AND revision=?4",
                params![
                    stored.view.id,
                    stored.view.revision,
                    payload,
                    stored.view.revision - 1
                ],
            )
        }
        .map_err(DbError::from)?;
        if changed != 1 {
            return Err(DbError::Invalid(
                "Transaction changed concurrently; reload it before continuing".into(),
            ));
        }
        for resource in resources {
            let prior: Option<String> = tx.query_row(
                "SELECT a.payload FROM send_reservations r JOIN send_artifacts a ON a.id=r.artifact_id WHERE r.resource=?1",
                [resource], |row| row.get(0)).optional().map_err(DbError::from)?;
            if let Some(prior) = prior {
                let previous: StoredSend = serde_json::from_str(&prior).map_err(DbError::from)?;
                previous
                    .validate()
                    .map_err(|e| DbError::Corrupt(e.to_string()))?;
                if previous.view.id != stored.view.id
                    && !permits_evm_replacement(&stored, &previous)
                {
                    return Err(DbError::Invalid("Transaction input is already reserved by another signed transaction; an EVM replacement requires an explicit nonce and higher fees".into()));
                }
                tx.execute(
                    "UPDATE send_reservations SET artifact_id=?2 WHERE resource=?1",
                    params![resource, stored.view.id],
                )
                .map_err(DbError::from)?;
            } else {
                tx.execute(
                    "INSERT INTO send_reservations(resource,artifact_id) VALUES(?1,?2)",
                    params![resource, stored.view.id],
                )
                .map_err(DbError::from)?;
            }
        }
        tx.commit().map_err(DbError::from)
    })
}

/// Execution proofs merge in their own atomic transaction without changing the
/// immutable review revision. A later ordinary artifact save also merges them,
/// so parallel endpoint responses cannot overwrite one another's checkpoints.
pub(crate) fn send_record_icp_receipt(
    database: &WalletDatabase,
    id: &str,
    receipt: crate::send::icp_staking::IcpStakingReceipt,
) -> Result<(), DbError> {
    receipt
        .validate()
        .map_err(|e| DbError::Invalid(e.to_string()))?;
    with_conn(database, |conn| {
        let tx = conn.unchecked_transaction().map_err(DbError::from)?;
        let payload: String = tx
            .query_row(
                "SELECT payload FROM send_artifacts WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(DbError::from)?;
        let mut stored: StoredSend = serde_json::from_str(&payload).map_err(DbError::from)?;
        merge_receipts(&mut stored.icp_staking_receipts, &[receipt])?;
        stored
            .validate()
            .map_err(|e| DbError::Invalid(e.to_string()))?;
        tx.execute(
            "UPDATE send_artifacts SET payload=?2 WHERE id=?1",
            params![id, serde_json::to_string(&stored).map_err(DbError::from)?],
        )
        .map_err(DbError::from)?;
        tx.commit().map_err(DbError::from)
    })
}
fn merge_receipts(
    into: &mut Vec<crate::send::icp_staking::IcpStakingReceipt>,
    incoming: &[crate::send::icp_staking::IcpStakingReceipt],
) -> Result<(), DbError> {
    for receipt in incoming {
        if let Some(existing) = into.iter().find(|r| r.request_id == receipt.request_id) {
            if existing.canister != receipt.canister
                || existing.kind != receipt.kind
                || existing.reply_hex != receipt.reply_hex
            {
                return Err(DbError::Corrupt(
                    "Conflicting certified ICP execution proofs".into(),
                ));
            }
        } else {
            into.push(receipt.clone());
        }
    }
    Ok(())
}

pub(crate) fn send_list(database: &WalletDatabase) -> Result<Vec<StoredSend>, DbError> {
    with_conn(database, |conn| {
        let mut stmt = conn
            .prepare("SELECT payload FROM send_artifacts ORDER BY rowid DESC")
            .map_err(DbError::from)?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(DbError::from)?;
        rows.map(|row| {
            let stored: StoredSend =
                serde_json::from_str(&row.map_err(DbError::from)?).map_err(DbError::from)?;
            stored
                .validate()
                .map_err(|e| DbError::Corrupt(e.to_string()))?;
            Ok(stored)
        })
        .collect()
    })
}

/// Only decode and validate signed artifacts belonging to this sender or wallet.
pub(crate) fn signed_sends_for_sender(
    database: &WalletDatabase,
    chain: crate::registry::Chain,
    sender: &str,
) -> Result<Vec<StoredSend>, DbError> {
    signed_sends_where(
        database,
        "json_extract(payload, '$.view.chain_id') = ?1 AND lower(json_extract(payload, '$.view.sender')) = lower(?2)",
        chain,
        sender,
    )
}

pub(crate) fn signed_sends_for_wallet(
    database: &WalletDatabase,
    chain: crate::registry::Chain,
    wallet: &str,
) -> Result<Vec<StoredSend>, DbError> {
    signed_sends_where(
        database,
        "json_extract(payload, '$.view.chain_id') = ?1 AND json_extract(payload, '$.view.wallet_id') = ?2",
        chain,
        wallet,
    )
}

fn signed_sends_where(
    database: &WalletDatabase,
    predicate: &str,
    chain: crate::registry::Chain,
    owner: &str,
) -> Result<Vec<StoredSend>, DbError> {
    with_conn(database, |conn| {
        let mut stmt = conn.prepare(&format!("SELECT payload FROM send_artifacts WHERE {predicate} AND json_extract(payload, '$.view.stage') = 'Signed'"))
            .map_err(DbError::from)?;
        let rows = stmt
            .query_map(params![chain, owner], |r| r.get::<_, String>(0))
            .map_err(DbError::from)?;
        rows.map(|row| {
            let stored: StoredSend =
                serde_json::from_str(&row.map_err(DbError::from)?).map_err(DbError::from)?;
            stored
                .validate()
                .map_err(|e| DbError::Corrupt(e.to_string()))?;
            Ok(stored)
        })
        .collect()
    })
}

pub(crate) fn send_exists(database: &WalletDatabase, id: &str) -> Result<bool, DbError> {
    with_conn(database, |conn| {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM send_artifacts WHERE id=?1)",
            [id],
            |r| r.get(0),
        )
        .map_err(DbError::from)
    })
}

fn permits_evm_replacement(next: &StoredSend, prior: &StoredSend) -> bool {
    use crate::send::stages::PreparedPayload;
    let (PreparedPayload::Evm(next_tx), PreparedPayload::Evm(prior_tx)) =
        (&next.prepared, &prior.prepared)
    else {
        return false;
    };
    let bumped = |new: u128, old: u128| {
        old.checked_add(old.div_ceil(10).max(1))
            .is_some_and(|minimum| new >= minimum)
    };
    next.request
        .evm_overrides
        .as_ref()
        .and_then(|o| o.nonce)
        .and_then(|n| u64::try_from(n).ok())
        == Some(next_tx.nonce)
        && next.view.wallet_id == prior.view.wallet_id
        && next.view.chain_id == prior.view.chain_id
        && next.view.sender.eq_ignore_ascii_case(&prior.view.sender)
        && next_tx.chain_id == prior_tx.chain_id
        && next_tx.nonce == prior_tx.nonce
        && bumped(next_tx.max_fee_per_gas, prior_tx.max_fee_per_gas)
        && bumped(
            next_tx.max_priority_fee_per_gas,
            prior_tx.max_priority_fee_per_gas,
        )
}

#[cfg(test)]
mod query_tests {
    use super::*;

    #[test]
    fn repaired_artifact_retains_its_reservation_while_other_artifacts_are_refused() {
        use crate::send::stages::{PreparedPayload, SendArtifact, SendArtifactReview, SendStage};
        let db = WalletDatabase::new(":memory:");
        let (key, prepared) = crate::send::icp_staking::tests::new_neuron_fixture();
        let resource = format!(
            "icp:neuron:{}:{}",
            prepared.controller_hex, prepared.subaccount_hex
        );
        let recipient = prepared.funding.as_ref().unwrap().recipient.clone();
        let mut stored = StoredSend {
            view: SendArtifact {
                id: "repair".into(),
                revision: 0,
                stage: SendStage::Prepared,
                wallet_id: "wallet".into(),
                chain_id: crate::registry::Chain::Icp,
                sender: prepared.sender.clone(),
                recipient: recipient.clone(),
                amount: "1".into(),
                asset: "ICP".into(),
                symbol: "ICP".into(),
                staking: None,
                created_at: 1.0,
                review_digest: String::new(),
                review: SendArtifactReview::default(),
                prepared_details: String::new(),
                signing_payload_hex: String::new(),
                signed_payload: None,
                transaction_hash: None,
                attempts: vec![],
                selected_endpoints: vec![],
            },
            request: crate::send::SendExecutionRequest {
                chain_id: crate::registry::Chain::Icp,
                wallet_id: "wallet".into(),
                password: None,
                to_address: recipient,
                amount_str: "1".into(),
                contract_address: None,
                token_standard: None,
                token_decimals: None,
                fee_rate_svb: None,
                fee_sat: None,
                gas_budget: None,
                fee_amount: None,
                evm_overrides: None,
                monero_priority: None,
                sign_only: false,
            },
            prepared: PreparedPayload::IcpStaking(prepared),
            submission: None,
            signed_digest: None,
            substrate_verified_through: None,
            icp_staking_receipts: vec![],
        };
        fn refresh(stored: &mut StoredSend) {
            stored.view.prepared_details = serde_json::to_string_pretty(&stored.prepared).unwrap();
            stored.view.review_digest = stored.digest().unwrap();
            stored.signed_digest = stored.submission_digest().unwrap();
        }
        fn sign(stored: &mut StoredSend, key: &crate::send::keys::Ed25519Seed) {
            let PreparedPayload::IcpStaking(prepared) = &stored.prepared else {
                panic!("ICP")
            };
            let calls = prepared.sign(key).unwrap();
            let payload = serde_json::to_string(&calls).unwrap();
            let hash = calls.last().map(|call| call.request_id.clone());
            stored.submission = Some(crate::send::payload::PreparedSubmission {
                payload: payload.clone(),
                result_field: "hash".into(),
                transaction_hash: hash.clone(),
                nonce: None,
            });
            stored.view.stage = SendStage::Signed;
            stored.view.signed_payload = Some(payload);
            stored.view.transaction_hash = hash;
            refresh(stored);
        }
        refresh(&mut stored);
        send_save(&db, &stored, &[]).unwrap();
        stored.view.revision = 1;
        sign(&mut stored, &key);
        send_save(&db, &stored, std::slice::from_ref(&resource)).unwrap();
        let first: Vec<crate::send::icp_staking::SignedIcpStakingCall> =
            serde_json::from_str(&stored.submission.as_ref().unwrap().payload).unwrap();
        let PreparedPayload::IcpStaking(prepared) = &mut stored.prepared else {
            panic!("ICP")
        };
        prepared.completed_calls.push(first[0].clone());
        prepared.prior_calls = first;
        prepared.calls.remove(0);
        prepared.ingress_expiry_ns += 1;
        prepared.fee = 0;
        prepared.funding_confirmed_by_ledger = true;
        stored.view.revision = 2;
        stored.view.stage = SendStage::Prepared;
        stored.view.signed_payload = None;
        stored.view.transaction_hash = None;
        stored.submission = None;
        refresh(&mut stored);
        send_save(&db, &stored, &[]).unwrap();
        stored = send_load(&db, "repair").unwrap();
        stored.view.revision = 3;
        sign(&mut stored, &key);
        send_save(&db, &stored, std::slice::from_ref(&resource)).unwrap();
        assert_eq!(send_load(&db, "repair").unwrap().view.revision, 3);
        let mut conflicting = stored.clone();
        conflicting.view.id = "another-artifact".into();
        conflicting.view.revision = 0;
        refresh(&mut conflicting);
        assert!(send_save(&db, &conflicting, &[resource]).is_err());
        assert!(!send_exists(&db, "another-artifact").unwrap());
    }

    #[test]
    fn signed_queries_skip_unrelated_artifacts_but_refuse_invalid_selected_artifacts() {
        let db = WalletDatabase::new(":memory:");
        with_conn(&db, |conn| {
            for (id, chain, sender, wallet, stage) in [
                ("other-chain", "base", "0xabc", "w", "Signed"),
                ("other-owner", "ethereum", "0xdef", "other", "Signed"),
                ("unsigned", "ethereum", "0xabc", "w", "Prepared"),
            ] {
                // Valid JSON but deliberately not a valid StoredSend: irrelevant
                // artifacts must not be deserialized or validated by a scoped read.
                let payload = serde_json::json!({"view": {
                    "chain_id": chain, "sender": sender, "wallet_id": wallet, "stage": stage,
                }}).to_string();
                conn.execute("INSERT INTO send_artifacts VALUES (?1, 0, ?2)", params![id, payload]).unwrap();
            }
            for (predicate, index) in [
                ("json_extract(payload, '$.view.chain_id') = 'ethereum' AND lower(json_extract(payload, '$.view.sender')) = '0xabc'", "idx_send_sender"),
                ("json_extract(payload, '$.view.wallet_id') = 'w' AND json_extract(payload, '$.view.chain_id') = 'ethereum'", "idx_send_wallet"),
            ] {
                let plan: Vec<String> = conn.prepare(&format!("EXPLAIN QUERY PLAN SELECT payload FROM send_artifacts WHERE {predicate} AND json_extract(payload, '$.view.stage') = 'Signed'"))
                    .unwrap().query_map([], |r| r.get(3)).unwrap().map(Result::unwrap).collect();
                assert!(plan.iter().any(|line| line.contains(index)), "{plan:?}");
            }
            Ok::<_, DbError>(())
        }).unwrap();
        assert!(
            signed_sends_for_sender(&db, crate::registry::Chain::Ethereum, "0xABC")
                .unwrap()
                .is_empty()
        );
        assert!(
            signed_sends_for_wallet(&db, crate::registry::Chain::Ethereum, "w")
                .unwrap()
                .is_empty()
        );
        with_conn(&db, |conn| {
            conn.execute("UPDATE send_artifacts SET payload = json_set(payload, '$.view.stage', 'Signed') WHERE id = 'unsigned'", []).unwrap();
            Ok::<_, DbError>(())
        }).unwrap();
        assert!(signed_sends_for_sender(&db, crate::registry::Chain::Ethereum, "0xABC").is_err());
        assert!(signed_sends_for_wallet(&db, crate::registry::Chain::Ethereum, "w").is_err());
    }
}
