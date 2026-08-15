use serde::Deserialize;

// ── Per-chain `sign_and_send` parameter shapes ────────────────────────────
//
// Each chain's `sign_and_send` arm in `super::send` historically read its
// inputs by pulling individual fields out of a `serde_json::Value` with
// inline `.as_str()` / `.as_u64()` / `try_into()` chains. That style hides
// the contract — a reader can't see at a glance what shape the Polkadot
// endpoint expects without scanning the full arm body.
//
// Defining a typed struct per chain reverses that: the type doc *is* the
// API contract, serde gives field-name-aware error messages for free, and
// the dispatch arm collapses to one `parse_params` call.
//
// These structs accept the same JSON shape Swift already produces so this
// migration is internal — no FFI signature changes.

/// `Chain::Polkadot` send parameters. `planck` is the smallest unit
/// (10⁻¹⁰ DOT). The 32-byte `private_key_hex` is the sr25519 mini-secret
/// produced by `derive_polkadot`, *not* a 64-byte ed25519 secret.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PolkadotSendParams {
    pub from: String,
    pub to: String,
    /// Accepts either a JSON string ("12500000000") or a JSON number for
    /// backward compatibility with Swift call sites that emitted both forms.
    #[serde(deserialize_with = "deserialize_u128_from_string_or_number")]
    pub planck: u128,
    pub private_key_hex: String,
    pub public_key_hex: String,
    /// SCALE-encoded era bytes. `None` → immortal (`[0x00]`).
    #[serde(default)]
    pub era: Option<Vec<u8>>,
    /// Tip in planck. `None` → 0.
    #[serde(
        default,
        deserialize_with = "deserialize_option_u128_from_string_or_number"
    )]
    pub tip: Option<u128>,
}

