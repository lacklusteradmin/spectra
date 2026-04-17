//! In-memory balance cache with TTL expiry.
//!
//! Keyed by `(chain_id, address)`. Values are JSON strings (the same shape
//! that `WalletService::fetch_balance` returns) together with the instant they
//! were stored. Reads return `None` once the TTL has elapsed so the caller
//! fetches fresh and repopulates.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

/// Cache entry: the JSON-encoded balance snapshot and when it was inserted.
struct Entry {
    json: String,
    inserted_at: Instant,
}

/// Thread-safe in-memory balance cache.
pub struct BalanceCache {
    inner: RwLock<HashMap<(u32, String), Entry>>,
    ttl: Duration,
}

impl BalanceCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Return the cached balance JSON if present and not expired.
    pub fn get(&self, chain_id: u32, address: &str) -> Option<String> {
        let map = self.inner.read().ok()?;
        let entry = map.get(&(chain_id, address.to_string()))?;
        if entry.inserted_at.elapsed() < self.ttl {
            Some(entry.json.clone())
        } else {
            None
        }
    }

    /// Insert or refresh a cache entry.
    pub fn set(&self, chain_id: u32, address: &str, json: String) {
        if let Ok(mut map) = self.inner.write() {
            map.insert(
                (chain_id, address.to_string()),
                Entry { json, inserted_at: Instant::now() },
            );
        }
    }

    /// Explicitly invalidate one entry (e.g. after a send completes).
    pub fn invalidate(&self, chain_id: u32, address: &str) {
        if let Ok(mut map) = self.inner.write() {
            map.remove(&(chain_id, address.to_string()));
        }
    }

    /// Drop all entries whose TTL has elapsed (called opportunistically).
    pub fn evict_expired(&self) {
        if let Ok(mut map) = self.inner.write() {
            let ttl = self.ttl;
            map.retain(|_, entry| entry.inserted_at.elapsed() < ttl);
        }
    }

    /// Return the number of live (non-expired) entries.
    pub fn live_count(&self) -> usize {
        let map = self.inner.read().unwrap_or_else(|p| p.into_inner());
        let ttl = self.ttl;
        map.values().filter(|e| e.inserted_at.elapsed() < ttl).count()
    }
}
