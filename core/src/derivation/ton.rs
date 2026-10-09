//! TON address decode + validation + derivation
//!
//! - `parse_ton_address`: parse raw or
//!   base64url user-friendly addresses.
//! - `derive_ton_seed`: TON's own 24-word mnemonic (ton-crypto / Tonkeeper /
//!   Tonhub), checked before it is expanded.
//! - `TonWalletVersion`: the wallet contracts a key can hold an account
//!   under. Each version's account id is the hash of its embedded code BOC
//!   and a freshly built data cell; the code is checked against its
//!   published hash before any address is computed from it.

use crate::derivation::error::DerivationError;
use crate::registry::Chain;

use super::ton_cell::Cell;
use ed25519_dalek::SigningKey;
use zeroize::Zeroizing;

/// A checked TON address retains routing flags until the send is encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TonAddress {
    pub workchain: i8,
    pub account_id: [u8; 32],
    pub bounceable: bool,
    pub test_only: bool,
}

pub(crate) fn parse_ton_address(address: &str) -> Result<TonAddress, DerivationError> {
    let (workchain, account_id, bounceable, test_only) =
        if let Some((wc, hash)) = address.split_once(':') {
            if !matches!(wc, "0" | "-1") || hash.len() != 64 {
                return Err(DerivationError::Invalid("TON: invalid raw address".into()));
            }
            let account_id: [u8; 32] = hex::decode(hash)
                .map_err(|_| DerivationError::Invalid("TON: invalid account id".into()))?
                .try_into()
                .map_err(|_| DerivationError::Invalid("TON: invalid account id length".into()))?;
            (
                wc.parse::<i8>()
                    .map_err(|_| DerivationError::Invalid("TON: invalid workchain".into()))?,
                account_id,
                false,
                false,
            )
        } else {
            use base64::Engine;
            if address.len() != 48 {
                return Err(DerivationError::Invalid(
                    "TON: friendly address must be 48 characters".into(),
                ));
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(address.replace('-', "+").replace('_', "/"))
                .map_err(|_| DerivationError::Invalid("TON: invalid base64 address".into()))?;
            if bytes.len() != 36 || crc16_xmodem(&bytes[..34]).to_be_bytes() != bytes[34..] {
                return Err(DerivationError::Invalid(
                    "TON: invalid address checksum".into(),
                ));
            }
            let tag = bytes[0] & 0x7f;
            if !matches!(tag, 0x11 | 0x51) || !matches!(bytes[1], 0 | 255) {
                return Err(DerivationError::Invalid(
                    "TON: invalid address flags or workchain".into(),
                ));
            }
            (
                bytes[1] as i8,
                bytes[2..34]
                    .try_into()
                    .map_err(|_| DerivationError::Invalid("TON: invalid address".into()))?,
                tag == 0x11,
                bytes[0] & 0x80 != 0,
            )
        };
    Ok(TonAddress {
        workchain,
        account_id,
        bounceable,
        test_only,
    })
}

impl TonAddress {
    pub(crate) fn for_network(self, testnet: bool) -> Result<Self, DerivationError> {
        if self.test_only && !testnet {
            return Err(DerivationError::Invalid(
                "TON: testnet-only address refused on mainnet".into(),
            ));
        }
        Ok(self)
    }
}

// ── TON mnemonic ─────────────────────────────────────────────────────────
//
// TON wallets (Tonkeeper, Tonhub, ton-crypto) use their own 24-word
// mnemonic, drawn from the BIP-39 English list but not BIP-39: there is no
// checksum word. A phrase is a TON mnemonic when PBKDF2 over its entropy
// starts with a zero byte; a password-protected one, when a one-round PBKDF2
// with another salt starts with a one. Every rule here is ton-crypto 3.3.0's
// (`mnemonicValidate`, `mnemonicNew`, `mnemonicToSeed`).

/// The length of every TON mnemonic.
pub(crate) const TON_MNEMONIC_WORDS: usize = 24;
const TON_PBKDF_ITERATIONS: u32 = 100_000;

/// HMAC-SHA512 keyed by the phrase, over the password.
fn ton_entropy(mnemonic: &str, password: &str) -> Result<Zeroizing<[u8; 64]>, DerivationError> {
    hmac_sha512(mnemonic.as_bytes(), &[password.as_bytes()])
}

fn pbkdf2_first_byte(entropy: &[u8; 64], salt: &str, iterations: u32) -> u8 {
    let mut seed = Zeroizing::new([0u8; 64]);
    crate::kdf::pbkdf2_sha512(entropy, salt.as_bytes(), iterations, &mut *seed);
    seed[0]
}

fn is_basic_seed(entropy: &[u8; 64]) -> bool {
    pbkdf2_first_byte(
        entropy,
        "TON seed version",
        (TON_PBKDF_ITERATIONS / 256).max(1),
    ) == 0
}

fn is_password_seed(entropy: &[u8; 64]) -> bool {
    pbkdf2_first_byte(entropy, "TON fast seed version", 1) == 1
}

/// What kind of TON mnemonic 24 English-list words are, if any: one that
/// needs no password, or one that needs one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TonMnemonicKind {
    Basic,
    PasswordProtected,
}

