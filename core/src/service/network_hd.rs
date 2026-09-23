//! Network hd: service adapters and dispatch.
use super::*;

impl WalletService {
    /// The aggregated balance across an xpub's derived addresses.
    ///
    /// Returned the serialized `HdXpubBalance` as a JSON string before, which
    /// both callers immediately parsed back for `confirmed_sats` (and one of
    /// them `utxo_count`) — a serialization and a re-parse between two
    /// functions in the same call. The struct is what crosses now.
    pub(crate) async fn bitcoin_xpub_balance(
        &self,
        chain_id: &str,
        xpub: String,
        receive_count: u32,
        change_count: u32,
    ) -> Result<crate::derivation::xpub_walker::HdXpubBalance, SpectraBridgeError> {
        // The network's endpoints, not Bitcoin's: an xpub on Testnet4 is
        // walked against Testnet4.
        let endpoints = self.endpoints_for(chain_id, &["balance"]).await;
        let client = BitcoinClient::new(HttpClient::shared(), endpoints);
        Ok(crate::derivation::xpub_walker::fetch_xpub_balance(
            &client,
            &xpub,
            receive_count,
            change_count,
        )
        .await?)
    }
}
