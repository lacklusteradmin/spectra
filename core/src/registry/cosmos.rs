//! The Cosmos SDK family: each network's chain id, address prefix, fee coin,
//! key type, gas price and block time.
//!
//! Every fact here is bound into what a transaction signs (the chain id, the
//! fee denomination, the public key's type) or decides what an address is
//! (the prefix, the key type), so a network is known by its variant rather
//! than by anything a node answers. The sources and the dated readings that
//! confirm each value are in `docs/audits/cosmos-2026-11-24/`.

use super::*;

/// How a Cosmos network's accounts are keyed: what an address hashes and
/// which hash a signature is made over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CosmosKey {
    /// The SDK's own: the address is RIPEMD-160 of SHA-256 of the 33-byte
    /// compressed key, and a signature is over SHA-256 of the sign document.
    Secp256k1,
    /// Ethermint's: the address is the last 20 bytes of Keccak-256 of the
    /// 64-byte uncompressed key, as on Ethereum, and a signature is over
    /// Keccak-256 of the sign document. `type_url` names the public key's
    /// protobuf type, which differs between chains that forked it.
    EthSecp256k1 { type_url: &'static str },
}

impl CosmosKey {
    /// The protobuf type of the public key a transaction's signer carries.
    pub(crate) fn type_url(self) -> &'static str {
        match self {
            Self::Secp256k1 => "/cosmos.crypto.secp256k1.PubKey",
            Self::EthSecp256k1 { type_url } => type_url,
        }
    }
}

/// A fee coin's price per unit of gas, in its base units, as an exact
/// fraction: `0.025` is `25 / 1000`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GasPrice {
    pub numerator: u128,
    pub denominator: u128,
}

impl GasPrice {
    /// The fee `gas` costs at this price, rounded up as the ante handler
    /// requires: `fee * denominator >= gas * numerator`.
    pub(crate) fn fee_for(self, gas: u64) -> Option<u128> {
        let product = u128::from(gas).checked_mul(self.numerator)?;
        Some(product.div_ceil(self.denominator))
    }
}

/// Where a network's current minimum gas price is read, when it moves with
/// demand instead of being fixed by each validator's configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CosmosFeeMarket {
    /// Skip's `x/feemarket`: `GET /feemarket/v1/gas_price/{denom}`.
    Skip,
    /// Osmosis's `txfees` module: `GET /osmosis/txfees/v1beta1/cur_eip_base_fee`.
    OsmosisTxFees,
    /// Injective's `txfees` module: `GET /injective/txfees/v1beta1/cur_eip_base_fee`.
    InjectiveTxFees,
}

/// How a node's transaction search takes its event query: SDK 0.50 and
/// later as `query=`, earlier ones as `events=`, which 0.50 refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CosmosTxSearch {
    Query,
    Events,
}

/// One Cosmos SDK network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CosmosNetwork {
    /// The chain id every sign document binds, which a node's `node_info`
    /// must name.
    pub chain_id: &'static str,
    /// The bech32 prefix of an account address.
    pub hrp: &'static str,
    /// The base denomination of the network's own coin, which pays fees.
    pub denom: &'static str,
    pub key: CosmosKey,
    /// The least a transaction pays per unit of gas: the chain registry's
    /// `low` price for the fee coin, which validators accept. Where the
    /// network runs a fee market, its live price with a fifth more is used
    /// when that is higher.
    pub gas_price: GasPrice,
    pub fee_market: Option<CosmosFeeMarket>,
    /// The network's `max_memo_characters`, which the SDK counts in bytes.
    pub memo_limit: usize,
    pub tx_search: CosmosTxSearch,
    /// The time between blocks, measured and rounded down, which sizes how
    /// many blocks a signed transaction stays valid for.
    pub block_ms: u32,
}

const fn price(numerator: u128, denominator: u128) -> GasPrice {
    GasPrice {
        numerator,
        denominator,
    }
}

const COSMOS_HUB: CosmosNetwork = CosmosNetwork {
    chain_id: "cosmoshub-4",
    hrp: "cosmos",
    denom: "uatom",
    key: CosmosKey::Secp256k1,
    gas_price: price(1, 100),
    fee_market: Some(CosmosFeeMarket::Skip),
    memo_limit: 512,
    tx_search: CosmosTxSearch::Query,
    block_ms: 5_600,
};

const CELESTIA: CosmosNetwork = CosmosNetwork {
    chain_id: "celestia",
    hrp: "celestia",
    denom: "utia",
    key: CosmosKey::Secp256k1,
    gas_price: price(1, 100),
    fee_market: None,
    memo_limit: 256,
    tx_search: CosmosTxSearch::Query,
    block_ms: 2_800,
};

const DYDX: CosmosNetwork = CosmosNetwork {
    chain_id: "dydx-mainnet-1",
    hrp: "dydx",
    denom: "adydx",
    key: CosmosKey::Secp256k1,
    gas_price: price(12_500_000_000, 1),
    fee_market: None,
    memo_limit: 256,
    tx_search: CosmosTxSearch::Query,
    block_ms: 630,
};

const TERRA: CosmosNetwork = CosmosNetwork {
    chain_id: "phoenix-1",
    hrp: "terra",
    denom: "uluna",
    key: CosmosKey::Secp256k1,
    gas_price: price(2, 100),
    fee_market: None,
    memo_limit: 512,
    tx_search: CosmosTxSearch::Query,
    block_ms: 5_700,
};

