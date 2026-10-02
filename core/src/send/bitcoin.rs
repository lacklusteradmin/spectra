//! Bitcoin send path: P2WPKH / P2PKH / P2TR signers, coin selection and fee
//! calculation.

use crate::send::error::SendError;

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

use crate::api::utxo::{FeeRate, Utxo};

/// Parameters for building a Bitcoin transaction.
#[derive(Debug)]
pub struct BitcoinSendParams {
    /// From address (used to find the script type).
    pub from_address: String,
    /// WIF or hex-encoded 32-byte private key.
    pub private_key_hex: crate::send::keys::SecretHex,
    /// Primary recipient address.
    pub to_address: String,
    /// Primary send amount in satoshis.
    pub amount_sats: u64,
    /// Fee rate (sats per virtual byte).
    pub fee_rate: FeeRate,
    /// UTXOs available for automatic coin selection. Ignored when
    /// `pinned_utxos` is set. Fetched from Esplora if empty.
    pub available_utxos: Vec<Utxo>,
    /// Which network, as a registry chain id (`"bitcoin"`,
    /// `"bitcoin-testnet-4"`, …).
    pub chain_id: crate::registry::Chain,
    /// Whether to signal RBF (replace-by-fee) on inputs.
    pub enable_rbf: bool,
    /// Minimum change output in satoshis; dust below this is absorbed into fee.
    /// `None` uses the standard 546-sat P2PKH dust threshold.
    pub dust_threshold: Option<u64>,
    /// Manually selected UTXOs to spend. When `Some`, bypasses automatic coin
    /// selection entirely. Their total still has to cover `amount_sats` and the
    /// fee — the builder refuses rather than signing a transaction that pays
    /// out more than it spends.
    pub pinned_utxos: Option<Vec<Utxo>>,
}

/// What one input of a given script type costs to spend and what one of its
/// outputs costs to create, in virtual bytes.
///
/// One table, so the pinned-UTXO branch and coin selection size a spend the
/// same way.
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

/// Coin selection: accumulate the largest UTXOs until we cover `target + fee`.
fn select_coins(
    utxos: &[Utxo],
    target_sats: u64,
    fee_rate: FeeRate,
    sizing: SpendSizing,
    output_count: usize,
) -> Result<(Vec<&Utxo>, u64), SendError> {
    let mut sorted: Vec<&Utxo> = utxos.iter().collect();
    sorted.sort_by_key(|utxo| std::cmp::Reverse(utxo.value));

    let mut selected: Vec<&Utxo> = Vec::new();
    let mut total: u64 = 0;

    for utxo in sorted {
        selected.push(utxo);
        // Values come from an endpoint, so they are checked rather than
        // trusted to stay inside a sum: release builds wrap on overflow, and a
        // wrapped total is a wallet that thinks it can afford the spend.
        total = total
            .checked_add(utxo.value)
            .ok_or_else(|| SendError::invalid("amount overflow"))?;

        let fee = sizing.fee(selected.len(), output_count, fee_rate);
        if total >= target_sats.saturating_add(fee) {
            return Ok((selected, fee));
        }
    }

    Err(SendError::insufficient_funds())
}

/// Sum UTXO values, refusing an overflow rather than wrapping past it.
///
/// The fixed-fee signers get this and their change subtraction together from
/// [`super::accounting::checked_change`]. Bitcoin's does not fit that shape:
/// its fee is derived from the input count coin selection settles on, the dust
/// rule changes what is actually paid out, and its shortfall is decided by
/// coin selection. What is shared is the refusal to let an endpoint's numbers
/// wrap a sum.
fn total_value<'a>(utxos: impl IntoIterator<Item = &'a Utxo>) -> Result<u64, SendError> {
    utxos
        .into_iter()
        .try_fold(0u64, |total, utxo| total.checked_add(utxo.value))
        .ok_or_else(|| SendError::invalid("amount overflow"))
}

