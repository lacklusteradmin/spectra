//! The EVM family: EIP-155 ids, gas headroom and rollup fee oracles.

use super::*;

impl Chain {
    /// EIP-155 chain id. Refuses chains outside the EVM family.
    pub fn evm_chain_id(self) -> Result<u64, RegistryError> {
        Ok(match self {
            Chain::Ethereum => 1,
            Chain::Arbitrum => 42161,
            Chain::Optimism => 10,
            Chain::Avalanche => 43114,
            Chain::Base => 8453,
            Chain::EthereumClassic => 61,
            Chain::BnbChain => 56,
            Chain::Hyperliquid => 999,
            Chain::Polygon => 137,
            Chain::Linea => 59144,
            Chain::Scroll => 534352,
            Chain::Blast => 81457,
            Chain::Mantle => 5000,
            Chain::Sei => 1329,
            Chain::Celo => 42220,
            Chain::Cronos => 25,
            Chain::OpBnb => 204,
            Chain::ZkSyncEra => 324,
            Chain::Sonic => 146,
            Chain::Berachain => 80094,
            Chain::Unichain => 130,
            Chain::Ink => 57073,
            Chain::XLayer => 196,
            Chain::Plasma => 9745,
            Chain::Monad => 143,
            Chain::WorldChain => 480,
            Chain::EthereumSepolia => 11155111,
            Chain::EthereumHoodi => 560048,
            Chain::ArbitrumSepolia => 421614,
            Chain::OptimismSepolia => 11155420,
            Chain::BaseSepolia => 84532,
            Chain::BnbChainTestnet => 97,
            Chain::AvalancheFuji => 43113,
            Chain::PolygonAmoy => 80002,
            Chain::HyperliquidTestnet => 998,
            Chain::LineaSepolia => 59141,
            Chain::CeloSepolia => 11142220,
            Chain::CronosTestnet => 338,
            Chain::ZkSyncEraSepolia => 300,
            Chain::SonicTestnet => 14601,
            Chain::InkSepolia => 763373,
            Chain::XLayerTestnet => 1952,
            Chain::EthereumClassicMordor => 63,
            _ => return Err(self.not_in("EVM")),
        })
    }

    /// Gas-estimate headroom in basis points. Monad charges the entire gas
    /// limit, so its default buffer is smaller than other EVM networks'.
    pub fn evm_gas_buffer_bps(self) -> u32 {
        if self == Self::Monad { 750 } else { 2000 }
    }

    /// Additional rollup fees use the oracle methods deployed on the
    /// concrete network; a missing method must never become a zero fee.
    pub(crate) fn evm_rollup_fee_model(self) -> Option<OpStackFeeModel> {
        if self == Self::CeloSepolia {
            return Some(OpStackFeeModel::Fjord);
        }
        match self.mainnet_counterpart() {
            Self::Optimism
            | Self::Base
            | Self::Celo
            | Self::Unichain
            | Self::Ink
            | Self::WorldChain => Some(OpStackFeeModel::FjordWithOperator),
            Self::OpBnb => Some(OpStackFeeModel::Fjord),
            Self::Blast => Some(OpStackFeeModel::Bedrock),
            _ => None,
        }
    }

    /// Historical operator-fee activation, pinned to the official chain
    /// configurations in docs/audits/chain-support-2026-10-04. Other deployments
    /// require an authoritative historical oracle answer; absence is unknown.
    pub(crate) fn evm_operator_fee_activation(self) -> Option<u64> {
        match self {
            Self::Optimism | Self::Base | Self::Unichain | Self::Ink => Some(1_746_806_401),
            Self::OptimismSepolia | Self::BaseSepolia | Self::InkSepolia => Some(1_744_905_600),
            Self::Celo => Some(1_752_073_200),
            Self::WorldChain => Some(1_764_072_000),
            _ => None,
        }
    }
}