/// `Chain::Bittensor` send parameters. `rao` is the smallest unit
/// (10⁻⁹ TAO). Same sr25519 32-byte mini-secret rules as Polkadot.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BittensorSendParams {
    pub from: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u128_from_string_or_number")]
    pub rao: u128,
    pub private_key_hex: String,
    pub public_key_hex: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct BitcoinNativeSendParams {
    pub from: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u64_from_string_or_number")]
    pub amount_sat: u64,
    #[serde(default)]
    pub fee_rate_svb: Option<f64>,
    pub private_key_hex: String,
    #[serde(default)]
    pub dust_threshold_sats: Option<u64>,
    #[serde(default)]
    pub sign_only: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct EvmNativeSendParams {
    pub from: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u128_from_string_or_number")]
    pub value_wei: u128,
    pub private_key_hex: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SolanaNativeSendParams {
    pub from_pubkey_hex: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u64_from_string_or_number")]
    pub lamports: u64,
    pub private_key_hex: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct XrpSendParams {
    pub from: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u64_from_string_or_number")]
    pub drops: u64,
    pub private_key_hex: String,
    #[serde(default)]
    pub public_key_hex: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TronNativeSendParams {
    pub from: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u64_from_string_or_number")]
    pub amount_sun: u64,
    pub private_key_hex: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SuiSendParams {
    pub from: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u64_from_string_or_number")]
    pub mist: u64,
    #[serde(default)]
    pub gas_budget: Option<u64>,
    pub private_key_hex: String,
    pub public_key_hex: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AptosSendParams {
    pub from: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u64_from_string_or_number")]
    pub octas: u64,
    pub private_key_hex: String,
    pub public_key_hex: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct NearNativeSendParams {
    pub from: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u128_from_string_or_number")]
    pub yocto_near: u128,
    pub private_key_hex: String,
    pub public_key_hex: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct UtxoFixedFeeSendParams {
    pub from: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u64_from_string_or_number")]
    pub amount_sat: u64,
    #[serde(
        default,
        deserialize_with = "deserialize_option_u64_from_string_or_number"
    )]
    pub fee_sat: Option<u64>,
    pub private_key_hex: String,
    #[serde(
        default,
        deserialize_with = "deserialize_option_u64_from_string_or_number"
    )]
    pub dust_threshold_sats: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ZcashSendParams {
    pub from: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u64_from_string_or_number")]
    pub amount_sat: u64,
    #[serde(
        default,
        deserialize_with = "deserialize_option_u64_from_string_or_number"
    )]
    pub fee_sat: Option<u64>,
    pub private_key_hex: String,
    #[serde(
        default,
        deserialize_with = "deserialize_option_u64_from_string_or_number"
    )]
    pub dust_threshold_zats: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DecredSendParams {
    pub from: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u64_from_string_or_number")]
    pub amount_sat: u64,
    #[serde(
        default,
        deserialize_with = "deserialize_option_u64_from_string_or_number"
    )]
    pub fee_sat: Option<u64>,
    pub private_key_hex: String,
    #[serde(
        default,
        deserialize_with = "deserialize_option_u64_from_string_or_number"
    )]
    pub dust_threshold_atoms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct KaspaSendParams {
    pub from: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u64_from_string_or_number")]
    pub amount_sat: u64,
    #[serde(
        default,
        deserialize_with = "deserialize_option_u64_from_string_or_number"
    )]
    pub fee_sat: Option<u64>,
    pub private_key_hex: String,
    #[serde(
        default,
        deserialize_with = "deserialize_option_u64_from_string_or_number"
    )]
    pub min_fee_sompi: Option<u64>,
    #[serde(
        default,
        deserialize_with = "deserialize_option_u64_from_string_or_number"
    )]
    pub dust_threshold_sompi: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct StellarSendParams {
    pub from: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_i64_from_string_or_number")]
    pub stroops: i64,
    pub private_key_hex: String,
    #[serde(default)]
    pub public_key_hex: Option<String>,
    #[serde(default)]
    pub network_passphrase: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CardanoSendParams {
    pub from: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u64_from_string_or_number")]
    pub amount_lovelace: u64,
    #[serde(
        default,
        deserialize_with = "deserialize_option_u64_from_string_or_number"
    )]
    pub fee_lovelace: Option<u64>,
    pub private_key_hex: String,
    pub public_key_hex: String,
    #[serde(
        default,
        deserialize_with = "deserialize_option_u64_from_string_or_number"
    )]
    pub ttl_slots: Option<u64>,
    #[serde(
        default,
        deserialize_with = "deserialize_option_u64_from_string_or_number"
    )]
    pub min_change_lovelace: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TonSendParams {
    pub from: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u64_from_string_or_number")]
    pub nanotons: u64,
    #[serde(default)]
    pub comment: Option<String>,
    pub private_key_hex: String,
    pub public_key_hex: String,
    #[serde(
        default,
        deserialize_with = "deserialize_option_u64_from_string_or_number"
    )]
    pub subwallet_id: Option<u64>,
    #[serde(
        default,
        deserialize_with = "deserialize_option_u64_from_string_or_number"
    )]
    pub expiry_seconds: Option<u64>,
    #[serde(
        default,
        deserialize_with = "deserialize_option_u64_from_string_or_number"
    )]
    pub send_mode: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct IcpSendParams {
    pub from: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u64_from_string_or_number")]
    pub e8s: u64,
    pub private_key_hex: String,
    #[serde(default)]
    pub public_key_hex: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MoneroSendParams {
    pub to: String,
    #[serde(deserialize_with = "deserialize_u64_from_string_or_number")]
    pub piconeros: u64,
    #[serde(
        default,
        deserialize_with = "deserialize_option_u64_from_string_or_number"
    )]
    pub priority: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TokenAmountSendParams {
    pub from: String,
    pub contract: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u128_from_string_or_number")]
    pub amount_raw: u128,
    pub private_key_hex: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TronTokenSendParams {
    pub from: String,
    pub contract: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u128_from_string_or_number")]
    pub amount_raw: u128,
    #[serde(
        default,
        deserialize_with = "deserialize_option_u64_from_string_or_number"
    )]
    pub fee_limit_sun: Option<u64>,
    pub private_key_hex: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct StellarTokenSendParams {
    pub from: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_i64_from_string_or_number")]
    pub stroops: i64,
    pub asset_code: String,
    pub asset_issuer: String,
    pub private_key_hex: String,
    pub public_key_hex: String,
    #[serde(default)]
    pub network_passphrase: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct NearTokenSendParams {
    pub from: String,
    pub contract: String,
    pub to: String,
    #[serde(deserialize_with = "deserialize_u128_from_string_or_number")]
    pub amount_raw: u128,
    pub private_key_hex: String,
    pub public_key_hex: String,
    #[serde(
        default,
        deserialize_with = "deserialize_option_u64_from_string_or_number"
    )]
    pub gas_tgas: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SolanaTokenSendParams {
    pub from_pubkey_hex: String,
    pub to: String,
    pub mint: String,
    #[serde(deserialize_with = "deserialize_u64_from_string_or_number")]
    pub amount_raw: u64,
    #[serde(deserialize_with = "deserialize_u8_from_string_or_number")]
    pub decimals: u8,
    pub private_key_hex: String,
}

