use zeroize::Zeroizing;

// ── Output types ─────────────────────────────────────────────────────────────

/// Which of a derivation's outputs a caller wants. The others come back
/// `None`, and a private key no caller asked for is never encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wants {
    pub address: bool,
    pub public_key: bool,
    pub private_key: bool,
}

impl Wants {
    pub const ADDRESS: Self = Self {
        address: true,
        public_key: false,
        private_key: false,
    };
    pub const PUBLIC_KEY: Self = Self {
        address: false,
        public_key: true,
        private_key: false,
    };
    pub const PRIVATE_KEY: Self = Self {
        address: false,
        public_key: false,
        private_key: true,
    };
    pub const ALL: Self = Self {
        address: true,
        public_key: true,
        private_key: true,
    };

    /// `key` in hex, when the public key is wanted.
    pub(crate) fn public_key_hex(self, key: &[u8]) -> Option<String> {
        self.public_key.then(|| hex::encode(key))
    }

    /// `key` in hex, when the private key is wanted.
    pub(crate) fn private_key_hex(self, key: &[u8]) -> Option<Zeroizing<String>> {
        self.private_key.then(|| Zeroizing::new(hex::encode(key)))
    }
}

/// What a derivation returns: the outputs its [`Wants`] named.
pub struct DerivationResult {
    pub address: Option<String>,
    pub public_key_hex: Option<String>,
    /// Wiped when dropped.
    pub private_key_hex: Option<Zeroizing<String>>,
}

// ── Script type (UTXO chains) ─────────────────────────────────────────────────

/// Address encoding format for UTXO chains. Fully independent of the
/// derivation path — any path can be paired with any script type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitcoinScriptType {
    /// Legacy P2PKH (pay-to-public-key-hash). Base58Check, prefix `1`.
    P2pkh,
    /// Nested SegWit P2SH-P2WPKH. Base58Check, prefix `3`.
    P2shP2wpkh,
    /// Native SegWit P2WPKH. Bech32, prefix `bc1q`.
    P2wpkh,
    /// Taproot P2TR. Bech32m, prefix `bc1p`.
    P2tr,
}
