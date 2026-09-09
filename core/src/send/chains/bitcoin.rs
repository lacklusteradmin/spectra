//! Bitcoin send path: P2WPKH / P2PKH / P2TR signers, coin selection, fee
//! calculation, and Esplora broadcast.

use std::str::FromStr;

use bitcoin::absolute::LockTime;
use bitcoin::hashes::Hash as _;
use bitcoin::key::{TapTweak, TweakedKeypair};
use bitcoin::script::{Builder, PushBytesBuf};
use bitcoin::secp256k1::{Message, Secp256k1, SecretKey};
use bitcoin::sighash::{EcdsaSighashType, SighashCache, TapSighashType};
use bitcoin::transaction::Version;
use bitcoin::{
    Address, Amount, CompressedPublicKey, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
    Txid, Witness,
};
use zeroize::Zeroize;

use crate::http::{with_fallback, RetryProfile};

use crate::fetch::chains::bitcoin::{BitcoinClient, BitcoinSendResult, EsploraUtxo, FeeRate};

impl BitcoinClient {
    pub async fn broadcast_raw_tx(&self, raw_tx_hex: &str) -> Result<String, String> {
        let raw = raw_tx_hex.to_string();
        let http = self.http.clone();
        let endpoints = self.endpoints.clone();

        with_fallback(&endpoints, |base| {
            let raw = raw.clone();
            let http = http.clone();
            async move {
                let url = format!("{base}/tx");
                // Esplora broadcast: POST hex-encoded tx as plain text, returns txid.
                http.post_text(&url, raw, RetryProfile::ChainWrite).await
            }
        })
        .await
        .map(|s| s.trim().to_string())
    }
}

// ── Transaction construction & signing

// ── Professional coin-selection and output customization ──────────────────

/// UTXO coin-selection strategy.
#[derive(Debug, Clone, Default)]
pub enum CoinSelectionStrategy {
    /// Accumulate the largest UTXOs first. Minimizes input count; may leave
    /// dust change when total overshoots the target significantly.
    #[default]
    LargestFirst,
    /// Accumulate the smallest UTXOs first. Consolidates dust coins at the
    /// cost of more inputs and higher fees. Useful for UTXO set hygiene.
    SmallestFirst,
}

/// An additional output appended to the transaction after the primary
/// recipient and change outputs. Enables batch sends and on-chain memos.
#[derive(Debug, Clone)]
pub enum BitcoinExtraOutput {
    /// Secondary payment to another address in the same transaction.
    Address { address: String, amount_sats: u64 },
    /// OP_RETURN metadata payload (max 80 bytes, zero value). The `data_hex`
    /// field is hex-encoded; no `0x` prefix required.
    OpReturn { data_hex: String },
}

/// Parameters for building a Bitcoin transaction.
#[derive(Debug)]
pub struct BitcoinSendParams {
    /// From address (used to find the script type).
    pub from_address: String,
    /// WIF or hex-encoded 32-byte private key.
    pub private_key_hex: String,
    /// Primary recipient address.
    pub to_address: String,
    /// Primary send amount in satoshis.
    pub amount_sats: u64,
    /// Fee rate (sats per virtual byte).
    pub fee_rate: FeeRate,
    /// UTXOs available for automatic coin selection. Ignored when
    /// `pinned_utxos` is set. Fetched from Esplora if empty.
    pub available_utxos: Vec<EsploraUtxo>,
    /// Which network, as a registry chain id (`"bitcoin"`,
    /// `"bitcoin-testnet-4"`, …). It was a mode string, which meant a table
    /// mapping those strings back to a network beside the one the registry
    /// already has.
    pub network_chain_id: String,
    /// Whether to signal RBF (replace-by-fee) on inputs.
    pub enable_rbf: bool,
    /// Minimum change output in satoshis; dust below this is absorbed into fee.
    /// `None` uses the standard 546-sat P2PKH dust threshold.
    pub dust_threshold: Option<u64>,
    // ── Professional controls ────────────────────────────────────────────────
    /// Manually selected UTXOs to spend. When `Some`, bypasses automatic coin
    /// selection entirely — the caller is responsible for ensuring the total
    /// input value covers `amount_sats` plus fee.
    pub pinned_utxos: Option<Vec<EsploraUtxo>>,
    /// Additional outputs appended after the primary recipient and change.
    /// Enables batch sends (multiple recipients in one tx) and OP_RETURN memos.
    pub extra_outputs: Vec<BitcoinExtraOutput>,
    /// Coin selection algorithm used when `pinned_utxos` is `None`.
    pub coin_selection: CoinSelectionStrategy,
    /// Sign the transaction but skip the Esplora broadcast. The signed raw
    /// transaction hex is returned in `BitcoinSendResult.raw_tx_hex` and
    /// `txid` is left empty. Useful for offline signing, PSBT handoffs, or
    /// pre-flight fee inspection.
    pub sign_only: bool,
}

