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
            // Qtum Core chainparams: `Q…` and `M…`.
            Self::Qtum => Ok((0x3a, &[0x32])),
            // Fractal's chainparams keep Bitcoin's versions.
            Self::FractalBitcoin => Ok((0x00, &[0x05])),
            // DigiByte Core: `D…`, `S…`, and the `3…` P2SH its nodes still
            // decode as SCRIPT_ADDRESS_OLD.
            Self::Digibyte => Ok((0x1e, &[0x3f, 0x05])),
            _ => Err(self.not_in("Base58 UTXO")),
        }
    }

    /// The fork id a network's SIGHASH_FORKID signatures carry: Bitcoin
    /// Gold's 79, and 0 on Bitcoin Cash, Bitcoin SV and eCash, whose replay
    /// protection is the flag alone.
    pub(crate) fn sighash_fork_id(self) -> Result<u32, RegistryError> {
        match self.mainnet_counterpart() {
            Self::BitcoinCash | Self::BitcoinSV | Self::Ecash => Ok(0),
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
                | Self::Qtum
                | Self::FractalBitcoin
                | Self::Ecash
                | Self::Digibyte
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

    /// The confirmations a coinbase or coinstake output needs before it can
    /// be spent, where an indexer lists immature ones among an address's
    /// spendable outputs: Peercoin's, Qtum's 2,000 (Qtum Core's
    /// `nRBTCoinbaseMaturity` since its block time was cut to 32 seconds at
    /// height 845,000) and eCash's 100 (Bitcoin ABC's `COINBASE_MATURITY`).
    pub(crate) fn generated_output_maturity(self) -> Result<u32, RegistryError> {
        match self {
            Self::Peercoin => Ok(500),
            Self::PeercoinTestnet => Ok(60),
            Self::Qtum => Ok(2_000),
            Self::Ecash => Ok(100),
            _ => Err(self.not_in("maturing-reward UTXO")),
        }
    }

    /// Whether a send reads each output from its hash-verified transaction:
    /// on Peercoin and Qtum, whose staking rewards mature and may pay a key
    /// rather than an address.
    pub(crate) fn verifies_spent_outputs(self) -> bool {
        matches!(self.mainnet_counterpart(), Self::Peercoin | Self::Qtum)
    }

    /// The hash of the network's genesis block, as an indexer names the
    /// block at height 0.
    pub(crate) fn genesis_block_hash(self) -> Result<String, RegistryError> {
        match self {
            Self::Litecoin => {
                Ok("12a765e31ffd4059bada1e25190f6e98c99d9714d334efa41a195a7e7e04bfe2".into())
            }
            Self::LitecoinTestnet => {
                Ok("4966625a4b2851d9fdee139e56211a0d88575f59ed816ff5e6a63deb4e3e29a0".into())
            }
            Self::Zcash | Self::ZcashTestnet => self.zcash_genesis().map(str::to_string),
            // Qtum Core's and DigiByte Core's chainparams.
            Self::Qtum => {
                Ok("000075aef83cf2853580f8ae8ce6f8c3096cfa21d98334d6e3f95e5582ed986c".into())
            }
            Self::Digibyte => {
                Ok("7497ea1b465eb39f1c8f507bc877078fe016d6fcb6dfad3a64c98dcc6e1e8496".into())
            }
            _ => self
                .bitcoin_network()
                .map(|network| {
                    bitcoin::blockdata::constants::genesis_block(network)
                        .block_hash()
                        .to_string()
                })
                .ok_or_else(|| self.not_in("Bitcoin, Litecoin or Zcash")),
        }
    }

    /// The block whose hash tells this network apart from every other an
    /// indexer of its API could serve, as `(height, hash)`: what a UTXO
    /// indexer is checked against before core broadcasts through it, and
    /// before a send on a network that shares another's addresses signs.
    ///
    /// Usually the genesis block. Fractal Bitcoin and eCash start from
    /// Bitcoin's genesis block, so theirs is a later one: Fractal's block 1
    /// (its chainparams' `BIP34Hash`) and eCash's block 896,800 (Bitcoin
    /// ABC's assumeutxo snapshot, after its 2020 split from Bitcoin Cash).
    /// Bitcoin's own is its block 1, so a Fractal indexer, which names
    /// Bitcoin's genesis block, is not taken for a Bitcoin one.
    pub(crate) fn identity_block(self) -> Result<(u64, String), RegistryError> {
        Ok(match self {
            Self::Bitcoin => (
                1,
                "00000000839a8e6886ab5951d76f411475428afc90947ee320161bbf18eb6048".into(),
            ),
            Self::FractalBitcoin => (
                1,
                "00000000000000005a5c13fe33f6717c7ad81fc8837ae75e4693c16acbdd0f66".into(),
            ),
            Self::Ecash => (
                896_800,
                "0000000000000000297efb200794348b44bff4bfb31716cf64dc45bac0a251ea".into(),
            ),
            _ => (0, self.genesis_block_hash()?),
        })
    }

    /// Whether a send on this network checks, before it signs, that the
    /// indexers it read its inputs from name the network's
    /// [`Chain::identity_block`]: Fractal Bitcoin's addresses are Bitcoin's
    /// and its keys the same, so a Bitcoin indexer configured for it would
    /// hand over Bitcoin outputs the wallet's keys also spend.
    pub(crate) fn verifies_indexer_before_signing(self) -> bool {
        self.mainnet_counterpart() == Self::FractalBitcoin
    }

    /// The lowest fee rate, in sat/vB, a send on the network pays, whatever
    /// an indexer estimates: Qtum Core's and DigiByte Core's
    /// `DEFAULT_MIN_RELAY_TX_FEE` (400,000 and 100,000 per virtual kilobyte),
    /// and 1 on Fractal, the least its mempool recommends.
    pub(crate) fn min_fee_rate(self) -> Option<u64> {
        match self.mainnet_counterpart() {
            Self::Qtum => Some(400),
            Self::Digibyte => Some(100),
            Self::FractalBitcoin => Some(1),
            _ => None,
        }
    }

    /// Whether a send on the network is priced at its indexers' fee rate:
    /// Bitcoin's, Fractal's and DigiByte's, and Litecoin's MWEB quote. Every
    /// other UTXO network pays the registry's fee, Qtum at
    /// [`Chain::min_fee_rate`] since its Blockbook estimates zero.
    pub(crate) fn reads_indexer_fee_rate(self) -> bool {
        matches!(
            self.mainnet_counterpart(),
            Self::Bitcoin | Self::FractalBitcoin | Self::Digibyte | Self::Litecoin
        )
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
            Self::Qtum => Some("qc"),
            Self::FractalBitcoin => Some("bc"),
            Self::Digibyte => Some("dgb"),
            _ => None,
        }
    }

    /// Zcash's two-byte transparent P2PKH version (`t1…` on mainnet, `tm…` on
    /// testnet), from zcashd's chainparams.
    pub(crate) fn zcash_transparent_p2pkh_version(self) -> Result<[u8; 2], RegistryError> {
        match self {
            Self::Zcash => Ok([0x1c, 0xb8]),
            Self::ZcashTestnet => Ok([0x1d, 0x25]),
            _ => Err(self.not_in("Zcash")),
        }
    }

    /// The human-readable part of a Kaspa address (`kaspa:`, `kaspatest:`).
    pub(crate) fn kaspa_address_hrp(self) -> Result<&'static str, RegistryError> {
        match self {
            Self::Kaspa => Ok("kaspa"),
            Self::KaspaTestnet => Ok("kaspatest"),
            _ => Err(self.not_in("Kaspa")),
        }
    }

    pub(crate) fn cashaddr_prefix(self) -> Option<&'static str> {
        match self {
            Self::BitcoinCash => Some("bitcoincash"),
            Self::BitcoinCashTestnet => Some("bchtest"),
            Self::Ecash => Some("ecash"),
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
            // Fractal enforces Taproot from its block 1; DigiByte buried its
            // activation at height 21,168,000.
            Self::FractalBitcoin | Self::Digibyte => {
                (version == 0 && matches!(program_length, 20 | 32))
                    || (version == 1 && program_length == 32)
            }
            // BTG's Taproot deployment is not an established active consensus
            // rule, nor has Qtum's BIP9 deployment been seen active.
            Self::BitcoinGold | Self::Qtum => version == 0 && matches!(program_length, 20 | 32),
            _ => false,
        }
    }

    /// The smallest output an account UTXO network relays, in its smallest
    /// unit: a recipient amount below it is refused, and change below it
    /// joins the fee. Where the node's dust rule is a relay rate, the
    /// threshold of a P2PKH output, which takes 182 bytes to create and
    /// spend: 546 at Bitcoin Core's 3 sat/vB (kept by Bitcoin ABC and
    /// Fractal), 5,460 at DigiByte Core's 30 and 72,800 at Qtum Core's 400.
    /// Dogecoin Core's is its dust limit of 0.01 DOGE (policy.h
    /// `DEFAULT_DUST_LIMIT`), below which an output pays that much again in
    /// fee and, below a tenth of it, is not relayed at all.
    pub(crate) fn relay_dust_threshold(self) -> Result<u64, RegistryError> {
        match self.mainnet_counterpart() {
            Self::Bitcoin
            | Self::Zcash
            | Self::BitcoinCash
            | Self::BitcoinSV
            | Self::BitcoinGold
            | Self::Dash
            | Self::Ecash
            | Self::FractalBitcoin => Ok(546),
            Self::Digibyte => Ok(5_460),
            Self::Qtum => Ok(72_800),
            Self::Dogecoin => Ok(1_000_000),
            Self::Decred => Ok(6_030),
            Self::Kaspa => Ok(1_000),
            _ => Err(self.not_in("account UTXO")),
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
        use crate::derivation::{bitcoin as btc, litecoin as ltc};
        Ok(match self {
            Self::Bitcoin => return btc::encode_address_inner(btc::BTC_MAINNET, script, key),
            Self::BitcoinTestnet | Self::BitcoinTestnet4 | Self::BitcoinSignet => {
                return btc::encode_address_inner(btc::BTC_TESTNET, script, key);
            }
            Self::BitcoinCash
            | Self::BitcoinCashTestnet
            | Self::BitcoinSV
            | Self::BitcoinSVTestnet
            | Self::Dogecoin
            | Self::DogecoinTestnet
            | Self::BitcoinGold
            | Self::Dash
            | Self::DashTestnet => {
                btc::encode_p2pkh(self.fixed_utxo_address_versions()?.0, &key.serialize())
            }
            Self::Litecoin | Self::LitecoinTestnet => {
                return ltc::encode_litecoin_address(self, script, key);
            }
            Self::Peercoin | Self::PeercoinTestnet => {
                return crate::derivation::peercoin::encode_peercoin_address(self, script, key);
            }
            Self::Qtum | Self::FractalBitcoin | Self::Digibyte => {
                return btc::encode_address_inner(self.segwit_network_params()?, script, key);
            }
            Self::Ecash => crate::derivation::utxo_address::encode_cashaddr(
                self,
                &crate::derivation::utxo_address::ParsedUtxoAddress::P2pkh(btc::hash160(
                    &key.serialize(),
                )),
            )?,
            Self::Zcash | Self::ZcashTestnet => crate::derivation::zcash::zcash_p2pkh_addr(
                self.zcash_transparent_p2pkh_version()?,
                key,
            ),
            Self::Decred | Self::DecredTestnet => crate::derivation::decred::encode_decred_p2pkh(
                self,
                &crate::derivation::decred::dcr_hash160(&key.serialize()),
            )?,
            Self::Kaspa | Self::KaspaTestnet => {
                return crate::derivation::kaspa::encode_kaspa_address(
                    0,
                    &key.x_only_public_key().0.serialize(),
                    self.kaspa_address_hrp()?,
                );
            }
            _ => {
                return Err(DerivationError::invalid(
                    "chain does not support UTXO discovery",
                ));
            }
        })
    }

    /// The Base58 versions and SegWit prefix of a network that kept
    /// Bitcoin's address formats.
    pub(crate) fn segwit_network_params(
        self,
    ) -> Result<crate::derivation::bitcoin::BitcoinNetworkParams, RegistryError> {
        let (p2pkh_version, p2sh_versions) = self.fixed_utxo_address_versions()?;
        Ok(crate::derivation::bitcoin::BitcoinNetworkParams {
            p2pkh_version,
            p2sh_version: p2sh_versions[0],
            bech32_hrp: self
                .fixed_utxo_segwit_hrp()
                .ok_or_else(|| self.not_in("SegWit UTXO"))?,
        })
    }

    /// Whether a wallet account on this network spends from its addresses
    /// of witness `version`: native and nested P2WPKH for 0, a Taproot key
    /// path for 1. Its account signer signs them, and the network has them.
    pub(crate) fn account_spends_witness(self, version: u8) -> bool {
        match self.mainnet_counterpart() {
            Self::Bitcoin | Self::Peercoin | Self::FractalBitcoin | Self::Digibyte => version <= 1,
            Self::Litecoin | Self::Qtum => version == 0,
            _ => false,
        }
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
