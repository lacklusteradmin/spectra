//! The token protocols a network hosts.

use super::*;

impl TokenStandard {
    pub const ALL: [Self; 14] = [
        Self::Erc20,
        Self::Bep20,
        Self::Arc20,
        Self::Spl,
        Self::Trc10,
        Self::Trc20,
        Self::Tep74,
        Self::Nep141,
        Self::SuiCoin,
        Self::AptosCoin,
        Self::Aip21,
        Self::TrustLineToken,
        Self::StellarAsset,
        Self::CardanoNativeToken,
    ];

    /// The protocol's name, as the catalogs, the store and a person spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Erc20 => "ERC-20",
            Self::Bep20 => "BEP-20",
            Self::Arc20 => "ARC-20",
            Self::Spl => "SPL",
            Self::Trc10 => "TRC-10",
            Self::Trc20 => "TRC-20",
            Self::Tep74 => "TEP-74",
            Self::Nep141 => "NEP-141",
            Self::SuiCoin => "Sui Coin",
            Self::AptosCoin => "Aptos Coin",
            Self::Aip21 => "AIP-21",
            Self::TrustLineToken => "Trust Line Token",
            Self::StellarAsset => "Stellar Asset",
            Self::CardanoNativeToken => "Cardano Native Token",
        }
    }

    /// A protocol named as [`TokenStandard::as_str`] spells it.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|standard| standard.as_str() == name)
    }

    /// What a token of this protocol is identified by, as a field asks for it.
    pub fn identifier_prompt(self) -> &'static str {
        match self {
            Self::Erc20 | Self::Bep20 | Self::Arc20 | Self::Trc20 => "Contract Address",
            Self::Trc10 => "Token ID",
            Self::Spl => "Mint Address",
            Self::Tep74 => "Jetton Master Address",
            Self::Nep141 => "Contract Account ID",
            Self::SuiCoin => "Coin Standard Type",
            Self::AptosCoin => "Coin Type",
            Self::Aip21 => "Fungible Asset Metadata Address",
            Self::TrustLineToken => "Currency Code and Issuer (CODE.rIssuer)",
            Self::StellarAsset => "Asset Code and Issuer (CODE:ISSUER)",
            Self::CardanoNativeToken => "Policy ID and Asset Name (POLICY.NAME)",
        }
    }

    /// The places a protocol's amounts always have, when it fixes them rather
    /// than each token: seven for Stellar assets (stroops), and the fifteen
    /// Spectra keeps of an XRP Ledger issued currency, whose values are
    /// sixteen significant digits at a floating exponent.
    pub fn fixed_decimals(self) -> Option<u32> {
        match self {
            Self::StellarAsset => Some(7),
            Self::TrustLineToken => Some(15),
            _ => None,
        }
    }
}

impl std::fmt::Display for TokenStandard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A protocol's name, for a front end to show.
#[uniffi::export]
pub fn token_standard_name(standard: TokenStandard) -> String {
    standard.as_str().to_string()
}

impl Chain {
    /// Whether this network supports any token protocols.
    pub fn hosts_tokens(self) -> bool {
        !self.token_standards().is_empty()
    }

    /// Protocols supported by this concrete network. A deployment carries its
    /// own standard, and no member of this list is a network-wide default.
    pub fn token_standards(self) -> &'static [TokenStandard] {
        &crate::catalog::chains::declared(self).token_standards
    }

    pub fn allows_token_standard(self, standard: TokenStandard) -> bool {
        self.token_standards().contains(&standard)
    }

    /// Infer a protocol only where identifier shapes distinguish it. Explicit
    /// deployments keep their recorded standard, including EVM aliases.
    pub fn token_standard_for_identifier(self, identifier: &str) -> Option<TokenStandard> {
        Some(match self {
            chain if chain.is_evm() => TokenStandard::Erc20,
            Self::Solana | Self::SolanaDevnet => TokenStandard::Spl,
            Self::Tron | Self::TronNile if identifier.bytes().all(|b| b.is_ascii_digit()) => {
                TokenStandard::Trc10
            }
            Self::Tron | Self::TronNile => TokenStandard::Trc20,
            Self::Ton | Self::TonTestnet => TokenStandard::Tep74,
            Self::Near | Self::NearTestnet => TokenStandard::Nep141,
            Self::Sui | Self::SuiTestnet => TokenStandard::SuiCoin,
            Self::Aptos | Self::AptosTestnet if identifier.contains("::") => {
                TokenStandard::AptosCoin
            }
            Self::Aptos | Self::AptosTestnet => TokenStandard::Aip21,
            Self::Xrp | Self::XrpTestnet => TokenStandard::TrustLineToken,
            Self::Stellar | Self::StellarTestnet => TokenStandard::StellarAsset,
            Self::Cardano | Self::CardanoPreprod => TokenStandard::CardanoNativeToken,
            _ => return None,
        })
    }

    /// Whether core has a transfer builder for this actual protocol.
    pub fn sends_token_standard(self, standard: TokenStandard) -> bool {
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