/// What one input of a given script type costs to spend and what one of its
/// outputs costs to create, in virtual bytes.
///
/// These were four sets of magic numbers inlined twice each — once in the
/// pinned-UTXO branch and once in the `select_coins` call — and the two copies
/// had drifted: every type but P2PKH agreed on 31 bytes per output, and P2PKH
/// used 34 when the caller pinned its own UTXOs but 31 when coin selection
/// picked them, because `select_coins` hardcoded 31 and ignored what it was
/// told. A legacy send therefore underpaid by 3 vbytes per output whenever it
/// selected automatically.
#[derive(Clone, Copy)]
struct SpendSizing {
    input_vbytes: usize,
    output_vbytes: usize,
}

impl SpendSizing {
    /// Version, locktime, and the two `CompactSize` counts.
    const OVERHEAD_VBYTES: usize = 10;

    /// Segwit v0 inputs get the witness discount.
    const P2WPKH: Self = Self {
        input_vbytes: 68,
        output_vbytes: 31,
    };
    /// Nested spends carry the redeem script in `script_sig`, undiscounted.
    const P2SH_P2WPKH: Self = Self {
        input_vbytes: 91,
        output_vbytes: 31,
    };
    /// Legacy: signature and pubkey both sit in `script_sig`.
    const P2PKH: Self = Self {
        input_vbytes: 148,
        output_vbytes: 34,
    };
    /// Taproot key-path spends are 57.5 vbytes, rounded up.
    const P2TR: Self = Self {
        input_vbytes: 58,
        output_vbytes: 31,
    };

    fn fee(self, inputs: usize, outputs: usize, rate: FeeRate) -> u64 {
        let vbytes =
            Self::OVERHEAD_VBYTES + inputs * self.input_vbytes + outputs * self.output_vbytes;
        (vbytes as f64 * rate.sats_per_vbyte).ceil() as u64
    }
}

/// Coin selection: accumulate UTXOs until we cover `target + fee`.
/// Sorting order is determined by `strategy`.
fn select_coins<'a>(
    utxos: &'a [EsploraUtxo],
    target_sats: u64,
    fee_rate: FeeRate,
    sizing: SpendSizing,
    output_count: usize,
    strategy: &CoinSelectionStrategy,
) -> Result<(Vec<&'a EsploraUtxo>, u64), String> {
    let mut sorted: Vec<&EsploraUtxo> = utxos.iter().collect();
    match strategy {
        CoinSelectionStrategy::LargestFirst => sorted.sort_by(|a, b| b.value.cmp(&a.value)),
        CoinSelectionStrategy::SmallestFirst => sorted.sort_by(|a, b| a.value.cmp(&b.value)),
    }

    let mut selected: Vec<&EsploraUtxo> = Vec::new();
    let mut total: u64 = 0;

    for utxo in sorted {
        selected.push(utxo);
        total += utxo.value;

        let fee = sizing.fee(selected.len(), output_count, fee_rate);
        if total >= target_sats.saturating_add(fee) {
            return Ok((selected, fee));
        }
    }

    Err("utxo.insufficientFunds".to_string())
}

/// Build `TxOut` items for `extra_outputs`. OP_RETURN outputs are zero-value.
fn build_extra_outputs(
    extra: &[BitcoinExtraOutput],
    network: bitcoin::Network,
) -> Result<Vec<TxOut>, String> {
    let mut out = Vec::new();
    for item in extra {
        match item {
            BitcoinExtraOutput::Address {
                address,
                amount_sats,
            } => {
                let addr = Address::from_str(address)
                    .map_err(|e| format!("extra output bad address: {e}"))?
                    .require_network(network)
                    .map_err(|e| format!("extra output wrong network: {e}"))?;
                out.push(TxOut {
                    value: Amount::from_sat(*amount_sats),
                    script_pubkey: addr.script_pubkey(),
                });
            }
            BitcoinExtraOutput::OpReturn { data_hex } => {
                let data = hex::decode(data_hex.trim_start_matches("0x"))
                    .map_err(|e| format!("OP_RETURN bad hex: {e}"))?;
                if data.len() > 80 {
                    return Err(format!(
                        "OP_RETURN data too large: {} bytes (max 80)",
                        data.len()
                    ));
                }
                let push_bytes = bitcoin::script::PushBytesBuf::try_from(data)
                    .map_err(|_| "OP_RETURN data exceeds PushBytes limit".to_string())?;
                out.push(TxOut {
                    value: Amount::ZERO,
                    script_pubkey: ScriptBuf::new_op_return(push_bytes.as_push_bytes()),
                });
            }
        }
    }
    Ok(out)
}

