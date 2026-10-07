//! Signing a message with a wallet's key, to prove it holds its address.
//!
//! The key is the one the wallet sends with, resolved as a send resolves it,
//! for the address the wallet shows; the scheme is its network's
//! ([`crate::send::message`]).

use super::*;
use crate::send::message::MessageScheme;

/// A message signed by a wallet: what was signed, by which address, how,
/// and the signature to hand over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SignedMessage {
    pub scheme: MessageScheme,
    pub address: String,
    pub message: String,
    pub signature: String,
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The scheme the wallet's address signs messages in, or `None` where
    /// its network has no message standard. Reads no secret, so a watched
    /// wallet answers too: its signatures can be checked.
    pub async fn wallet_message_scheme(
        &self,
        wallet_id: String,
    ) -> Result<Option<MessageScheme>, SpectraBridgeError> {
        let wallet = self.stored_wallet(&wallet_id).await?;
        Ok(wallet
            .address_on(wallet.chain_id)
            .and_then(|address| crate::send::message::scheme_for(wallet.chain_id, address)))
    }

    /// Sign `message` with the wallet's key, for the address it shows.
    pub async fn sign_wallet_message(
        &self,
        wallet_id: String,
        message: String,
        password: Option<String>,
    ) -> Result<SignedMessage, SpectraBridgeError> {
        let chain = self.stored_wallet(&wallet_id).await?.chain_id;
        let password = password.map(zeroize::Zeroizing::new);
        let identity = self
            .resolve_send_identity(chain, &wallet_id, password.as_deref().map(String::as_str))
            .await?;
        let (scheme, signature) = crate::send::message::sign_message(
            chain,
            &identity.from_address,
            &identity.private_key_hex,
            &message,
        )?;
        Ok(SignedMessage {
            scheme,
            address: identity.from_address,
            message,
            signature,
        })
    }
}

#[cfg(test)]
#[path = "tests/wallet_messages.rs"]
mod tests;
