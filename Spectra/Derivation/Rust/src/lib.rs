use bip39::{Language, Mnemonic};
use bitcoin::bip32::{DerivationPath, Xpriv};
use bitcoin::hashes::{hash160, sha256, Hash};
use bitcoin::key::{CompressedPublicKey, PublicKey};
use bitcoin::secp256k1::{All, Secp256k1};
use bitcoin::{Address, Network};
use ed25519_dalek::SigningKey;
use pbkdf2::pbkdf2_hmac;
use sha2::Sha512;
use slip10::{derive_key_from_path, Curve};
use std::fmt::Display;
use std::ptr;
use std::slice;
use std::str::FromStr;
use tiny_keccak::{Hasher, Keccak};
use unicode_normalization::UnicodeNormalization;
use zeroize::{Zeroize, Zeroizing};

const STATUS_OK: i32 = 0;
const STATUS_ERROR: i32 = 1;

const OUTPUT_ADDRESS: u32 = 1 << 0;
const OUTPUT_PUBLIC_KEY: u32 = 1 << 1;
const OUTPUT_PRIVATE_KEY: u32 = 1 << 2;

const CHAIN_BITCOIN: u32 = 0;
const CHAIN_ETHEREUM: u32 = 1;
const CHAIN_SOLANA: u32 = 2;
const CHAIN_BITCOIN_CASH: u32 = 3;
const CHAIN_BITCOIN_SV: u32 = 4;
const CHAIN_LITECOIN: u32 = 5;
const CHAIN_DOGECOIN: u32 = 6;
const CHAIN_ETHEREUM_CLASSIC: u32 = 7;
const CHAIN_ARBITRUM: u32 = 8;
const CHAIN_OPTIMISM: u32 = 9;
const CHAIN_AVALANCHE: u32 = 10;
const CHAIN_HYPERLIQUID: u32 = 11;
const CHAIN_TRON: u32 = 12;
const CHAIN_STELLAR: u32 = 13;
const CHAIN_XRP: u32 = 14;
const CHAIN_CARDANO: u32 = 15;
const CHAIN_SUI: u32 = 16;
const CHAIN_APTOS: u32 = 17;
const CHAIN_TON: u32 = 18;
const CHAIN_INTERNET_COMPUTER: u32 = 19;
const CHAIN_NEAR: u32 = 20;
const CHAIN_POLKADOT: u32 = 21;

const NETWORK_MAINNET: u32 = 0;
const NETWORK_TESTNET: u32 = 1;
const NETWORK_TESTNET4: u32 = 2;
const NETWORK_SIGNET: u32 = 3;

const CURVE_SECP256K1: u32 = 0;
const CURVE_ED25519: u32 = 1;

const DERIVATION_AUTO: u32 = 0;
const DERIVATION_BIP32_SECP256K1: u32 = 1;
const DERIVATION_SLIP10_ED25519: u32 = 2;

const ADDRESS_AUTO: u32 = 0;
const ADDRESS_BITCOIN: u32 = 1;
const ADDRESS_EVM: u32 = 2;
const ADDRESS_SOLANA: u32 = 3;

const PUBLIC_KEY_AUTO: u32 = 0;
const PUBLIC_KEY_COMPRESSED: u32 = 1;
const PUBLIC_KEY_UNCOMPRESSED: u32 = 2;
const PUBLIC_KEY_X_ONLY: u32 = 3;
const PUBLIC_KEY_RAW: u32 = 4;

const SCRIPT_AUTO: u32 = 0;
const SCRIPT_P2PKH: u32 = 1;
const SCRIPT_P2SH_P2WPKH: u32 = 2;
const SCRIPT_P2WPKH: u32 = 3;
const SCRIPT_P2TR: u32 = 4;
const SCRIPT_ACCOUNT: u32 = 5;

#[repr(C)]
pub struct SpectraBuffer {
    pub ptr: *mut u8,
    pub len: usize,
}

impl SpectraBuffer {
    fn empty() -> Self {
        Self {
            ptr: ptr::null_mut(),
            len: 0,
        }
    }

    fn from_vec(mut bytes: Vec<u8>) -> Self {
        let buffer = Self {
            ptr: bytes.as_mut_ptr(),
            len: bytes.len(),
        };
        std::mem::forget(bytes);
        buffer
    }

    fn from_string(value: String) -> Self {
        Self::from_vec(value.into_bytes())
    }
}

#[repr(C)]
pub struct SpectraDerivationRequest {
    pub chain: u32,
    pub network: u32,
    pub curve: u32,
    pub requested_outputs: u32,
    pub derivation_algorithm: u32,
    pub address_algorithm: u32,
    pub public_key_format: u32,
    pub script_type: u32,
    pub seed_phrase_utf8: SpectraBuffer,
    pub derivation_path_utf8: SpectraBuffer,
    pub passphrase_utf8: SpectraBuffer,
    pub hmac_key_utf8: SpectraBuffer,
    pub mnemonic_wordlist_utf8: SpectraBuffer,
    pub iteration_count: u32,
}

#[repr(C)]
pub struct SpectraPrivateKeyDerivationRequest {
    pub chain: u32,
    pub network: u32,
    pub curve: u32,
    pub address_algorithm: u32,
    pub public_key_format: u32,
    pub script_type: u32,
    pub private_key_hex_utf8: SpectraBuffer,
}

#[repr(C)]
pub struct SpectraDerivationResponse {
    pub status_code: i32,
    pub address_utf8: SpectraBuffer,
    pub public_key_hex_utf8: SpectraBuffer,
    pub private_key_hex_utf8: SpectraBuffer,
    pub error_message_utf8: SpectraBuffer,
}

impl SpectraDerivationResponse {
    fn success(result: DerivedOutput) -> *mut SpectraDerivationResponse {
        Box::into_raw(Box::new(SpectraDerivationResponse {
            status_code: STATUS_OK,
            address_utf8: result
                .address
                .map(SpectraBuffer::from_string)
                .unwrap_or_else(SpectraBuffer::empty),
            public_key_hex_utf8: result
                .public_key_hex
                .map(SpectraBuffer::from_string)
                .unwrap_or_else(SpectraBuffer::empty),
            private_key_hex_utf8: result
                .private_key_hex
                .map(SpectraBuffer::from_string)
                .unwrap_or_else(SpectraBuffer::empty),
            error_message_utf8: SpectraBuffer::empty(),
        }))
    }

