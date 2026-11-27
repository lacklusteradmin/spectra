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
            Chain::Kaia => 8217,
            Chain::Xdc => 50,
            Chain::RobinhoodChain => 4663,
            Chain::KavaEvm => 2222,
            Chain::Chiliz => 88888,
            Chain::Stable => 988,
            Chain::Arc => 5042,
            Chain::MantaPacific => 169,
            Chain::Metis => 1088,
            Chain::Flare => 14,
            Chain::Katana => 747474,
            Chain::MegaEth => 4326,
            Chain::Merlin => 4200,
            Chain::Peaq => 3338,
            Chain::Somnia => 5031,
            Chain::ZeroG => 16661,
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

    /// The network an EIP-155 chain id names, if the registry has it.
    pub fn from_evm_chain_id(id: u64) -> Option<Self> {
        Self::all().find(|chain| chain.evm_chain_id().ok() == Some(id))
    }

    /// Gas-estimate headroom in basis points. Monad charges the entire gas
    /// limit, so its default buffer is smaller than other EVM networks'.
    pub fn evm_gas_buffer_bps(self) -> u32 {
        if self == Self::Monad { 750 } else { 2000 }
    }

    /// Networks whose nodes take only legacy (EIP-155 type 0) transactions:
    /// London is not active on Kava EVM or Ethereum Classic (CoreGeth's
    /// blocks carry no base fee), and Merlin's zkEVM and Metis's l2geth
    /// decode no typed envelope. A send there carries one gas price, read
    /// from `eth_gasPrice`, as both of its fee fields.
    pub fn evm_legacy_transactions(self) -> bool {
        matches!(
            self.mainnet_counterpart(),
            Self::KavaEvm | Self::Merlin | Self::Metis | Self::EthereumClassic
        )
    }

    /// The smallest priority fee the network's nodes accept, in wei. Where
    /// system transactions paying nothing fill a block, the fee history's
    /// lowest reward is zero and would price a transfer that is never
    /// included: Chiliz documents a 1 gwei minimum, and 0G includes no
    /// transaction tipping under 4 gwei (docs/audits/evm-mainnets-2026-11-24).
    pub(crate) fn evm_min_priority_fee_wei(self) -> u128 {
        match self.mainnet_counterpart() {
            Self::Chiliz => 1_000_000_000,
            Self::ZeroG => 4_000_000_000,
            _ => 0,
        }
    }

    /// The ERC-20 interface a network gives its own coin: the same balance
    /// `eth_getBalance` reads, so an indexer that lists it as a token, or
    /// its transfers as token transfers, would count the coin twice. Celo's
    /// GoldToken, Stable's USDT0 and Arc's USDC (both the gas coin at 18
    /// places behind a 6-place ERC-20), Metis's `0xdead…0000` and peaq's
    /// balances precompile. An address is a fact of one concrete network, so
    /// a test network names its own or none.
    pub(crate) fn evm_native_coin_contract(self) -> Option<&'static str> {
        match self {
            Self::Celo => Some("0x471ece3750da237f93b8e339c536989b8978a438"),
            Self::Stable => Some("0x779ded0c9e1022225f8e0630b35a9b54be713736"),
            Self::Arc => Some("0x3600000000000000000000000000000000000000"),
            Self::Metis => Some("0xdeaddeaddeaddeaddeaddeaddeaddeaddead0000"),
            Self::Peaq => Some("0x0000000000000000000000000000000000000809"),
            _ => None,
        }
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
            | Self::WorldChain
            | Self::MantaPacific
            | Self::Katana
            | Self::MegaEth => Some(OpStackFeeModel::FjordWithOperator),
            Self::OpBnb => Some(OpStackFeeModel::Fjord),
            Self::Blast | Self::Metis => Some(OpStackFeeModel::Bedrock),
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
