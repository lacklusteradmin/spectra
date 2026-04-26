// Per-chain send-payload JSON builders.
//
// Each function takes a human-scale decimal amount plus addresses/keys, performs
// the raw-unit conversion, and returns the exact JSON body the Rust signer
// expects. Consolidates ~17 scattered `UInt64(amount * 1eN)` + sendPayload()
// call sites from Swift into one place.
//
// Shared broadcast-result classification also lives here .

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn amount_u64(amount: f64, scale: f64) -> u64 {
    // Round to nearest to avoid sub-unit rounding errors from floating-point drift.
    (amount * scale).round() as u64
}

fn amount_i64(amount: f64, scale: f64) -> i64 {
    (amount * scale).round() as i64
}

fn amount_raw_string(amount: f64, decimals: u32) -> String {
    crate::send::preview_decode::amount_to_raw_units_string(amount, decimals)
}

// --- Simple "from, to, amount-unit, priv, [pub]" chain payloads ---

pub fn build_xrp_send_payload(
    from: String,
    to: String,
    amount_xrp: f64,
    private_key_hex: String,
    public_key_hex: Option<String>,
) -> String {
    let drops = amount_u64(amount_xrp, 1e6);
    match public_key_hex {
        Some(pub_hex) => format!(
            "{{\"from\":\"{}\",\"to\":\"{}\",\"drops\":{},\"private_key_hex\":\"{}\",\"public_key_hex\":\"{}\"}}",
            json_escape(&from), json_escape(&to), drops,
            json_escape(&private_key_hex), json_escape(&pub_hex)
        ),
        None => format!(
            "{{\"from\":\"{}\",\"to\":\"{}\",\"drops\":{},\"private_key_hex\":\"{}\"}}",
            json_escape(&from), json_escape(&to), drops, json_escape(&private_key_hex)
        ),
    }
}

pub fn build_stellar_send_payload(
    from: String,
    to: String,
    amount_xlm: f64,
    private_key_hex: String,
    public_key_hex: Option<String>,
) -> String {
    let stroops = amount_i64(amount_xlm, 1e7);
    match public_key_hex {
        Some(pub_hex) => format!(
            "{{\"from\":\"{}\",\"to\":\"{}\",\"stroops\":{},\"private_key_hex\":\"{}\",\"public_key_hex\":\"{}\"}}",
            json_escape(&from), json_escape(&to), stroops,
            json_escape(&private_key_hex), json_escape(&pub_hex)
        ),
        None => format!(
            "{{\"from\":\"{}\",\"to\":\"{}\",\"stroops\":{},\"private_key_hex\":\"{}\"}}",
            json_escape(&from), json_escape(&to), stroops, json_escape(&private_key_hex)
        ),
    }
}

pub fn build_sui_send_payload(
    from: String,
    to: String,
    amount_sui: f64,
    gas_budget_sui: f64,
    private_key_hex: String,
    public_key_hex: String,
) -> String {
    let mist = amount_u64(amount_sui, 1e9);
    let gas_budget = amount_u64(gas_budget_sui, 1e9);
    format!(
        "{{\"from\":\"{}\",\"to\":\"{}\",\"mist\":{},\"gas_budget\":{},\"private_key_hex\":\"{}\",\"public_key_hex\":\"{}\"}}",
        json_escape(&from), json_escape(&to), mist, gas_budget,
        json_escape(&private_key_hex), json_escape(&public_key_hex)
    )
}

pub fn build_aptos_send_payload(
    from: String,
    to: String,
    amount_apt: f64,
    private_key_hex: String,
    public_key_hex: String,
) -> String {
    let octas = amount_u64(amount_apt, 1e8);
    format!(
        "{{\"from\":\"{}\",\"to\":\"{}\",\"octas\":{},\"private_key_hex\":\"{}\",\"public_key_hex\":\"{}\"}}",
        json_escape(&from), json_escape(&to), octas,
        json_escape(&private_key_hex), json_escape(&public_key_hex)
    )
}