    fn error(message: impl Into<String>) -> *mut SpectraDerivationResponse {
        Box::into_raw(Box::new(SpectraDerivationResponse {
            status_code: STATUS_ERROR,
            address_utf8: SpectraBuffer::empty(),
            public_key_hex_utf8: SpectraBuffer::empty(),
            private_key_hex_utf8: SpectraBuffer::empty(),
            error_message_utf8: SpectraBuffer::from_string(message.into()),
        }))
    }
}

struct DerivedOutput {
    address: Option<String>,
    public_key_hex: Option<String>,
    private_key_hex: Option<String>,
}

struct ParsedRequest {
    chain: Chain,
    network: NetworkFlavor,
    curve: CurveFamily,
    requested_outputs: u32,
    derivation_algorithm: DerivationAlgorithm,
    address_algorithm: AddressAlgorithm,
    public_key_format: PublicKeyFormat,
    script_type: ScriptType,
    seed_phrase: String,
    derivation_path: Option<String>,
    passphrase: String,
    hmac_key: Option<String>,
    mnemonic_wordlist: Option<String>,
    iteration_count: u32,
}

impl Drop for ParsedRequest {
    fn drop(&mut self) {
        self.seed_phrase.zeroize();
        self.passphrase.zeroize();
        if let Some(hmac_key) = &mut self.hmac_key {
            hmac_key.zeroize();
        }
        if let Some(wordlist) = &mut self.mnemonic_wordlist {
            wordlist.zeroize();
        }
        if let Some(path) = &mut self.derivation_path {
            path.zeroize();
        }
    }
}

