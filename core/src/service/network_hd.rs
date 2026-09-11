//! Network hd: service adapters and dispatch.
use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Derive the account-level xpub (mainnet, canonical `xpub…` encoding)
    /// from a BIP39 mnemonic phrase.
    ///
    /// `account_path` is the **hardened account path** only, e.g.:
    ///   - `"m/84'/0'/0'"` → native SegWit (BIP84)
    ///   - `"m/49'/0'/0'"` → nested SegWit (BIP49)
    ///   - `"m/44'/0'/0'"` → legacy P2PKH (BIP44)
    ///
    /// `passphrase` is the optional BIP39 passphrase — pass `""` for none.
    pub fn derive_bitcoin_account_xpub_typed(
        &self,
        mnemonic_phrase: String,
        passphrase: String,
        account_path: String,
    ) -> Result<String, SpectraBridgeError> {
        crate::derivation::xpub_walker::derive_account_xpub(
            &mnemonic_phrase,
            &passphrase,
            &account_path,
        )
        .map_err(Into::into)
    }

    /// Derive a contiguous range of child addresses from an account-level
    /// extended public key (xpub/ypub/zpub).
    ///
    /// - `change` — 0 for external/receive, 1 for internal/change.
    /// - `start_index`, `count` — [start, start+count) scan window.
    pub(crate) async fn derive_bitcoin_hd_address_strings(
        &self,
        xpub: String,
        change: u32,
        start_index: u32,
        count: u32,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        let children =
            crate::derivation::xpub_walker::derive_children(&xpub, change, start_index, count)?;
        Ok(children.into_iter().map(|c| c.address).collect())
    }

    /// Return the first address on the `change` leg (0 = receive, 1 = change)
    /// that has zero confirmed/unconfirmed history, scanning up to
    /// `gap_limit` candidates. Returns the derived address string, or
    /// `None` if every candidate in the `gap_limit` window had activity.
    pub async fn fetch_bitcoin_next_unused_address_typed(
        &self,
        xpub: String,
        change: u32,
        gap_limit: u32,
    ) -> Result<Option<String>, SpectraBridgeError> {
        let endpoints = self.endpoints_for("bitcoin").await;
        let client = BitcoinClient::new(HttpClient::shared(), endpoints);
        let next = crate::derivation::xpub_walker::fetch_next_unused_address(
            &client, &xpub, change, gap_limit,
        )
        .await?;
        Ok(next.map(|c| c.address))
    }
}
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
        let endpoints = self.endpoints_for(chain_id).await;
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