pub(crate) fn ton_mnemonic_kind(
    mnemonic: &str,
) -> Result<Option<TonMnemonicKind>, DerivationError> {
    let entropy = ton_entropy(mnemonic, "")?;
    Ok(if is_basic_seed(&entropy) {
        Some(TonMnemonicKind::Basic)
    } else if is_password_seed(&entropy) {
        Some(TonMnemonicKind::PasswordProtected)
    } else {
        None
    })
}

/// Refuse anything a TON wallet would not restore: the wrong length, a word
/// outside the list, a phrase that is not a TON mnemonic, or a password that
/// does not open a password-protected one.
pub(crate) fn check_ton_mnemonic(mnemonic: &str, password: &str) -> Result<(), DerivationError> {
    let words: Vec<&str> = mnemonic.split_whitespace().collect();
    if words.len() != TON_MNEMONIC_WORDS {
        return Err(DerivationError::refused(
            "A TON mnemonic has 24 words, not %@.",
            [words.len()],
        ));
    }
    if let Some(word) = words
        .iter()
        .find(|word| bip39::Language::English.find_word(word).is_none())
    {
        return Err(DerivationError::refused(
            "%@ is not a TON mnemonic word.",
            [word.to_string()],
        ));
    }
    let mnemonic = words.join(" ");
    let valid = match ton_mnemonic_kind(&mnemonic)? {
        Some(TonMnemonicKind::Basic) => password.is_empty(),
        Some(TonMnemonicKind::PasswordProtected) => {
            !password.is_empty() && is_basic_seed(&*ton_entropy(&mnemonic, password)?)
        }
        None => false,
    };
    if valid {
        Ok(())
    } else {
        Err(DerivationError::invalid(
            "This is not a TON mnemonic, or its password does not match.",
        ))
    }
}

/// A new TON mnemonic that needs no password, as ton-crypto's `mnemonicNew`
/// makes one: random words until the phrase passes the TON check.
pub(crate) fn generate_ton_mnemonic() -> Result<Zeroizing<String>, DerivationError> {
    use rand::Rng;
    let list = bip39::Language::English.word_list();
    let mut rng = rand::thread_rng();
    loop {
        let mnemonic = Zeroizing::new(
            (0..TON_MNEMONIC_WORDS)
                .map(|_| list[rng.gen_range(0..list.len())])
                .collect::<Vec<_>>()
                .join(" "),
        );
        if ton_mnemonic_kind(&mnemonic)? == Some(TonMnemonicKind::Basic) {
            return Ok(mnemonic);
        }
    }
}

