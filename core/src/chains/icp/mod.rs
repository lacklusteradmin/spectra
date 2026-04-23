//! Internet Computer Protocol (ICP) chain client.
//!
//! Uses the IC management canister and Rosetta API for balance and history.
//! ICP ledger interactions use CBOR-encoded Candid messages.
//! Keys are derived with secp256k1 (BIP32); identity is verified via
//! self-authenticating principals (SHA-224 of the DER-encoded public key).
//!
//! For production send, the full Ingress message flow is:
//!   1. Build a `call` envelope (CBOR)
//!   2. Sign with the private key (ECDSA/secp256k1)
//!   3. POST to /api/v2/canister/{canister_id}/call
//!   4. Query with /api/v2/canister/{canister_id}/read_state

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http::{with_fallback, HttpClient, RetryProfile};

pub mod derive;
pub mod fetch;
pub mod send;

pub use derive::*;

// ----------------------------------------------------------------
// Constants
// ----------------------------------------------------------------

/// ICP ledger canister ID (mainnet).
#[allow(dead_code)]
pub(crate) const ICP_LEDGER_CANISTER: &str = "ryjl3-tyaaa-aaaaa-aaaba-cai";
/// ICP e8s per ICP.
pub(crate) const E8S_PER_ICP: u64 = 100_000_000;

// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcpBalance {
    /// E8s (1 ICP = 100_000_000 e8s).
    pub e8s: u64,
    pub icp_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcpHistoryEntry {
    pub block_index: u64,
    pub timestamp_ns: u64,
    pub from: String,
    pub to: String,
    pub amount_e8s: u64,
    pub fee_e8s: u64,
    pub is_incoming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcpSendResult {
    pub block_index: u64,
}

// ----------------------------------------------------------------
// Client (Rosetta-based for read, direct for write)
// ----------------------------------------------------------------

pub struct IcpClient {
    /// Rosetta API endpoint (https://rosetta-api.internetcomputer.org).
    rosetta_endpoints: Vec<String>,
    /// IC HTTP gateway (https://ic0.app).
    #[allow(dead_code)]
    ic_endpoints: Vec<String>,
    client: std::sync::Arc<HttpClient>,
}

impl IcpClient {
    pub fn new(rosetta_endpoints: Vec<String>, ic_endpoints: Vec<String>) -> Self {
        Self {
            rosetta_endpoints,
            ic_endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn rosetta_post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &Value,
    ) -> Result<T, String> {
        let path = path.to_string();
        let body = body.clone();
        with_fallback(&self.rosetta_endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            let body = body.clone();
            async move { client.post_json(&url, &body, RetryProfile::ChainRead).await }
        })
        .await
    }
}
