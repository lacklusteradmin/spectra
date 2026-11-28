//! Every token a wallet's address holds on its network, its token list's or
//! not.
//!
//! A balance refresh asks only about the tokens the list names, the
//! catalog's and the user's, so a token sent to the address that the list
//! lacks is held and never shown. This asks the network's indexer what the
//! address holds and marks each token the list tracks. An untracked one is
//! named by its contract alone: its on-chain symbol is whoever deployed it's
//! to write, and an airdrop can call itself anything. A front end offers to
//! add it as a custom token, whose symbol and name the user gives.

use super::*;

const NOT_LISTED: &str = "No service lists the tokens an address holds on this network.";

/// One token the wallet's address holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletHeldToken {
    /// The contract, mint, coin type or issued asset, as the token list spells
    /// it when it tracks the token and as the network does when it does not.
    pub contract: String,
    pub standard: Option<crate::registry::TokenStandard>,
    /// The tracked token's symbol and name; empty for one the list lacks.
    pub symbol: String,
    pub name: String,
    /// The places the network states, else the token list's; zero when
    /// neither states any, and `balance` is then a count of base units.
    pub decimals: u32,
    /// What the address holds, as an exact decimal.
    pub balance: String,
    /// The wallet's token list has this token, so its balance already shows.
    pub is_tracked: bool,
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The tokens the wallet's address holds, read from the network's
    /// indexer: the tracked ones first, by symbol, then the rest by
    /// contract. A network with no indexer configured is an error, not an
    /// empty list: a contract answers only about a holder named to it.
    pub async fn wallet_held_tokens(
        &self,
        wallet_id: String,
    ) -> Result<Vec<WalletHeldToken>, SpectraBridgeError> {
        let this = self.clone();
        crate::service::worker::run(async move {
            let (_, chain, address) = this
                .action_wallet(&wallet_id, WalletAction::HeldTokens, NOT_LISTED)
                .await?;
            let held = this.discover_token_balances(chain, address).await?;
            let state = this.app_state().await;
            let mut tokens: Vec<WalletHeldToken> = held
                .into_iter()
                .map(|token| {
                    let tracked = crate::store::token_preferences::tracked_token(
                        &state,
                        chain,
                        &token.contract_address,
                    );
                    WalletHeldToken {
                        contract: tracked
                            .map_or(token.contract_address, |entry| entry.token.contract.clone()),
                        standard: tracked
                            .and_then(|entry| entry.token.token_standard)
                            .or(token.standard),
                        symbol: tracked
                            .map(|entry| entry.token.symbol.clone())
                            .unwrap_or_default(),
                        name: tracked
                            .map(|entry| entry.token.name.clone())
                            .unwrap_or_default(),
                        decimals: u32::from(token.decimals),
                        balance: token.balance_display,
                        is_tracked: tracked.is_some(),
                    }
                })
                .collect();
            tokens.sort_by(|a, b| {
                (!a.is_tracked, &a.symbol, &a.contract).cmp(&(
                    !b.is_tracked,
                    &b.symbol,
                    &b.contract,
                ))
            });
            Ok(tokens)
        })
        .await
    }
}