/// TON mnemonic → 64-byte seed: PBKDF2-HMAC-SHA512 over the entropy, salted
/// "TON default seed". The private key is the first 32 bytes.
pub(crate) fn derive_ton_seed(
    mnemonic: &str,
    password: &str,
) -> Result<Zeroizing<[u8; 64]>, DerivationError> {
    check_ton_mnemonic(mnemonic, password)?;
    let entropy = ton_entropy(mnemonic, password)?;
    let mut seed = Zeroizing::new([0u8; 64]);
    crate::kdf::pbkdf2_sha512(
        &*entropy,
        b"TON default seed",
        TON_PBKDF_ITERATIONS,
        &mut *seed,
    );
    Ok(seed)
}

// ── Wallet contract code (embedded BOCs) ─────────────────────────────────
// Each version's code, copied from @ton/ton's `WalletContractV4.js` and
// `WalletContractV5R1.js`. Decoding checks the recomputed root cell hash
// against the version's published code hash, so a damaged constant or a
// parser fault refuses rather than deriving a stranger's address.

const V4R2_CODE_BOC_HEX: &str = "b5ee9c7241021401000\
2d4000114ff00f4a413f4bcf2c80b010201200203020148040504f8f28308d71820\
d31fd31fd31f02f823bbf264ed44d0d31fd31fd3fff404d15143baf2a15151baf2a\
205f901541064f910f2a3f80024a4c8cb1f5240cb1f5230cbff5210f400c9ed54f8\
0f01d30721c0009f6c519320d74a96d307d402fb00e830e021c001e30021c002e30\
001c0039130e30d03a4c8cb1f12cb1fcbff1011121302e6d001d0d3032171b0925f\
04e022d749c120925f04e002d31f218210706c7567bd22821064737472bdb0925f0\
5e003fa403020fa4401c8ca07cbffc9d0ed44d0810140d721f404305c810108f40a\
6fa131b3925f07e005d33fc8258210706c7567ba923830e30d03821064737472ba9\
25f06e30d06070201200809007801fa00f40430f8276f2230500aa121bef2e05082\
10706c7567831eb17080185004cb0526cf1658fa0219f400cb6917cb1f5260cb3f2\
0c98040fb0006008a5004810108f45930ed44d0810140d720c801cf16f400c9ed54\
0172b08e23821064737472831eb17080185005cb055003cf1623fa0213cb6acb1fc\
b3fc98040fb00925f03e20201200a0b0059bd242b6f6a2684080a06b90fa0218470\
d4080847a4937d29910ce6903e9ff9837812801b7810148987159f31840201580c0\
d0011b8c97ed44d0d70b1f8003db29dfb513420405035c87d010c00b23281f2fff2\
74006040423d029be84c600201200e0f0019adce76a26840206b90eb85ffc00019a\
f1df6a26840106b90eb858fc0006ed207fa00d4d422f90005c8ca0715cbffc9d077\
748018c8cb05cb0222cf165005fa0214cb6b12ccccc973fb00c84014810108f451f\
2a7020070810108d718fa00d33fc8542047810108f451f2a782106e6f7465707480\
18c8cb05cb025006cf165004fa0214cb6a12cb1fcb3fc973fb0002006c810108d71\
8fa00d33f305224810108f459f2a782106473747270748018c8cb05cb025005cf16\
5003fa0213cb6acb1f12cb3fc973fb00000af400c9ed54696225e5";

const V4R2_CODE_HASH: &str = "feb5ff6820e2ff0d9483e7e0d62c817d846789fb4ae580c878866d959dabd5c0";

