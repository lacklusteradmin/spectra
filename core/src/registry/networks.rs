//! Each account network's identity: genesis hashes, network names and ids, and
//! the protocol constants a transaction on it carries. The Substrate
//! networks' are in `substrate`.

use super::*;

impl Chain {
    pub(crate) fn monero_network_name(self) -> Result<&'static str, RegistryError> {
        match self {
            Self::Monero => Ok("mainnet"),
            Self::MoneroStagenet => Ok("stagenet"),
            _ => Err(self.not_in("Monero")),
        }
    }

    pub(crate) fn icp_ledger_id(self) -> Result<&'static str, RegistryError> {
        match self {
            Self::Icp => Ok("00000000000000020101"),
            _ => Err(self.not_in("ICP ledger")),
        }
    }

    pub(crate) fn icp_governance_id(self) -> Result<&'static str, RegistryError> {
        match self {
            Self::Icp => Ok("rrkah-fqaaa-aaaaa-aaaaq-cai"),
            _ => Err(self.not_in("ICP governance")),
        }
    }

    /// NNS governance/disburse_maturity.rs MINIMUM_DISBURSEMENT_E8S and
    /// MAX_NUM_DISBURSEMENTS. Maturity is minted without a ledger transfer fee.
    pub(crate) fn icp_maturity_disbursement_limits(self) -> Result<(u64, usize), RegistryError> {
        match self {
            Self::Icp => Ok((100_000_000, 10)),
            _ => Err(self.not_in("ICP maturity disbursement")),
        }
    }

    /// Mainnet root of trust, from DFINITY agent-rs IC_ROOT_KEY. Never fetched
    /// from a configurable provider, which could otherwise replace the network.
    pub(crate) fn icp_root_key(self) -> Result<Vec<u8>, RegistryError> {
        match self {
            Self::Icp => Ok(hex::decode("308182301d060d2b0601040182dc7c0503010201060c2b0601040182dc7c05030201036100814c0e6ec71fab583b08bd81373c255c3c371b2e84863c98a4f1e08b74235d14fb5d9c0cd546d9685f913a0c0b2cc5341583bf4b4392e467db96d65b9bb4cb717112f8472e0d5a4d14505ffd7484b01291091c5f87b98883463f98091a0baaae").expect("IC mainnet root key")),
            _ => Err(self.not_in("ICP root key")),
        }
    }

    pub fn stellar_network_passphrase(self) -> Result<&'static str, RegistryError> {
        match self {
            Self::Stellar => Ok("Public Global Stellar Network ; September 2015"),
            Self::StellarTestnet => Ok("Test SDF Network ; September 2015"),
            _ => Err(self.not_in("Stellar")),
        }
    }

    /// Official cluster identities returned by getGenesisHash; see the saved
    /// official-node responses in the chain support audit.
    pub(crate) fn solana_genesis_hash(self) -> Result<&'static str, RegistryError> {
        match self {
            Self::Solana => Ok("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"),
            Self::SolanaDevnet => Ok("EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG"),
            _ => Err(self.not_in("Solana")),
        }
    }

    /// The network magic a Cardano node's genesis states.
    pub(crate) fn cardano_network_magic(self) -> Result<u64, RegistryError> {
        match self {
            Self::Cardano => Ok(764_824_073),
            Self::CardanoPreprod => Ok(1),
            _ => Err(self.not_in("Cardano")),
        }
    }

    /// The name a Lotus node's `StateNetworkName` answers on this Filecoin
    /// network.
    pub(crate) fn filecoin_network_name(self) -> Result<&'static str, RegistryError> {
        match self {
            Self::Filecoin => Ok("mainnet"),
            _ => Err(self.not_in("Filecoin")),
        }
    }

    /// The CID of this Filecoin network's genesis block, which a Lotus
    /// node's `ChainGetGenesis` names. A message binds no network, so a node
    /// is proven to be on this one before anything is signed through it.
    pub(crate) fn filecoin_genesis_cid(self) -> Result<&'static str, RegistryError> {
        match self {
            Self::Filecoin => Ok("bafy2bzacecnamqgqmifpluoeldx7zzglxcljo6oja4vrmtj7432rphldpdmm2"),
            _ => Err(self.not_in("Filecoin")),
        }
    }

    /// The letter this Filecoin network's addresses start with: `f` on
    /// mainnet, as `t` is on the test networks.
    pub(crate) fn filecoin_address_network(self) -> Result<char, RegistryError> {
        match self {
            Self::Filecoin => Ok('f'),
            _ => Err(self.not_in("Filecoin")),
        }
    }

    pub(crate) fn near_network_name(self) -> Result<&'static str, RegistryError> {
        match self {
            Self::Near => Ok("mainnet"),
            Self::NearTestnet => Ok("testnet"),
            _ => Err(self.not_in("NEAR")),
        }
    }

    /// Aptos network identity bound into each locally constructed transaction.
    pub fn aptos_chain_id(self) -> Result<u8, RegistryError> {
        match self {
            Self::Aptos => Ok(1),
            Self::AptosTestnet => Ok(2),
            _ => Err(self.not_in("Aptos")),
        }
    }

    /// Gas units reserved by Aptos previews and committed into its builders.
    pub fn aptos_max_gas_amount(self) -> Result<u64, RegistryError> {
        match self.mainnet_counterpart() {
            Self::Aptos => Ok(10_000),
            _ => Err(self.not_in("Aptos")),
        }
    }

    /// Seconds from signing until an Aptos transaction it builds expires.
    pub(crate) fn aptos_transaction_lifetime_secs(self) -> Result<u64, RegistryError> {
        match self.mainnet_counterpart() {
            Self::Aptos => Ok(600),
            _ => Err(self.not_in("Aptos")),
        }
    }

    /// Seconds from signing until a TON external message it builds expires.
    pub(crate) fn ton_message_lifetime_secs(self) -> Result<u64, RegistryError> {
        match self.mainnet_counterpart() {
            Self::Ton => Ok(60),
            _ => Err(self.not_in("TON")),
        }
    }

    pub(crate) fn near_token_gas_limit(self) -> Result<u64, RegistryError> {
        match self.mainnet_counterpart() {
            Self::Near => Ok(30_000_000_000_000),
            _ => Err(self.not_in("NEAR")),
        }
    }

    /// NEP-448: accounts with no more than 770 storage bytes need no storage stake.
    pub(crate) fn near_zero_balance_storage_limit(self) -> Result<u64, RegistryError> {
        match self.mainnet_counterpart() {
            Self::Near => Ok(770),
            _ => Err(self.not_in("NEAR")),
        }
    }

    /// Ripple's public Mainnet/Testnet IDs, checked against official RPCs.
    pub(crate) fn xrp_network_id(self) -> Option<u64> {
        match self {
            Self::Xrp => Some(0),
            Self::XrpTestnet => Some(1),
            _ => None,
        }
    }

    /// Cross-checked at block zero on TronGrid, PublicNode and Nile's official
    /// endpoint on 2026-10-04; evidence is in the chain-support audit directory.
    pub(crate) fn tron_genesis_block_id(self) -> Option<&'static str> {
        match self {
            Self::Tron => Some("00000000000000001ebf88508a03865c71d452e25f4d51194196a1d22b6653dc"),
            Self::TronNile => {
                Some("0000000000000000d698d4192c56cb6be724a558448e2684802de4d6cd8690dc")
            }
            _ => None,
        }
    }

    /// Last name-mode block timestamp. Proposal 14 on mainnet and proposal 2
    /// on Nile enable parameter 15 after processing the maintenance block's
    /// transactions. The following block uses canonical token IDs. Official
    /// proposal/header evidence: audits/chain-support-2026-10-04/trc10-name-activation.json.
    pub(crate) fn tron_trc10_name_end_ms(self) -> Option<u64> {
        match self {
            Self::Tron => Some(1_546_668_000_000),
            Self::TronNile => Some(1_572_597_600_000),
            _ => None,
        }
    }

    /// Official checkpoint-zero identifiers, cross-checked with public JSON-RPC
    /// and Mysten's GraphQL deployments on 2026-10-04.
    pub(crate) fn sui_network_identity(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Sui => Some(("35834a8a", "4btiuiMPvEENsttpZC7CZ53DruC3MAgfznDbASZ7DR6S")),
            Self::SuiTestnet => Some(("4c78adac", "69WiPg3DAQiwdxfncX6wYQ2siKwAe6L9BZthQea3JNMD")),
            _ => None,
        }
    }

    /// TON masterchain zero-state root/file hashes from the official global
    /// configurations, cross-checked with TON Center on 2026-10-04.
    pub(crate) fn ton_zero_state(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Ton => Some((
                "F6OpKZKqvqeFp6CQmFomXNMfMj2EnaUSOXN+Mh+wVWk=",
                "XplPz01CXAps5qeSWUtxcyBfdAo5zVb1N979KLSKD24=",
            )),
            Self::TonTestnet => Some((
                "gj+B8wb/AmlPk1z1AhVI484rhrUpgSr2oSFIh56VoSg=",
                "Z+IKwYS54DmmJmesw/nAD5DzWadnOCMzee+kdgSYDOg=",
            )),
            _ => None,
        }
    }

    /// The first indexed masterchain block pins v3 indexers that omit the
    /// zero state. Cross-checked with official main/testnet on 2026-10-04.
    pub(crate) fn ton_first_block(self) -> Option<(i64, &'static str, &'static str)> {
        match self {
            Self::Ton => Some((
                -239,
                "8GYhhrigd8CwZGrRT59iulLDcgiTYuvOAzFJxugc0Ts=",
                "V+XzykEwun4yePZhAEPZk77RbMfMOgS/S4GiJkSKY6s=",
            )),
            Self::TonTestnet => Some((
                -3,
                "HBZqdwFA3MSjq0O8ntk6gX1Sibnw7cbWwEjKZt3JJpQ=",
                "eocxdO1VHjKnalajy5t+bM/X7A+rdMMX3Lj2VRRNtFs=",
            )),
            _ => None,
        }
    }
}