fn tx_input_for_utxo(utxo: &Utxo, sequence: Sequence) -> Result<TxIn, SendError> {
    let txid = Txid::from_str(&utxo.txid)
        .map_err(|e| SendError::Invalid(format!("bad txid {}: {e}", utxo.txid).into()))?;
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
) -> Result<SpendIdentity, SendError> {
    let network = params.chain_id.bitcoin_network();

    let mut key_bytes = zeroize::Zeroizing::new(
        hex::decode(params.private_key_hex.as_bytes())
            .map_err(|e| SendError::Invalid(format!("bad private key hex: {e}").into()))?,
    );
    let secret_key = SecretKey::from_slice(&key_bytes)
        .map_err(|e| SendError::Invalid(format!("bad private key: {e}").into()))?;
    key_bytes.zeroize();

    let public_key = CompressedPublicKey::from_slice(
        &secp256k1::PublicKey::from_secret_key(secp, &secret_key).serialize(),
    )
    .map_err(|e| SendError::Invalid(format!("pk: {e}").into()))?;

    let to = Address::from_str(&params.to_address)
        .map_err(|e| SendError::Invalid(format!("bad recipient address: {e}").into()))?
        .require_network(network)
        .map_err(|e| SendError::Invalid(format!("recipient on wrong network: {e}").into()))?;

    let from = Address::from_str(&params.from_address)
        .map_err(|e| SendError::Invalid(format!("bad from address: {e}").into()))?
        .require_network(network)
        .map_err(|e| SendError::Invalid(format!("from address on wrong network: {e}").into()))?;

    Ok(SpendIdentity {
        secret_key,
        public_key,
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
) -> Result<UnsignedSpend, SendError> {
    build_unsigned_layout(params, &identity.from, &identity.to, sizing)
}

fn build_unsigned_layout(
    params: &BitcoinSendParams,
    from: &Address,
    to: &Address,
    sizing: SpendSizing,
) -> Result<UnsignedSpend, SendError> {
    let spend_sats = params.amount_sats;

    // The fee is sized for recipient + change whether or not the change
    // output survives the dust check below.
    let output_count = 2;

    let (selected, fee) = match params.pinned_utxos.as_deref() {
        Some(pinned) => {
            let fee = sizing.fee(pinned.len(), output_count, params.fee_rate);
            if total_value(pinned)? < spend_sats.saturating_add(fee) {
                return Err(SendError::insufficient_funds());
            }
            (pinned.iter().collect::<Vec<_>>(), fee)
        }
        None => select_coins(
            &params.available_utxos,
            spend_sats,
            params.fee_rate,
            sizing,
            output_count,
        )?,
    };

    let total_in = total_value(selected.iter().copied())?;
    // Both branches above refuse unless the inputs cover this, so an underflow
    // here is a bug in one of them rather than a small change. Saying so
    // costs a `checked_sub` and keeps the failure from becoming a signature.
    let change_sats = total_in
        .checked_sub(spend_sats)
        .and_then(|rest| rest.checked_sub(fee))
        .ok_or_else(SendError::insufficient_funds)?;

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
        script_pubkey: to.script_pubkey(),
    }];
    if change_sats > params.dust_threshold.unwrap_or(546) {
        output.push(TxOut {
            value: Amount::from_sat(change_sats),
            script_pubkey: from.script_pubkey(),
        });
    }

    // What a transaction pays out cannot exceed what it spends. This holds
    // for every path through this function — pinned or selected, change kept
    // or dropped — so stating it here catches the next arithmetic change
    // before it is signed rather than at the node.
    let total_out: u64 = output
        .iter()
        .try_fold(0u64, |total, out| total.checked_add(out.value.to_sat()))
        .ok_or_else(|| SendError::invalid("amount overflow"))?;
    if total_out > total_in {
        return Err(SendError::insufficient_funds());
    }

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
) -> Result<(Transaction, String), SendError> {
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
            .map_err(|e| SendError::Internal(format!("sighash: {e}")))?;
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
                .map_err(|_| SendError::Invalid("redeem script exceeds PushBytes limit".into()))?,
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
pub fn sign_p2wpkh(params: &mut BitcoinSendParams) -> Result<(Transaction, String), SendError> {
    let secp = Secp256k1::new();
    let identity = parse_spend_identity(&secp, params)?;
    let spend = build_unsigned_spend(params, &identity, SpendSizing::P2WPKH)?;
    sign_segwit_v0(&secp, &identity, spend, false)
}

/// Build, sign, and serialize a nested SegWit P2SH-P2WPKH transaction.
pub fn sign_p2sh_p2wpkh(
    params: &mut BitcoinSendParams,
) -> Result<(Transaction, String), SendError> {
    let secp = Secp256k1::new();
    let identity = parse_spend_identity(&secp, params)?;

    let redeem_script = ScriptBuf::new_p2wpkh(&identity.public_key.wpubkey_hash());
    if identity.spent_script() != redeem_script.to_p2sh() {
        return Err(SendError::Invalid(
            "from P2SH address does not match the supplied private key".into(),
        ));
    }

    let spend = build_unsigned_spend(params, &identity, SpendSizing::P2SH_P2WPKH)?;
    sign_segwit_v0(&secp, &identity, spend, true)
}

/// Build, sign, and serialize a P2PKH (legacy) transaction.
pub fn sign_p2pkh(params: &mut BitcoinSendParams) -> Result<(Transaction, String), SendError> {
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
            .map_err(|e| SendError::Internal(format!("sighash: {e}")))?;
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
pub fn sign_p2tr(params: &mut BitcoinSendParams) -> Result<(Transaction, String), SendError> {
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
            .map_err(|e| SendError::Internal(format!("taproot sighash: {e}")))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::utxo::UtxoStatus;

    const KEY_HEX: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const TXID: &str = "0000000000000000000000000000000000000000000000000000000000000001";
    /// A recipient nobody here holds the key for; only its script pubkey matters.
    const TO: &str = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";

    #[derive(Clone, Copy, Debug)]
    enum Kind {
        P2wpkh,
        P2sh,
        P2pkh,
        P2tr,
    }

    impl Kind {
        fn sign(self, params: &mut BitcoinSendParams) -> Result<(Transaction, String), SendError> {
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

    fn utxo(vout: u32, value: u64) -> Utxo {
        Utxo {
            txid: TXID.to_string(),
            vout,
            status: UtxoStatus {
                confirmed: true,
                block_height: Some(800_000),
            },
            value,
        }
    }

    fn params(kind: Kind, utxos: Vec<Utxo>, amount: u64) -> BitcoinSendParams {
        BitcoinSendParams {
            from_address: kind.address(),
            private_key_hex: KEY_HEX.to_string().into(),
            to_address: TO.to_string(),
            amount_sats: amount,
            fee_rate: FeeRate {
                sats_per_vbyte: 10.0,
            },
            available_utxos: utxos,
            chain_id: crate::registry::Chain::Bitcoin,
            enable_rbf: true,
            dust_threshold: None,
            pinned_utxos: None,
        }
    }

    fn change_of(tx: &Transaction) -> u64 {
        tx.output[1].value.to_sat()
    }

    /// Everything the transaction hands out. What is left of the inputs is the
    /// fee, so this must never exceed what it spends.
    fn paid_out(tx: &Transaction) -> u64 {
        tx.output.iter().map(|out| out.value.to_sat()).sum()
    }

    /// The three deterministic (ECDSA, RFC6979) script types, byte for byte.
    /// Taproot is absent on purpose: `sign_schnorr` mixes in auxiliary
    /// randomness, so its serialization differs run to run.
    #[test]
    fn signed_transactions_are_stable() {
        for (kind, expected) in [
            (
                Kind::P2wpkh,
                "0200000000010101000000000000000000000000000000000000000000000000000000000000000000000000fdffffff0260ea000000000000160014751e76e8199196d454941c45d1b3a323f1433bd6c896000000000000160014fc7250a211deddc70ee5a2738de5f07817351cef02473044022070231963b5c9439fcc27ec031107384bf86a2c30252fc8039c9653979793fc7a022023100c560d717f0285a66cd5faf7ff49341f850a1ec0f5d08c83e13e576378710121034f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa00000000",
            ),
            (
                Kind::P2sh,
                "0200000000010101000000000000000000000000000000000000000000000000000000000000000000000017160014fc7250a211deddc70ee5a2738de5f07817351ceffdffffff0260ea000000000000160014751e76e8199196d454941c45d1b3a323f1433bd6e29500000000000017a914ec8f3d9c2763a0997a465b968d99db47e82e69d287024730440220605e64f3b86dcbe8257ab75ab4e5b7c3cb31c707f730c233cf6781f719fda98c0220463b2f03f7e3464210ae05493cf6aa790f14ea103a00c00407d8ec55a64d02800121034f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa00000000",
            ),
            (
                Kind::P2pkh,
                "02000000010100000000000000000000000000000000000000000000000000000000000000000000006b4830450221009be163ca641bb99837e4eae72a67d3a53cc2fce44ca719b5117a2f1a5723e41b022031139b2ae7f92d02e6833ddfeb0062cceac2da21c60522ba615d16de17545c260121034f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aafdffffff0260ea000000000000160014751e76e8199196d454941c45d1b3a323f1433bd66c930000000000001976a914fc7250a211deddc70ee5a2738de5f07817351cef88ac00000000",
            ),
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
    /// and pinning UTXOs must not change it.
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

    /// Every path through the builder — pinned or selected, change kept or
    /// absorbed — pays out no more than it spends.
    #[test]
    fn no_layout_pays_out_more_than_it_spends() {
        for kind in [Kind::P2wpkh, Kind::P2sh, Kind::P2pkh, Kind::P2tr] {
            for (values, amount, dust) in [
                (&[200_000u64] as &[u64], 60_000u64, None),
                (&[100_000, 50_000], 120_000, None),
                // Change under the threshold is dropped into the fee.
                (&[60_630], 60_000, Some(10_000u64)),
            ] {
                let utxos = values
                    .iter()
                    .enumerate()
                    .map(|(vout, value)| utxo(vout as u32, *value))
                    .collect();
                let mut p = params(kind, utxos, amount);
                p.dust_threshold = dust;
                let Ok((tx, _)) = kind.sign(&mut p) else {
                    continue; // insufficient funds is its own assertion above
                };
                let spent: u64 = values.iter().sum();
                assert!(
                    paid_out(&tx) <= spent,
                    "{kind:?} paid out {} of {spent}",
                    paid_out(&tx)
                );
            }
        }
    }

    #[test]
    fn insufficient_funds_is_reported_the_same_way_pinned_or_selected() {
        let mut selected = params(Kind::P2wpkh, vec![utxo(0, 60_100)], 60_000);
        assert!(matches!(
            sign_p2wpkh(&mut selected).unwrap_err(),
            SendError::InsufficientFunds(_)
        ));

        let mut pinned = params(Kind::P2wpkh, vec![], 60_000);
        pinned.pinned_utxos = Some(vec![utxo(0, 60_100)]);
        assert!(matches!(
            sign_p2wpkh(&mut pinned).unwrap_err(),
            SendError::InsufficientFunds(_)
        ));
    }

    /// A nested spend is only valid if the P2SH address wraps *this* key.
    #[test]
    fn nested_spend_rejects_a_foreign_p2sh_address() {
        let mut p = params(Kind::P2sh, vec![utxo(0, 100_000)], 60_000);
        p.from_address = "3EktnHQD7RiAE6uzMj2ZifT9YgRrkSgzQX".to_string();
        assert!(
            sign_p2sh_p2wpkh(&mut p)
                .unwrap_err()
                .to_string()
                .contains("does not match the supplied private key")
        );
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

/// Frozen inputs and outputs for all four supported Bitcoin address scripts.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PreparedBitcoinTransaction {
    pub chain_id: crate::registry::Chain,
    pub from: String,
    pub to: String,
    pub amount: u64,
    pub fee_rate: f64,
    pub inputs: Vec<Utxo>,
    pub unsigned_hex: String,
    pub fee_sats: u64,
}
impl PreparedBitcoinTransaction {
    fn params(&self, key: String) -> BitcoinSendParams {
        BitcoinSendParams {
            chain_id: self.chain_id,
            from_address: self.from.clone(),
            to_address: self.to.clone(),
            amount_sats: self.amount,
            private_key_hex: key.into(),
            fee_rate: FeeRate {
                sats_per_vbyte: self.fee_rate,
            },
            available_utxos: Vec::new(),
            pinned_utxos: Some(self.inputs.clone()),
            enable_rbf: true,
            dust_threshold: None,
        }
    }
    pub fn prepare(
        chain: crate::registry::Chain,
        from: &str,
        to: &str,
        amount: u64,
        fee_rate: f64,
        inputs: Vec<Utxo>,
    ) -> Result<Self, SendError> {
        if !fee_rate.is_finite() || fee_rate <= 0.0 {
            return Err(SendError::Invalid("Invalid Bitcoin fee rate".into()));
        }
        let mut result = Self {
            chain_id: chain,
            from: from.into(),
            to: to.into(),
            amount,
            fee_rate,
            inputs,
            unsigned_hex: String::new(),
            fee_sats: 0,
        };
        let mut params = result.params(String::new());
        params.available_utxos = result.inputs.clone();
        params.pinned_utxos = None;
        let from = Address::from_str(from)
            .map_err(SendError::invalid)?
            .require_network(chain.bitcoin_network())
            .map_err(SendError::invalid)?;
        let to = Address::from_str(to)
            .map_err(SendError::invalid)?
            .require_network(chain.bitcoin_network())
            .map_err(SendError::invalid)?;
        let script = from.script_pubkey();
        let sizing = if script.is_p2tr() {
            SpendSizing::P2TR
        } else if script.is_p2wpkh() {
            SpendSizing::P2WPKH
        } else if script.is_p2sh() {
            SpendSizing::P2SH_P2WPKH
        } else if script.is_p2pkh() {
            SpendSizing::P2PKH
        } else {
            return Err(SendError::Invalid(
                "Unsupported Bitcoin sender script".into(),
            ));
        };
        let spend = build_unsigned_layout(&params, &from, &to, sizing)?;
        result.inputs = spend
            .tx
            .input
            .iter()
            .map(|i| {
                result
                    .inputs
                    .iter()
                    .find(|u| {
                        u.txid == i.previous_output.txid.to_string()
                            && u.vout == i.previous_output.vout
                    })
                    .cloned()
                    .ok_or_else(|| SendError::Invalid("Missing selected input".into()))
            })
            .collect::<Result<_, _>>()?;
        result.fee_sats = total_value(&result.inputs)?
            .checked_sub(spend.tx.output.iter().map(|o| o.value.to_sat()).sum())
            .ok_or_else(|| SendError::Invalid("Invalid Bitcoin fee".into()))?;
        result.unsigned_hex = bitcoin::consensus::encode::serialize_hex(&spend.tx);
        Ok(result)
    }
    pub fn sign(&self, key: String) -> Result<String, SendError> {
        let mut params = self.params(key);
        let script = Address::from_str(&self.from)
            .map_err(SendError::invalid)?
            .assume_checked()
            .script_pubkey();
        let (mut tx, hex) = if script.is_p2tr() {
            sign_p2tr(&mut params)?
        } else if script.is_p2wpkh() {
            sign_p2wpkh(&mut params)?
        } else if script.is_p2sh() {
            sign_p2sh_p2wpkh(&mut params)?
        } else {
            sign_p2pkh(&mut params)?
        };
        for input in &mut tx.input {
            input.script_sig = ScriptBuf::new();
            input.witness = Witness::new();
        }
        if bitcoin::consensus::encode::serialize_hex(&tx) != self.unsigned_hex {
            return Err(SendError::Invalid(
                "Signed transaction differs from reviewed Bitcoin transaction".into(),
            ));
        }
        Ok(hex)
    }
}
