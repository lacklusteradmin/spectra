//! In-memory transaction-history cache with TTL expiry.
//!
//! Same shape as `balance_cache` — keyed by `"{chain_id}:{address}"`, values
//! are JSON strings (the array that `WalletService::fetch_history` returns).
//! Default TTL is longer than balance (5 minutes) because history is append-only
//! and stale entries are far less harmful than stale balances.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

struct Entry {
    json: String,
    inserted_at: Instant,
}

pub struct HistoryCache {
    inner: RwLock<HashMap<(u32, String), Entry>>,
    ttl: Duration,
}

impl HistoryCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    pub fn get(&self, chain_id: u32, address: &str) -> Option<String> {
        let map = self.inner.read().ok()?;
        let entry = map.get(&(chain_id, address.to_string()))?;
        if entry.inserted_at.elapsed() < self.ttl {
            Some(entry.json.clone())
        } else {
            None
        }
    }

    pub fn set(&self, chain_id: u32, address: &str, json: String) {
        if let Ok(mut map) = self.inner.write() {
            map.insert(
                (chain_id, address.to_string()),
                Entry { json, inserted_at: Instant::now() },
            );
        }
    }

    pub fn invalidate(&self, chain_id: u32, address: &str) {
        if let Ok(mut map) = self.inner.write() {
            map.remove(&(chain_id, address.to_string()));
        }
    }

    pub fn evict_expired(&self) {
        if let Ok(mut map) = self.inner.write() {
            let ttl = self.ttl;
            map.retain(|_, entry| entry.inserted_at.elapsed() < ttl);
        }
    }
}
