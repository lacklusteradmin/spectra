//! Keys, derivation paths and the secrets a wallet is imported from.

use super::*;

impl Chain {
    pub fn supports_derivation_passphrase(self) -> bool {
        self.mainnet_counterpart() != Self::Monero
    }
    pub fn supports_derivation_hmac_override(self) -> bool {
        matches!(
            self.mainnet_counterpart(),
            Self::Solana | Self::Stellar | Self::Polkadot
        )
    }

    /// Named accounts authorize keys on chain instead of encoding the key in their address.
    pub fn supports_named_sender_accounts(self) -> bool {
        matches!(self, Self::Near | Self::NearTestnet)
    }

    /// Key under which this chain's address is stored during wallet import.
    ///
    /// Most chains own their address, so the slot is just [`Chain::str_id`].
    /// The exceptions are the EVM family: one derived secp256k1 address serves
    /// Ethereum, Arbitrum, Base and the rest, so they all share the
    /// `"ethereum"` slot rather than each carrying a copy.
    ///
    /// The slot describes address encoding, not network or derivation identity.
    /// Every EVM network, including Ethereum Classic and testnets, uses it;
    /// each wallet separately records its concrete network and derivation path.
    /// Other networks keep separate slots when their address encoding differs.
    pub fn address_slot(self) -> &'static str {
        match self {
            _ if self.is_evm() => Chain::Ethereum.str_id(),
            _ => self.str_id(),
        }
    }

    /// The signature scheme a raw key on this chain is a secret of, or `None`
    /// where a key alone yields no address. Two chains with one scheme read
    /// the same key as the same secret; across schemes the same bytes are a
    /// different key, which a wallet's key must not become.
    pub fn key_scheme(self) -> Option<KeyScheme> {
        if !self.derives_from_private_key() {
            return None;
        }
        Some(match self.mainnet_counterpart() {
            Self::Cardano => KeyScheme::Ed25519Extended,
            Self::Polkadot | Self::Bittensor => KeyScheme::Sr25519,
            Self::Solana | Self::Stellar | Self::Sui | Self::Aptos | Self::Ton | Self::Near => {
                KeyScheme::Ed25519
            }
            _ => KeyScheme::Secp256k1,
        })
    }

    /// Whether a raw private key yields an address on this chain.
    /// Shared by derivation and import eligibility. Testnets follow their
    /// mainnet; derivation handles network-specific address encoding.
    pub fn derives_from_private_key(self) -> bool {
        self.mainnet_counterpart() != Self::Monero
    }

    /// The private-key encodings an import on this chain accepts, or none
    /// where a key alone yields no address.
    pub fn private_key_formats(self) -> Vec<crate::derivation::setup::WalletSecretFormat> {
        use crate::derivation::setup::WalletSecretFormat as F;
        if !self.derives_from_private_key() {
            return Vec::new();
        }
        let native = match self.mainnet_counterpart() {
            Self::Cardano => return vec![F::CardanoExtendedKey],
            _ if self.wif_version().is_some() => Some(F::Wif),
            Self::Solana => Some(F::SolanaKeypair),
            Self::Stellar => Some(F::StellarSecretSeed),
            Self::Sui => Some(F::SuiPrivateKey),
            Self::Aptos => Some(F::AptosPrivateKey),
            Self::Near => Some(F::NearSecretKey),
            _ => None,
        };
        std::iter::once(F::HexSecret32).chain(native).collect()
    }

    /// The version byte a Wallet Import Format key carries on this network,
    /// for the Base58Check-WIF chains: the Bitcoin family's own, and `0xEF`
    /// (Dogecoin `0xF1`) on their test networks.
    pub fn wif_version(self) -> Option<u8> {
        let mainnet = match self.mainnet_counterpart() {
            Self::Bitcoin
            | Self::BitcoinCash
            | Self::BitcoinSV
            | Self::BitcoinGold
            | Self::Zcash => 0x80,
            Self::Litecoin => 0xb0,
            Self::Dogecoin => 0x9e,
            Self::Dash => 0xcc,
            Self::Peercoin => 0xb7,
            _ => return None,
        };
        Some(match (self.is_testnet(), self.mainnet_counterpart()) {
            (false, _) => mainnet,
            (true, Self::Dogecoin) => 0xf1,
            (true, _) => 0xef,
        })
    }

    /// The phrase encodings an import on this chain restores from: the ones
    /// its own wallets write. Monero and TON wallets do not read BIP-39.
    pub fn phrase_formats(self) -> Vec<crate::derivation::setup::WalletSecretFormat> {
        use crate::derivation::setup::WalletSecretFormat as F;
        match self.mainnet_counterpart() {
            Self::Monero => vec![F::MoneroPhrase, F::Polyseed],
            Self::Ton => vec![F::TonMnemonic],
            _ => vec![F::Bip39Phrase],
        }
    }

    /// The phrase encoding a wallet created on this chain is generated in:
    /// one every wallet for the chain restores. For Monero that is the
    /// 25-word seed, which older wallets read and Polyseed's do not replace.
    pub fn created_phrase_format(self) -> crate::derivation::setup::WalletSecretFormat {
        use crate::derivation::setup::WalletSecretFormat as F;
        match self.mainnet_counterpart() {
            Self::Monero => F::MoneroPhrase,
            Self::Ton => F::TonMnemonic,
            _ => F::Bip39Phrase,
        }
    }

    /// Whether this chain's derivation reads a BIP-32 path.
    ///
    /// Read from the catalog rather than restated: `derivation_path = []` is
    /// how a row says its keys do not come from a BIP-32 path. Monero's spend
    /// and view keys come from the seed directly, TON's from its mnemonic,
    /// and Polkadot's and Bittensor's from Substrate junctions instead
    /// (`derives_along_junctions`).
    ///
    /// For those "no path" is the answer, not an error. See
    /// `default_path_from_catalog`.
    pub fn uses_derivation_path(self) -> bool {
        crate::catalog::chains::default_derivation_path_template(self).is_some()
    }

    /// Whether a phrase wallet on this chain derives along a Substrate path
    /// of hard (`//`) and soft (`/`) junctions under its sr25519 root key,
    /// as subkey and polkadot.js do (`derivation::substrate_path`). No path
    /// is the root key; there are no profiles or account indices.
    pub fn derives_along_junctions(self) -> bool {
        self.key_scheme() == Some(KeyScheme::Sr25519)
    }

    /// The derivation profiles a phrase wallet on this chain can use, the
    /// default first. Empty where the chain derives without a path (Monero,
    /// TON, Polkadot, Bittensor), which also means it has no account index.
    pub fn derivation_profiles(self) -> Vec<crate::catalog::chains::DerivationProfile> {
        let entries = &self.entry().derivation_path;
        entries
            .iter()
            .filter(|entry| entry.is_default)
            .chain(entries.iter().filter(|entry| !entry.is_default))
            .map(|entry| entry.profile)
            .collect()
    }

    /// The path `profile` derives at account `account` on this chain, or
    /// `None` when the chain does not offer the profile or the index does not
    /// fit a hardened segment.
    pub fn derivation_profile_path(
        self,
        profile: crate::catalog::chains::DerivationProfile,
        account: u32,
    ) -> Option<String> {
        if account >= 1 << 31 {
            return None;
        }
        self.entry()
            .derivation_path
            .iter()
            .find(|entry| entry.profile == profile)
            .map(|entry| entry.path.replace("{account}", &account.to_string()))
    }

    /// Whether a key holds one account per wallet contract version (TON's
    /// W5 and v4R2), so a wallet is a key and the version it signs as.
    pub fn has_wallet_versions(self) -> bool {
        self.mainnet_counterpart() == Self::Ton
    }

    /// Whether a phrase wallet's address names a key besides the one it
    /// signs with: Cardano's base address carries its account's stake key,
    /// so the payment key alone imports as another (enterprise) address.
    pub(crate) fn phrase_address_has_stake_key(self) -> bool {
        self.mainnet_counterpart() == Self::Cardano
    }

    /// `true` when an account's extended public key can be watched on this
    /// chain, standing in for the whole account as one wallet rather than
    /// one per address: every account UTXO network, each in the encodings
    /// `account_key_versions` lists.
    pub fn accepts_account_xpub(self) -> bool {
        !self.account_key_versions().is_empty()
    }

    /// The form a multisig account's policy takes on this chain, where its
    /// address derives from one: a UTXO network's `sortedmulti` descriptor,
    /// Sui's multisig public key, Aptos's MultiKey, Cardano's native script,
    /// a Substrate multisig's signatories.
    pub fn multisig_policy_format(self) -> Option<crate::derivation::setup::WalletSecretFormat> {
        use crate::derivation::setup::WalletSecretFormat;
        if self.utxo_multisig_script().is_some() {
            return Some(WalletSecretFormat::MultisigDescriptor);
        }
        match self.mainnet_counterpart() {
            Self::Sui => Some(WalletSecretFormat::SuiMultisigPublicKey),
            Self::Aptos => Some(WalletSecretFormat::AptosMultiKey),
            Self::Cardano => Some(WalletSecretFormat::CardanoNativeScript),
            Self::Polkadot | Self::Bittensor => Some(WalletSecretFormat::SubstrateMultisig),
            _ => None,
        }
    }

    /// `true` when a wallet on this chain can be watched from its address
    /// and private view key, scanning what it receives without its spend
    /// key: Monero's view-only wallet.
    pub fn watches_with_view_key(self) -> bool {
        self.mainnet_counterpart() == Chain::Monero
    }

    /// The prefixes an account public key starts with on this network, in
    /// `account_key_versions` order: what a front end names in its prompt.
    pub fn account_key_prefixes(self) -> Vec<&'static str> {
        self.account_key_versions()
            .iter()
            .map(|version| version.prefix)
            .collect()
    }

    /// The encodings an account public key is read in on this network, the
    /// one its own wallets export first. Each names the script the account's
    /// addresses pay. From SLIP-132 (Bitcoin, Litecoin), Trezor's coin
    /// definitions (Litecoin's zpub, Dogecoin's dgub, Dash's drkp, Decred's
    /// dpub and its own tpub, and the xpub of Bitcoin Cash, Bitcoin Gold,
    /// Peercoin and Zcash), the nodes' own chain parameters (Dash Core's
    /// xpub, Dogecoin Core's testnet tpub), ElectrumSV's xpub on Bitcoin SV
    /// and rusty-kaspa's kpub and ktub. A Taproot account has no version of
    /// its own, nor do the SegWit accounts of a network Spectra signs only
    /// P2PKH on.
    pub(crate) fn account_key_versions(self) -> &'static [AccountKeyVersion] {
        use crate::derivation::types::BitcoinScriptType::{P2pkh, P2shP2wpkh, P2wpkh};
        const fn v(
            version: u32,
            prefix: &'static str,
            script: crate::derivation::types::BitcoinScriptType,
        ) -> AccountKeyVersion {
            AccountKeyVersion {
                version: version.to_be_bytes(),
                prefix,
                script,
            }
        }
        const XPUB: AccountKeyVersion = v(0x0488_b21e, "xpub", P2pkh);
        const YPUB: AccountKeyVersion = v(0x049d_7cb2, "ypub", P2shP2wpkh);
        const ZPUB: AccountKeyVersion = v(0x04b2_4746, "zpub", P2wpkh);
        const TPUB: AccountKeyVersion = v(0x0435_87cf, "tpub", P2pkh);
        const UPUB: AccountKeyVersion = v(0x044a_5262, "upub", P2shP2wpkh);
        const VPUB: AccountKeyVersion = v(0x045f_1cf6, "vpub", P2wpkh);
        const LTUB: AccountKeyVersion = v(0x019d_a462, "Ltub", P2pkh);
        const MTUB: AccountKeyVersion = v(0x01b2_6ef6, "Mtub", P2shP2wpkh);
        const TTUB: AccountKeyVersion = v(0x0436_f6e1, "ttub", P2pkh);
        const DRKP: AccountKeyVersion = v(0x02fe_52cc, "drkp", P2pkh);
        const DGUB: AccountKeyVersion = v(0x02fa_cafd, "dgub", P2pkh);
        const DPUB: AccountKeyVersion = v(0x02fd_a926, "dpub", P2pkh);
        const DECRED_TPUB: AccountKeyVersion = v(0x0435_87d1, "tpub", P2pkh);
        const KPUB: AccountKeyVersion = v(0x038f_332e, "kpub", P2pkh);
        const KTUB: AccountKeyVersion = v(0x0390_a241, "ktub", P2pkh);
        match self {
            Self::Bitcoin | Self::Peercoin => &[XPUB, YPUB, ZPUB],
            Self::BitcoinTestnet
            | Self::BitcoinTestnet4
            | Self::BitcoinSignet
            | Self::PeercoinTestnet => &[TPUB, UPUB, VPUB],
            Self::Litecoin => &[LTUB, MTUB, ZPUB],
            Self::LitecoinTestnet => &[TPUB, UPUB, VPUB, TTUB],
            Self::BitcoinCash | Self::BitcoinSV | Self::BitcoinGold | Self::Zcash => &[XPUB],
            Self::Dash => &[XPUB, DRKP],
            Self::Dogecoin => &[DGUB],
            Self::Decred => &[DPUB],
            Self::DecredTestnet => &[DECRED_TPUB],
            Self::Kaspa => &[KPUB],
            Self::KaspaTestnet => &[KTUB],
            Self::BitcoinCashTestnet
            | Self::BitcoinSVTestnet
            | Self::ZcashTestnet
            | Self::DashTestnet
            | Self::DogecoinTestnet => &[TPUB],
            _ => &[],
        }
    }

    /// Whether a message is proven for a SegWit or Taproot address with a
    /// BIP-322 signature: Bitcoin's networks, whose verifiers read it.
    pub(crate) fn proves_with_bip322(self) -> bool {
        self.mainnet_counterpart() == Self::Bitcoin
    }

    /// `true` when a wallet on this chain can be imported watch-only from an
    /// address alone.
    ///
    /// Monero is the notable exclusion: watching a Monero account needs the
    /// private view key, which an address does not carry. Addresses are
    /// validated and stored against their concrete mainnet or testnet.
    pub fn supports_watch_only_import(self) -> bool {
        if self.is_evm() {
            return true;
        }
        matches!(
            self.mainnet_counterpart(),
            Chain::Bitcoin
                | Chain::BitcoinCash
                | Chain::BitcoinSV
                | Chain::Litecoin
                | Chain::Dogecoin
                | Chain::Tron
                | Chain::Solana
                | Chain::Xrp
                | Chain::Stellar
                | Chain::Cardano
                | Chain::Sui
                | Chain::Aptos
                | Chain::Ton
                | Chain::Icp
                | Chain::Near
                | Chain::Polkadot
                | Chain::Zcash
                | Chain::BitcoinGold
                | Chain::Decred
                | Chain::Kaspa
                | Chain::Dash
                | Chain::Peercoin
                | Chain::Bittensor
        )
    }
}