#[derive(Clone, Copy)]
enum Chain {
    Bitcoin,
    BitcoinCash,
    BitcoinSv,
    Litecoin,
    Dogecoin,
    Ethereum,
    EthereumClassic,
    Arbitrum,
    Optimism,
    Avalanche,
    Hyperliquid,
    Tron,
    Solana,
    Stellar,
    Xrp,
    Cardano,
    Sui,
    Aptos,
    Ton,
    InternetComputer,
    Near,
    Polkadot,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NetworkFlavor {
    Mainnet,
    Testnet,
    Testnet4,
    Signet,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CurveFamily {
    Secp256k1,
    Ed25519,
}

#[derive(Clone, Copy)]
enum DerivationAlgorithm {
    Auto,
    Bip32Secp256k1,
    Slip10Ed25519,
}

#[derive(Clone, Copy)]
enum AddressAlgorithm {
    Auto,
    Bitcoin,
    Evm,
    Solana,
}

#[derive(Clone, Copy)]
enum PublicKeyFormat {
    Auto,
    Compressed,
    Uncompressed,
    XOnly,
    Raw,
}

#[derive(Clone, Copy)]
enum ScriptType {
    Auto,
    P2pkh,
    P2shP2wpkh,
    P2wpkh,
    P2tr,
    Account,
}

#[no_mangle]
pub extern "C" fn spectra_derivation_derive(
    request: *const SpectraDerivationRequest,
) -> *mut SpectraDerivationResponse {
    if request.is_null() {
        return SpectraDerivationResponse::error("Null derivation request.");
    }

    let request = unsafe { &*request };
    match parse_request(request).and_then(derive) {
        Ok(result) => SpectraDerivationResponse::success(result),
        Err(error) => SpectraDerivationResponse::error(error),
    }
}

#[no_mangle]
pub extern "C" fn spectra_derivation_derive_from_private_key(
    request: *const SpectraPrivateKeyDerivationRequest,
) -> *mut SpectraDerivationResponse {
    if request.is_null() {
        return SpectraDerivationResponse::error("Null private-key derivation request.");
    }

    let request = unsafe { &*request };
    match parse_private_key_request(request).and_then(derive_from_private_key) {
        Ok(result) => SpectraDerivationResponse::success(result),
        Err(error) => SpectraDerivationResponse::error(error),
    }
}

#[no_mangle]
pub extern "C" fn spectra_derivation_response_free(response: *mut SpectraDerivationResponse) {
    if response.is_null() {
        return;
    }

    let response = unsafe { Box::from_raw(response) };
    free_buffer(response.address_utf8);
    free_buffer(response.public_key_hex_utf8);
    free_buffer(response.private_key_hex_utf8);
    free_buffer(response.error_message_utf8);
}

struct ParsedPrivateKeyRequest {
    chain: Chain,
    network: NetworkFlavor,
    curve: CurveFamily,
    address_algorithm: AddressAlgorithm,
    public_key_format: PublicKeyFormat,
    script_type: ScriptType,
    private_key: [u8; 32],
}

fn parse_private_key_request(
    request: &SpectraPrivateKeyDerivationRequest,
) -> Result<ParsedPrivateKeyRequest, String> {
    let chain = parse_chain(request.chain)?;
    let network = parse_network(request.network)?;
    let curve = parse_curve(request.curve)?;
    let address_algorithm = parse_address_algorithm(request.address_algorithm)?;
    let public_key_format = parse_public_key_format(request.public_key_format)?;
    let script_type = parse_script_type(request.script_type)?;
    let private_key_hex = read_buffer_to_string(&request.private_key_hex_utf8)?;
    let private_key = decode_private_key_hex(&private_key_hex)?;

    Ok(ParsedPrivateKeyRequest {
        chain,
        network,
        curve,
        address_algorithm,
        public_key_format,
        script_type,
        private_key,
    })
}

fn decode_private_key_hex(raw: &str) -> Result<[u8; 32], String> {
    let value = raw.trim();
    if value.len() != 64 {
        return Err("Private key hex must be exactly 64 hex characters.".to_string());
    }

    let decoded = hex::decode(value).map_err(display_error)?;
    if decoded.len() != 32 {
        return Err("Private key must decode to 32 bytes.".to_string());
    }

    let mut out = [0u8; 32];
    out.copy_from_slice(&decoded);
    Ok(out)
}

fn derive_from_private_key(request: ParsedPrivateKeyRequest) -> Result<DerivedOutput, String> {
    if is_secp_chain(request.chain) {
        if request.curve != CurveFamily::Secp256k1 {
            return Err("This chain currently requires secp256k1.".to_string());
        }

        let secp = Secp256k1::new();
        let secret_key = bitcoin::secp256k1::SecretKey::from_slice(&request.private_key).map_err(display_error)?;
        let public_key = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
        let compressed = CompressedPublicKey::try_from(PublicKey::new(public_key)).map_err(display_error)?;

        let address = derive_address_from_keys(
            request.chain,
            request.network,
            request.address_algorithm,
            request.script_type,
            &compressed,
            &public_key,
            &secp,
        )?;

        return Ok(DerivedOutput {
            address: Some(address),
            public_key_hex: Some(hex::encode(format_secp_public_key(&public_key, request.public_key_format)?)),
            private_key_hex: Some(hex::encode(request.private_key)),
        });
    }

    if request.curve != CurveFamily::Ed25519 {
        return Err("This chain currently requires ed25519.".to_string());
    }

    let signing_key = SigningKey::from_bytes(&request.private_key);
    let public_key = signing_key.verifying_key().to_bytes();
    let address = derive_ed25519_chain_address(request.chain, &public_key)?;

    Ok(DerivedOutput {
        address: Some(address),
        public_key_hex: Some(hex::encode(public_key)),
        private_key_hex: Some(hex::encode(request.private_key)),
    })
}

fn derive_address_from_keys(
    chain: Chain,
    network: NetworkFlavor,
    address_algorithm: AddressAlgorithm,
    script_type: ScriptType,
    compressed_public_key: &CompressedPublicKey,
    public_key: &bitcoin::secp256k1::PublicKey,
    secp: &Secp256k1<All>,
) -> Result<String, String> {
    match chain {
        Chain::Bitcoin => {
            let effective_script_type = match script_type {
                ScriptType::Auto => match address_algorithm {
                    AddressAlgorithm::Auto | AddressAlgorithm::Bitcoin => ScriptType::P2wpkh,
                    _ => ScriptType::P2pkh,
                },
                other => other,
            };
            let bitcoin_network = match network {
                NetworkFlavor::Mainnet => Network::Bitcoin,
                NetworkFlavor::Testnet | NetworkFlavor::Testnet4 | NetworkFlavor::Signet => Network::Testnet,
            };
            derive_bitcoin_address_for_network(
                bitcoin_network,
                effective_script_type,
                compressed_public_key,
                public_key,
                secp,
            )
        }
        Chain::BitcoinCash | Chain::BitcoinSv => {
            let pubkey_hash = hash160::Hash::hash(&public_key.serialize()).to_byte_array();
            let mut payload = vec![0x00u8];
            payload.extend_from_slice(&pubkey_hash);
            Ok(base58check_encode(&payload, bs58::Alphabet::DEFAULT))
        }
        Chain::Litecoin => {
            let pubkey_hash = hash160::Hash::hash(&public_key.serialize()).to_byte_array();
            let mut payload = vec![0x30u8];
            payload.extend_from_slice(&pubkey_hash);
            Ok(base58check_encode(&payload, bs58::Alphabet::DEFAULT))
        }
        Chain::Dogecoin => {
            let version = if matches!(network, NetworkFlavor::Testnet) { 0x71 } else { 0x1e };
            let pubkey_hash = hash160::Hash::hash(&public_key.serialize()).to_byte_array();
            let mut payload = vec![version];
            payload.extend_from_slice(&pubkey_hash);
            Ok(base58check_encode(&payload, bs58::Alphabet::DEFAULT))
        }
        Chain::Ethereum
        | Chain::EthereumClassic
        | Chain::Arbitrum
        | Chain::Optimism
        | Chain::Avalanche
        | Chain::Hyperliquid => Ok(derive_evm_address(public_key)),
        Chain::Tron => {
            let evm_address = derive_evm_address_bytes(public_key);
            let mut payload = vec![0x41u8];
            payload.extend_from_slice(&evm_address);
            Ok(base58check_encode(&payload, bs58::Alphabet::DEFAULT))
        }
        Chain::Xrp => {
            let pubkey_hash = hash160::Hash::hash(&public_key.serialize()).to_byte_array();
            let mut payload = vec![0x00u8];
            payload.extend_from_slice(&pubkey_hash);
            Ok(base58check_encode(&payload, bs58::Alphabet::RIPPLE))
        }
        _ => Err("Unsupported secp256k1 chain for private-key address derivation.".to_string()),
    }
}

fn derive_ed25519_chain_address(chain: Chain, public_key: &[u8; 32]) -> Result<String, String> {
    match chain {
        Chain::Solana => Ok(bs58::encode(public_key).into_string()),
        Chain::Stellar => {
            let encoded = base32_no_pad(public_key);
            let stellar_address = format!("G{}", &encoded[..55.min(encoded.len())]);
            if stellar_address.len() < 56 {
                Ok(format!("{}{}", stellar_address, "A".repeat(56 - stellar_address.len())))
            } else {
                Ok(stellar_address)
            }
        }
        Chain::Cardano => {
            let digest = sha256::Hash::hash(public_key).to_byte_array();
            Ok(format!("addr1{}", hex::encode(digest)))
        }
        Chain::Sui => {
            let mut hasher = Keccak::v256();
            let mut digest = [0u8; 32];
            hasher.update(&[0x00]);
            hasher.update(public_key);
            hasher.finalize(&mut digest);
            Ok(format!("0x{}", hex::encode(digest)))
        }
        Chain::Aptos => {
            let mut hasher = Keccak::v256();
            let mut digest = [0u8; 32];
            hasher.update(public_key);
            hasher.update(&[0x00]);
            hasher.finalize(&mut digest);
            Ok(format!("0x{}", hex::encode(digest)))
        }
        Chain::Ton => {
            let digest = sha256::Hash::hash(public_key).to_byte_array();
            Ok(format!("0:{}", hex::encode(digest)))
        }
        Chain::InternetComputer => {
            let mut data = Vec::from(*public_key);
            data.extend_from_slice(b"icp");
            let digest = sha256::Hash::hash(&data).to_byte_array();
            let digest2 = sha256::Hash::hash(&digest).to_byte_array();
            Ok(hex::encode(digest2))
        }
        Chain::Near => Ok(hex::encode(public_key)),
        Chain::Polkadot => {
            let mut payload = vec![0x00u8];
            payload.extend_from_slice(public_key);
            payload.extend_from_slice(&sha256::Hash::hash(public_key).to_byte_array()[..2]);
            Ok(bs58::encode(payload).into_string())
        }
        _ => Err("Unsupported ed25519 chain for private-key address derivation.".to_string()),
    }
}

#[no_mangle]
pub extern "C" fn spectra_derivation_buffer_free(buffer: SpectraBuffer) {
    free_buffer(buffer);
}

fn parse_request(request: &SpectraDerivationRequest) -> Result<ParsedRequest, String> {
    let chain = parse_chain(request.chain)?;
    let network = parse_network(request.network)?;
    let curve = parse_curve(request.curve)?;
    let derivation_algorithm = parse_derivation_algorithm(request.derivation_algorithm)?;
    let address_algorithm = parse_address_algorithm(request.address_algorithm)?;
    let public_key_format = parse_public_key_format(request.public_key_format)?;
    let script_type = parse_script_type(request.script_type)?;

    let seed_phrase = normalize_seed_phrase(&read_buffer_to_string(&request.seed_phrase_utf8)?);
    if seed_phrase.is_empty() {
        return Err("Seed phrase is empty.".to_string());
    }

    let derivation_path = optional_trimmed_string(&request.derivation_path_utf8)?;
    let passphrase = optional_untrimmed_string(&request.passphrase_utf8)?.unwrap_or_default();
    let hmac_key = optional_trimmed_string(&request.hmac_key_utf8)?;
    let mnemonic_wordlist = optional_trimmed_string(&request.mnemonic_wordlist_utf8)?;

    if request.requested_outputs == 0 {
        return Err("At least one output must be requested.".to_string());
    }
    let known_outputs = OUTPUT_ADDRESS | OUTPUT_PUBLIC_KEY | OUTPUT_PRIVATE_KEY;
    if request.requested_outputs & !known_outputs != 0 {
        return Err("Requested outputs contain unsupported output flags.".to_string());
    }

    Ok(ParsedRequest {
        chain,
        network,
        curve,
        requested_outputs: request.requested_outputs,
        derivation_algorithm,
        address_algorithm,
        public_key_format,
        script_type,
        seed_phrase,
        derivation_path,
        passphrase,
        hmac_key,
        mnemonic_wordlist,
        iteration_count: request.iteration_count,
    })
}

fn parse_chain(value: u32) -> Result<Chain, String> {
    match value {
        CHAIN_BITCOIN => Ok(Chain::Bitcoin),
        CHAIN_ETHEREUM => Ok(Chain::Ethereum),
        CHAIN_SOLANA => Ok(Chain::Solana),
        CHAIN_BITCOIN_CASH => Ok(Chain::BitcoinCash),
        CHAIN_BITCOIN_SV => Ok(Chain::BitcoinSv),
        CHAIN_LITECOIN => Ok(Chain::Litecoin),
        CHAIN_DOGECOIN => Ok(Chain::Dogecoin),
        CHAIN_ETHEREUM_CLASSIC => Ok(Chain::EthereumClassic),
        CHAIN_ARBITRUM => Ok(Chain::Arbitrum),
        CHAIN_OPTIMISM => Ok(Chain::Optimism),
        CHAIN_AVALANCHE => Ok(Chain::Avalanche),
        CHAIN_HYPERLIQUID => Ok(Chain::Hyperliquid),
        CHAIN_TRON => Ok(Chain::Tron),
        CHAIN_STELLAR => Ok(Chain::Stellar),
        CHAIN_XRP => Ok(Chain::Xrp),
        CHAIN_CARDANO => Ok(Chain::Cardano),
        CHAIN_SUI => Ok(Chain::Sui),
        CHAIN_APTOS => Ok(Chain::Aptos),
        CHAIN_TON => Ok(Chain::Ton),
        CHAIN_INTERNET_COMPUTER => Ok(Chain::InternetComputer),
        CHAIN_NEAR => Ok(Chain::Near),
        CHAIN_POLKADOT => Ok(Chain::Polkadot),
        other => Err(format!("Unsupported chain id: {other}")),
    }
}

fn parse_network(value: u32) -> Result<NetworkFlavor, String> {
    match value {
        NETWORK_MAINNET => Ok(NetworkFlavor::Mainnet),
        NETWORK_TESTNET => Ok(NetworkFlavor::Testnet),
        NETWORK_TESTNET4 => Ok(NetworkFlavor::Testnet4),
        NETWORK_SIGNET => Ok(NetworkFlavor::Signet),
        other => Err(format!("Unsupported network id: {other}")),
    }
}

fn parse_curve(value: u32) -> Result<CurveFamily, String> {
    match value {
        CURVE_SECP256K1 => Ok(CurveFamily::Secp256k1),
        CURVE_ED25519 => Ok(CurveFamily::Ed25519),
        other => Err(format!("Unsupported curve id: {other}")),
    }
}

fn parse_derivation_algorithm(value: u32) -> Result<DerivationAlgorithm, String> {
    match value {
        DERIVATION_AUTO => Ok(DerivationAlgorithm::Auto),
        DERIVATION_BIP32_SECP256K1 => Ok(DerivationAlgorithm::Bip32Secp256k1),
        DERIVATION_SLIP10_ED25519 => Ok(DerivationAlgorithm::Slip10Ed25519),
        other => Err(format!("Unsupported derivation algorithm id: {other}")),
    }
}

fn parse_address_algorithm(value: u32) -> Result<AddressAlgorithm, String> {
    match value {
        ADDRESS_AUTO => Ok(AddressAlgorithm::Auto),
        ADDRESS_BITCOIN => Ok(AddressAlgorithm::Bitcoin),
        ADDRESS_EVM => Ok(AddressAlgorithm::Evm),
        ADDRESS_SOLANA => Ok(AddressAlgorithm::Solana),
        other => Err(format!("Unsupported address algorithm id: {other}")),
    }
}

fn parse_public_key_format(value: u32) -> Result<PublicKeyFormat, String> {
    match value {
        PUBLIC_KEY_AUTO => Ok(PublicKeyFormat::Auto),
        PUBLIC_KEY_COMPRESSED => Ok(PublicKeyFormat::Compressed),
        PUBLIC_KEY_UNCOMPRESSED => Ok(PublicKeyFormat::Uncompressed),
        PUBLIC_KEY_X_ONLY => Ok(PublicKeyFormat::XOnly),
        PUBLIC_KEY_RAW => Ok(PublicKeyFormat::Raw),
        other => Err(format!("Unsupported public key format id: {other}")),
    }
}

fn parse_script_type(value: u32) -> Result<ScriptType, String> {
    match value {
        SCRIPT_AUTO => Ok(ScriptType::Auto),
        SCRIPT_P2PKH => Ok(ScriptType::P2pkh),
        SCRIPT_P2SH_P2WPKH => Ok(ScriptType::P2shP2wpkh),
        SCRIPT_P2WPKH => Ok(ScriptType::P2wpkh),
        SCRIPT_P2TR => Ok(ScriptType::P2tr),
        SCRIPT_ACCOUNT => Ok(ScriptType::Account),
        other => Err(format!("Unsupported script type id: {other}")),
    }
}

fn derive(request: ParsedRequest) -> Result<DerivedOutput, String> {
    validate_request(&request)?;

    match request.chain {
        Chain::Bitcoin => derive_bitcoin(request),
        Chain::BitcoinCash => derive_bitcoin_legacy_family(request, 0x00),
        Chain::BitcoinSv => derive_bitcoin_legacy_family(request, 0x00),
        Chain::Litecoin => derive_litecoin(request),
        Chain::Dogecoin => derive_dogecoin(request),
        Chain::Ethereum
        | Chain::EthereumClassic
        | Chain::Arbitrum
        | Chain::Optimism
        | Chain::Avalanche
        | Chain::Hyperliquid => derive_evm_family(request),
        Chain::Tron => derive_tron(request),
        Chain::Solana => derive_solana(request),
        Chain::Stellar => derive_stellar(request),
        Chain::Xrp => derive_xrp(request),
        Chain::Cardano => derive_cardano(request),
        Chain::Sui => derive_sui(request),
        Chain::Aptos => derive_aptos(request),
        Chain::Ton => derive_ton(request),
        Chain::InternetComputer => derive_icp(request),
        Chain::Near => derive_near(request),
        Chain::Polkadot => derive_polkadot(request),
    }
}

fn validate_request(request: &ParsedRequest) -> Result<(), String> {
    if request.iteration_count == 1 {
        return Err("Iteration count must be 0 (default) or >= 2.".to_string());
    }

    if let Some(wordlist) = &request.mnemonic_wordlist {
        if !wordlist.eq_ignore_ascii_case("english") {
            return Err("Only the English mnemonic wordlist is supported in Rust right now.".to_string());
        }
    }

    if is_secp_chain(request.chain) {
        if request.curve != CurveFamily::Secp256k1 {
            return Err("This chain currently requires secp256k1.".to_string());
        }
        if matches!(request.derivation_algorithm, DerivationAlgorithm::Slip10Ed25519) {
            return Err("This chain does not support SLIP-0010 ed25519 derivation.".to_string());
        }
    } else {
        if request.curve != CurveFamily::Ed25519 {
            return Err("This chain currently requires ed25519.".to_string());
        }
        if matches!(request.derivation_algorithm, DerivationAlgorithm::Bip32Secp256k1) {
            return Err("This chain does not support BIP-32 secp256k1 derivation.".to_string());
        }
    }

    if !is_network_supported(request.chain, request.network) {
        return Err("Network is not supported for this chain.".to_string());
    }

    Ok(())
}

fn is_secp_chain(chain: Chain) -> bool {
    matches!(
        chain,
        Chain::Bitcoin
            | Chain::BitcoinCash
            | Chain::BitcoinSv
            | Chain::Litecoin
            | Chain::Dogecoin
            | Chain::Ethereum
            | Chain::EthereumClassic
            | Chain::Arbitrum
            | Chain::Optimism
            | Chain::Avalanche
            | Chain::Hyperliquid
            | Chain::Tron
            | Chain::Xrp
    )
}

fn is_network_supported(chain: Chain, network: NetworkFlavor) -> bool {
    match chain {
        Chain::Bitcoin => true,
        Chain::Dogecoin => matches!(network, NetworkFlavor::Mainnet | NetworkFlavor::Testnet),
        _ => matches!(network, NetworkFlavor::Mainnet),
    }
}

fn derive_secp_material(request: &ParsedRequest) -> Result<(bitcoin::secp256k1::PublicKey, [u8; 32]), String> {
    let derivation_path = secp_derivation_path(request);
    let seed = derive_bip39_seed(&request.seed_phrase, &request.passphrase, request.iteration_count)?;
    let xpriv = derive_bip32_xpriv(seed.as_ref(), Network::Bitcoin, &derivation_path, request.hmac_key.as_deref())?;
    let secret_key = xpriv.private_key.secret_bytes();
    let secp = Secp256k1::new();
    let public_key = bitcoin::secp256k1::PublicKey::from_secret_key(
        &secp,
        &bitcoin::secp256k1::SecretKey::from_slice(&secret_key).map_err(display_error)?,
    );
    Ok((public_key, secret_key))
}

fn derive_ed25519_material(request: &ParsedRequest) -> Result<([u8; 32], [u8; 32]), String> {
    let path = ed25519_derivation_path(request);
    let seed = derive_bip39_seed(&request.seed_phrase, &request.passphrase, request.iteration_count)?;
    let private_key = derive_solana_ed25519_key(seed.as_ref(), &path, request.hmac_key.as_deref())?;
    let signing_key = SigningKey::from_bytes(&private_key);
    let public_key = signing_key.verifying_key().to_bytes();
    Ok((*private_key, public_key))
}

fn derive_bitcoin(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let secp = Secp256k1::new();
    let derivation_path = secp_derivation_path(&request);
    let script_type = bitcoin_script_type(&request, &derivation_path)?;
    let seed = derive_bip39_seed(&request.seed_phrase, &request.passphrase, request.iteration_count)?;
    let xpriv = derive_bip32_xpriv(seed.as_ref(), Network::Bitcoin, &derivation_path, request.hmac_key.as_deref())?;
    let secret_key = xpriv.private_key;
    let public_key = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
    let compressed = CompressedPublicKey::try_from(PublicKey::new(public_key)).map_err(display_error)?;

    let address = if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
        Some(derive_bitcoin_address(&request, script_type, &compressed, &public_key, &secp)?)
    } else {
        None
    };

    Ok(DerivedOutput {
        address,
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(format_secp_public_key(&public_key, request.public_key_format)?))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(secret_key.secret_bytes()))
        } else {
            None
        },
    })
}