pub fn build_ton_send_payload(
    from: String,
    to: String,
    amount_ton: f64,
    private_key_hex: String,
    public_key_hex: String,
) -> String {
    let nanotons = amount_u64(amount_ton, 1e9);
    format!(
        "{{\"from\":\"{}\",\"to\":\"{}\",\"nanotons\":{},\"private_key_hex\":\"{}\",\"public_key_hex\":\"{}\"}}",
        json_escape(&from), json_escape(&to), nanotons,
        json_escape(&private_key_hex), json_escape(&public_key_hex)
    )
}

pub fn build_icp_send_payload(
    from: String,
    to: String,
    amount_icp: f64,
    private_key_hex: String,
    public_key_hex: Option<String>,
) -> String {
    let e8s = amount_u64(amount_icp, 1e8);
    match public_key_hex {
        Some(pub_hex) => format!(
            "{{\"from\":\"{}\",\"to\":\"{}\",\"e8s\":{},\"private_key_hex\":\"{}\",\"public_key_hex\":\"{}\"}}",
            json_escape(&from), json_escape(&to), e8s,
            json_escape(&private_key_hex), json_escape(&pub_hex)
        ),
        None => format!(
            "{{\"from\":\"{}\",\"to\":\"{}\",\"e8s\":{},\"private_key_hex\":\"{}\"}}",
            json_escape(&from), json_escape(&to), e8s, json_escape(&private_key_hex)
        ),
    }
}

pub fn build_cardano_send_payload(
    from: String,
    to: String,
    amount_ada: f64,
    fee_ada: f64,
    private_key_hex: String,
    public_key_hex: String,
) -> String {
    let amount_lovelace = amount_u64(amount_ada, 1e6);
    let fee_lovelace = amount_u64(fee_ada, 1e6);
    format!(
        "{{\"from\":\"{}\",\"to\":\"{}\",\"amount_lovelace\":{},\"fee_lovelace\":{},\"private_key_hex\":\"{}\",\"public_key_hex\":\"{}\"}}",
        json_escape(&from), json_escape(&to), amount_lovelace, fee_lovelace,
        json_escape(&private_key_hex), json_escape(&public_key_hex)
    )
}

pub fn build_near_send_payload(
    from: String,
    to: String,
    amount_near: f64,
    private_key_hex: String,
    public_key_hex: String,
) -> String {
    let yocto = amount_raw_string(amount_near, 24);
    format!(
        "{{\"from\":\"{}\",\"to\":\"{}\",\"yocto_near\":\"{}\",\"private_key_hex\":\"{}\",\"public_key_hex\":\"{}\"}}",
        json_escape(&from), json_escape(&to), json_escape(&yocto),
        json_escape(&private_key_hex), json_escape(&public_key_hex)
    )
}

pub fn build_near_token_send_payload(
    from: String,
    contract: String,
    to: String,
    amount: f64,
    decimals: u32,
    private_key_hex: String,
    public_key_hex: String,
) -> String {
    let scale = 10f64.powi(decimals as i32);
    let amount_raw = amount_u64(amount, scale);
    format!(
        "{{\"from\":\"{}\",\"contract\":\"{}\",\"to\":\"{}\",\"amount_raw\":\"{}\",\"private_key_hex\":\"{}\",\"public_key_hex\":\"{}\"}}",
        json_escape(&from), json_escape(&contract), json_escape(&to), amount_raw,
        json_escape(&private_key_hex), json_escape(&public_key_hex)
    )
}

pub fn build_polkadot_send_payload(
    from: String,
    to: String,
    amount_dot: f64,
    private_key_hex: String,
    public_key_hex: String,
) -> String {
    let planck = amount_raw_string(amount_dot, 10);
    format!(
        "{{\"from\":\"{}\",\"to\":\"{}\",\"planck\":\"{}\",\"private_key_hex\":\"{}\",\"public_key_hex\":\"{}\"}}",
        json_escape(&from), json_escape(&to), json_escape(&planck),
        json_escape(&private_key_hex), json_escape(&public_key_hex)
    )
}