/// Accepts JSON `"12345"` or `12345` for u128 fields. Swift sends planck
/// values as strings (since u128 doesn't round-trip safely through JSON
/// numbers) but legacy call sites emitted them as `as_u64`-able numbers.
fn deserialize_u128_from_string_or_number<'de, D>(deserializer: D) -> Result<u128, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = serde_json::Value::deserialize(deserializer)?;
    if let Some(s) = value.as_str() {
        return s.parse::<u128>().map_err(D::Error::custom);
    }
    if let Some(n) = value.as_u64() {
        return Ok(n as u128);
    }
    Err(D::Error::custom("expected u128 as string or number"))
}

fn deserialize_option_u128_from_string_or_number<'de, D>(
    deserializer: D,
) -> Result<Option<u128>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = serde_json::Value::deserialize(deserializer)?;
    if value.is_null() {
        return Ok(None);
    }
    if let Some(s) = value.as_str() {
        return s.parse::<u128>().map(Some).map_err(D::Error::custom);
    }
    if let Some(n) = value.as_u64() {
        return Ok(Some(n as u128));
    }
    Err(D::Error::custom("expected u128 as string, number, or null"))
}

fn deserialize_u64_from_string_or_number<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = serde_json::Value::deserialize(deserializer)?;
    if let Some(s) = value.as_str() {
        return s.parse::<u64>().map_err(D::Error::custom);
    }
    if let Some(n) = value.as_u64() {
        return Ok(n);
    }
    Err(D::Error::custom("expected u64 as string or number"))
}

fn deserialize_option_u64_from_string_or_number<'de, D>(
    deserializer: D,
) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = serde_json::Value::deserialize(deserializer)?;
    if value.is_null() {
        return Ok(None);
    }
    if let Some(s) = value.as_str() {
        return s.parse::<u64>().map(Some).map_err(D::Error::custom);
    }
    if let Some(n) = value.as_u64() {
        return Ok(Some(n));
    }
    Err(D::Error::custom("expected u64 as string, number, or null"))
}

fn deserialize_i64_from_string_or_number<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = serde_json::Value::deserialize(deserializer)?;
    if let Some(s) = value.as_str() {
        return s.parse::<i64>().map_err(D::Error::custom);
    }
    if let Some(n) = value.as_i64() {
        return Ok(n);
    }
    Err(D::Error::custom("expected i64 as string or number"))
}

fn deserialize_u8_from_string_or_number<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = serde_json::Value::deserialize(deserializer)?;
    if let Some(s) = value.as_str() {
        return s.parse::<u8>().map_err(D::Error::custom);
    }
    if let Some(n) = value.as_u64() {
        return u8::try_from(n).map_err(D::Error::custom);
    }
    Err(D::Error::custom("expected u8 as string or number"))
}
