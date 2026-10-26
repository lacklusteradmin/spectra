//! The Bitcoin family: address versions, consensus limits and Zcash upgrades.

use super::*;

impl Chain {
    /// Legacy output address versions: P2PKH, followed by accepted P2SH aliases.
    pub(crate) fn fixed_utxo_address_versions(self) -> Result<(u8, &'static [u8]), RegistryError> {
        match self {
            Self::Bitcoin => Ok((0x00, &[0x05])),
            Self::BitcoinTestnet | Self::BitcoinTestnet4 | Self::BitcoinSignet => {
                Ok((0x6f, &[0xc4]))
            }
            Self::BitcoinCash | Self::BitcoinSV => Ok((0x00, &[0x05])),
            Self::BitcoinCashTestnet | Self::BitcoinSVTestnet => Ok((0x6f, &[0xc4])),
            Self::Dogecoin => Ok((0x1e, &[0x16])),
            Self::DogecoinTestnet => Ok((0x71, &[0xc4])),
            Self::Litecoin => Ok((0x30, &[0x32, 0x05])),
            Self::LitecoinTestnet => Ok((0x6f, &[0x3a, 0xc4])),
            Self::Dash => Ok((0x4c, &[0x10])),
            Self::DashTestnet => Ok((0x8c, &[0x13])),
            Self::BitcoinGold => Ok((0x26, &[0x17])),
            Self::Peercoin => Ok((0x37, &[0x75])),
            Self::PeercoinTestnet => Ok((0x6f, &[0xc4])),
            _ => Err(self.not_in("Base58 UTXO")),
        }
    }

    /// The fork id a network's SIGHASH_FORKID signatures carry: Bitcoin
    /// Gold's 79, and 0 on Bitcoin Cash and Bitcoin SV, whose replay
    /// protection is the flag alone.
    pub(crate) fn sighash_fork_id(self) -> Result<u32, RegistryError> {
        match self.mainnet_counterpart() {
            Self::BitcoinCash | Self::BitcoinSV => Ok(0),
            Self::BitcoinGold => Ok(79),
            _ => Err(self.not_in("SIGHASH_FORKID")),
        }
    }

    /// Decred's two-byte address versions, `(P2PKH, P2SH)`, from dcrd's
    /// chaincfg: secp256k1 ECDSA pubkey hash (`Ds`/`Ts`) and script hash
    /// (`Dc`/`Tc`).
    pub(crate) fn decred_address_versions(self) -> Result<([u8; 2], [u8; 2]), RegistryError> {
        match self {
            Self::Decred => Ok(([0x07, 0x3f], [0x07, 0x1a])),
            Self::DecredTestnet => Ok(([0x0f, 0x21], [0x0e, 0xfc])),
            _ => Err(self.not_in("Decred")),
        }
    }

    /// A wallet on this chain is a BIP-44 account rather than one address:
    /// a phrase wallet stores its account public key, a gap scan finds the
    /// receive and change addresses it has used, receive addresses rotate,
    /// its balance and history sum every one of them, and a send spends from
    /// each with that address's own key. A private-key or watched-address
    /// wallet is the account's one address.
    pub fn uses_account_utxo(self) -> bool {
        matches!(
            self.mainnet_counterpart(),
            Self::Bitcoin
                | Self::BitcoinCash
                | Self::BitcoinSV
                | Self::Litecoin
                | Self::Dogecoin
                | Self::Peercoin
                | Self::Zcash
                | Self::BitcoinGold
                | Self::Decred
                | Self::Kaspa
                | Self::Dash
        )
    }

    /// Peercoin Core amount.h: amount range sanity bound, not a supply cap.
    pub(crate) fn peercoin_max_money(self) -> Result<u64, RegistryError> {
        match self.mainnet_counterpart() {
            Self::Peercoin => Ok(21_000_000 * 1_000_000),
            _ => Err(self.not_in("Peercoin")),
        }
    }

    /// Standard recipient and retained-change floor: CENT, or 0.01 PPC.
    pub(crate) fn peercoin_min_output_units(self) -> Result<u64, RegistryError> {
        match self.mainnet_counterpart() {
            Self::Peercoin => Ok(10_000),
            _ => Err(self.not_in("Peercoin")),
        }
    }

    pub(crate) fn peercoin_min_fee_units(self) -> Result<u64, RegistryError> {
        match self.mainnet_counterpart() {
            Self::Peercoin => Ok(1_000),
            _ => Err(self.not_in("Peercoin")),
        }
    }

    /// Consensus fees charge complete serialized bytes, including witness bytes.
    pub(crate) fn peercoin_fee_per_kb_units(self) -> Result<u64, RegistryError> {
        match self.mainnet_counterpart() {
            Self::Peercoin => Ok(10_000),
            _ => Err(self.not_in("Peercoin")),
        }
    }

    /// Both proof-of-work coinbase and proof-of-stake coinstake outputs mature.
    pub(crate) fn peercoin_generated_output_maturity(self) -> Result<u32, RegistryError> {
        match self {
            Self::Peercoin => Ok(500),
            Self::PeercoinTestnet => Ok(60),
            _ => Err(self.not_in("Peercoin")),
        }
    }

