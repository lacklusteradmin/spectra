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

/// Derive the account-level xpub (mainnet, canonical `xpub…` encoding)
/// from a BIP39 mnemonic phrase.
///
/// `account_path` is the **hardened account path** only, e.g.:
///   - `"m/84'/0'/0'"` → native SegWit (BIP84)
///   - `"m/49'/0'/0'"` → nested SegWit (BIP49)
///   - `"m/44'/0'/0'"` → legacy P2PKH (BIP44)
///
/// `passphrase` is the optional BIP39 passphrase — pass `""` for none.
///
/// Exported as a function rather than a method on `WalletService`: it takes a
/// phrase and a path and reads no wallet, no database and no endpoint. Hanging
/// it off the service said otherwise to everyone who called it.
pub fn derive_bitcoin_account_xpub_typed(
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
