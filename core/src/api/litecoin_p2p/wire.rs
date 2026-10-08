//! MWEB's wire format: outputs, inputs, kernels and the transaction that
//! holds them, as Litecoin Core serializes them in blocks, in relayed
//! transactions and in light-client answers; the HogEx a block's MWEB header
//! is committed by; and the hashes that name each part. What needs elliptic
//! curve arithmetic — signatures, range proofs, balances — is
//! `send::litecoin_mweb`'s.
//!
//! Lists and byte strings take Bitcoin's CompactSize lengths; amounts and
//! heights take Litecoin Core's `WriteVarInt` (seven bits a byte, most
//! significant first, each continuation adding one). Unknown feature bits are
//! refused rather than skipped: a field they would gate could not be read.

use bitcoin::hashes::Hash as _;
use secp256k1::PublicKey;

use crate::api::error::ApiError;

fn malformed(what: &str) -> ApiError {
    ApiError::Decode(format!("Malformed MWEB data: {what}"))
}

pub(crate) fn blake3(data: &[u8]) -> [u8; 32] {
    *::blake3::hash(data).as_bytes()
}

/// The bytes of a 64-bit bulletproof, as MWEB outputs carry it.
pub(crate) const RANGE_PROOF_SIZE: usize = 675;

/// A Pedersen commitment as the protocol serializes it: `0x08` or `0x09`
/// for y's quadratic residuosity, then x.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Commitment(pub [u8; 33]);

/// Reads protocol fields off a byte slice.
pub(crate) struct Reader<'a>(pub &'a [u8]);

impl<'a> Reader<'a> {
    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], ApiError> {
        if self.0.len() < n {
            return Err(malformed("truncated"));
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }

    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N], ApiError> {
        Ok(self.take(N)?.try_into().expect("N bytes"))
    }

    pub(crate) fn u8(&mut self) -> Result<u8, ApiError> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn u16_le(&mut self) -> Result<u16, ApiError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub(crate) fn u32_le(&mut self) -> Result<u32, ApiError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub(crate) fn u64_le(&mut self) -> Result<u64, ApiError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    /// Bitcoin's CompactSize, canonical only.
    pub(crate) fn compact_size(&mut self) -> Result<u64, ApiError> {
        let first = self.u8()?;
        let (value, minimum) = match first {
            0xfd => (u64::from(self.u16_le()?), 0xfd),
            0xfe => (u64::from(self.u32_le()?), 0x1_0000),
            0xff => (self.u64_le()?, 0x1_0000_0000),
            small => return Ok(u64::from(small)),
        };
        if value < minimum {
            return Err(malformed("a non-canonical length"));
        }
        Ok(value)
    }

    /// A length, bounded by what is left to read.
    pub(crate) fn length(&mut self, each: usize) -> Result<usize, ApiError> {
        let n = usize::try_from(self.compact_size()?).map_err(|_| malformed("a length"))?;
        if n.saturating_mul(each.max(1)) > self.0.len() {
            return Err(malformed("a length past the data"));
        }
        Ok(n)
    }

    pub(crate) fn bytes(&mut self) -> Result<Vec<u8>, ApiError> {
        let n = self.length(1)?;
        Ok(self.take(n)?.to_vec())
    }

    /// Litecoin Core's `ReadVarInt`.
    pub(crate) fn varint(&mut self) -> Result<u64, ApiError> {
        let mut n: u64 = 0;
        loop {
            let byte = self.u8()?;
            if n > (u64::MAX >> 7) {
                return Err(malformed("a varint overflow"));
            }
            n = (n << 7) | u64::from(byte & 0x7f);
            if byte & 0x80 == 0 {
                return Ok(n);
            }
            n = n
                .checked_add(1)
                .ok_or_else(|| malformed("a varint overflow"))?;
        }
    }

    pub(crate) fn public_key(&mut self) -> Result<PublicKey, ApiError> {
        PublicKey::from_slice(self.take(33)?).map_err(|_| malformed("a public key"))
    }

    pub(crate) fn commitment(&mut self) -> Result<Commitment, ApiError> {
        let commitment = Commitment(self.array()?);
        if !matches!(commitment.0[0], 0x08 | 0x09) {
            return Err(malformed("a commitment"));
        }
        Ok(commitment)
    }

    pub(crate) fn finished(&self) -> Result<(), ApiError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(malformed("trailing bytes"))
        }
    }
}

