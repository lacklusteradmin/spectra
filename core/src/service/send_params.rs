use crate::send::keys::SecretHex;

// Internal typed requests. No JSON deserialization or secret serialization.

/// What `execute_send` signs, once `build_send_params` has resolved which
/// chain and which struct. One `match` — in `sign_and_broadcast_send` — reads
/// this; nothing downstream re-derives `Chain` from a string to find out.
#[derive(Debug)]
pub(crate) enum SendParams {
    Bitcoin(BitcoinNativeSendParams),
    /// Carries the overrides alongside the params: they cross to the client
    /// call as a separate argument (`sign_and_broadcast_with_overrides`), not
    /// through a field on `EvmNativeSendParams`.
    Evm(
        EvmNativeSendParams,
        crate::send::chains::evm::EvmSendOverrides,
    ),
    Solana(SolanaNativeSendParams),
    Xrp(XrpSendParams),
    Tron(TronNativeSendParams),
    Sui(SuiSendParams),
    Aptos(AptosSendParams),
    Near(NearNativeSendParams),
    Utxo(UtxoFixedFeeSendParams),
    Zcash(ZcashSendParams),
    Decred(DecredSendParams),
    Kaspa(KaspaSendParams),
    Stellar(StellarSendParams),
    Cardano(CardanoSendParams),
    Polkadot(PolkadotSendParams),
    Bittensor(BittensorSendParams),
    Ton(TonSendParams),
    Icp(IcpSendParams),
    Monero(MoneroSendParams),
}

/// The token-send counterpart of [`SendParams`]. Fewer variants: only the
/// families `execute_send` can build a token transfer for today.
#[derive(Debug)]
pub(crate) enum SendTokenParams {
    Evm(
        TokenAmountSendParams,
        crate::send::chains::evm::EvmSendOverrides,
    ),
    Tron(TronTokenSendParams),
    Near(NearTokenSendParams),
    Solana(SolanaTokenSendParams),
}

/// What `build_send_params` hands `sign_and_broadcast_send`: which of the two
/// kinds this send is, carrying the one already-typed struct that decides.
#[derive(Debug)]
pub(crate) enum ExecuteSendParams {
    Native(SendParams),
    Token(SendTokenParams),
}

/// `Chain::Polkadot` send parameters. `planck` is the smallest unit
/// (10⁻¹⁰ DOT). The 32-byte `private_key_hex` is the sr25519 mini-secret
/// produced by `derive_polkadot`, *not* a 64-byte ed25519 secret.
#[derive(Debug)]
pub(crate) struct PolkadotSendParams {
    pub from: String,
    pub to: String,
    pub planck: u128,
    pub private_key_hex: SecretHex,
    pub public_key_hex: String,
    /// SCALE-encoded era bytes. `None` → immortal (`[0x00]`).
    pub era: Option<Vec<u8>>,
    /// Tip in planck. `None` → 0.
    pub tip: Option<u128>,
}

/// `Chain::Bittensor` send parameters. `rao` is the smallest unit
/// (10⁻⁹ TAO). Same sr25519 32-byte mini-secret rules as Polkadot.
#[derive(Debug)]
pub(crate) struct BittensorSendParams {
    pub from: String,
    pub to: String,
    pub rao: u128,
    pub private_key_hex: SecretHex,
    pub public_key_hex: String,
}

#[derive(Debug)]
pub(crate) struct BitcoinNativeSendParams {
    pub from: String,
    pub to: String,
    pub amount_sat: u64,
    pub fee_rate_svb: Option<f64>,
    pub private_key_hex: SecretHex,
    pub dust_threshold_sats: Option<u64>,
    pub sign_only: bool,
}

#[derive(Debug)]
pub(crate) struct EvmNativeSendParams {
    pub from: String,
    pub to: String,
    pub value_wei: u128,
    pub private_key_hex: SecretHex,
}

#[derive(Debug)]
pub(crate) struct SolanaNativeSendParams {
    pub from_pubkey_hex: String,
    pub to: String,
    pub lamports: u64,
    pub private_key_hex: SecretHex,
}

#[derive(Debug)]
pub(crate) struct XrpSendParams {
    pub from: String,
    pub to: String,
    pub drops: u64,
    pub private_key_hex: SecretHex,
    pub public_key_hex: Option<String>,
}

