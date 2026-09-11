use super::*;
use std::sync::{mpsc, Arc};
use std::time::Duration;

fn path() -> String {
    std::env::temp_dir()
        .join(format!(
            "spectra-connection-{}.sqlite",
            crate::store::new_event_id()
        ))
        .to_string_lossy()
        .into_owned()
}

#[test]
fn one_blocked_database_does_not_block_another_database() {
    let a = WalletDatabase::acquire(&path());
    let b = WalletDatabase::acquire(&path());
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        a.with_connection(|_| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        })
    });
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    let second = std::thread::spawn(move || {
        let result = b.with_connection(|conn| {
            conn.query_row("SELECT 1", [], |row| row.get::<_, i32>(0))
                .map_err(|e| e.to_string())
        });
        done_tx.send(result).unwrap();
    });
    let result = done_rx.recv_timeout(Duration::from_secs(5));
    release_tx.send(()).unwrap();
    worker.join().unwrap().unwrap();
    second.join().unwrap();
    assert_eq!(result.unwrap().unwrap(), 1);
}

#[test]
fn same_database_shares_a_handle_and_last_owner_releases_connection() {
    let path = path();
    let first = WalletDatabase::acquire(&path);
    let second = WalletDatabase::acquire(&path);
    assert!(Arc::ptr_eq(&first, &second));
    first
        .with_connection(|conn| {
            conn.execute_batch("CREATE TEMP TABLE lifetime_marker (id INTEGER)")
                .map_err(|e| e.to_string())
        })
        .unwrap();
    let weak = Arc::downgrade(&first);
    drop(first);
    assert!(weak.upgrade().is_some());
    drop(second);
    assert!(weak.upgrade().is_none());
    let reopened = WalletDatabase::acquire(&path);
    reopened
        .with_connection(|conn| {
            let count: i32 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_temp_master WHERE name = 'lifetime_marker'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
            Ok(())
        })
        .unwrap();
}

#[tokio::test]
async fn service_holds_connection_until_rebind_or_drop() {
    let service = crate::service::WalletService::new_typed(vec![]).unwrap();
    let first_path = path();
    service.open_state(first_path.clone()).await.unwrap();
    let weak = Arc::downgrade(&WalletDatabase::acquire(&first_path));
    assert!(weak.upgrade().is_some());
    service.open_state(path()).await.unwrap();
    assert!(weak.upgrade().is_none());
    let active = service
        .state_database
        .read()
        .await
        .as_ref()
        .map(Arc::downgrade)
        .unwrap();
    drop(service);
    assert!(active.upgrade().is_none());
}