pub(crate) fn write_compact_size(n: u64, out: &mut Vec<u8>) {
    match n {
        0..=0xfc => out.push(n as u8),
        0xfd..=0xffff => {
            out.push(0xfd);
            out.extend((n as u16).to_le_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(0xfe);
            out.extend((n as u32).to_le_bytes());
        }
        _ => {
            out.push(0xff);
            out.extend(n.to_le_bytes());
        }
    }
}

pub(crate) fn write_bytes(bytes: &[u8], out: &mut Vec<u8>) {
    write_compact_size(bytes.len() as u64, out);
    out.extend(bytes);
}

/// Litecoin Core's `WriteVarInt`.
pub(crate) fn write_varint(mut n: u64, out: &mut Vec<u8>) {
    let mut digits = [0u8; 10];
    let mut len = 0;
    loop {
        digits[len] = (n & 0x7f) as u8 | if len == 0 { 0 } else { 0x80 };
        if n <= 0x7f {
            break;
        }
        n = (n >> 7) - 1;
        len += 1;
    }
    out.extend(digits[..=len].iter().rev());
}

/// Output message feature bits.
pub(crate) const OUTPUT_STANDARD_FIELDS: u8 = 0x01;
const OUTPUT_EXTRA_DATA: u8 = 0x02;

/// What an output tells its recipient, masked by their shared secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StandardFields {
    pub key_exchange: PublicKey,
    pub view_tag: u8,
    pub masked_value: u64,
    pub masked_nonce: [u8; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutputMessage {
    pub standard: Option<StandardFields>,
    pub extra: Option<Vec<u8>>,
}

impl OutputMessage {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut features = 0;
        if self.standard.is_some() {
            features |= OUTPUT_STANDARD_FIELDS;
        }
        if self.extra.is_some() {
            features |= OUTPUT_EXTRA_DATA;
        }
        let mut out = vec![features];
        if let Some(fields) = &self.standard {
            out.extend(fields.key_exchange.serialize());
            out.push(fields.view_tag);
            out.extend(fields.masked_value.to_le_bytes());
            out.extend(fields.masked_nonce);
        }
        if let Some(extra) = &self.extra {
            write_bytes(extra, &mut out);
        }
        out
    }

    fn decode(reader: &mut Reader<'_>) -> Result<Self, ApiError> {
        let features = reader.u8()?;
        if features & !(OUTPUT_STANDARD_FIELDS | OUTPUT_EXTRA_DATA) != 0 {
            return Err(malformed("unknown output features"));
        }
        let standard = if features & OUTPUT_STANDARD_FIELDS != 0 {
            Some(StandardFields {
                key_exchange: reader.public_key()?,
                view_tag: reader.u8()?,
                masked_value: reader.u64_le()?,
                masked_nonce: reader.array()?,
            })
        } else {
            None
        };
        let extra = if features & OUTPUT_EXTRA_DATA != 0 {
            Some(reader.bytes()?)
        } else {
            None
        };
        Ok(Self { standard, extra })
    }
}

/// An output's range proof: the proof itself, or the hash a compact answer
/// gives in its place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RangeProof {
    Full(Box<[u8; RANGE_PROOF_SIZE]>),
    Hash([u8; 32]),
}