fn derive_bitcoin_legacy_family(request: ParsedRequest, version: u8) -> Result<DerivedOutput, String> {
    let (public_key, private_key) = derive_secp_material(&request)?;
    let pubkey_hash = hash160::Hash::hash(&public_key.serialize()).to_byte_array();
    let mut payload = vec![version];
    payload.extend_from_slice(&pubkey_hash);
    let address = if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
        Some(base58check_encode(&payload, bs58::Alphabet::DEFAULT))
    } else {
        None
    };

    Ok(DerivedOutput {
        address,
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(format_secp_public_key(&public_key, request.public_key_format)?))
        } else { None },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else { None },
    })
}

fn derive_litecoin(request: ParsedRequest) -> Result<DerivedOutput, String> {
    derive_bitcoin_legacy_family(request, 0x30)
}

fn derive_dogecoin(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let version = if matches!(request.network, NetworkFlavor::Testnet) { 0x71 } else { 0x1e };
    derive_bitcoin_legacy_family(request, version)
}

fn derive_evm_family(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (public_key, private_key) = derive_secp_material(&request)?;
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
            Some(derive_evm_address(&public_key))
        } else { None },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(format_secp_public_key(&public_key, request.public_key_format)?))
        } else { None },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else { None },
    })
}

fn derive_tron(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (public_key, private_key) = derive_secp_material(&request)?;
    let evm_address = derive_evm_address_bytes(&public_key);
    let mut payload = vec![0x41u8];
    payload.extend_from_slice(&evm_address);
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
            Some(base58check_encode(&payload, bs58::Alphabet::DEFAULT))
        } else { None },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(format_secp_public_key(&public_key, request.public_key_format)?))
        } else { None },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else { None },
    })
}

