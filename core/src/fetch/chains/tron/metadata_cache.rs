//! Service-owned read cache. Sending deliberately bypasses this cache.
use super::Trc20Metadata;
use std::{collections::HashMap, future::Future, sync::Arc, time::Duration};
use tokio::{
    sync::{Mutex, OnceCell},
    time::Instant,
};

#[derive(Clone, Hash, PartialEq, Eq)]
pub(super) struct Key {
    pub chain: String,
    pub endpoints: Arc<Vec<String>>,
    pub contract: String,
}
type Entry = OnceCell<Result<(Instant, Trc20Metadata), String>>;

pub(crate) struct MetadataCache {
    entries: Mutex<HashMap<Key, Arc<Entry>>>,
    ttl: Duration,
    capacity: usize,
}

impl Default for MetadataCache {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            ttl: Duration::from_secs(300),
            capacity: 256,
        }
    }
}

impl MetadataCache {
    pub(super) async fn get_or_fetch(
        &self,
        key: Key,
        fetch: impl Future<Output = Result<Trc20Metadata, String>>,
    ) -> Result<Trc20Metadata, String> {
        let entry = {
            let mut entries = self.entries.lock().await;
            if entries
                .get(&key)
                .and_then(|entry| entry.get())
                .is_some_and(|result| {
                    result
                        .as_ref()
                        .map_or(true, |(at, _)| at.elapsed() >= self.ttl)
                })
            {
                entries.remove(&key);
            }
            if let Some(entry) = entries.get(&key) {
                entry.clone()
            } else {
                if entries.len() >= self.capacity {
                    // Never evict an in-flight lookup: its callers must keep sharing it.
                    let idle = entries
                        .iter()
                        .find(|(_, entry)| Arc::strong_count(entry) == 1)
                        .map(|(key, _)| key.clone());
                    if let Some(idle) = idle {
                        entries.remove(&idle);
                    }
                }
                let entry = Arc::new(OnceCell::new());
                if entries.len() < self.capacity {
                    entries.insert(key.clone(), entry.clone());
                }
                entry
            }
        };
        let result = entry
            .get_or_init(|| async { fetch.await.map(|metadata| (Instant::now(), metadata)) })
            .await;
        if result.is_err() {
            let mut entries = self.entries.lock().await;
            if entries
                .get(&key)
                .is_some_and(|current| Arc::ptr_eq(current, &entry))
            {
                entries.remove(&key);
            }
        }
        result
            .as_ref()
            .map(|(_, metadata)| metadata.clone())
            .map_err(Clone::clone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(contract: &str) -> Key {
        Key {
            chain: "tron".into(),
            endpoints: Arc::new(vec![]),
            contract: contract.into(),
        }
    }
    fn metadata() -> Trc20Metadata {
        Trc20Metadata {
            symbol: "T".into(),
            decimals: 6,
        }
    }

    #[tokio::test]
    async fn expiry_failures_and_capacity_do_not_retain_stale_metadata() {
        let cache = MetadataCache {
            ttl: Duration::ZERO,
            capacity: 2,
            ..Default::default()
        };
        cache
            .get_or_fetch(key("a"), async { Ok(metadata()) })
            .await
            .unwrap();
        assert!(cache
            .get_or_fetch(key("a"), async { Err("provider unavailable".into()) })
            .await
            .is_err());
        cache
            .get_or_fetch(key("a"), async { Ok(metadata()) })
            .await
            .unwrap();
        for contract in ["b", "c", "d"] {
            cache
                .get_or_fetch(key(contract), async { Ok(metadata()) })
                .await
                .unwrap();
        }
        assert_eq!(cache.entries.lock().await.len(), 2);
    }

    #[tokio::test]
    async fn cancelled_initializer_does_not_strand_other_readers() {
        let cache = Arc::new(MetadataCache::default());
        let (started, ready) = tokio::sync::oneshot::channel();
        let worker_cache = cache.clone();
        let worker = tokio::spawn(async move {
            worker_cache
                .get_or_fetch(key("a"), async {
                    started.send(()).unwrap();
                    std::future::pending().await
                })
                .await
        });
        ready.await.unwrap();
        worker.abort();
        let _ = worker.await;
        let recovered = tokio::time::timeout(
            Duration::from_secs(1),
            cache.get_or_fetch(key("a"), async { Ok(metadata()) }),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(recovered.decimals, 6);
    }
}

#[cfg(test)]
mod rpc_tests {
    use super::*;
    use crate::fetch::chains::tron::TronClient;
    use serde_json::json;
    use wiremock::{matchers::body_partial_json, Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn concurrent_balance_reads_share_metadata_but_sends_and_new_sources_read_fresh() {
        let server = MockServer::start().await;
        for (selector, result, expected) in [
            ("balanceOf(address)", "0f4240".to_string(), 8),
            ("decimals()", "06".to_string(), 4),
            ("symbol()", format!("{:0<64}", hex::encode("TOKEN")), 4),
        ] {
            Mock::given(body_partial_json(json!({"function_selector":selector})))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"constant_result":[result]}))
                        .set_delay(Duration::from_millis(20)),
                )
                .expect(expected)
                .mount(&server)
                .await;
        }
        let cache = Arc::new(MetadataCache::default());
        let endpoints = Arc::new(vec![server.uri()]);
        let contract = "TR7NHqjeKQxGTCi8q8ZY4pL8otgjLj6t";
        let holder = "TLa2f6VPqDgRE67v1736s7bJ8Ray5wYjU7";
        // Separate short-lived clients, just like separate wallet refreshes.
        let results = futures::future::join_all((0..8).map(|_| {
            let client = TronClient::with_metadata_cache(endpoints.clone(), "tron", cache.clone());
            async move { client.fetch_trc20_balance(contract, holder).await.unwrap() }
        }))
        .await;
        for balance in results {
            assert_eq!(balance.decimals, 6);
            assert_eq!(balance.symbol, "TOKEN");
            assert_eq!(balance.balance_raw, "1000000");
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 10);
        // Even a cache-enabled client must bypass the cache for the explicit
        // metadata API used by the send builder.
        let reader = TronClient::with_metadata_cache(endpoints.clone(), "tron", cache.clone());
        reader.fetch_trc20_metadata(contract).await.unwrap();
        TronClient::with_metadata_cache(endpoints.clone(), "tron-nile", cache.clone())
            .read_metadata(contract)
            .await
            .unwrap();
        // A changed endpoint list is a different source, even for the same chain.
        let changed = Arc::new(vec![format!("{}/", server.uri())]);
        TronClient::with_metadata_cache(changed, "tron", cache)
            .read_metadata(contract)
            .await
            .unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), 16);
    }
}