// From ton-blockchain/wallet-contract-v5's `wallet_v5.compiled.json`.
const W5_CODE_BOC_HEX: &str = "b5ee9c7241021401000\
281000114ff00f4a413f4bcf2c80b01020120020d020148030402dcd020d749c12\
0915b8f6320d70b1f2082106578746ebd21821073696e74bdb0925f03e08210657\
8746eba8eb48020d72101d074d721fa4030fa44f828fa443058bd915be0ed44d08\
10141d721f4058307f40e6fa1319130e18040d721707fdb3ce03120d749810280b\
99130e070e2100f020120050c020120060902016e07080019adce76a2684020eb9\
0eb85ffc00019af1df6a2684010eb90eb858fc00201480a0b0017b325fb51341c7\
5c875c2c7e00011b262fb513435c280200019be5f0f6a2684080a0eb90fa02c010\
2f20e011e20d70b1f82107369676ebaf2e08a7f0f01e68ef0eda2edfb218308d72\
2028308d723208020d721d31fd31fd31fed44d0d200d31f20d31fd3ffd70a000af\
90140ccf9109a28945f0adb31e1f2c087df02b35007b0f2d0845125baf2e085503\
6baf2e086f823bbf2d0882292f800de01a47fc8ca00cb1f01cf16c9ed542092f80\
fde70db3cd81003f6eda2edfb02f404216e926c218e4c0221d73930709421c700b\
38e2d01d72820761e436c20d749c008f2e09320d74ac002f2e09320d71d06c712c\
2005230b0f2d089d74cd7393001a4e86c128407bbf2e093d74ac000f2e093ed55e\
2d20001c000915be0ebd72c08142091709601d72c081c12e25210b1e30f20d74a1\
11213009601fa4001fa44f828fa443058baf2e091ed44d0810141d718f405049d7\
fc8ca0040048307f453f2e08b8e14038307f45bf2e08c22d70a00216e01b3b0f2d\
090e2c85003cf1612f400c9ed54007230d72c08248e2d21f2e092d200ed44d0d20\
05113baf2d08f54503091319c01810140d721d70a00f2e08ee2c8ca0058cf16c9e\
d5493f2c08de20010935bdb31e1d74cd0b4d6c35e";

const W5_CODE_HASH: &str = "20834b7b72b112147e1b2fb457b84e74d1a30f04f737d4f62a668e9552d2b72f";

/// The wallet contract a TON key holds an account under. One key has a
/// different account, and so a different address and balance, under each
/// version, and a wallet signs in the message format of its own.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Default,
    serde::Serialize,
    serde::Deserialize,
    uniffi::Enum,
)]
#[serde(rename_all = "camelCase")]
pub enum TonWalletVersion {
    /// Wallet v5r1, "W5": what current wallets (Tonkeeper, MyTonWallet)
    /// create.
    #[default]
    W5,
    /// Wallet v4r2: what most wallets created before 2024 hold.
    V4R2,
}

impl TonWalletVersion {
    pub const ALL: [Self; 2] = [Self::W5, Self::V4R2];

    fn code(self) -> Result<Cell, DerivationError> {
        use std::sync::OnceLock;
        static V4R2: OnceLock<Result<Cell, DerivationError>> = OnceLock::new();
        static W5: OnceLock<Result<Cell, DerivationError>> = OnceLock::new();
        let (cell, boc, hash) = match self {
            Self::V4R2 => (&V4R2, V4R2_CODE_BOC_HEX, V4R2_CODE_HASH),
            Self::W5 => (&W5, W5_CODE_BOC_HEX, W5_CODE_HASH),
        };
        cell.get_or_init(|| {
            let (cells, root) = parse_boc(&hex::decode(boc).map_err(DerivationError::invalid)?)?;
            let code = cell_from_rows(&cells, root)?;
            if hex::encode(code.hash_depth().0) != hash {
                return Err(DerivationError::Internal(
                    "TON: embedded wallet code does not match its published hash".into(),
                ));
            }
            Ok(code)
        })
        .clone()
    }

