//! MWEB as libmw defines it, written apart from Spectra's core so each
//! checks the other: tagged BLAKE3, Pedersen and switch commitments, the
//! Schnorr scheme of secp256k1-zkp's first `schnorrsig` module, stealth
//! addresses and outputs, and the checks a node makes of a transaction.

use std::collections::{HashMap, HashSet};

use litecoin::blockdata::mimblewimble as mw;
use num_bigint::BigUint;
use secp256k1::{PublicKey, SECP256K1, Scalar, SecretKey};
use sha2::{Digest, Sha256};

pub fn blake(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

/// BLAKE3 of a one-byte tag, then `data`.
pub fn tagged(tag: u8, data: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[tag]);
    hasher.update(data);
    *hasher.finalize().as_bytes()
}

fn prime() -> BigUint {
    BigUint::parse_bytes(
        b"FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFC2F",
        16,
    )
    .unwrap()
}

fn order() -> BigUint {
    BigUint::parse_bytes(
        b"FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141",
        16,
    )
    .unwrap()
}

fn is_square(y: &BigUint) -> bool {
    let p = prime();
    y.modpow(&((&p - 1u32) >> 1), &p) == BigUint::from(1u32)
}

fn y_of(point: &PublicKey) -> BigUint {
    BigUint::from_bytes_be(&point.serialize_uncompressed()[33..])
}

fn bytes32(value: &BigUint) -> [u8; 32] {
    let raw = value.to_bytes_be();
    let mut out = [0u8; 32];
    out[32 - raw.len()..].copy_from_slice(&raw);
    out
}

pub fn scalar(bytes: [u8; 32]) -> Scalar {
    Scalar::from_be_bytes(bytes).expect("a scalar below the group order")
}

pub fn public(key: &SecretKey) -> PublicKey {
    PublicKey::from_secret_key(SECP256K1, key)
}

pub fn mul(point: &PublicKey, by: &[u8; 32]) -> PublicKey {
    point.mul_tweak(SECP256K1, &scalar(*by)).expect("a point")
}

/// `point · by⁻¹`.
fn divide(point: &PublicKey, by: &[u8; 32]) -> PublicKey {
    let n = order();
    let inverse = BigUint::from_bytes_be(by).modpow(&(&n - 2u32), &n);
    mul(point, &bytes32(&inverse))
}

/// The sum of `points`, `None` for none or for the point at infinity.
fn sum(points: &[PublicKey]) -> Option<PublicKey> {
    let refs: Vec<&PublicKey> = points.iter().collect();
    if refs.is_empty() {
        return None;
    }
    PublicKey::combine_keys(&refs).ok()
}

/// secp256k1-zkp's value generator `H`.
fn generator_h() -> PublicKey {
    PublicKey::from_slice(&hex::decode("0450929b74c1a04954b78b4b6035e97a5e078a5a0f28ec96d547bfee9ace803ac031d3c6863973926e049e637cb1b5f40a36dac28af1766968c30c2313f3a38904").unwrap()).unwrap()
}

/// Grin's switch-commitment generator `J`.
fn generator_j() -> PublicKey {
    PublicKey::from_slice(
        &hex::decode("02b860f56795fc03f3c21685383d1b5a2f2954f49b7e398b8d2a0193933621155f").unwrap(),
    )
    .unwrap()
}

fn value_point(value: u64) -> PublicKey {
    let mut v = [0u8; 32];
    v[24..].copy_from_slice(&value.to_be_bytes());
    mul(&generator_h(), &v)
}

/// A commitment as libmw writes one: `0x08` when y is a square, else `0x09`,
/// then x.
pub fn commitment_of(point: &PublicKey) -> [u8; 33] {
    let mut out = [0u8; 33];
    out[0] = if is_square(&y_of(point)) { 0x08 } else { 0x09 };
    out[1..].copy_from_slice(&point.serialize_uncompressed()[1..33]);
    out
}

