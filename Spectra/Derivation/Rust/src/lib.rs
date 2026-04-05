use bip39::{Language, Mnemonic};
use bitcoin::bip32::{DerivationPath, Xpriv};
use bitcoin::key::{CompressedPublicKey, PublicKey};
use bitcoin::secp256k1::{All, Secp256k1};
use bitcoin::{Address, Network};
use std::fmt::Display;
use std::ptr;
use std::slice;

const STATUS_OK: i32 = 0;
const STATUS_ERROR: i32 = 1;

const OUTPUT_ADDRESS: u32 = 1 << 0;
const OUTPUT_PUBLIC_KEY: u32 = 1 << 1;
const OUTPUT_PRIVATE_KEY: u32 = 1 << 2;

const CHAIN_BITCOIN: u32 = 0;
const CHAIN_ETHEREUM: u32 = 1;
const CHAIN_SOLANA: u32 = 2;

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

#[derive(Clone, Copy)]
enum Chain {
    Bitcoin,
    Ethereum,
    Solana,
}

#[derive(Clone, Copy)]
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
    let passphrase = optional_trimmed_string(&request.passphrase_utf8)?.unwrap_or_default();
    let hmac_key = optional_trimmed_string(&request.hmac_key_utf8)?;
    let mnemonic_wordlist = optional_trimmed_string(&request.mnemonic_wordlist_utf8)?;

    if request.requested_outputs == 0 {
        return Err("At least one output must be requested.".to_string());
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
        other => Err(unsupported_chain_error(other)),
    }
}

fn validate_request(request: &ParsedRequest) -> Result<(), String> {
    if request.iteration_count != 0 && request.iteration_count != 2048 {
        return Err(format!(
            "Unsupported iteration count: {}. Only the standard BIP-39 count is supported right now.",
            request.iteration_count
        ));
    }

    if let Some(hmac_key) = &request.hmac_key {
        if !hmac_key.is_empty() {
            return Err("Custom HMAC key string is not supported yet.".to_string());
        }
    }

    if let Some(wordlist) = &request.mnemonic_wordlist {
        if !wordlist.eq_ignore_ascii_case("english") {
            return Err("Only the English mnemonic wordlist is supported in Rust right now.".to_string());
        }
    }

    match request.chain {
        Chain::Bitcoin => {
            if request.curve != CurveFamily::Secp256k1 {
                return Err("Bitcoin currently requires secp256k1.".to_string());
            }
            match request.derivation_algorithm {
                DerivationAlgorithm::Auto | DerivationAlgorithm::Bip32Secp256k1 => {}
                DerivationAlgorithm::Slip10Ed25519 => {
                    return Err("Bitcoin does not support the SLIP-0010 ed25519 derivation algorithm.".to_string())
                }
            }
        }
        other => return Err(unsupported_chain_error(other)),
    }

    Ok(())
}

fn unsupported_chain_error(chain: Chain) -> String {
    match chain {
        Chain::Bitcoin => "Bitcoin is supported in the Rust core.".to_string(),
        Chain::Ethereum => "Rust derivation does not support Ethereum yet.".to_string(),
        Chain::Solana => "Rust derivation does not support Solana yet.".to_string(),
    }
}

fn derive_bitcoin(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let secp = Secp256k1::new();
    let derivation_path = bitcoin_derivation_path(&request);
    let script_type = bitcoin_script_type(&request, &derivation_path)?;
    let xpriv = derive_bip32_xpriv(&request.seed_phrase, &request.passphrase, &derivation_path)?;
    let secret_key = xpriv.private_key;
    let public_key = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
    let bitcoin_public_key = PublicKey::new(public_key);
    let compressed_public_key =
        CompressedPublicKey::try_from(bitcoin_public_key).map_err(display_error)?;

    let address = if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
        Some(derive_bitcoin_address(
            &request,
            script_type,
            &compressed_public_key,
            &public_key,
            &secp,
        )?)
    } else {
        None
    };

    let public_key_hex = if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
        Some(hex::encode(format_secp_public_key(
            &public_key,
            request.public_key_format,
        )?))
    } else {
        None
    };

    let private_key_hex = if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
        Some(hex::encode(secret_key.secret_bytes()))
    } else {
        None
    };

    Ok(DerivedOutput {
        address,
        public_key_hex,
        private_key_hex,
    })
}

fn derive_bip32_xpriv(
    seed_phrase: &str,
    passphrase: &str,
    derivation_path: &str,
) -> Result<Xpriv, String> {
    let mnemonic = Mnemonic::parse_in_normalized(Language::English, seed_phrase).map_err(display_error)?;
    let seed = mnemonic.to_seed_normalized(passphrase);
    let network = Network::Bitcoin;
    let master = Xpriv::new_master(network, &seed).map_err(display_error)?;
    let path: DerivationPath = derivation_path.parse().map_err(display_error)?;
    let secp = Secp256k1::<All>::new();
    Ok(master.derive_priv(&secp, &path).map_err(display_error)?)
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

    let address = match script_type {
        ScriptType::P2pkh => Address::p2pkh(compressed_public_key, network),
        ScriptType::P2shP2wpkh => Address::p2shwpkh(compressed_public_key, network).map_err(display_error)?,
        ScriptType::P2wpkh => Address::p2wpkh(compressed_public_key, network).map_err(display_error)?,
        ScriptType::P2tr => {
            let (x_only, _) = public_key.x_only_public_key();
            Address::p2tr(secp, x_only, None, network)
        }
        _ => return Err("Unsupported Bitcoin script type.".to_string()),
    };

    Ok(address.to_string())
}

fn bitcoin_derivation_path(request: &ParsedRequest) -> String {
    request
        .derivation_path
        .clone()
        .unwrap_or_else(|| "m/84'/0'/0'/0/0".to_string())
}

fn bitcoin_script_type(request: &ParsedRequest, derivation_path: &str) -> Result<ScriptType, String> {
    match request.script_type {
        ScriptType::Auto => infer_bitcoin_script_type(request.address_algorithm, derivation_path),
        other => Ok(other),
    }
}

fn infer_bitcoin_script_type(
    address_algorithm: AddressAlgorithm,
    derivation_path: &str,
) -> Result<ScriptType, String> {
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
                _ => Err(format!("Unsupported Bitcoin purpose: {purpose}")),
            }
        }
        _ => Err("Bitcoin requests require the Bitcoin address algorithm.".to_string()),
    }
}

fn format_secp_public_key(
    public_key: &bitcoin::secp256k1::PublicKey,
    format: PublicKeyFormat,
) -> Result<Vec<u8>, String> {
    Ok(match format {
        PublicKeyFormat::Auto | PublicKeyFormat::Compressed => public_key.serialize().to_vec(),
        PublicKeyFormat::Uncompressed => public_key.serialize_uncompressed().to_vec(),
        PublicKeyFormat::XOnly => public_key.x_only_public_key().0.serialize().to_vec(),
        PublicKeyFormat::Raw => public_key.serialize().to_vec(),
    })
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