pub fn build_monero_send_payload(to: String, amount_xmr: f64, priority: u32) -> String {
    let piconeros = amount_u64(amount_xmr, 1e12);
    format!(
        "{{\"to\":\"{}\",\"piconeros\":{},\"priority\":{}}}",
        json_escape(&to), piconeros, priority
    )
}

pub fn build_solana_native_send_payload(
    from_pubkey_hex: String,
    to: String,
    amount_sol: f64,
    private_key_hex: String,
) -> String {
    let lamports = amount_u64(amount_sol, 1e9);
    format!(
        "{{\"from_pubkey_hex\":\"{}\",\"to\":\"{}\",\"lamports\":{},\"private_key_hex\":\"{}\"}}",
        json_escape(&from_pubkey_hex), json_escape(&to), lamports, json_escape(&private_key_hex)
    )
}

pub fn build_solana_token_send_payload(
    from_pubkey_hex: String,
    mint: String,
    to: String,
    amount: f64,
    decimals: u32,
    private_key_hex: String,
) -> String {
    let scale = 10f64.powi(decimals as i32);
    let amount_raw = amount_u64(amount, scale);
    format!(
        "{{\"from_pubkey_hex\":\"{}\",\"to\":\"{}\",\"mint\":\"{}\",\"amount_raw\":\"{}\",\"decimals\":{},\"private_key_hex\":\"{}\"}}",
        json_escape(&from_pubkey_hex), json_escape(&to), json_escape(&mint), amount_raw,
        decimals, json_escape(&private_key_hex)
    )
}

pub fn build_tron_native_send_payload(
    from: String,
    to: String,
    amount_trx: f64,
    private_key_hex: String,
) -> String {
    let amount_sun = amount_u64(amount_trx, 1e6);
    format!(
        "{{\"from\":\"{}\",\"to\":\"{}\",\"amount_sun\":{},\"private_key_hex\":\"{}\"}}",
        json_escape(&from), json_escape(&to), amount_sun, json_escape(&private_key_hex)
    )
}

pub fn build_tron_token_send_payload(
    from: String,
    contract: String,
    to: String,
    amount: f64,
    decimals: u32,
    private_key_hex: String,
) -> String {
    let scale = 10f64.powi(decimals as i32);
    let amount_raw = amount_u64(amount, scale);
    format!(
        "{{\"from\":\"{}\",\"contract\":\"{}\",\"to\":\"{}\",\"amount_raw\":\"{}\",\"private_key_hex\":\"{}\"}}",
        json_escape(&from), json_escape(&contract), json_escape(&to), amount_raw,
        json_escape(&private_key_hex)
    )
}

pub fn build_btc_send_payload(
    from: String,
    to: String,
    amount_btc: f64,
    fee_rate_svb: f64,
    private_key_hex: String,
) -> String {
    let amount_sat = amount_u64(amount_btc, 1e8);
    format!(
        "{{\"from\":\"{}\",\"to\":\"{}\",\"amount_sat\":{},\"fee_rate_svb\":{},\"private_key_hex\":\"{}\"}}",
        json_escape(&from), json_escape(&to), amount_sat, fee_rate_svb, json_escape(&private_key_hex)
    )
}

pub fn build_doge_send_payload(
    from: String,
    to: String,
    amount_doge: f64,
    fee_rate_doge_per_kb: f64,
    private_key_hex: String,
) -> String {
    let amount_sat = amount_u64(amount_doge, 1e8);
    // Doge sign path uses a fee_sat derived from kb fee rate × tx-bytes estimate (350).
    let fee_sat = amount_u64(fee_rate_doge_per_kb * 350.0 / 1000.0, 1e8);
    format!(
        "{{\"from\":\"{}\",\"to\":\"{}\",\"amount_sat\":{},\"fee_sat\":{},\"private_key_hex\":\"{}\"}}",
        json_escape(&from), json_escape(&to), amount_sat, fee_sat, json_escape(&private_key_hex)
    )
}

