//! WalletService — Bitcoin xpub / HD address derivation.
//!
//! Sliced out of `service/mod.rs`. The `WalletService` type itself stays in
//! `mod.rs`; methods here live in a separate `impl` block (Rust permits
//! multiple impl blocks per type, and UniFFI exports them as if they were one).

#![allow(unused_imports)]

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
        crate::derivation::utxo_hd::derive_account_xpub(
            &mnemonic_phrase,
            &passphrase,
            &account_path,
        )
        .map_err(SpectraBridgeError::from)
    }

    /// Derive a contiguous range of child addresses from an account-level
    /// extended public key (xpub/ypub/zpub).
    ///
    /// - `change` — 0 for external/receive, 1 for internal/change.
    /// - `start_index`, `count` — [start, start+count) scan window.
    pub async fn derive_bitcoin_hd_address_strings(
        &self,
        xpub: String,
        change: u32,
        start_index: u32,
        count: u32,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        let children =
            crate::derivation::utxo_hd::derive_children(&xpub, change, start_index, count)
                .map_err(SpectraBridgeError::from)?;
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
        let endpoints = self.endpoints_for(0).await;
        let client = BitcoinClient::new(HttpClient::shared(), endpoints);
        let next = crate::derivation::utxo_hd::fetch_next_unused_address(
            &client,
            &xpub,
            change,
            gap_limit,
        )
        .await
        .map_err(SpectraBridgeError::from)?;
        Ok(next.map(|c| c.address))
    }
}