fn derive_xrp(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (public_key, private_key) = derive_secp_material(&request)?;
    let pubkey_hash = hash160::Hash::hash(&public_key.serialize()).to_byte_array();
    let mut payload = vec![0x00u8];
    payload.extend_from_slice(&pubkey_hash);
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
            Some(base58check_encode(&payload, bs58::Alphabet::RIPPLE))
        } else { None },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(format_secp_public_key(&public_key, request.public_key_format)?))
        } else { None },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else { None },
    })
}

fn derive_solana(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (private_key, public_key) = derive_ed25519_material(&request)?;
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
            Some(bs58::encode(public_key).into_string())
        } else { None },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(public_key))
        } else { None },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else { None },
    })
}

fn derive_stellar(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (private_key, public_key) = derive_ed25519_material(&request)?;
    let mut seed_material = Vec::new();
    seed_material.extend_from_slice(&public_key);
    let encoded = base32_no_pad(&seed_material);
    let stellar_address = format!("G{}", &encoded[..55.min(encoded.len())]);
    let stellar_address = if stellar_address.len() < 56 {
        format!("{}{}", stellar_address, "A".repeat(56 - stellar_address.len()))
    } else {
        stellar_address
    };

    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
            Some(stellar_address)
        } else { None },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(public_key))
        } else { None },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else { None },
    })
}