fn tx_input_for_utxo(utxo: &EsploraUtxo, sequence: Sequence) -> Result<TxIn, String> {
    let txid = Txid::from_str(&utxo.txid).map_err(|e| format!("bad txid {}: {e}", utxo.txid))?;
    Ok(TxIn {
        previous_output: OutPoint {
            txid,
            vout: utxo.vout,
        },
        script_sig: ScriptBuf::new(),
        sequence,
        witness: Witness::new(),
    })
}

/// The spending key and the two addresses, parsed once and checked against
/// the chain's network.
struct SpendIdentity {
    secret_key: SecretKey,
    public_key: CompressedPublicKey,
    network: bitcoin::Network,
    from: Address,
    to: Address,
}

impl SpendIdentity {
    /// The script pubkey of the address being spent from — where change goes,
    /// and what every prevout in a taproot spend carries.
    fn spent_script(&self) -> ScriptBuf {
        self.from.script_pubkey()
    }
}

fn parse_spend_identity(
    secp: &Secp256k1<bitcoin::secp256k1::All>,
    params: &BitcoinSendParams,
) -> Result<SpendIdentity, String> {
    let network = crate::registry::Chain::from_str_id(&params.network_chain_id)
        .unwrap_or(crate::registry::Chain::Bitcoin)
        .bitcoin_network();

    let mut key_bytes =
        hex::decode(&params.private_key_hex).map_err(|e| format!("bad private key hex: {e}"))?;
    let secret_key =
        SecretKey::from_slice(&key_bytes).map_err(|e| format!("bad private key: {e}"))?;
    key_bytes.zeroize();

    let public_key = CompressedPublicKey::from_slice(
        &secp256k1::PublicKey::from_secret_key(secp, &secret_key).serialize(),
    )
    .map_err(|e| format!("pk: {e}"))?;

    let to = Address::from_str(&params.to_address)
        .map_err(|e| format!("bad recipient address: {e}"))?
        .require_network(network)
        .map_err(|e| format!("recipient on wrong network: {e}"))?;

    let from = Address::from_str(&params.from_address)
        .map_err(|e| format!("bad from address: {e}"))?
        .require_network(network)
        .map_err(|e| format!("from address on wrong network: {e}"))?;

    Ok(SpendIdentity {
        secret_key,
        public_key,
        network,
        from,
        to,
    })
}

/// The unsigned transaction plus the input values the signatures need.
struct UnsignedSpend {
    tx: Transaction,
    /// Value of each input, in `tx.input` order. Segwit and taproot sighashes
    /// commit to it and a `Transaction` does not carry it.
    input_values: Vec<u64>,
}

/// Select coins, size the fee, and lay out inputs and outputs. Everything the
/// four signers do before they differ.
fn build_unsigned_spend(
    params: &BitcoinSendParams,
    identity: &SpendIdentity,
    sizing: SpendSizing,
) -> Result<UnsignedSpend, String> {
    // The fee is sized for recipient + change + extras whether or not the
    // change output survives the dust check below.
    let output_count = 2 + params.extra_outputs.len();

    let (selected, fee) = match params.pinned_utxos.as_deref() {
        Some(pinned) => {
            let fee = sizing.fee(pinned.len(), output_count, params.fee_rate);
            let total_in: u64 = pinned.iter().map(|u| u.value).sum();
            if total_in < params.amount_sats.saturating_add(fee) {
                return Err("utxo.insufficientFunds".to_string());
            }
            (pinned.iter().collect::<Vec<_>>(), fee)
        }
        None => select_coins(
            &params.available_utxos,
            params.amount_sats,
            params.fee_rate,
            sizing,
            output_count,
            &params.coin_selection,
        )?,
    };

    let total_in: u64 = selected.iter().map(|u| u.value).sum();
    let change_sats = total_in
        .saturating_sub(params.amount_sats)
        .saturating_sub(fee);

    let sequence = if params.enable_rbf {
        Sequence::ENABLE_RBF_NO_LOCKTIME
    } else {
        Sequence::MAX
    };
    let input: Vec<TxIn> = selected
        .iter()
        .map(|u| tx_input_for_utxo(u, sequence))
        .collect::<Result<_, _>>()?;

    let mut output = vec![TxOut {
        value: Amount::from_sat(params.amount_sats),
        script_pubkey: identity.to.script_pubkey(),
    }];
    if change_sats > params.dust_threshold.unwrap_or(546) {
        output.push(TxOut {
            value: Amount::from_sat(change_sats),
            script_pubkey: identity.spent_script(),
        });
    }
    output.extend(build_extra_outputs(
        &params.extra_outputs,
        identity.network,
    )?);

    Ok(UnsignedSpend {
        input_values: selected.iter().map(|u| u.value).collect(),
        tx: Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input,
            output,
        },
    })
}