#[derive(Debug)]
pub(crate) struct TronNativeSendParams {
    pub from: String,
    pub to: String,
    pub amount_sun: u64,
    pub private_key_hex: SecretHex,
}

#[derive(Debug)]
pub(crate) struct SuiSendParams {
    pub from: String,
    pub to: String,
    pub mist: u64,
    pub gas_budget: Option<u64>,
    pub private_key_hex: SecretHex,
    pub public_key_hex: String,
}

#[derive(Debug)]
pub(crate) struct AptosSendParams {
    pub from: String,
    pub to: String,
    pub octas: u64,
    pub private_key_hex: SecretHex,
    pub public_key_hex: String,
}

#[derive(Debug)]
pub(crate) struct NearNativeSendParams {
    pub from: String,
    pub to: String,
    pub yocto_near: u128,
    pub private_key_hex: SecretHex,
    pub public_key_hex: String,
}

#[derive(Debug)]
pub(crate) struct UtxoFixedFeeSendParams {
    pub from: String,
    pub to: String,
    pub amount_sat: u64,
    pub fee_sat: Option<u64>,
    pub private_key_hex: SecretHex,
    pub dust_threshold_sats: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct ZcashSendParams {
    pub from: String,
    pub to: String,
    pub amount_sat: u64,
    pub fee_sat: Option<u64>,
    pub private_key_hex: SecretHex,
    pub dust_threshold_zats: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct DecredSendParams {
    pub from: String,
    pub to: String,
    pub amount_sat: u64,
    pub fee_sat: Option<u64>,
    pub private_key_hex: SecretHex,
    pub dust_threshold_atoms: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct KaspaSendParams {
    pub from: String,
    pub to: String,
    pub amount_sat: u64,
    pub fee_sat: Option<u64>,
    pub private_key_hex: SecretHex,
    pub min_fee_sompi: Option<u64>,
    pub dust_threshold_sompi: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct StellarSendParams {
    pub from: String,
    pub to: String,
    pub stroops: i64,
    pub private_key_hex: SecretHex,
    pub public_key_hex: Option<String>,
    pub network_passphrase: Option<String>,
}

#[derive(Debug)]
pub(crate) struct CardanoSendParams {
    pub from: String,
    pub to: String,
    pub amount_lovelace: u64,
    pub fee_lovelace: Option<u64>,
    pub private_key_hex: SecretHex,
    pub public_key_hex: String,
    pub ttl_slots: Option<u64>,
    pub min_change_lovelace: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct TonSendParams {
    pub from: String,
    pub to: String,
    pub nanotons: u64,
    pub comment: Option<String>,
    pub private_key_hex: SecretHex,
    pub public_key_hex: String,
    pub subwallet_id: Option<u64>,
    pub expiry_seconds: Option<u64>,
    pub send_mode: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct IcpSendParams {
    pub from: String,
    pub to: String,
    pub e8s: u64,
    pub private_key_hex: SecretHex,
    pub public_key_hex: Option<String>,
}

#[derive(Debug)]
pub(crate) struct MoneroSendParams {
    pub from: String,
    pub to: String,
    pub piconeros: u64,
    pub priority: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct TokenAmountSendParams {
    pub from: String,
    pub contract: String,
    pub to: String,
    pub amount_raw: u128,
    pub private_key_hex: SecretHex,
}

#[derive(Debug)]
pub(crate) struct TronTokenSendParams {
    pub from: String,
    pub contract: String,
    pub to: String,
    pub amount_raw: u128,
    pub fee_limit_sun: Option<u64>,
    pub private_key_hex: SecretHex,
}

#[derive(Debug)]
pub(crate) struct NearTokenSendParams {
    pub from: String,
    pub contract: String,
    pub to: String,
    pub amount_raw: u128,
    pub private_key_hex: SecretHex,
    pub public_key_hex: String,
    pub gas_tgas: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct SolanaTokenSendParams {
    pub from_pubkey_hex: String,
    pub to: String,
    pub mint: String,
    pub amount_raw: u64,
    pub decimals: u8,
    pub private_key_hex: SecretHex,
}