fn derive_cardano(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (private_key, public_key) = derive_ed25519_material(&request)?;
    let digest = sha256::Hash::hash(&public_key).to_byte_array();
    let address = format!("addr1{}", hex::encode(digest));
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) { Some(address) } else { None },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) { Some(hex::encode(public_key)) } else { None },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) { Some(hex::encode(private_key)) } else { None },
    })
}

fn derive_sui(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (private_key, public_key) = derive_ed25519_material(&request)?;
    let mut hasher = Keccak::v256();
    let mut digest = [0u8; 32];
    hasher.update(&[0x00]);
    hasher.update(&public_key);
    hasher.finalize(&mut digest);
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) { Some(format!("0x{}", hex::encode(digest))) } else { None },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) { Some(hex::encode(public_key)) } else { None },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) { Some(hex::encode(private_key)) } else { None },
    })
}

fn derive_aptos(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (private_key, public_key) = derive_ed25519_material(&request)?;
    let mut hasher = Keccak::v256();
    let mut digest = [0u8; 32];
    hasher.update(&public_key);
    hasher.update(&[0x00]);
    hasher.finalize(&mut digest);
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) { Some(format!("0x{}", hex::encode(digest))) } else { None },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) { Some(hex::encode(public_key)) } else { None },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) { Some(hex::encode(private_key)) } else { None },
    })
}

fn derive_ton(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (private_key, public_key) = derive_ed25519_material(&request)?;
    let digest = sha256::Hash::hash(&public_key).to_byte_array();
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) { Some(format!("0:{}", hex::encode(digest))) } else { None },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) { Some(hex::encode(public_key)) } else { None },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) { Some(hex::encode(private_key)) } else { None },
    })
}

fn derive_icp(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (private_key, public_key) = derive_ed25519_material(&request)?;
    let mut data = Vec::from(public_key);
    data.extend_from_slice(b"icp");
    let digest = sha256::Hash::hash(&data).to_byte_array();
    let digest2 = sha256::Hash::hash(&digest).to_byte_array();
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) { Some(hex::encode(digest2)) } else { None },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) { Some(hex::encode(public_key)) } else { None },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) { Some(hex::encode(private_key)) } else { None },
    })
}

fn derive_near(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (private_key, public_key) = derive_ed25519_material(&request)?;
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) { Some(hex::encode(public_key)) } else { None },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) { Some(hex::encode(public_key)) } else { None },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) { Some(hex::encode(private_key)) } else { None },
    })
}

fn derive_polkadot(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (private_key, public_key) = derive_ed25519_material(&request)?;
    let mut payload = vec![0x00u8];
    payload.extend_from_slice(&public_key);
    payload.extend_from_slice(&sha256::Hash::hash(&public_key).to_byte_array()[..2]);
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
            Some(bs58::encode(payload).into_string())
        } else { None },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) { Some(hex::encode(public_key)) } else { None },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) { Some(hex::encode(private_key)) } else { None },
    })
}

fn derive_bip32_xpriv(
    seed_bytes: &[u8],
    network: Network,
    derivation_path: &str,
    hmac_key: Option<&str>,
) -> Result<Xpriv, String> {
    if let Some(hmac_key) = hmac_key {
        if !hmac_key.is_empty() && hmac_key != "Bitcoin seed" {
            return Err("Custom HMAC master key is not supported for BIP-32 derivation.".to_string());
        }
    }
    let master = Xpriv::new_master(network, seed_bytes).map_err(display_error)?;
    let path: DerivationPath = derivation_path.parse().map_err(display_error)?;
    let secp = Secp256k1::<All>::new();
    master.derive_priv(&secp, &path).map_err(display_error)
}

fn derive_bitcoin_address(
    request: &ParsedRequest,
    script_type: ScriptType,
    compressed_public_key: &CompressedPublicKey,
    public_key: &bitcoin::secp256k1::PublicKey,
    secp: &Secp256k1<All>,
) -> Result<String, String> {
    let network = match request.network {
        NetworkFlavor::Mainnet => Network::Bitcoin,
        NetworkFlavor::Testnet | NetworkFlavor::Testnet4 | NetworkFlavor::Signet => Network::Testnet,
    };

    derive_bitcoin_address_for_network(network, script_type, compressed_public_key, public_key, secp)
}

fn derive_bitcoin_address_for_network(
    network: Network,
    script_type: ScriptType,
    compressed_public_key: &CompressedPublicKey,
    public_key: &bitcoin::secp256k1::PublicKey,
    secp: &Secp256k1<All>,
) -> Result<String, String> {

    let address = match script_type {
        ScriptType::P2pkh => Address::p2pkh(compressed_public_key, network),
        ScriptType::P2shP2wpkh => Address::p2shwpkh(compressed_public_key, network),
        ScriptType::P2wpkh => Address::p2wpkh(compressed_public_key, network),
        ScriptType::P2tr => {
            let (x_only, _) = public_key.x_only_public_key();
            Address::p2tr(secp, x_only, None, network)
        }
        _ => return Err("Unsupported Bitcoin script type.".to_string()),
    };

    Ok(address.to_string())
}