impl RangeProof {
    pub(crate) fn hash(&self) -> [u8; 32] {
        match self {
            Self::Full(proof) => blake3(&proof[..]),
            Self::Hash(hash) => *hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Output {
    pub commitment: Commitment,
    pub sender: PublicKey,
    pub receiver: PublicKey,
    pub message: OutputMessage,
    pub range_proof: RangeProof,
    pub signature: [u8; 64],
}

impl Output {
    fn preimage(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(196);
        out.extend(self.commitment.0);
        out.extend(self.sender.serialize());
        out.extend(self.receiver.serialize());
        out.extend(blake3(&self.message.encode()));
        out.extend(self.range_proof.hash());
        out
    }

    /// What the sender's signature signs.
    pub(crate) fn signature_message(&self) -> [u8; 32] {
        blake3(&self.preimage())
    }

    /// The output's id: what inputs spend and the output MMR's leaves hold.
    pub(crate) fn id(&self) -> [u8; 32] {
        let mut preimage = self.preimage();
        preimage.extend(self.signature);
        blake3(&preimage)
    }

    pub(crate) fn encode(&self, out: &mut Vec<u8>) {
        out.extend(self.commitment.0);
        out.extend(self.sender.serialize());
        out.extend(self.receiver.serialize());
        out.extend(self.message.encode());
        match &self.range_proof {
            RangeProof::Full(proof) => out.extend(&proof[..]),
            RangeProof::Hash(hash) => out.extend(hash),
        }
        out.extend(self.signature);
    }

    /// An output as blocks and transactions carry it, with its range proof.
    pub(crate) fn decode(reader: &mut Reader<'_>) -> Result<Self, ApiError> {
        Self::decode_as(reader, false)
    }

    /// An output as a compact light-client answer carries it, with the
    /// range proof's hash.
    pub(crate) fn decode_compact(reader: &mut Reader<'_>) -> Result<Self, ApiError> {
        Self::decode_as(reader, true)
    }

    fn decode_as(reader: &mut Reader<'_>, compact: bool) -> Result<Self, ApiError> {
        let commitment = reader.commitment()?;
        let sender = reader.public_key()?;
        let receiver = reader.public_key()?;
        let message = OutputMessage::decode(reader)?;
        let range_proof = if compact {
            RangeProof::Hash(reader.array()?)
        } else {
            RangeProof::Full(Box::new(reader.array()?))
        };
        Ok(Self {
            commitment,
            sender,
            receiver,
            message,
            range_proof,
            signature: reader.array()?,
        })
    }
}

/// Input feature bits.
pub(crate) const INPUT_STEALTH_KEY: u8 = 0x01;
const INPUT_EXTRA_DATA: u8 = 0x02;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Input {
    pub output_id: [u8; 32],
    pub commitment: Commitment,
    pub input_key: Option<PublicKey>,
    pub output_key: PublicKey,
    pub extra: Option<Vec<u8>>,
    pub signature: [u8; 64],
}

impl Input {
    pub(crate) fn features(&self) -> u8 {
        let mut features = 0;
        if self.input_key.is_some() {
            features |= INPUT_STEALTH_KEY;
        }
        if self.extra.is_some() {
            features |= INPUT_EXTRA_DATA;
        }
        features
    }

    /// What the input's signature signs: its features and the output it
    /// spends.
    pub(crate) fn signature_message(&self) -> [u8; 32] {
        let mut message = vec![self.features()];
        message.extend(self.output_id);
        blake3(&message)
    }

    pub(crate) fn encode(&self, out: &mut Vec<u8>) {
        out.push(self.features());
        out.extend(self.output_id);
        out.extend(self.commitment.0);
        out.extend(self.output_key.serialize());
        if let Some(key) = &self.input_key {
            out.extend(key.serialize());
        }
        if let Some(extra) = &self.extra {
            write_bytes(extra, out);
        }
        out.extend(self.signature);
    }

    pub(crate) fn decode(reader: &mut Reader<'_>) -> Result<Self, ApiError> {
        let features = reader.u8()?;
        if features & !(INPUT_STEALTH_KEY | INPUT_EXTRA_DATA) != 0 {
            return Err(malformed("unknown input features"));
        }
        let output_id = reader.array()?;
        let commitment = reader.commitment()?;
        let output_key = reader.public_key()?;
        let input_key = if features & INPUT_STEALTH_KEY != 0 {
            Some(reader.public_key()?)
        } else {
            None
        };
        let extra = if features & INPUT_EXTRA_DATA != 0 {
            Some(reader.bytes()?)
        } else {
            None
        };
        Ok(Self {
            output_id,
            commitment,
            input_key,
            output_key,
            extra,
            signature: reader.array()?,
        })
    }
}

/// A canonical output a kernel's peg-out creates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PegOut {
    pub amount: u64,
    pub script: Vec<u8>,
}

/// Kernel feature bits.
pub(crate) const KERNEL_FEE: u8 = 0x01;
pub(crate) const KERNEL_PEGIN: u8 = 0x02;
pub(crate) const KERNEL_PEGOUT: u8 = 0x04;
pub(crate) const KERNEL_HEIGHT_LOCK: u8 = 0x08;
pub(crate) const KERNEL_STEALTH_EXCESS: u8 = 0x10;
pub(crate) const KERNEL_EXTRA_DATA: u8 = 0x20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Kernel {
    pub fee: Option<u64>,
    pub pegin: Option<u64>,
    pub pegouts: Vec<PegOut>,
    pub lock_height: Option<u64>,
    pub stealth_excess: Option<PublicKey>,
    pub extra: Option<Vec<u8>>,
    pub excess: Commitment,
    pub signature: [u8; 64],
}

impl Kernel {
    pub(crate) fn features(&self) -> u8 {
        let mut features = 0;
        if self.fee.is_some() {
            features |= KERNEL_FEE;
        }
        if self.pegin.is_some() {
            features |= KERNEL_PEGIN;
        }
        if !self.pegouts.is_empty() {
            features |= KERNEL_PEGOUT;
        }
        if self.lock_height.is_some() {
            features |= KERNEL_HEIGHT_LOCK;
        }
        if self.stealth_excess.is_some() {
            features |= KERNEL_STEALTH_EXCESS;
        }
        if self.extra.is_some() {
            features |= KERNEL_EXTRA_DATA;
        }
        features
    }

    fn optional_fields(&self, out: &mut Vec<u8>) {
        if let Some(fee) = self.fee {
            write_varint(fee, out);
        }
        if let Some(pegin) = self.pegin {
            write_varint(pegin, out);
        }
        if !self.pegouts.is_empty() {
            write_compact_size(self.pegouts.len() as u64, out);
            for pegout in &self.pegouts {
                write_varint(pegout.amount, out);
                write_bytes(&pegout.script, out);
            }
        }
        if let Some(height) = self.lock_height {
            write_varint(height, out);
        }
        if let Some(stealth) = &self.stealth_excess {
            out.extend(stealth.serialize());
        }
        if let Some(extra) = &self.extra {
            write_bytes(extra, out);
        }
    }

    /// What the kernel's signature signs: its features, excess and fields.
    pub(crate) fn signature_message(&self) -> [u8; 32] {
        let mut message = vec![self.features()];
        message.extend(self.excess.0);
        self.optional_fields(&mut message);
        blake3(&message)
    }

    /// The value the kernel adds to MWEB: what it pegs in, less its fee and
    /// what it pegs out.
    pub(crate) fn supply_change(&self) -> i128 {
        i128::from(self.pegin.unwrap_or(0))
            - i128::from(self.fee.unwrap_or(0))
            - self
                .pegouts
                .iter()
                .map(|p| i128::from(p.amount))
                .sum::<i128>()
    }

    pub(crate) fn encode(&self, out: &mut Vec<u8>) {
        out.push(self.features());
        self.optional_fields(out);
        out.extend(self.excess.0);
        out.extend(self.signature);
    }

    /// The kernel's id; a peg-in's canonical output commits to it.
    pub(crate) fn id(&self) -> [u8; 32] {
        let mut out = Vec::new();
        self.encode(&mut out);
        blake3(&out)
    }

    pub(crate) fn decode(reader: &mut Reader<'_>) -> Result<Self, ApiError> {
        let features = reader.u8()?;
        if features & !0x3f != 0 {
            return Err(malformed("unknown kernel features"));
        }
        let fee = (features & KERNEL_FEE != 0)
            .then(|| reader.varint())
            .transpose()?;
        let pegin = (features & KERNEL_PEGIN != 0)
            .then(|| reader.varint())
            .transpose()?;
        let mut pegouts = Vec::new();
        if features & KERNEL_PEGOUT != 0 {
            for _ in 0..reader.length(2)? {
                pegouts.push(PegOut {
                    amount: reader.varint()?,
                    script: reader.bytes()?,
                });
            }
            if pegouts.is_empty() {
                return Err(malformed("a peg-out kernel with no peg-outs"));
            }
        }
        let lock_height = (features & KERNEL_HEIGHT_LOCK != 0)
            .then(|| reader.varint())
            .transpose()?;
        let stealth_excess = (features & KERNEL_STEALTH_EXCESS != 0)
            .then(|| reader.public_key())
            .transpose()?;
        let extra = (features & KERNEL_EXTRA_DATA != 0)
            .then(|| reader.bytes())
            .transpose()?;
        Ok(Self {
            fee,
            pegin,
            pegouts,
            lock_height,
            stealth_excess,
            extra,
            excess: reader.commitment()?,
            signature: reader.array()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct TxBody {
    pub inputs: Vec<Input>,
    pub outputs: Vec<Output>,
    pub kernels: Vec<Kernel>,
}

impl TxBody {
    /// The order Litecoin Core requires: inputs by the output they spend,
    /// outputs by id, kernels by supply change, largest first, then by id.
    pub(crate) fn sort(&mut self) {
        self.inputs.sort_by_key(|input| input.output_id);
        self.outputs.sort_by_cached_key(Output::id);
        self.kernels
            .sort_by_cached_key(|kernel| (std::cmp::Reverse(kernel.supply_change()), kernel.id()));
    }

    pub(crate) fn encode(&self, out: &mut Vec<u8>) {
        write_compact_size(self.inputs.len() as u64, out);
        for input in &self.inputs {
            input.encode(out);
        }
        write_compact_size(self.outputs.len() as u64, out);
        for output in &self.outputs {
            output.encode(out);
        }
        write_compact_size(self.kernels.len() as u64, out);
        for kernel in &self.kernels {
            kernel.encode(out);
        }
    }

    pub(crate) fn decode(reader: &mut Reader<'_>) -> Result<Self, ApiError> {
        let mut body = Self::default();
        for _ in 0..reader.length(1)? {
            body.inputs.push(Input::decode(reader)?);
        }
        for _ in 0..reader.length(1)? {
            body.outputs.push(Output::decode(reader)?);
        }
        for _ in 0..reader.length(1)? {
            body.kernels.push(Kernel::decode(reader)?);
        }
        Ok(body)
    }
}

/// An MWEB transaction: its offsets and body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MwebTx {
    pub kernel_offset: [u8; 32],
    pub stealth_offset: [u8; 32],
    pub body: TxBody,
}

impl MwebTx {
    pub(crate) fn encode(&self, out: &mut Vec<u8>) {
        out.extend(self.kernel_offset);
        out.extend(self.stealth_offset);
        self.body.encode(out);
    }

    pub(crate) fn decode(reader: &mut Reader<'_>) -> Result<Self, ApiError> {
        let tx = Self {
            kernel_offset: reader.array()?,
            stealth_offset: reader.array()?,
            body: TxBody::decode(reader)?,
        };
        if tx.body.kernels.is_empty() {
            return Err(malformed("a transaction with no kernel"));
        }
        Ok(tx)
    }
}

/// A HogEx (integrating) transaction, as a light-client answer carries it:
/// its id, and its outputs' scripts and values.
#[derive(Debug, Clone)]
pub(crate) struct HogEx {
    pub txid: [u8; 32],
    pub outputs: Vec<bitcoin::TxOut>,
}

impl HogEx {
    /// Read a transaction that must carry flag `0x08` and no MWEB body.
    pub(crate) fn decode(reader: &mut Reader<'_>) -> Result<Self, ApiError> {
        let tx = ExtendedTransaction::decode(reader)?;
        if tx.mweb.is_some() {
            return Err(malformed("a HogEx carrying an MWEB body"));
        }
        if tx.canonical.output.is_empty() {
            return Err(malformed("a HogEx with no outputs"));
        }
        Ok(Self {
            txid: tx.canonical.compute_txid().to_byte_array(),
            outputs: tx.canonical.output,
        })
    }
}

/// A transaction in Litecoin's extended format, which flag bit `0x08`
/// marks: its canonical part, then the MWEB transaction it carries, if any.
/// A HogEx carries none; a transaction a wallet relays, one.
#[derive(Debug, Clone)]
pub(crate) struct ExtendedTransaction {
    pub canonical: bitcoin::Transaction,
    pub mweb: Option<MwebTx>,
}

impl ExtendedTransaction {
    pub(crate) fn decode(reader: &mut Reader<'_>) -> Result<Self, ApiError> {
        use bitcoin::consensus::Decodable;
        let version = bitcoin::transaction::Version(i32::from_le_bytes(reader.array()?));
        if reader.u8()? != 0x00 {
            return Err(malformed("a transaction without the extension marker"));
        }
        let flag = reader.u8()?;
        if flag & 0x08 == 0 || flag & !0x09 != 0 {
            return Err(malformed("a transaction without the MWEB flag"));
        }
        let mut cursor = reader.0;
        let mut input = Vec::<bitcoin::TxIn>::consensus_decode(&mut cursor)
            .map_err(|_| malformed("transaction inputs"))?;
        let output = Vec::<bitcoin::TxOut>::consensus_decode(&mut cursor)
            .map_err(|_| malformed("transaction outputs"))?;
        if flag & 0x01 != 0 {
            for input in &mut input {
                input.witness = bitcoin::Witness::consensus_decode(&mut cursor)
                    .map_err(|_| malformed("a witness"))?;
            }
        }
        let used = reader.0.len() - cursor.len();
        reader.take(used)?;
        let mweb = match reader.u8()? {
            0x00 => None,
            0x01 => Some(MwebTx::decode(reader)?),
            _ => return Err(malformed("the MWEB transaction marker")),
        };
        let lock_time = bitcoin::absolute::LockTime::from_consensus(reader.u32_le()?);
        Ok(Self {
            canonical: bitcoin::Transaction {
                version,
                lock_time,
                input,
                output,
            },
            mweb,
        })
    }
}

/// An MWEB header, as each block's HogEx commits to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MwebHeader {
    pub height: u64,
    pub output_root: [u8; 32],
    pub kernel_root: [u8; 32],
    pub leafset_root: [u8; 32],
    pub kernel_offset: [u8; 32],
    pub stealth_offset: [u8; 32],
    /// How many outputs the chain has created, spent ones included.
    pub output_mmr_size: u64,
    pub kernel_mmr_size: u64,
}

impl MwebHeader {
    pub(crate) fn encode(&self, out: &mut Vec<u8>) {
        write_varint(self.height, out);
        out.extend(self.output_root);
        out.extend(self.kernel_root);
        out.extend(self.leafset_root);
        out.extend(self.kernel_offset);
        out.extend(self.stealth_offset);
        write_varint(self.output_mmr_size, out);
        write_varint(self.kernel_mmr_size, out);
    }

    pub(crate) fn hash(&self) -> [u8; 32] {
        let mut out = Vec::new();
        self.encode(&mut out);
        blake3(&out)
    }

    pub(crate) fn decode(reader: &mut Reader<'_>) -> Result<Self, ApiError> {
        Ok(Self {
            height: reader.varint()?,
            output_root: reader.array()?,
            kernel_root: reader.array()?,
            leafset_root: reader.array()?,
            kernel_offset: reader.array()?,
            stealth_offset: reader.array()?,
            output_mmr_size: reader.varint()?,
            kernel_mmr_size: reader.varint()?,
        })
    }
}

/// The script of a peg-in's canonical output: witness version 9 and the
/// kernel's id.
pub(crate) fn pegin_script(kernel_id: &[u8; 32]) -> Vec<u8> {
    let mut script = vec![0x59, 0x20];
    script.extend(kernel_id);
    script
}

/// The script of a HogEx's first output: witness version 8 and the MWEB
/// header's hash.
pub(crate) fn hogaddr_script(header_hash: &[u8; 32]) -> Vec<u8> {
    let mut script = vec![0x58, 0x20];
    script.extend(header_hash);
    script
}