fn serialized(tx: Transaction) -> (Transaction, String) {
    let raw_hex = bitcoin::consensus::encode::serialize_hex(&tx);
    (tx, raw_hex)
}

/// Sign every input with the BIP143 sighash and attach `<sig> <pubkey>`.
///
/// P2WPKH and nested P2SH-P2WPKH sign the identical script code — the nested
/// form's redeem script *is* the P2WPKH script — and differ only in that the
/// nested one also pushes that script into `script_sig`.
fn sign_segwit_v0(
    secp: &Secp256k1<bitcoin::secp256k1::All>,
    identity: &SpendIdentity,
    spend: UnsignedSpend,
    nested_in_p2sh: bool,
) -> Result<(Transaction, String), String> {
    let UnsignedSpend {
        mut tx,
        input_values,
    } = spend;
    let script_code = ScriptBuf::new_p2wpkh(&identity.public_key.wpubkey_hash());
    let pk_bytes = identity.public_key.to_bytes();

    let mut sighash_cache = SighashCache::new(&mut tx);
    let mut signatures: Vec<Vec<u8>> = Vec::new();
    for (i, value) in input_values.iter().enumerate() {
        let sighash = sighash_cache
            .p2wpkh_signature_hash(
                i,
                &script_code,
                Amount::from_sat(*value),
                EcdsaSighashType::All,
            )
            .map_err(|e| format!("sighash: {e}"))?;
        let msg = Message::from_digest(sighash.to_raw_hash().to_byte_array());
        let mut sig_der = secp
            .sign_ecdsa(&msg, &identity.secret_key)
            .serialize_der()
            .to_vec();
        sig_der.push(EcdsaSighashType::All as u8);
        signatures.push(sig_der);
    }

    let redeem_push = if nested_in_p2sh {
        Some(
            PushBytesBuf::try_from(script_code.as_bytes().to_vec())
                .map_err(|_| "redeem script exceeds PushBytes limit".to_string())?,
        )
    } else {
        None
    };

    let tx_ref = sighash_cache.into_transaction();
    for (i, sig) in signatures.iter().enumerate() {
        if let Some(push) = &redeem_push {
            tx_ref.input[i].script_sig = Builder::new().push_slice(push).into_script();
        }
        let mut witness = Witness::new();
        witness.push(sig);
        witness.push(pk_bytes.as_slice());
        tx_ref.input[i].witness = witness;
    }

    Ok(serialized(tx_ref.clone()))
}

/// Build, sign, and serialize a P2WPKH transaction.
pub fn sign_p2wpkh(params: &mut BitcoinSendParams) -> Result<(Transaction, String), String> {
    let secp = Secp256k1::new();
    let identity = parse_spend_identity(&secp, params)?;
    let spend = build_unsigned_spend(params, &identity, SpendSizing::P2WPKH)?;
    sign_segwit_v0(&secp, &identity, spend, false)
}

/// Build, sign, and serialize a nested SegWit P2SH-P2WPKH transaction.
pub fn sign_p2sh_p2wpkh(params: &mut BitcoinSendParams) -> Result<(Transaction, String), String> {
    let secp = Secp256k1::new();
    let identity = parse_spend_identity(&secp, params)?;

    let redeem_script = ScriptBuf::new_p2wpkh(&identity.public_key.wpubkey_hash());
    if identity.spent_script() != redeem_script.to_p2sh() {
        return Err("from P2SH address does not match the supplied private key".to_string());
    }

    let spend = build_unsigned_spend(params, &identity, SpendSizing::P2SH_P2WPKH)?;
    sign_segwit_v0(&secp, &identity, spend, true)
}