// --- Broadcast-result classification ---

#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum SendChain {
    Bitcoin,
    BitcoinCash,
    BitcoinSV,
    Litecoin,
    Dogecoin,
    Ethereum,
    Tron,
    Solana,
    Xrp,
    Stellar,
    Monero,
    Cardano,
    Sui,
    Aptos,
    Ton,
    Icp,
    Near,
    Polkadot,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct SendBroadcastOutcome {
    pub transaction_hash: String,
    pub payload_format: String,
}

fn hash_field_for(chain: SendChain) -> &'static str {
    match chain {
        SendChain::Sui => "digest",
        SendChain::Icp => "block_index",
        SendChain::Solana => "signature",
        _ => "txid",
    }
}

fn format_key_for(chain: SendChain) -> &'static str {
    match chain {
        SendChain::Bitcoin => "bitcoin.rust_json",
        SendChain::BitcoinCash => "bitcoin_cash.rust_json",
        SendChain::BitcoinSV => "bitcoin_sv.rust_json",
        SendChain::Litecoin => "litecoin.rust_json",
        SendChain::Dogecoin => "dogecoin.rust_json",
        SendChain::Ethereum => "ethereum.rust_json",
        SendChain::Tron => "tron.rust_json",
        SendChain::Solana => "solana.rust_json",
        SendChain::Xrp => "xrp.rust_json",
        SendChain::Stellar => "stellar.rust_json",
        SendChain::Monero => "monero.rust_json",
        SendChain::Cardano => "cardano.rust_json",
        SendChain::Sui => "sui.rust_json",
        SendChain::Aptos => "aptos.rust_json",
        SendChain::Ton => "ton.rust_json",
        SendChain::Icp => "icp.rust_json",
        SendChain::Near => "near.rust_json",
        SendChain::Polkadot => "polkadot.rust_json",
    }
}

pub fn classify_send_broadcast_result(chain: SendChain, result_json: String) -> SendBroadcastOutcome {
    let field = hash_field_for(chain);
    let mut hash = crate::send::preview_decode::extract_json_string_field(
        result_json.clone(),
        field.to_string(),
    );
    // ICP: fallback to raw JSON when block_index is absent (matches Swift behavior).
    if matches!(chain, SendChain::Icp) && hash.is_empty() {
        hash = result_json.clone();
    }
    SendBroadcastOutcome {
        transaction_hash: hash,
        payload_format: format_key_for(chain).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xrp_payload_round_trip() {
        let s = build_xrp_send_payload("fa".into(), "to".into(), 1.5, "priv".into(), Some("pub".into()));
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["drops"], 1_500_000);
        assert_eq!(v["public_key_hex"], "pub");
    }

    #[test]
    fn xrp_payload_priv_only() {
        let s = build_xrp_send_payload("fa".into(), "to".into(), 1.0, "priv".into(), None);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["drops"], 1_000_000);
        assert!(v.get("public_key_hex").is_none());
    }

    #[test]
    fn monero_payload() {
        let s = build_monero_send_payload("addr".into(), 2.0, 2);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["piconeros"], 2_000_000_000_000u64);
        assert_eq!(v["priority"], 2);
    }

    #[test]
    fn classify_sui_digest() {
        let o = classify_send_broadcast_result(
            SendChain::Sui,
            r#"{"digest":"abc"}"#.into(),
        );
        assert_eq!(o.transaction_hash, "abc");
        assert_eq!(o.payload_format, "sui.rust_json");
    }

    #[test]
    fn classify_icp_fallback_when_no_block_index() {
        let o = classify_send_broadcast_result(
            SendChain::Icp,
            r#"{"other":1}"#.into(),
        );
        assert_eq!(o.transaction_hash, r#"{"other":1}"#);
    }
}