pub fn commitment_point(commitment: &[u8; 33]) -> Option<PublicKey> {
    if !matches!(commitment[0], 0x08 | 0x09) {
        return None;
    }
    let p = prime();
    let x = BigUint::from_bytes_be(&commitment[1..]);
    let rhs = (x.modpow(&BigUint::from(3u32), &p) + 7u32) % &p;
    let mut y = rhs.modpow(&((&p + 1u32) >> 2), &p);
    if (&y * &y) % &p != rhs {
        return None;
    }
    if commitment[0] == 0x09 {
        y = &p - y;
    }
    let mut raw = [0u8; 65];
    raw[0] = 4;
    raw[1..33].copy_from_slice(&commitment[1..]);
    raw[33..].copy_from_slice(&bytes32(&y));
    PublicKey::from_slice(&raw).ok()
}

/// `blind·G + value·H`.
pub fn commit(blind: &SecretKey, value: u64) -> [u8; 33] {
    let point = if value == 0 {
        public(blind)
    } else {
        public(blind).combine(&value_point(value)).unwrap()
    };
    commitment_of(&point)
}

/// `blind + SHA-256(commit(blind, value) ‖ blind·J)`.
pub fn switch_blind(blind: &SecretKey, value: u64) -> SecretKey {
    let mut hash = Sha256::new();
    hash.update(commit(blind, value));
    hash.update(mul(&generator_j(), &blind.secret_bytes()).serialize());
    let tweak: [u8; 32] = hash.finalize().into();
    blind.add_tweak(&scalar(tweak)).unwrap()
}