    /// The network's genesis block, as explorers write its hash: what an
    /// indexer on it names at height 0.
    /// The hash of the network's genesis block, as an indexer names the
    /// block at height 0: what a custom Bitcoin or Litecoin indexer is
    /// checked against before it broadcasts.
    pub(crate) fn genesis_block_hash(self) -> Result<String, RegistryError> {
        match self {
            Self::Litecoin => {
                Ok("12a765e31ffd4059bada1e25190f6e98c99d9714d334efa41a195a7e7e04bfe2".into())
            }
            Self::LitecoinTestnet => {
                Ok("4966625a4b2851d9fdee139e56211a0d88575f59ed816ff5e6a63deb4e3e29a0".into())
            }
            _ => self
                .bitcoin_network()
                .map(|network| {
                    bitcoin::blockdata::constants::genesis_block(network)
                        .block_hash()
                        .to_string()
                })
                .ok_or_else(|| self.not_in("Bitcoin or Litecoin")),
        }
    }

    pub(crate) fn litecoin_max_money(self) -> Result<u64, RegistryError> {
        match self {
            Self::Litecoin | Self::LitecoinTestnet => Ok(84_000_000 * 100_000_000),
            _ => Err(self.not_in("Litecoin")),
        }
    }

    /// Litecoin Core's default dust relay fee, in litoshis per virtual kilobyte.
    pub(crate) fn litecoin_dust_relay_fee_per_kvb(self) -> Result<u64, RegistryError> {
        match self {
            Self::Litecoin | Self::LitecoinTestnet => Ok(30_000),
            _ => Err(self.not_in("Litecoin")),
        }
    }

