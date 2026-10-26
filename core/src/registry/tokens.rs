//! The token protocols a network hosts.

use super::*;

impl Chain {
    /// Whether this network supports any token protocols.
    pub fn hosts_tokens(self) -> bool {
        !self.token_standards().is_empty()
    }

    /// Protocols supported by this concrete network. A deployment carries its
    /// own standard, and no member of this list is a network-wide default.
    pub fn token_standards(self) -> &'static [String] {
        &crate::catalog::chains::declared(self).token_standards
    }

    pub fn allows_token_standard(self, standard: &str) -> bool {
        self.token_standards().iter().any(|s| s == standard)
    }

    /// Infer a protocol only where identifier shapes distinguish it. Explicit
    /// deployments keep their recorded standard, including EVM aliases.
    pub fn token_standard_for_identifier(self, identifier: &str) -> &'static str {
        match self {
            chain if chain.is_evm() => "ERC-20",
            Self::Solana | Self::SolanaDevnet => "SPL",
            Self::Tron | Self::TronNile if identifier.bytes().all(|b| b.is_ascii_digit()) => {
                "TRC-10"
            }
            Self::Tron | Self::TronNile => "TRC-20",
            Self::Ton | Self::TonTestnet => "TEP-74",
            Self::Near | Self::NearTestnet => "NEP-141",
            Self::Sui | Self::SuiTestnet => "Sui Coin",
            Self::Aptos | Self::AptosTestnet if identifier.contains("::") => "Aptos Coin",
            Self::Aptos | Self::AptosTestnet => "AIP-21",
            Self::Xrp | Self::XrpTestnet => "Trust Line Token",
            Self::Stellar | Self::StellarTestnet => "Stellar Asset",
            Self::Cardano | Self::CardanoPreprod => "Cardano Native Token",
            _ => "",
        }
    }

    /// Whether core has a transfer builder for this actual protocol.
    pub fn sends_token_standard(self, standard: &str) -> bool {
        self.allows_token_standard(standard) && self.sends_tokens()
    }

    /// Whether a tracked token on this chain can be sent. Every chain sends
    /// its own asset.
    ///
    /// Every representable fungible-token standard has a transfer builder.
    pub fn sends_tokens(self) -> bool {
        let chain = self.mainnet_counterpart();
        match chain {
            Chain::Solana
            | Chain::Tron
            | Chain::Near
            | Chain::Sui
            | Chain::Aptos
            | Chain::Ton
            | Chain::Xrp
            | Chain::Stellar
            | Chain::Cardano => true,
            _ => chain.is_evm(),
        }
    }
}