fn challenge(r: &[u8], key: &PublicKey, message: &[u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(r);
    hash.update(key.serialize());
    hash.update(message);
    hash.finalize().into()
}

pub fn sign(key: &SecretKey, message: &[u8; 32]) -> [u8; 64] {
    let mut nonce = Sha256::new();
    nonce.update(key.secret_bytes());
    nonce.update(message);
    let mut k = SecretKey::from_slice(&nonce.finalize()).unwrap();
    let r = public(&k);
    if !is_square(&y_of(&r)) {
        k = k.negate();
    }
    let r_x = &r.serialize_uncompressed()[1..33];
    let e = challenge(r_x, &public(key), message);
    let s = key
        .mul_tweak(&scalar(e))
        .unwrap()
        .add_tweak(&scalar(k.secret_bytes()))
        .unwrap();
    let mut signature = [0u8; 64];
    signature[..32].copy_from_slice(r_x);
    signature[32..].copy_from_slice(&s.secret_bytes());
    signature
}

pub fn verify(key: &PublicKey, message: &[u8; 32], signature: &[u8; 64]) -> bool {
    let Ok(s) = SecretKey::from_slice(&signature[32..]) else {
        return false;
    };
    let e = challenge(&signature[..32], key, message);
    let Ok(r) = public(&s).combine(&mul(key, &e).negate(SECP256K1)) else {
        return false;
    };
    r.serialize_uncompressed()[1..33] == signature[..32] && is_square(&y_of(&r))
}

fn zkp() -> secp256k1zkp::Secp256k1 {
    secp256k1zkp::Secp256k1::with_caps(secp256k1zkp::ContextFlag::Commit)
}

fn prove(value: u64, blind: &SecretKey, extra: &[u8]) -> [u8; 675] {
    let context = zkp();
    let key = |bytes: [u8; 32]| secp256k1zkp::SecretKey::from_slice(&context, &bytes).unwrap();
    let proof = context
        .bullet_proof(
            value,
            key(blind.secret_bytes()),
            key(rand::random()),
            key(rand::random()),
            Some(extra.to_vec()),
            None,
        )
        .expect("a range proof");
    proof.proof[..proof.plen]
        .try_into()
        .expect("a 675-byte proof")
}

fn proof_holds(commitment: &[u8; 33], proof: &[u8; 675], extra: &[u8]) -> bool {
    let mut range = secp256k1zkp::pedersen::RangeProof {
        proof: [0; secp256k1zkp::constants::MAX_PROOF_SIZE],
        plen: 675,
    };
    range.proof[..675].copy_from_slice(proof);
    zkp()
        .verify_bullet_proof(
            secp256k1zkp::pedersen::Commitment(*commitment),
            range,
            Some(extra.to_vec()),
        )
        .is_ok()
}

/// Litecoin Core's `WriteVarInt`.
pub fn varint(mut n: u64, out: &mut Vec<u8>) {
    let mut tmp = Vec::new();
    loop {
        tmp.push((n & 0x7f) as u8 | if tmp.is_empty() { 0 } else { 0x80 });
        if n <= 0x7f {
            break;
        }
        n = (n >> 7) - 1;
    }
    tmp.reverse();
    out.extend(tmp);
}

pub fn compact_size(n: u64, out: &mut Vec<u8>) {
    out.extend(litecoin::consensus::encode::serialize(&litecoin::VarInt(n)));
}

pub fn message_bytes(message: &mw::OutputMessage) -> Vec<u8> {
    let mut out = vec![message.features];
    if let Some(fields) = &message.standard_fields {
        out.extend(fields.key_exchange_pubkey.serialize());
        out.push(fields.view_tag);
        out.extend(fields.masked_value.to_le_bytes());
        out.extend(fields.masked_nonce);
    }
    if message.features & 2 != 0 {
        compact_size(message.extra_data.len() as u64, &mut out);
        out.extend(&message.extra_data);
    }
    out
}

fn output_preimage(output: &mw::Output) -> Vec<u8> {
    let mut preimage = output.commitment.to_vec();
    preimage.extend(output.sender_public_key.serialize());
    preimage.extend(output.receiver_public_key.serialize());
    preimage.extend(blake(&message_bytes(&output.message)));
    preimage.extend(blake(&output.range_proof));
    preimage
}

pub fn output_id(output: &mw::Output) -> [u8; 32] {
    let mut preimage = output_preimage(output);
    preimage.extend(output.signature);
    blake(&preimage)
}

/// An output as a compact light-client answer carries it: its range
/// proof's hash in place of the proof.
pub fn compact_output(output: &mw::Output) -> Vec<u8> {
    let mut out = output.commitment.to_vec();
    out.extend(output.sender_public_key.serialize());
    out.extend(output.receiver_public_key.serialize());
    out.extend(message_bytes(&output.message));
    out.extend(blake(&output.range_proof));
    out.extend(output.signature);
    out
}

pub fn kernel_id(kernel: &mw::Kernel) -> [u8; 32] {
    blake(&litecoin::consensus::encode::serialize(kernel))
}

fn kernel_message(kernel: &mw::Kernel) -> Vec<u8> {
    let mut out = vec![kernel.features];
    out.extend(kernel.excess);
    if let Some(fee) = kernel.fee {
        varint(fee as u64, &mut out);
    }
    if let Some(pegin) = kernel.pegin {
        varint(pegin as u64, &mut out);
    }
    if kernel.features & 4 != 0 {
        compact_size(kernel.pegouts.len() as u64, &mut out);
        for coin in &kernel.pegouts {
            varint(coin.amount as u64, &mut out);
            compact_size(coin.script_pub_key.len() as u64, &mut out);
            out.extend(coin.script_pub_key.as_bytes());
        }
    }
    if let Some(lock) = kernel.lock_height {
        varint(u64::from(lock), &mut out);
    }
    if let Some(stealth) = kernel.stealth_excess {
        out.extend(stealth.serialize());
    }
    if kernel.features & 0x20 != 0 {
        compact_size(kernel.extra_data.len() as u64, &mut out);
        out.extend(&kernel.extra_data);
    }
    out
}

fn supply_change(kernel: &mw::Kernel) -> i128 {
    i128::from(kernel.pegin.unwrap_or(0))
        - i128::from(kernel.fee.unwrap_or(0))
        - kernel
            .pegouts
            .iter()
            .map(|coin| i128::from(coin.amount))
            .sum::<i128>()
}

/// A wallet's MWEB keys: BIP-32 `m/1000'/0'` (scan) and `m/1000'/1'`
/// (spend) of its BIP-39 seed, and the spend keys of its first addresses.
pub struct Keys {
    pub scan: SecretKey,
    pub spend: SecretKey,
    spends: HashMap<PublicKey, u32>,
}

/// How many addresses of a wallet the fixture recognizes.
const LOOKAHEAD: u32 = 100;

impl Keys {
    pub fn from_phrase(phrase: &str) -> Self {
        let seed = bip39::Mnemonic::parse(phrase)
            .expect("a BIP-39 phrase")
            .to_seed("");
        let master = litecoin::bip32::Xpriv::new_master(litecoin::NetworkKind::Main, &seed)
            .expect("a master key");
        let derive = |path: &str| {
            master
                .derive_priv(
                    SECP256K1,
                    &path.parse::<litecoin::bip32::DerivationPath>().unwrap(),
                )
                .unwrap()
                .private_key
        };
        let mut keys = Self {
            scan: derive("m/1000'/0'"),
            spend: derive("m/1000'/1'"),
            spends: HashMap::new(),
        };
        keys.spends = (0..LOOKAHEAD).map(|i| (keys.address(i).1, i)).collect();
        keys
    }

    /// Address `index`: `Bᵢ = B + BLAKE3('A' ‖ i ‖ a)·G` and `Aᵢ = a·Bᵢ`.
    pub fn address(&self, index: u32) -> (PublicKey, PublicKey) {
        let mut preimage = index.to_le_bytes().to_vec();
        preimage.extend(self.scan.secret_bytes());
        let tweak = SecretKey::from_slice(&tagged(b'A', &preimage)).unwrap();
        let spend = public(&self.spend).combine(&public(&tweak)).unwrap();
        (mul(&spend, &self.scan.secret_bytes()), spend)
    }

    /// Address `index` as the `litecoin` crate writes it.
    pub fn encoded(&self, index: u32, network: litecoin::NetworkKind) -> String {
        let (scan, spend) = self.address(index);
        litecoin::Address::mweb(scan, spend, network).to_string()
    }

    /// The address index and value of an output paying the wallet.
    pub fn rewind(&self, output: &mw::Output) -> Option<(u32, u64)> {
        let fields = output.message.standard_fields.as_ref()?;
        let shared = mul(&fields.key_exchange_pubkey, &self.scan.secret_bytes()).serialize();
        if tagged(b'T', &shared)[0] != fields.view_tag {
            return None;
        }
        let t = tagged(b'D', &shared);
        let spend = divide(&output.receiver_public_key, &tagged(b'O', &t));
        let index = *self.spends.get(&spend)?;
        let mask = u64::from_le_bytes(tagged(b'Y', &t)[..8].try_into().unwrap());
        let value = fields.masked_value ^ mask;
        let blind = switch_blind(&SecretKey::from_slice(&tagged(b'B', &t)).ok()?, value);
        (commit(&blind, value) == output.commitment).then_some((index, value))
    }
}

/// An output paying `value` to the address `(scan, spend)`, made with the
/// sender key `sender`.
pub fn create_output(
    scan: &PublicKey,
    spend: &PublicKey,
    value: u64,
    sender: &SecretKey,
) -> mw::Output {
    let nonce: [u8; 16] = tagged(b'N', &sender.secret_bytes())[..16]
        .try_into()
        .unwrap();
    let mut preimage = scan.serialize().to_vec();
    preimage.extend(spend.serialize());
    preimage.extend(value.to_le_bytes());
    preimage.extend(nonce);
    let s = tagged(b'S', &preimage);
    let shared = mul(scan, &s).serialize();
    let t = tagged(b'D', &shared);
    let blind = switch_blind(&SecretKey::from_slice(&tagged(b'B', &t)).unwrap(), value);
    let value_mask = u64::from_le_bytes(tagged(b'Y', &t)[..8].try_into().unwrap());
    let nonce_mask = tagged(b'X', &t);
    let message = mw::OutputMessage {
        features: 1,
        standard_fields: Some(mw::OutputMessageStandardFields {
            key_exchange_pubkey: mul(spend, &s),
            view_tag: tagged(b'T', &shared)[0],
            masked_value: value ^ value_mask,
            masked_nonce: std::array::from_fn(|i| nonce[i] ^ nonce_mask[i]),
        }),
        extra_data: Vec::new(),
    };
    let range_proof = prove(value, &blind, &message_bytes(&message));
    let mut output = mw::Output {
        commitment: commit(&blind, value),
        sender_public_key: public(sender),
        receiver_public_key: mul(spend, &tagged(b'O', &t)),
        message,
        range_proof,
        signature: [0; 64],
    };
    output.signature = sign(sender, &blake(&output_preimage(&output)));
    output
}

/// What an unspent output is, as an input spending it must name it.
pub struct Unspent {
    pub commitment: [u8; 33],
    pub receiver: PublicKey,
}

/// What a transaction moves, once it checks out.
pub struct Checked {
    pub fee: u64,
    pub pegins: Vec<([u8; 32], u64)>,
    pub pegouts: Vec<(litecoin::ScriptBuf, u64)>,
}

/// Check an MWEB transaction as a node checks one alone: each signature and
/// range proof, its parts in order, both balances, a fee no less than its
/// weight's, and each input an unspent output `unspent` names.
pub fn check_transaction(
    tx: &mw::Transaction,
    unspent: impl Fn(&[u8; 32]) -> Option<Unspent>,
) -> Result<Checked, String> {
    let body = &tx.body;
    if body.kernels.is_empty() {
        return Err("a transaction with no kernel".into());
    }
    let ids: Vec<[u8; 32]> = body.outputs.iter().map(output_id).collect();
    if ids.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("outputs out of order".into());
    }
    if body
        .inputs
        .windows(2)
        .any(|pair| pair[0].output_id >= pair[1].output_id)
    {
        return Err("inputs out of order".into());
    }
    let kernel_order: Vec<(i128, [u8; 32])> = body
        .kernels
        .iter()
        .map(|kernel| (-supply_change(kernel), kernel_id(kernel)))
        .collect();
    if kernel_order.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("kernels out of order".into());
    }
    for output in &body.outputs {
        let message = message_bytes(&output.message);
        if !verify(
            &output.sender_public_key,
            &blake(&output_preimage(output)),
            &output.signature,
        ) {
            return Err("an output's signature".into());
        }
        if !proof_holds(&output.commitment, &output.range_proof, &message) {
            return Err("an output's range proof".into());
        }
    }
    let mut spent = HashSet::new();
    for input in &body.inputs {
        let coin = unspent(&input.output_id).ok_or("an input spending no unspent output")?;
        if coin.commitment != input.commitment || coin.receiver != input.output_public_key {
            return Err("an input naming another output".into());
        }
        if !spent.insert(input.output_id) {
            return Err("an output spent twice".into());
        }
        let stealth = input
            .input_public_key
            .ok_or("an input without a stealth key")?;
        let mut preimage = stealth.serialize().to_vec();
        preimage.extend(input.output_public_key.serialize());
        let key = mul(&input.output_public_key, &blake(&preimage))
            .combine(&stealth)
            .map_err(|_| "an input's key")?;
        let mut message = vec![input.features];
        message.extend(input.output_id);
        if !verify(&key, &blake(&message), &input.signature) {
            return Err("an input's signature".into());
        }
    }
    let mut checked = Checked {
        fee: 0,
        pegins: Vec::new(),
        pegouts: Vec::new(),
    };
    let mut weight = 0u64;
    let mut supply: i128 = 0;
    for kernel in &body.kernels {
        let excess = commitment_point(&kernel.excess).ok_or("a kernel's excess")?;
        let key = match kernel.stealth_excess {
            Some(stealth) => {
                let mut preimage = excess.serialize().to_vec();
                preimage.extend(stealth.serialize());
                mul(&excess, &blake(&preimage))
                    .combine(&stealth)
                    .map_err(|_| "a kernel's key")?
            }
            None => excess,
        };
        if !verify(&key, &blake(&kernel_message(kernel)), &kernel.signature) {
            return Err("a kernel's signature".into());
        }
        weight += 2 + u64::from(kernel.stealth_excess.is_some());
        checked.fee += kernel.fee.unwrap_or(0) as u64;
        if let Some(pegin) = kernel.pegin {
            checked.pegins.push((kernel_id(kernel), pegin as u64));
        }
        for coin in &kernel.pegouts {
            weight += (coin.script_pub_key.len() as u64).div_ceil(42);
            checked
                .pegouts
                .push((coin.script_pub_key.clone(), coin.amount as u64));
        }
        supply += supply_change(kernel);
    }
    weight += body
        .outputs
        .iter()
        .map(|output| 17 + u64::from(output.message.standard_fields.is_some()))
        .sum::<u64>();
    if checked.fee < 100 * weight {
        return Err(format!(
            "a fee of {} below its weight's {}",
            checked.fee,
            100 * weight
        ));
    }

    // Σ outputs − Σ inputs = Σ excess + offset·G + supply·H.
    let point = |c: &[u8; 33]| commitment_point(c).ok_or("a commitment");
    let mut left = Vec::new();
    let mut right = Vec::new();
    for output in &body.outputs {
        left.push(point(&output.commitment)?);
    }
    for input in &body.inputs {
        right.push(point(&input.commitment)?);
    }
    for kernel in &body.kernels {
        right.push(point(&kernel.excess)?);
    }
    if let Ok(offset) = SecretKey::from_slice(&tx.kernel_offset) {
        right.push(public(&offset));
    }
    let magnitude = u64::try_from(supply.unsigned_abs()).map_err(|_| "a supply change")?;
    if magnitude > 0 {
        if supply > 0 {
            right.push(value_point(magnitude));
        } else {
            left.push(value_point(magnitude));
        }
    }
    if sum(&left) != sum(&right) {
        return Err("commitments that do not balance".into());
    }

    // Σ sender keys + Σ input keys = stealth offset·G + Σ stealth excess + Σ spent output keys.
    let mut left: Vec<PublicKey> = body.outputs.iter().map(|o| o.sender_public_key).collect();
    let mut right = Vec::new();
    for input in &body.inputs {
        left.push(
            input
                .input_public_key
                .ok_or("an input without a stealth key")?,
        );
        right.push(input.output_public_key);
    }
    for kernel in &body.kernels {
        right.extend(kernel.stealth_excess);
    }
    if let Ok(offset) = SecretKey::from_slice(&tx.stealth_offset) {
        right.push(public(&offset));
    }
    if sum(&left) != sum(&right) {
        return Err("stealth keys that do not balance".into());
    }
    Ok(checked)
}

/// The script of a peg-in's canonical output.
pub fn pegin_script(kernel_id: &[u8; 32]) -> Vec<u8> {
    let mut script = vec![0x59, 0x20];
    script.extend(kernel_id);
    script
}

/// The script of a HogEx's first output.
pub fn hogaddr_script(header_hash: &[u8; 32]) -> Vec<u8> {
    let mut script = vec![0x58, 0x20];
    script.extend(header_hash);
    script
}