    pub(crate) fn fixed_utxo_segwit_hrp(self) -> Option<&'static str> {
        match self {
            Self::Bitcoin => Some("bc"),
            Self::BitcoinTestnet | Self::BitcoinTestnet4 | Self::BitcoinSignet => Some("tb"),
            Self::Litecoin => Some("ltc"),
            Self::LitecoinTestnet => Some("tltc"),
            Self::BitcoinGold => Some("btg"),
            Self::Peercoin => Some("pc"),
            Self::PeercoinTestnet => Some("tpc"),
            _ => None,
        }
    }

    pub(crate) fn cashaddr_prefix(self) -> Option<&'static str> {
        match self {
            Self::BitcoinCash => Some("bitcoincash"),
            Self::BitcoinCashTestnet => Some("bchtest"),
            _ => None,
        }
    }

    pub(crate) fn fixed_utxo_supports_witness(self, version: u8, program_length: usize) -> bool {
        match self {
            Self::Bitcoin | Self::BitcoinTestnet | Self::BitcoinTestnet4 | Self::BitcoinSignet => {
                (version == 0 && matches!(program_length, 20 | 32))
                    || (version == 1 && program_length == 32)
            }
            Self::Litecoin | Self::LitecoinTestnet => {
                (version == 0 && matches!(program_length, 20 | 32))
                    || (version == 1 && program_length == 32)
            }
            Self::Peercoin | Self::PeercoinTestnet => {
                (version == 0 && matches!(program_length, 20 | 32))
                    || (version == 1 && program_length == 32)
            }
            // BTG's Taproot deployment is not an established active consensus rule.
            Self::BitcoinGold => version == 0 && matches!(program_length, 20 | 32),
            _ => false,
        }
    }

    /// The smallest output the legacy-format P2PKH networks relay: 546
    /// units at Bitcoin Core's dust relay rate, and Dogecoin Core's dust
    /// limit of 0.01 DOGE (policy.h `DEFAULT_DUST_LIMIT`), below which an
    /// output pays that much again in fee and, below a tenth of it, is not
    /// relayed at all.
    pub(crate) fn legacy_change_dust(self) -> Result<u64, RegistryError> {
        match self.mainnet_counterpart() {
            Self::BitcoinCash | Self::BitcoinSV | Self::BitcoinGold | Self::Dash => Ok(546),
            Self::Dogecoin => Ok(1_000_000),
            _ => Err(self.not_in("fixed-fee P2PKH")),
        }
    }

    /// The consensus parameters librustzcash builds and scans this network
    /// with: its upgrade schedule is the one schedule Spectra uses.
    pub(crate) fn zcash_network(self) -> Result<zcash_protocol::consensus::Network, RegistryError> {
        match self {
            Self::Zcash => Ok(zcash_protocol::consensus::Network::MainNetwork),
            Self::ZcashTestnet => Ok(zcash_protocol::consensus::Network::TestNetwork),
            _ => Err(self.not_in("Zcash")),
        }
    }

    /// The consensus branch at `height`, from librustzcash's upgrade schedule.
    /// Before NU5 no V5 transaction is valid, so there is none to build for.
    pub(crate) fn zcash_consensus_branch(self, height: u32) -> Result<u32, RegistryError> {
        use zcash_protocol::consensus::{BlockHeight, BranchId, NetworkUpgrade, Parameters};
        let network = self.zcash_network()?;
        if !network.is_nu_active(NetworkUpgrade::Nu5, BlockHeight::from_u32(height)) {
            return Err(RegistryError::ZcashV5Inactive(height));
        }
        Ok(u32::from(BranchId::for_height(
            &network,
            BlockHeight::from_u32(height),
        )))
    }

    pub(crate) fn zcash_genesis(self) -> Result<&'static str, RegistryError> {
        match self {
            Self::Zcash => Ok("00040fe8ec8471911baa1db1266ea15dd06b4a8a5c453883c000b031973dce08"),
            Self::ZcashTestnet => {
                Ok("05a60a92d99d85997cce3b87616c089f6124d7342af37106edc76126334a2c38")
            }
            _ => Err(self.not_in("Zcash")),
        }
    }

    /// Encode a discovered public child using this chain's supported address format.
    pub(crate) fn encode_discovery_address(
        self,
        key: &secp256k1::PublicKey,
        script: crate::derivation::types::BitcoinScriptType,
    ) -> Result<String, crate::derivation::error::DerivationError> {
        use crate::derivation::error::DerivationError;
        use crate::derivation::{
            bitcoin as btc, bitcoin_cash as bch, dogecoin as doge, litecoin as ltc,
        };
        Ok(match self {
            Self::Bitcoin => return btc::encode_address_inner(btc::BTC_MAINNET, script, key),
            Self::BitcoinTestnet | Self::BitcoinTestnet4 | Self::BitcoinSignet => {
                return btc::encode_address_inner(btc::BTC_TESTNET, script, key);
            }
            Self::BitcoinCash => btc::encode_p2pkh(bch::BCH_MAINNET_VERSION, &key.serialize()),
            Self::BitcoinCashTestnet => {
                btc::encode_p2pkh(bch::BCH_TESTNET_VERSION, &key.serialize())
            }
            Self::BitcoinSV | Self::BitcoinSVTestnet => {
                btc::encode_p2pkh(self.fixed_utxo_address_versions()?.0, &key.serialize())
            }
            Self::Dogecoin => btc::encode_p2pkh(doge::DOGE_MAINNET_VERSION, &key.serialize()),
            Self::DogecoinTestnet => {
                btc::encode_p2pkh(doge::DOGE_TESTNET_VERSION, &key.serialize())
            }
            Self::Litecoin | Self::LitecoinTestnet => {
                return ltc::encode_litecoin_address(self, script, key);
            }
            Self::Peercoin | Self::PeercoinTestnet => {
                return crate::derivation::peercoin::encode_peercoin_address(self, script, key);
            }
            Self::BitcoinGold | Self::Dash | Self::DashTestnet => {
                btc::encode_p2pkh(self.fixed_utxo_address_versions()?.0, &key.serialize())
            }
            Self::Zcash | Self::ZcashTestnet => {
                let version = if self.is_testnet() {
                    [0x1d, 0x25]
                } else {
                    [0x1c, 0xb8]
                };
                let mut payload = version.to_vec();
                payload.extend(btc::hash160(&key.serialize()));
                btc::base58check_encode(&payload)
            }
            Self::Decred | Self::DecredTestnet => crate::derivation::decred::encode_decred_p2pkh(
                self,
                &crate::derivation::decred::dcr_hash160(&key.serialize()),
            )?,
            Self::Kaspa | Self::KaspaTestnet => {
                let hrp = if self.is_testnet() {
                    crate::derivation::kaspa::KASPA_TESTNET_HRP
                } else {
                    crate::derivation::kaspa::KASPA_HRP
                };
                return crate::derivation::kaspa::encode_kaspa_address(
                    0,
                    &key.x_only_public_key().0.serialize(),
                    hrp,
                );
            }
            _ => {
                return Err(DerivationError::invalid(
                    "chain does not support UTXO discovery",
                ));
            }
        })
    }

    /// The rust-bitcoin network a Bitcoin chain is; `None` off Bitcoin.
    pub(crate) fn bitcoin_network(self) -> Option<bitcoin::Network> {
        match self {
            Chain::Bitcoin => Some(bitcoin::Network::Bitcoin),
            Chain::BitcoinTestnet => Some(bitcoin::Network::Testnet),
            Chain::BitcoinTestnet4 => Some(bitcoin::Network::Testnet4),
            Chain::BitcoinSignet => Some(bitcoin::Network::Signet),
            _ => None,
        }
    }

    /// The script a UTXO multisig account pays its `sortedmulti` through:
    /// P2WSH on Bitcoin and Litecoin, P2SH on Bitcoin Cash and Dogecoin,
    /// which have no SegWit.
    pub(crate) fn utxo_multisig_script(
        self,
    ) -> Option<crate::derivation::multisig::UtxoMultisigScript> {
        use crate::derivation::multisig::UtxoMultisigScript;
        match self.mainnet_counterpart() {
            Self::Bitcoin | Self::Litecoin => Some(UtxoMultisigScript::Wsh),
            Self::BitcoinCash | Self::Dogecoin => Some(UtxoMultisigScript::Sh),
            _ => None,
        }
    }
}
