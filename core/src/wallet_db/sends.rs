use super::*;
use crate::send::stages::StoredSend;
use rusqlite::OptionalExtension;

pub(crate) fn send_load(database: &WalletDatabase, id: &str) -> Result<StoredSend, String> {
    with_conn(database, |conn| {
        let payload: String = conn
            .query_row(
                "SELECT payload FROM send_artifacts WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(|e| format!("Transaction artifact not found: {e}"))?;
        let stored: StoredSend = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
        stored.validate()?;
        Ok(stored)
    })
}

/// Compare-and-swap protects against another process signing the same artifact.
/// Resource reservations and the signed bytes commit together, before broadcast.
pub(crate) fn send_save(
    database: &WalletDatabase,
    stored: &StoredSend,
    resources: &[String],
) -> Result<(), String> {
    stored.validate()?;
    let payload = serde_json::to_string(stored).map_err(|e| e.to_string())?;
    with_conn(database, |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
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
        .map_err(|e| e.to_string())?;
        if changed != 1 {
            return Err("Transaction changed concurrently; reload it before continuing".into());
        }
        for resource in resources {
            let prior: Option<String> = tx.query_row(
                "SELECT a.payload FROM send_reservations r JOIN send_artifacts a ON a.id=r.artifact_id WHERE r.resource=?1",
                [resource], |row| row.get(0)).optional().map_err(|e| e.to_string())?;
            if let Some(prior) = prior {
                let previous: StoredSend =
                    serde_json::from_str(&prior).map_err(|e| e.to_string())?;
                previous.validate()?;
                if !permits_evm_replacement(stored, &previous) {
                    return Err("Transaction input is already reserved by another signed transaction; an EVM replacement requires an explicit nonce and higher fees".into());
                }
                tx.execute(
                    "UPDATE send_reservations SET artifact_id=?2 WHERE resource=?1",
                    params![resource, stored.view.id],
                )
                .map_err(|e| e.to_string())?;
            } else {
                tx.execute(
                    "INSERT INTO send_reservations(resource,artifact_id) VALUES(?1,?2)",
                    params![resource, stored.view.id],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())
    })
}

pub(crate) fn send_list(database: &WalletDatabase) -> Result<Vec<StoredSend>, String> {
    with_conn(database, |conn| {
        let mut stmt = conn
            .prepare("SELECT payload FROM send_artifacts ORDER BY rowid DESC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|row| {
            let stored: StoredSend = serde_json::from_str(&row.map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            stored.validate()?;
            Ok(stored)
        })
        .collect()
    })
}

/// Only decode and validate signed artifacts belonging to this sender or wallet.
pub(crate) fn signed_sends_for_sender(
    database: &WalletDatabase,
    chain: &str,
    sender: &str,
) -> Result<Vec<StoredSend>, String> {
    signed_sends_where(
        database,
        "json_extract(payload, '$.view.chain_id') = ?1 AND lower(json_extract(payload, '$.view.sender')) = lower(?2)",
        chain,
        sender,
    )
}

pub(crate) fn signed_sends_for_wallet(
    database: &WalletDatabase,
    chain: &str,
    wallet: &str,
) -> Result<Vec<StoredSend>, String> {
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
    chain: &str,
    owner: &str,
) -> Result<Vec<StoredSend>, String> {
    with_conn(database, |conn| {
        let mut stmt = conn.prepare(&format!("SELECT payload FROM send_artifacts WHERE {predicate} AND json_extract(payload, '$.view.stage') = 'Signed'"))
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![chain, owner], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|row| {
            let stored: StoredSend = serde_json::from_str(&row.map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            stored.validate()?;
            Ok(stored)
        })
        .collect()
    })
}

pub(crate) fn send_exists(database: &WalletDatabase, id: &str) -> Result<bool, String> {
    with_conn(database, |conn| {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM send_artifacts WHERE id=?1)",
            [id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())
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
            Ok(())
        }).unwrap();
        assert!(
            signed_sends_for_sender(&db, "ethereum", "0xABC")
                .unwrap()
                .is_empty()
        );
        assert!(
            signed_sends_for_wallet(&db, "ethereum", "w")
                .unwrap()
                .is_empty()
        );
        with_conn(&db, |conn| {
            conn.execute("UPDATE send_artifacts SET payload = json_set(payload, '$.view.stage', 'Signed') WHERE id = 'unsigned'", []).unwrap();
            Ok(())
        }).unwrap();
        assert!(signed_sends_for_sender(&db, "ethereum", "0xABC").is_err());
        assert!(signed_sends_for_wallet(&db, "ethereum", "w").is_err());
    }
}