    /// The subwallet id this version signs with on `chain`. v4R2 uses one id
    /// on every network; W5 folds the network's global id into its own, so a
    /// key's W5 account on testnet is not its mainnet one.
    pub(crate) fn wallet_id(self, chain: Chain) -> Result<u32, DerivationError> {
        match self {
            // Hardcoded in every v4 wallet for the basic workchain.
            Self::V4R2 => Ok(698_983_191),
            Self::W5 => {
                let (global_id, _, _) = chain
                    .ton_first_block()
                    .ok_or_else(|| DerivationError::Invalid("TON: not a TON network".into()))?;
                // The client context: flag 1, workchain 0, version 0,
                // subwallet 0 — then XOR with the network's global id.
                Ok((global_id as i32 as u32) ^ 0x8000_0000)
            }
        }
    }

    /// The account's initial state: its code and the data cell a fresh
    /// wallet holds (seqno 0, its subwallet id and the key).
    pub(crate) fn state_init(
        self,
        public_key: &[u8; 32],
        chain: Chain,
    ) -> Result<Cell, DerivationError> {
        let mut data = Cell::default();
        if self == Self::W5 {
            // Signature authentication allowed.
            data.uint(1, 1)?;
        }
        data.uint(0, 32)?
            .uint(u64::from(self.wallet_id(chain)?), 32)?
            .bytes(public_key)?
            // No plugins (v4R2) or extensions (W5).
            .uint(0, 1)?;
        let mut init = Cell::default();
        init.uint(0b00110, 5)?
            .reference(self.code()?)?
            .reference(data)?;
        Ok(init)
    }

    /// The account this version gives `public_key` on `chain`, as a
    /// bounceable user-friendly address on the basic workchain.
    pub(crate) fn address(
        self,
        public_key: &[u8; 32],
        chain: Chain,
    ) -> Result<String, DerivationError> {
        let account_id = self.state_init(public_key, chain)?.hash_depth().0;
        Ok(friendly_address(0, &account_id, true))
    }

    /// The version whose account on `chain` `address` is for `public_key`,
    /// if any: how a stored wallet names the contract it signs as.
    pub(crate) fn of_address(public_key: &[u8; 32], address: &str, chain: Chain) -> Option<Self> {
        let parsed = parse_ton_address(address).ok()?;
        Self::ALL.into_iter().find(|version| {
            parsed.workchain == 0
                && version
                    .state_init(public_key, chain)
                    .is_ok_and(|init| init.hash_depth().0 == parsed.account_id)
        })
    }
}

#[derive(Clone)]
struct ParsedCell {
    d2: u8,
    data: Vec<u8>,
    refs: Vec<usize>,
    library: bool,
}