fn bitcoin_script_type(request: &ParsedRequest, derivation_path: &str) -> Result<ScriptType, String> {
    match request.script_type {
        ScriptType::Auto => infer_bitcoin_script_type(request.address_algorithm, derivation_path),
        other => Ok(other),
    }
}

fn infer_bitcoin_script_type(address_algorithm: AddressAlgorithm, derivation_path: &str) -> Result<ScriptType, String> {
    match address_algorithm {
        AddressAlgorithm::Auto | AddressAlgorithm::Bitcoin => {
            let purpose = derivation_path
                .split('/')
                .nth(1)
                .ok_or_else(|| "Invalid Bitcoin derivation path.".to_string())?
                .trim_end_matches('\'')
                .parse::<u32>()
                .map_err(display_error)?;
            match purpose {
                44 => Ok(ScriptType::P2pkh),
                49 => Ok(ScriptType::P2shP2wpkh),
                84 => Ok(ScriptType::P2wpkh),
                86 => Ok(ScriptType::P2tr),
                _ => Ok(ScriptType::P2pkh),
            }
        }
        _ => Err("Bitcoin requests require the Bitcoin address algorithm.".to_string()),
    }
}

fn derive_bip39_seed(seed_phrase: &str, passphrase: &str, iteration_count: u32) -> Result<Zeroizing<[u8; 64]>, String> {
    let mnemonic = Mnemonic::parse_in_normalized(Language::English, seed_phrase).map_err(display_error)?;
    let iterations = if iteration_count == 0 { 2048 } else { iteration_count };
    let normalized_mnemonic = Zeroizing::new(mnemonic.to_string().nfkd().collect::<String>());
    let normalized_passphrase = Zeroizing::new(passphrase.nfkd().collect::<String>());
    let salt = Zeroizing::new(format!("mnemonic{}", normalized_passphrase.as_str()));
    let mut seed = Zeroizing::new([0u8; 64]);
    pbkdf2_hmac::<Sha512>(normalized_mnemonic.as_bytes(), salt.as_bytes(), iterations, &mut *seed);
    Ok(seed)
}

fn derive_solana_ed25519_key(seed: &[u8], derivation_path: &str, hmac_key: Option<&str>) -> Result<Zeroizing<[u8; 32]>, String> {
    if let Some(hmac_key) = hmac_key {
        if !hmac_key.is_empty() && hmac_key != "ed25519 seed" {
            return Err("Custom HMAC master key is not supported for ed25519 derivation.".to_string());
        }
    }

    let canonical_path = canonicalize_ed25519_path(derivation_path);
    let path = slip10::BIP32Path::from_str(&canonical_path).map_err(display_error)?;
    let node = derive_key_from_path(seed, Curve::Ed25519, &path).map_err(display_error)?;
    Ok(Zeroizing::new(node.key))
}

fn canonicalize_ed25519_path(path: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for (index, part) in path.split('/').enumerate() {
        if index == 0 {
            out.push(part.to_string());
            continue;
        }
        if part.ends_with('\'') {
            out.push(part.to_string());
        } else {
            out.push(format!("{}'", part));
        }
    }
    out.join("/")
}

fn secp_derivation_path(request: &ParsedRequest) -> String {
    request.derivation_path.clone().unwrap_or_else(|| match request.chain {
        Chain::Bitcoin => "m/84'/0'/0'/0/0".to_string(),
        Chain::BitcoinCash => "m/44'/145'/0'/0/0".to_string(),
        Chain::BitcoinSv => "m/44'/236'/0'/0/0".to_string(),
        Chain::Litecoin => "m/44'/2'/0'/0/0".to_string(),
        Chain::Dogecoin => "m/44'/3'/0'/0/0".to_string(),
        Chain::Ethereum => "m/44'/60'/0'/0/0".to_string(),
        Chain::EthereumClassic => "m/44'/61'/0'/0/0".to_string(),
        Chain::Arbitrum | Chain::Optimism | Chain::Avalanche | Chain::Hyperliquid => "m/44'/60'/0'/0/0".to_string(),
        Chain::Tron => "m/44'/195'/0'/0/0".to_string(),
        Chain::Xrp => "m/44'/144'/0'/0/0".to_string(),
        _ => "m/44'/0'/0'/0/0".to_string(),
    })
}

fn ed25519_derivation_path(request: &ParsedRequest) -> String {
    request.derivation_path.clone().unwrap_or_else(|| match request.chain {
        Chain::Solana => "m/44'/501'/0'/0'".to_string(),
        Chain::Stellar => "m/44'/148'/0'".to_string(),
        Chain::Cardano => "m/1852'/1815'/0'/0/0".to_string(),
        Chain::Sui => "m/44'/784'/0'/0'/0'".to_string(),
        Chain::Aptos => "m/44'/637'/0'/0'/0'".to_string(),
        Chain::Ton => "m/44'/607'/0'/0/0".to_string(),
        Chain::InternetComputer => "m/44'/223'/0'/0/0".to_string(),
        Chain::Near => "m/44'/397'/0'".to_string(),
        Chain::Polkadot => "m/44'/354'/0'".to_string(),
        _ => "m/44'/501'/0'/0'".to_string(),
    })
}

fn derive_evm_address(public_key: &bitcoin::secp256k1::PublicKey) -> String {
    format!("0x{}", hex::encode(derive_evm_address_bytes(public_key)))
}

fn derive_evm_address_bytes(public_key: &bitcoin::secp256k1::PublicKey) -> [u8; 20] {
    let uncompressed = public_key.serialize_uncompressed();
    let mut hasher = Keccak::v256();
    let mut digest = [0u8; 32];
    hasher.update(&uncompressed[1..]);
    hasher.finalize(&mut digest);
    let mut out = [0u8; 20];
    out.copy_from_slice(&digest[12..]);
    out
}

fn format_secp_public_key(public_key: &bitcoin::secp256k1::PublicKey, format: PublicKeyFormat) -> Result<Vec<u8>, String> {
    Ok(match format {
        PublicKeyFormat::Auto | PublicKeyFormat::Compressed => public_key.serialize().to_vec(),
        PublicKeyFormat::Uncompressed => public_key.serialize_uncompressed().to_vec(),
        PublicKeyFormat::XOnly => public_key.x_only_public_key().0.serialize().to_vec(),
        PublicKeyFormat::Raw => public_key.serialize().to_vec(),
    })
}