/// Build, sign, and serialize a P2PKH (legacy) transaction.
pub fn sign_p2pkh(params: &mut BitcoinSendParams) -> Result<(Transaction, String), String> {
    let secp = Secp256k1::new();
    let identity = parse_spend_identity(&secp, params)?;
    let UnsignedSpend {
        mut tx,
        input_values,
    } = build_unsigned_spend(params, &identity, SpendSizing::P2PKH)?;

    let from_script = identity.spent_script();
    let pk_bytes = identity.public_key.to_bytes();

    let sighash_cache = SighashCache::new(&mut tx);
    let mut signatures: Vec<Vec<u8>> = Vec::new();
    for i in 0..input_values.len() {
        let sighash = sighash_cache
            .legacy_signature_hash(i, &from_script, EcdsaSighashType::All as u32)
            .map_err(|e| format!("sighash: {e}"))?;
        let msg = Message::from_digest(sighash.to_raw_hash().to_byte_array());
        let mut sig_der = secp
            .sign_ecdsa(&msg, &identity.secret_key)
            .serialize_der()
            .to_vec();
        sig_der.push(EcdsaSighashType::All as u8);
        signatures.push(sig_der);
    }

    let tx_ref = sighash_cache.into_transaction();
    for (i, sig) in signatures.iter().enumerate() {
        // P2PKH scriptSig: <OP_PUSH(sig_len)> <sig> <OP_PUSH(pk_len)> <pk>.
        let mut script_bytes = Vec::with_capacity(2 + sig.len() + pk_bytes.len());
        script_bytes.push(sig.len() as u8);
        script_bytes.extend_from_slice(sig);
        script_bytes.push(pk_bytes.len() as u8);
        script_bytes.extend_from_slice(&pk_bytes);
        tx_ref.input[i].script_sig = ScriptBuf::from_bytes(script_bytes);
    }

    Ok(serialized(tx_ref.clone()))
}