/// Minimal parser for BOC v0 (`b5ee9c72`) carrying level-0 cells, ordinary
/// or library references (a contract's code may name a library). Supports
/// the index and crc32c flags but validates neither; the parser's
/// correctness is instead locked by cell-hash tests.
fn parse_boc(bytes: &[u8]) -> Result<(Vec<ParsedCell>, usize), DerivationError> {
    if bytes.len() < 6 || bytes[0..4] != [0xb5, 0xee, 0x9c, 0x72] {
        return Err(DerivationError::Invalid("TON BOC: missing magic".into()));
    }
    let flags = bytes[4];
    let has_idx = (flags & 0x80) != 0;
    let _has_crc32c = (flags & 0x40) != 0;
    let ref_size = (flags & 0x07) as usize;
    if ref_size == 0 || ref_size > 4 {
        return Err(DerivationError::Invalid(
            format!("TON BOC: invalid ref size {ref_size}").into(),
        ));
    }
    let off_size = bytes[5] as usize;
    if off_size == 0 || off_size > 8 {
        return Err(DerivationError::Invalid(
            format!("TON BOC: invalid offset size {off_size}").into(),
        ));
    }
    let mut cursor = 6usize;
    let read_uint = |buf: &[u8], off: usize, n: usize| -> Result<u64, DerivationError> {
        if off + n > buf.len() {
            return Err(DerivationError::Invalid("TON BOC: unexpected EOF".into()));
        }
        let mut v = 0u64;
        for &b in &buf[off..off + n] {
            v = (v << 8) | u64::from(b);
        }
        Ok(v)
    };
    let cell_count = read_uint(bytes, cursor, ref_size)? as usize;
    cursor += ref_size;
    let root_count = read_uint(bytes, cursor, ref_size)? as usize;
    cursor += ref_size;
    let _absent = read_uint(bytes, cursor, ref_size)? as usize;
    cursor += ref_size;
    let _tot_cell_size = read_uint(bytes, cursor, off_size)? as usize;
    cursor += off_size;
    if root_count == 0 {
        return Err(DerivationError::Invalid("TON BOC: no roots".into()));
    }
    let root_idx = read_uint(bytes, cursor, ref_size)? as usize;
    cursor += ref_size * root_count;
    if has_idx {
        cursor += cell_count * off_size;
    }
    let mut cells = Vec::with_capacity(cell_count);
    for _ in 0..cell_count {
        if cursor + 2 > bytes.len() {
            return Err(DerivationError::Invalid("TON BOC: cell header EOF".into()));
        }
        let d1 = bytes[cursor];
        let d2 = bytes[cursor + 1];
        cursor += 2;
        let refs_count = (d1 & 0x07) as usize;
        let exotic = (d1 & 0x08) != 0;
        let level = (d1 >> 5) & 0x03;
        let data_len = (d2 as usize).div_ceil(2);
        if cursor + data_len > bytes.len() {
            return Err(DerivationError::Invalid("TON BOC: cell data EOF".into()));
        }
        let data = bytes[cursor..cursor + data_len].to_vec();
        // The one exotic cell read: a library reference, tag 2 and a hash.
        let library = exotic && refs_count == 0 && d2 == 0x42 && data.first() == Some(&2);
        if (exotic && !library) || level != 0 {
            return Err(DerivationError::Invalid(
                "TON BOC: exotic or leveled cells not supported".into(),
            ));
        }
        cursor += data_len;
        let mut refs = Vec::with_capacity(refs_count);
        for _ in 0..refs_count {
            refs.push(read_uint(bytes, cursor, ref_size)? as usize);
            cursor += ref_size;
        }
        cells.push(ParsedCell {
            d2,
            data,
            refs,
            library,
        });
    }
    Ok((cells, root_idx))
}

/// The tree under row `i`, at most `MAX_CELLS` cells once shared subtrees
/// are expanded, so a hostile BOC cannot make it explode.
fn cell_from_rows(cells: &[ParsedCell], i: usize) -> Result<Cell, DerivationError> {
    const MAX_CELLS: usize = 4096;
    fn expand(cells: &[ParsedCell], i: usize, budget: &mut usize) -> Result<Cell, DerivationError> {
        *budget = budget
            .checked_sub(1)
            .ok_or_else(|| DerivationError::Invalid("TON BOC: too many cells".into()))?;
        let row = cells
            .get(i)
            .ok_or_else(|| DerivationError::Invalid("TON: invalid cell reference".into()))?;
        if row.refs.iter().any(|r| *r <= i) {
            return Err(DerivationError::Invalid("TON: invalid cell order".into()));
        }
        if row.library {
            return Cell::library(
                row.data[1..].try_into().map_err(|_| {
                    DerivationError::Invalid("TON BOC: invalid library cell".into())
                })?,
            );
        }
        let refs = row
            .refs
            .iter()
            .map(|r| expand(cells, *r, budget))
            .collect::<Result<_, _>>()?;
        Cell::from_padded(row.data.clone(), row.d2, refs)
    }
    let mut budget = MAX_CELLS;
    expand(cells, i, &mut budget)
}

