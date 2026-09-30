use crate::registry::Chain;
use crate::store::wallet_domain::{AssetHolding, CoreTokenPreferenceEntry};

/// A token's contract and its own decimals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendTokenIdentity {
    pub contract: String,
    pub decimals: u32,
}

/// What a holding is, as far as sending it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendAssetKind {
    Native,
    Token(SendTokenIdentity),
    /// A token the user does not track. Its contract and scale are unknown,
    /// so it is never sent.
    UntrackedToken,
}

/// The asset a send moves, resolved from a holding and core's token list.
///
/// The one answer to "can this be sent": the send button, the preview and the
/// preflight all ask it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendAsset {
    pub chain: Chain,
    pub symbol: String,
    pub kind: SendAssetKind,
}

impl SendAsset {
    /// `None` for a holding on a chain core does not know.
    pub fn of(holding: &AssetHolding, preferences: &[CoreTokenPreferenceEntry]) -> Option<Self> {
        let chain = holding.chain_id;
        let kind = if holding.is_native() {
            SendAssetKind::Native
        } else {
            preferences
                .iter()
                .find(|entry| entry.is_enabled && entry.token.matches_holding(holding))
                .map_or(SendAssetKind::UntrackedToken, |entry| {
                    SendAssetKind::Token(SendTokenIdentity {
                        contract: entry.token.contract.clone(),
                        decimals: entry.token.decimals,
                    })
                })
        };
        Some(Self {
            chain,
            symbol: holding.symbol.clone(),
            kind,
        })
    }

    pub fn token(&self) -> Option<&SendTokenIdentity> {
        match &self.kind {
            SendAssetKind::Token(token) => Some(token),
            _ => None,
        }
    }

    /// Every chain sends its own asset; a token only where the chain has a
    /// token transfer and the user tracks it.
    pub fn is_sendable(&self) -> bool {
        match self.kind {
            SendAssetKind::Native => true,
            SendAssetKind::Token(_) => self.chain.sends_tokens(),
            SendAssetKind::UntrackedToken => false,
        }
    }

    /// Only a native EVM send may move nothing: a zero-value call is a real
    /// transaction there, and a zero transfer anywhere else is a mistake.
    pub fn allows_zero_amount(&self) -> bool {
        self.chain.is_evm() && self.kind == SendAssetKind::Native
    }
}

/// Whether the send button offers a holding: a wallet with something to sign
/// with, and an asset core can send.
pub(crate) fn can_send_coin(
    coin: &AssetHolding,
    has_signing_material: bool,
    token_preferences: &[CoreTokenPreferenceEntry],
) -> bool {
    has_signing_material
        && SendAsset::of(coin, token_preferences).is_some_and(|asset| asset.is_sendable())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::wallet_domain::CoreTokenPreferenceCategory;

    fn holding(chain: Chain, symbol: &str, contract: Option<&str>) -> AssetHolding {
        AssetHolding {
            id: String::new(),
            name: symbol.to_string(),
            symbol: symbol.to_string(),
            coingecko_id: String::new(),
            chain_id: chain,
            token_standard: if contract.is_some() {
                chain.token_standard().to_string()
            } else {
                "Native".to_string()
            },
            contract_address: contract.map(str::to_string),
            amount: "1".into(),
        }
    }

    fn tracked(
        chain: Chain,
        symbol: &str,
        contract: &str,
        decimals: u32,
    ) -> CoreTokenPreferenceEntry {
        CoreTokenPreferenceEntry {
            category: CoreTokenPreferenceCategory::Stablecoin,
            is_built_in: false,
            is_enabled: true,
            token: crate::tokens::TokenDeploymentEntry {
                deployment_id: "fixture:token".into(),
                token_id: "fixture:token".into(),
                kind: crate::tokens::TokenKind::Protocol {
                    standard: "fixture".into(),
                    identifier: "fixture".into(),
                },
                chain_id: chain,
                name: symbol.into(),
                symbol: symbol.into(),
                token_standard: chain.token_standard().to_string(),
                contract: contract.to_string(),
                coingecko_id: String::new(),
                coinpaprika_id: String::new(),
                decimals,
                tags: Vec::new(),
                color: None,
                artwork_name: String::new(),
                enabled: true,
            },
        }
    }

    fn sendable(holding: &AssetHolding, preferences: &[CoreTokenPreferenceEntry]) -> bool {
        SendAsset::of(holding, preferences).is_some_and(|asset| asset.is_sendable())
    }

    /// Every chain, mainnet and testnet, sends its own asset. The router used
    /// to reach this through a table of display names, and a chain missing
    /// from it could not send at all.
    #[test]
    fn every_chain_sends_its_native_asset() {
        for chain in Chain::all() {
            let native = holding(chain, chain.coin_symbol(), None);
            assert!(sendable(&native, &[]), "{} cannot send", chain.str_id());
        }
    }

    /// A tracked token is sendable exactly where the chain has a token
    /// transfer; an untracked one never is.
    #[test]
    fn a_token_needs_a_token_transfer_and_a_tracked_contract() {
        for chain in Chain::mainnets().filter(|c| c.hosts_tokens()) {
            let contract = "Contract1111111111111111111111111111111111";
            let token = holding(chain, "TOK", Some(contract));
            assert!(!sendable(&token, &[]), "{} untracked", chain.str_id());
            assert_eq!(
                sendable(&token, &[tracked(chain, "TOK", contract, 6)]),
                chain.sends_tokens(),
                "{}",
                chain.str_id()
            );
        }
    }

    /// A token carries its own decimals, matched by contract. Tron hard-coded
    /// six for every token — right for USDT, wrong for the eighteen-decimal
    /// ones — which is a scale guessed on the funds path.
    #[test]
    fn a_token_carries_its_own_decimals() {
        let usdt = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";
        let usdd = "TPYmHEhy5n8TCEfYGqW2rPxsghSfzghPDn";
        let preferences = [
            tracked(Chain::Tron, "USDT", usdt, 6),
            tracked(Chain::Tron, "USDD", usdd, 18),
        ];
        let identity = |contract| {
            SendAsset::of(&holding(Chain::Tron, "X", Some(contract)), &preferences)
                .and_then(|asset| asset.token().cloned())
                .map(|token| token.decimals)
        };
        assert_eq!(identity(usdd), Some(18));
        assert_eq!(identity(usdt), Some(6));
        assert_eq!(identity("TSomeOtherContractAddressEntirely"), None);
        let trx = SendAsset::of(&holding(Chain::Tron, "TRX", None), &preferences).unwrap();
        assert_eq!(trx.kind, SendAssetKind::Native);
    }

    #[test]
    fn only_a_native_evm_send_may_be_zero() {
        let asset = |chain: Chain, contract: Option<&str>| {
            let preferences = contract
                .map(|c| vec![tracked(chain, "TOK", c, 6)])
                .unwrap_or_default();
            SendAsset::of(&holding(chain, "X", contract), &preferences).unwrap()
        };
        assert!(asset(Chain::Ethereum, None).allows_zero_amount());
        assert!(asset(Chain::BaseSepolia, None).allows_zero_amount());
        assert!(!asset(Chain::Ethereum, Some("0xabc")).allows_zero_amount());
        assert!(!asset(Chain::Bitcoin, None).allows_zero_amount());
    }

    #[test]
    fn a_watch_only_wallet_on_a_live_chain_cannot_send() {
        let btc = holding(Chain::Bitcoin, "BTC", None);
        assert!(!can_send_coin(&btc, false, &[]));
        assert!(can_send_coin(&btc, true, &[]));
    }
}