/// Build, sign, and serialize a P2TR (Taproot key-path) transaction.
pub fn sign_p2tr(params: &mut BitcoinSendParams) -> Result<(Transaction, String), String> {
    let secp = Secp256k1::new();
    let identity = parse_spend_identity(&secp, params)?;
    let UnsignedSpend {
        mut tx,
        input_values,
    } = build_unsigned_spend(params, &identity, SpendSizing::P2TR)?;

    let tweaked_keypair: TweakedKeypair =
        bitcoin::key::Keypair::from_secret_key(&secp, &identity.secret_key).tap_tweak(&secp, None);
    // Every input is one of ours, so every prevout carries the from-script.
    let prevouts: Vec<TxOut> = input_values
        .iter()
        .map(|value| TxOut {
            value: Amount::from_sat(*value),
            script_pubkey: identity.spent_script(),
        })
        .collect();

    let mut sighash_cache = SighashCache::new(&mut tx);
    let mut signatures: Vec<Vec<u8>> = Vec::new();
    for i in 0..input_values.len() {
        use bitcoin::sighash::Prevouts;
        let sighash = sighash_cache
            .taproot_key_spend_signature_hash(i, &Prevouts::All(&prevouts), TapSighashType::Default)
            .map_err(|e| format!("taproot sighash: {e}"))?;
        let msg = Message::from_digest(sighash.to_raw_hash().to_byte_array());
        signatures.push(
            bitcoin::taproot::Signature {
                signature: secp.sign_schnorr(&msg, &tweaked_keypair.to_keypair()),
                sighash_type: TapSighashType::Default,
            }
            .to_vec(),
        );
    }

    let tx_ref = sighash_cache.into_transaction();
    for (i, sig) in signatures.iter().enumerate() {
        let mut witness = Witness::new();
        witness.push(sig);
        tx_ref.input[i].witness = witness;
    }

    Ok(serialized(tx_ref.clone()))
}
/// High-level send: auto-detect the from-address script type, sign,
/// and broadcast (unless `params.sign_only` is set).
pub async fn sign_and_broadcast(
    client: &BitcoinClient,
    mut params: BitcoinSendParams,
) -> Result<BitcoinSendResult, String> {
    // Fetch UTXOs when auto-selection is active and none were pre-supplied.
    if params.pinned_utxos.is_none() && params.available_utxos.is_empty() {
        params.available_utxos = client.fetch_utxos(&params.from_address).await?;
    }

    // Detect script type from the from-address prefix.
    let raw_hex = if params.from_address.starts_with("bc1p")
        || params.from_address.starts_with("tb1p")
        || params.from_address.starts_with("bcrt1p")
    {
        let (_, hex) = sign_p2tr(&mut params)?;
        hex
    } else if params.from_address.starts_with("bc1q")
        || params.from_address.starts_with("tb1q")
        || params.from_address.starts_with("bcrt1q")
    {
        let (_, hex) = sign_p2wpkh(&mut params)?;
        hex
    } else if Address::from_str(&params.from_address)
        .ok()
        .and_then(|addr| {
            addr.require_network(
                crate::registry::Chain::from_str_id(&params.network_chain_id)
                    .unwrap_or(crate::registry::Chain::Bitcoin)
                    .bitcoin_network(),
            )
            .ok()
        })
        .is_some_and(|addr| addr.script_pubkey().is_p2sh())
    {
        let (_, hex) = sign_p2sh_p2wpkh(&mut params)?;
        hex
    } else {
        let (_, hex) = sign_p2pkh(&mut params)?;
        hex
    };

    if params.sign_only {
        return Ok(BitcoinSendResult {
            txid: String::new(),
            raw_tx_hex: raw_hex,
        });
    }

    let txid = client.broadcast_raw_tx(&raw_hex).await?;

    Ok(BitcoinSendResult {
        txid,
        raw_tx_hex: raw_hex,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::chains::bitcoin::EsploraUtxoStatus;

    const KEY_HEX: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const TXID: &str = "0000000000000000000000000000000000000000000000000000000000000001";
    /// A recipient nobody here holds the key for; only its script pubkey matters.
    const TO: &str = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";

    #[derive(Clone, Copy)]
    enum Kind {
        P2wpkh,
        P2sh,
        P2pkh,
        P2tr,
    }

    impl Kind {
        fn sign(self, params: &mut BitcoinSendParams) -> Result<(Transaction, String), String> {
            match self {
                Kind::P2wpkh => sign_p2wpkh(params),
                Kind::P2sh => sign_p2sh_p2wpkh(params),
                Kind::P2pkh => sign_p2pkh(params),
                Kind::P2tr => sign_p2tr(params),
            }
        }

        fn address(self) -> String {
            let (secp, secret, pk) = keys();
            let net = bitcoin::Network::Bitcoin;
            match self {
                Kind::P2wpkh => Address::p2wpkh(&pk, net).to_string(),
                Kind::P2sh => Address::p2shwpkh(&pk, net).to_string(),
                Kind::P2pkh => Address::p2pkh(pk, net).to_string(),
                Kind::P2tr => {
                    let keypair = bitcoin::key::Keypair::from_secret_key(&secp, &secret);
                    Address::p2tr(&secp, keypair.x_only_public_key().0, None, net).to_string()
                }
            }
        }
    }

    fn keys() -> (
        Secp256k1<bitcoin::secp256k1::All>,
        SecretKey,
        CompressedPublicKey,
    ) {
        let secp = Secp256k1::new();
        let secret = SecretKey::from_slice(&hex::decode(KEY_HEX).unwrap()).unwrap();
        let pk = CompressedPublicKey::from_slice(
            &secp256k1::PublicKey::from_secret_key(&secp, &secret).serialize(),
        )
        .unwrap();
        (secp, secret, pk)
    }

    fn utxo(vout: u32, value: u64) -> EsploraUtxo {
        EsploraUtxo {
            txid: TXID.to_string(),
            vout,
            status: EsploraUtxoStatus {
                confirmed: true,
                block_height: Some(800_000),
            },
            value,
        }
    }

    fn params(kind: Kind, utxos: Vec<EsploraUtxo>, amount: u64) -> BitcoinSendParams {
        BitcoinSendParams {
            from_address: kind.address(),
            private_key_hex: KEY_HEX.to_string(),
            to_address: TO.to_string(),
            amount_sats: amount,
            fee_rate: FeeRate {
                sats_per_vbyte: 10.0,
            },
            available_utxos: utxos,
            network_chain_id: "bitcoin".to_string(),
            enable_rbf: true,
            dust_threshold: None,
            pinned_utxos: None,
            extra_outputs: vec![],
            coin_selection: CoinSelectionStrategy::default(),
            sign_only: true,
        }
    }

    fn change_of(tx: &Transaction) -> u64 {
        tx.output[1].value.to_sat()
    }

    /// The three deterministic (ECDSA, RFC6979) script types, byte for byte.
    /// Taproot is absent on purpose: `sign_schnorr` mixes in auxiliary
    /// randomness, so its serialization differs run to run.
    #[test]
    fn signed_transactions_are_stable() {
        for (kind, expected) in [
            (Kind::P2wpkh, "0200000000010101000000000000000000000000000000000000000000000000000000000000000000000000fdffffff0260ea000000000000160014751e76e8199196d454941c45d1b3a323f1433bd6c896000000000000160014fc7250a211deddc70ee5a2738de5f07817351cef02473044022070231963b5c9439fcc27ec031107384bf86a2c30252fc8039c9653979793fc7a022023100c560d717f0285a66cd5faf7ff49341f850a1ec0f5d08c83e13e576378710121034f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa00000000"),
            (Kind::P2sh, "0200000000010101000000000000000000000000000000000000000000000000000000000000000000000017160014fc7250a211deddc70ee5a2738de5f07817351ceffdffffff0260ea000000000000160014751e76e8199196d454941c45d1b3a323f1433bd6e29500000000000017a914ec8f3d9c2763a0997a465b968d99db47e82e69d287024730440220605e64f3b86dcbe8257ab75ab4e5b7c3cb31c707f730c233cf6781f719fda98c0220463b2f03f7e3464210ae05493cf6aa790f14ea103a00c00407d8ec55a64d02800121034f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa00000000"),
            (Kind::P2pkh, "02000000010100000000000000000000000000000000000000000000000000000000000000000000006b4830450221009be163ca641bb99837e4eae72a67d3a53cc2fce44ca719b5117a2f1a5723e41b022031139b2ae7f92d02e6833ddfeb0062cceac2da21c60522ba615d16de17545c260121034f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aafdffffff0260ea000000000000160014751e76e8199196d454941c45d1b3a323f1433bd66c930000000000001976a914fc7250a211deddc70ee5a2738de5f07817351cef88ac00000000"),
        ] {
            let mut p = params(kind, vec![utxo(0, 100_000), utxo(1, 50_000)], 60_000);
            let (_, hex) = kind.sign(&mut p).unwrap();
            assert_eq!(hex, expected);
        }
    }

    /// Taproot signs and lays out the same transaction every run even though
    /// the signature itself does not repeat.
    #[test]
    fn taproot_lays_out_a_stable_transaction() {
        let mut p = params(Kind::P2tr, vec![utxo(0, 100_000)], 60_000);
        let (tx, _) = sign_p2tr(&mut p).unwrap();
        assert_eq!(tx.input.len(), 1);
        assert_eq!(tx.output[0].value.to_sat(), 60_000);
        // 10 + 58 + 2 × 31 = 130 vbytes at 10 sat/vb.
        assert_eq!(change_of(&tx), 100_000 - 60_000 - 1_300);
        assert_eq!(tx.input[0].witness.len(), 1);
    }

    /// The fee a script type charges follows only from its own vsize table,
    /// and pinning UTXOs must not change it. Legacy sends used to pay 60 sats
    /// less here than when the same coins were pinned.
    #[test]
    fn pinned_and_selected_coins_charge_the_same_fee() {
        for (kind, fee) in [
            (Kind::P2wpkh, 10 * (10 + 68 + 2 * 31)),
            (Kind::P2sh, 10 * (10 + 91 + 2 * 31)),
            (Kind::P2pkh, 10 * (10 + 148 + 2 * 34)),
        ] {
            let mut selected = params(kind, vec![utxo(0, 100_000)], 60_000);
            let (tx_selected, _) = kind.sign(&mut selected).unwrap();

            let mut pinned = params(kind, vec![], 60_000);
            pinned.pinned_utxos = Some(vec![utxo(0, 100_000)]);
            let (tx_pinned, _) = kind.sign(&mut pinned).unwrap();

            assert_eq!(change_of(&tx_selected), 100_000 - 60_000 - fee);
            assert_eq!(change_of(&tx_pinned), change_of(&tx_selected));
        }
    }

    /// Change below the dust threshold is dropped and left to the miner.
    #[test]
    fn dust_change_is_absorbed_into_the_fee() {
        // P2WPKH at 1 sat/vb costs 140 vbytes, leaving 490 sats of change.
        let mut p = params(Kind::P2wpkh, vec![utxo(0, 60_630)], 60_000);
        p.fee_rate = FeeRate {
            sats_per_vbyte: 1.0,
        };
        let (tx, _) = sign_p2wpkh(&mut p).unwrap();
        assert_eq!(tx.output.len(), 1, "490 sats is under the 546 default");

        p.dust_threshold = Some(100);
        let (tx, _) = sign_p2wpkh(&mut p).unwrap();
        assert_eq!(tx.output.len(), 2);
        assert_eq!(change_of(&tx), 490);
    }

    #[test]
    fn extra_outputs_are_appended_after_change() {
        let mut p = params(Kind::P2wpkh, vec![utxo(0, 200_000)], 60_000);
        p.extra_outputs = vec![
            BitcoinExtraOutput::Address {
                address: TO.to_string(),
                amount_sats: 7_000,
            },
            BitcoinExtraOutput::OpReturn {
                data_hex: "0xdeadbeef".to_string(),
            },
        ];
        let (tx, _) = sign_p2wpkh(&mut p).unwrap();
        assert_eq!(tx.output.len(), 4);
        assert_eq!(tx.output[2].value.to_sat(), 7_000);
        assert_eq!(tx.output[3].value.to_sat(), 0);
        assert!(tx.output[3].script_pubkey.is_op_return());
        // Fee is sized for the extra outputs even before they are built.
        let fee = 10 * (10 + 68 + 4 * 31);
        assert_eq!(change_of(&tx), 200_000 - 60_000 - fee);
    }

    #[test]
    fn op_return_over_eighty_bytes_is_refused() {
        let mut p = params(Kind::P2wpkh, vec![utxo(0, 200_000)], 60_000);
        p.extra_outputs = vec![BitcoinExtraOutput::OpReturn {
            data_hex: "ab".repeat(81),
        }];
        assert!(sign_p2wpkh(&mut p)
            .unwrap_err()
            .contains("OP_RETURN data too large"));
    }

    #[test]
    fn insufficient_funds_is_reported_the_same_way_pinned_or_selected() {
        let mut selected = params(Kind::P2wpkh, vec![utxo(0, 60_100)], 60_000);
        assert_eq!(
            sign_p2wpkh(&mut selected).unwrap_err(),
            "utxo.insufficientFunds"
        );

        let mut pinned = params(Kind::P2wpkh, vec![], 60_000);
        pinned.pinned_utxos = Some(vec![utxo(0, 60_100)]);
        assert_eq!(
            sign_p2wpkh(&mut pinned).unwrap_err(),
            "utxo.insufficientFunds"
        );
    }

    /// A nested spend is only valid if the P2SH address wraps *this* key.
    #[test]
    fn nested_spend_rejects_a_foreign_p2sh_address() {
        let mut p = params(Kind::P2sh, vec![utxo(0, 100_000)], 60_000);
        p.from_address = "3EktnHQD7RiAE6uzMj2ZifT9YgRrkSgzQX".to_string();
        assert!(sign_p2sh_p2wpkh(&mut p)
            .unwrap_err()
            .contains("does not match the supplied private key"));
    }

    #[test]
    fn smallest_first_selection_consolidates_small_coins() {
        let mut p = params(
            Kind::P2wpkh,
            vec![utxo(0, 100_000), utxo(1, 40_000), utxo(2, 40_000)],
            60_000,
        );
        p.coin_selection = CoinSelectionStrategy::SmallestFirst;
        let (tx, _) = sign_p2wpkh(&mut p).unwrap();
        assert_eq!(tx.input.len(), 2, "two 40k coins beat one 100k coin");

        p.coin_selection = CoinSelectionStrategy::LargestFirst;
        let (tx, _) = sign_p2wpkh(&mut p).unwrap();
        assert_eq!(tx.input.len(), 1);
    }

    #[test]
    fn rbf_signalling_follows_the_flag() {
        let mut p = params(Kind::P2wpkh, vec![utxo(0, 100_000)], 60_000);
        let (tx, _) = sign_p2wpkh(&mut p).unwrap();
        assert_eq!(tx.input[0].sequence, Sequence::ENABLE_RBF_NO_LOCKTIME);

        p.enable_rbf = false;
        let (tx, _) = sign_p2wpkh(&mut p).unwrap();
        assert_eq!(tx.input[0].sequence, Sequence::MAX);
    }
}