/// CRC-16/XMODEM (poly=0x1021, init=0x0000, no reflection, no xor-out),
/// as required by TON user-friendly address checksums.
pub(crate) fn crc16_xmodem(bytes: &[u8]) -> u16 {
    const CRC: crc::Crc<u16> = crc::Crc::<u16>::new(&crc::CRC_16_XMODEM);
    CRC.checksum(bytes)
}

/// The ed25519 key pair a TON mnemonic (and its password) holds.
pub(crate) fn ton_key_pair(
    seed_phrase: &str,
    passphrase: Option<&str>,
) -> Result<(Zeroizing<[u8; 32]>, [u8; 32]), DerivationError> {
    let seed = derive_ton_seed(seed_phrase, passphrase.unwrap_or(""))?;
    let mut private_key = Zeroizing::new([0u8; 32]);
    private_key.copy_from_slice(&seed[..32]);
    let public_key = SigningKey::from_bytes(&private_key)
        .verifying_key()
        .to_bytes();
    Ok((private_key, public_key))
}

/// Derive a TON wallet from a TON mnemonic: the address is the default
/// version's account on `chain`.
pub(crate) fn derive_ton_standard(
    chain: Chain,
    seed_phrase: &str,
    passphrase: Option<&str>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<crate::derivation::primitives::OptionalKeyMaterial, DerivationError> {
    let (private_key, public_key) = ton_key_pair(seed_phrase, passphrase)?;
    let address = if want_address {
        Some(TonWalletVersion::default().address(&public_key, chain)?)
    } else {
        None
    };

    Ok((
        address,
        want_public_key.then(|| hex::encode(public_key)),
        want_private_key.then(|| hex::encode(*private_key)),
    ))
}

// ── Derivation entry points ────────────────────────────────────────────────────────

use crate::SpectraBridgeError;
use crate::derivation::primitives::hmac_sha512;
use crate::derivation::types::DerivationResult;

// Shared derivation logic for all TON networks.
fn ton_internal(
    chain: Chain,
    seed_phrase: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (address, public_key_hex, private_key_hex) = derive_ton_standard(
        chain,
        &seed_phrase,
        passphrase.as_deref(),
        want_address,
        want_public_key,
        want_private_key,
    )?;
    Ok(DerivationResult {
        address,
        public_key_hex,
        private_key_hex,
        account: 0,
        branch: 0,
        index: 0,
    })
}

/// Derive a TON mainnet wallet (the default version's address) from a seed
/// phrase.
pub fn derive_ton(
    seed_phrase: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    ton_internal(
        Chain::Ton,
        seed_phrase,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// Derive a TON testnet wallet from a seed phrase: the same key as on
/// mainnet, and the default version's testnet account.
pub fn derive_ton_testnet(
    seed_phrase: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    ton_internal(
        Chain::TonTestnet,
        seed_phrase,
        passphrase,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// A BOC's single root cell, as read from a node: level-0 cells, each
/// reference pointing forward.
pub(crate) fn cell_from_boc(bytes: &[u8]) -> Result<Cell, DerivationError> {
    let (cells, root) = parse_boc(bytes)?;
    cell_from_rows(&cells, root)
}

/// A user-friendly address: bounceable or not, base64url, as wallets show
/// one on the basic or master workchain.
pub(crate) fn friendly_address(workchain: i8, account: &[u8; 32], bounceable: bool) -> String {
    let mut buf = [0u8; 36];
    buf[0] = if bounceable { 0x11 } else { 0x51 };
    buf[1] = workchain as u8;
    buf[2..34].copy_from_slice(account);
    let crc = crc16_xmodem(&buf[..34]);
    buf[34..36].copy_from_slice(&crc.to_be_bytes());
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

/// Root representation hash of a locally built external-message BOC. This
/// identity is persisted before broadcast so uncertain submission can be polled.
pub(crate) fn boc_root_hash(bytes: &[u8]) -> Result<[u8; 32], DerivationError> {
    let (cells, root) = parse_boc(bytes)?;
    Ok(cell_from_rows(&cells, root)?.hash_depth().0)
}