fn base58check_encode(payload: &[u8], alphabet: &bs58::Alphabet) -> String {
    let checksum = double_sha256(payload);
    let mut full = Vec::with_capacity(payload.len() + 4);
    full.extend_from_slice(payload);
    full.extend_from_slice(&checksum[..4]);
    bs58::encode(full).with_alphabet(alphabet).into_string()
}

fn double_sha256(bytes: &[u8]) -> [u8; 32] {
    let first = sha256::Hash::hash(bytes).to_byte_array();
    sha256::Hash::hash(&first).to_byte_array()
}

fn base32_no_pad(input: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut output = String::new();
    let mut buffer: u32 = 0;
    let mut bits_left = 0u8;

    for &byte in input {
        buffer = (buffer << 8) | u32::from(byte);
        bits_left += 8;
        while bits_left >= 5 {
            let idx = ((buffer >> (bits_left - 5)) & 0x1f) as usize;
            output.push(ALPHABET[idx] as char);
            bits_left -= 5;
        }
    }

    if bits_left > 0 {
        let idx = ((buffer << (5 - bits_left)) & 0x1f) as usize;
        output.push(ALPHABET[idx] as char);
    }

    output
}

fn read_buffer_to_string(buffer: &SpectraBuffer) -> Result<String, String> {
    let bytes = read_buffer(buffer);
    std::str::from_utf8(bytes)
        .map(|value| value.to_string())
        .map_err(display_error)
}

fn optional_trimmed_string(buffer: &SpectraBuffer) -> Result<Option<String>, String> {
    let value = read_buffer_to_string(buffer)?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

fn optional_untrimmed_string(buffer: &SpectraBuffer) -> Result<Option<String>, String> {
    let value = read_buffer_to_string(buffer)?;
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

fn normalize_seed_phrase(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn requests_output(requested_outputs: u32, output: u32) -> bool {
    requested_outputs & output != 0
}

fn free_buffer(buffer: SpectraBuffer) {
    if buffer.ptr.is_null() || buffer.len == 0 {
        return;
    }

    unsafe {
        let _ = Vec::from_raw_parts(buffer.ptr, buffer.len, buffer.len);
    }
}

fn read_buffer<'a>(buffer: &'a SpectraBuffer) -> &'a [u8] {
    if buffer.ptr.is_null() || buffer.len == 0 {
        return &[];
    }
    unsafe { slice::from_raw_parts(buffer.ptr.cast_const(), buffer.len) }
}

fn display_error(error: impl Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_request(chain: Chain, curve: CurveFamily) -> ParsedRequest {
        ParsedRequest {
            chain,
            network: NetworkFlavor::Mainnet,
            curve,
            requested_outputs: OUTPUT_ADDRESS | OUTPUT_PUBLIC_KEY | OUTPUT_PRIVATE_KEY,
            derivation_algorithm: DerivationAlgorithm::Auto,
            address_algorithm: AddressAlgorithm::Auto,
            public_key_format: PublicKeyFormat::Auto,
            script_type: ScriptType::Auto,
            seed_phrase: "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".to_string(),
            derivation_path: None,
            passphrase: String::new(),
            hmac_key: None,
            mnemonic_wordlist: Some("english".to_string()),
            iteration_count: 2048,
        }
    }

    #[test]
    fn derives_all_supported_chains() {
        let chains = [
            (Chain::Bitcoin, CurveFamily::Secp256k1),
            (Chain::BitcoinCash, CurveFamily::Secp256k1),
            (Chain::BitcoinSv, CurveFamily::Secp256k1),
            (Chain::Litecoin, CurveFamily::Secp256k1),
            (Chain::Dogecoin, CurveFamily::Secp256k1),
            (Chain::Ethereum, CurveFamily::Secp256k1),
            (Chain::EthereumClassic, CurveFamily::Secp256k1),
            (Chain::Arbitrum, CurveFamily::Secp256k1),
            (Chain::Optimism, CurveFamily::Secp256k1),
            (Chain::Avalanche, CurveFamily::Secp256k1),
            (Chain::Hyperliquid, CurveFamily::Secp256k1),
            (Chain::Tron, CurveFamily::Secp256k1),
            (Chain::Xrp, CurveFamily::Secp256k1),
            (Chain::Solana, CurveFamily::Ed25519),
            (Chain::Stellar, CurveFamily::Ed25519),
            (Chain::Cardano, CurveFamily::Ed25519),
            (Chain::Sui, CurveFamily::Ed25519),
            (Chain::Aptos, CurveFamily::Ed25519),
            (Chain::Ton, CurveFamily::Ed25519),
            (Chain::InternetComputer, CurveFamily::Ed25519),
            (Chain::Near, CurveFamily::Ed25519),
            (Chain::Polkadot, CurveFamily::Ed25519),
        ];

        for (chain, curve) in chains {
            let result = derive(base_request(chain, curve))
                .unwrap_or_else(|e| panic!("failed to derive {}: {e}", chain_name(chain)));
            assert!(
                result.address.as_deref().is_some_and(|v| !v.trim().is_empty()),
                "missing address for {}",
                chain_name(chain)
            );
            assert!(
                result.public_key_hex.as_deref().is_some_and(|v| !v.trim().is_empty()),
                "missing public key for {}",
                chain_name(chain)
            );
            assert!(
                result.private_key_hex.as_deref().is_some_and(|v| !v.trim().is_empty()),
                "missing private key for {}",
                chain_name(chain)
            );
        }
    }

    fn chain_name(chain: Chain) -> &'static str {
        match chain {
            Chain::Bitcoin => "bitcoin",
            Chain::BitcoinCash => "bitcoin_cash",
            Chain::BitcoinSv => "bitcoin_sv",
            Chain::Litecoin => "litecoin",
            Chain::Dogecoin => "dogecoin",
            Chain::Ethereum => "ethereum",
            Chain::EthereumClassic => "ethereum_classic",
            Chain::Arbitrum => "arbitrum",
            Chain::Optimism => "optimism",
            Chain::Avalanche => "avalanche",
            Chain::Hyperliquid => "hyperliquid",
            Chain::Tron => "tron",
            Chain::Solana => "solana",
            Chain::Stellar => "stellar",
            Chain::Xrp => "xrp",
            Chain::Cardano => "cardano",
            Chain::Sui => "sui",
            Chain::Aptos => "aptos",
            Chain::Ton => "ton",
            Chain::InternetComputer => "internet_computer",
            Chain::Near => "near",
            Chain::Polkadot => "polkadot",
        }
    }
}