const KAVA: CosmosNetwork = CosmosNetwork {
    chain_id: "kava_2222-10",
    hrp: "kava",
    denom: "ukava",
    key: CosmosKey::Secp256k1,
    gas_price: price(5, 100),
    fee_market: None,
    memo_limit: 512,
    tx_search: CosmosTxSearch::Events,
    block_ms: 5_700,
};

const INJECTIVE: CosmosNetwork = CosmosNetwork {
    chain_id: "injective-1",
    hrp: "inj",
    denom: "inj",
    key: CosmosKey::EthSecp256k1 {
        type_url: "/injective.crypto.v1beta1.ethsecp256k1.PubKey",
    },
    gas_price: price(500_000_000, 1),
    fee_market: Some(CosmosFeeMarket::InjectiveTxFees),
    memo_limit: 256,
    tx_search: CosmosTxSearch::Query,
    block_ms: 590,
};

const AKASH: CosmosNetwork = CosmosNetwork {
    chain_id: "akashnet-2",
    hrp: "akash",
    denom: "uakt",
    key: CosmosKey::Secp256k1,
    gas_price: price(5, 100),
    fee_market: None,
    memo_limit: 256,
    tx_search: CosmosTxSearch::Query,
    block_ms: 5_800,
};

const BABYLON: CosmosNetwork = CosmosNetwork {
    chain_id: "bbn-1",
    hrp: "bbn",
    denom: "ubbn",
    key: CosmosKey::Secp256k1,
    gas_price: price(7, 1000),
    fee_market: None,
    memo_limit: 256,
    tx_search: CosmosTxSearch::Query,
    block_ms: 9_500,
};

const OSMOSIS: CosmosNetwork = CosmosNetwork {
    chain_id: "osmosis-1",
    hrp: "osmo",
    denom: "uosmo",
    key: CosmosKey::Secp256k1,
    gas_price: price(3, 100),
    fee_market: Some(CosmosFeeMarket::OsmosisTxFees),
    memo_limit: 256,
    tx_search: CosmosTxSearch::Query,
    block_ms: 940,
};

const DYMENSION: CosmosNetwork = CosmosNetwork {
    chain_id: "dymension_1100-1",
    hrp: "dym",
    denom: "adym",
    key: CosmosKey::EthSecp256k1 {
        type_url: "/ethermint.crypto.v1.ethsecp256k1.PubKey",
    },
    gas_price: price(20_000_000_000, 1),
    fee_market: None,
    memo_limit: 256,
    tx_search: CosmosTxSearch::Query,
    block_ms: 4_800,
};

const SAGA: CosmosNetwork = CosmosNetwork {
    chain_id: "ssc-1",
    hrp: "saga",
    denom: "usaga",
    key: CosmosKey::Secp256k1,
    gas_price: price(1, 100),
    fee_market: None,
    memo_limit: 256,
    tx_search: CosmosTxSearch::Query,
    block_ms: 5_600,
};

const ALLORA: CosmosNetwork = CosmosNetwork {
    chain_id: "allora-mainnet-1",
    hrp: "allo",
    denom: "uallo",
    key: CosmosKey::Secp256k1,
    gas_price: price(500_000_000, 1),
    fee_market: Some(CosmosFeeMarket::Skip),
    memo_limit: 256,
    tx_search: CosmosTxSearch::Query,
    block_ms: 5_500,
};

const BAND: CosmosNetwork = CosmosNetwork {
    chain_id: "laozi-mainnet",
    hrp: "band",
    denom: "uband",
    key: CosmosKey::Secp256k1,
    gas_price: price(25, 10_000),
    fee_market: None,
    memo_limit: 512,
    tx_search: CosmosTxSearch::Query,
    block_ms: 1_000,
};

/// The gas a transfer of a network's own coin is assumed to use where no
/// node has simulated it: what a quote prices before an account exists.
pub(crate) const TRANSFER_GAS_ESTIMATE: u64 = 100_000;

/// How long a signed Cosmos transaction stays valid: its `timeout_height`
/// is this many milliseconds of blocks past the height it was built at.
const SIGNED_LIFETIME_MS: u32 = 600_000;

impl Chain {
    /// This network's Cosmos SDK facts, or `None` outside the family.
    pub(crate) fn cosmos(self) -> Option<&'static CosmosNetwork> {
        Some(match self {
            Self::CosmosHub => &COSMOS_HUB,
            Self::Celestia => &CELESTIA,
            Self::Dydx => &DYDX,
            Self::Terra => &TERRA,
            Self::Kava => &KAVA,
            Self::Injective => &INJECTIVE,
            Self::Akash => &AKASH,
            Self::Babylon => &BABYLON,
            Self::Osmosis => &OSMOSIS,
            Self::Dymension => &DYMENSION,
            Self::Saga => &SAGA,
            Self::Allora => &ALLORA,
            Self::Band => &BAND,
            _ => return None,
        })
    }

    /// Whether this is a Cosmos SDK network, read and signed through
    /// `api::cosmos_rest` and `send::cosmos`.
    pub fn is_cosmos(self) -> bool {
        self.cosmos().is_some()
    }

    /// [`Chain::cosmos`], or the refusal a caller outside the family gets.
    pub(crate) fn cosmos_network(self) -> Result<&'static CosmosNetwork, RegistryError> {
        self.cosmos().ok_or_else(|| self.not_in("Cosmos SDK"))
    }

    /// How many blocks past the current one a signed transaction stays
    /// valid: about ten minutes of this network's blocks.
    pub(crate) fn cosmos_timeout_blocks(self) -> Result<u64, RegistryError> {
        let network = self.cosmos_network()?;
        Ok(u64::from(SIGNED_LIFETIME_MS / network.block_ms.max(1)))
    }
}
