//! Level-zero TON cells: built for wallet state initialization and sends,
//! and read from contract data a node returns. The one exotic cell is a
//! library reference, which a contract's code may be.

use crate::derivation::error::DerivationError;
use sha2::{Digest, Sha256};

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct Cell {
    data: Vec<u8>,
    bits: usize,
    refs: Vec<Cell>,
    /// A library reference: tag 2 and the library's code hash.
    library: bool,
}

impl std::fmt::Debug for Cell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cell({})", hex::encode(self.hash_depth().0))
    }
}

fn invalid(message: &'static str) -> DerivationError {
    DerivationError::Invalid(message.into())
}

impl Cell {
    pub fn uint(&mut self, value: u64, width: usize) -> Result<&mut Self, DerivationError> {
        if width > 64 || (width < 64 && value >> width != 0) || self.bits + width > 1023 {
            return Err(DerivationError::Invalid(
                "TON: integer or cell bit capacity exceeded".into(),
            ));
        }
        for shift in (0..width).rev() {
            if self.bits.is_multiple_of(8) {
                self.data.push(0);
            }
            let i = self.bits / 8;
            self.data[i] |= (((value >> shift) & 1) as u8) << (7 - self.bits % 8);
            self.bits += 1;
        }
        Ok(self)
    }
    pub fn bytes(&mut self, bytes: &[u8]) -> Result<&mut Self, DerivationError> {
        for &byte in bytes {
            self.uint(u64::from(byte), 8)?;
        }
        Ok(self)
    }
    pub fn reference(&mut self, cell: Cell) -> Result<&mut Self, DerivationError> {
        if self.refs.len() == 4 {
            return Err(DerivationError::Invalid(
                "TON: too many cell references".into(),
            ));
        }
        self.refs.push(cell);
        Ok(self)
    }
    pub fn append(&mut self, cell: Cell) -> Result<&mut Self, DerivationError> {
        for bit in 0..cell.bits {
            self.uint(u64::from((cell.data[bit / 8] >> (7 - bit % 8)) & 1), 1)?;
        }
        for r in cell.refs {
            self.reference(r)?;
        }
        Ok(self)
    }
    pub fn address(
        &mut self,
        workchain: i8,
        account: &[u8; 32],
    ) -> Result<&mut Self, DerivationError> {
        self.uint(4, 3)?
            .uint(u64::from(workchain as u8), 8)?
            .bytes(account)
    }
    pub fn coins(&mut self, amount: u64) -> Result<&mut Self, DerivationError> {
        self.coins_u128(u128::from(amount))
    }
    /// VarUInteger 16 carries at most 15 bytes, including TEP-74 amounts.
    pub fn coins_u128(&mut self, amount: u128) -> Result<&mut Self, DerivationError> {
        let size = (128 - amount.leading_zeros() as usize).div_ceil(8);
        if size > 15 {
            return Err(DerivationError::Invalid(
                "TON: amount exceeds 120-bit protocol range".into(),
            ));
        }
        self.uint(size as u64, 4)?;
        self.bytes(&amount.to_be_bytes()[16 - size..])
    }
    pub fn body(&mut self, body: Cell) -> Result<&mut Self, DerivationError> {
        if self.bits + 1 + body.bits <= 1023 && self.refs.len() + body.refs.len() <= 4 {
            self.uint(0, 1)?.append(body)
        } else {
            self.uint(1, 1)?.reference(body)
        }
    }
    /// A library cell referencing the code whose hash is `hash`.
    pub fn library(hash: &[u8; 32]) -> Result<Self, DerivationError> {
        let mut cell = Cell::default();
        cell.uint(2, 8)?.bytes(hash)?;
        cell.library = true;
        Ok(cell)
    }
    fn encoded(&self) -> Vec<u8> {
        let mut out = vec![
            self.refs.len() as u8 | if self.library { 8 } else { 0 },
            (self.bits / 8 + self.bits.div_ceil(8)) as u8,
        ];
        out.extend_from_slice(&self.data);
        if !self.bits.is_multiple_of(8) {
            *out.last_mut().expect("nonempty partial byte") |= 1 << (7 - self.bits % 8);
        }
        out
    }
    pub fn hash_depth(&self) -> ([u8; 32], u16) {
        let mut representation = self.encoded();
        let children: Vec<_> = self.refs.iter().map(Cell::hash_depth).collect();
        for (_, depth) in &children {
            representation.extend_from_slice(&depth.to_be_bytes());
        }
        for (hash, _) in &children {
            representation.extend_from_slice(hash);
        }
        let depth = children.iter().map(|(_, d)| d + 1).max().unwrap_or(0);
        (Sha256::digest(representation).into(), depth)
    }
    /// Single-root, no index/CRC. Two-byte references and four-byte offsets;
    /// ordering is parent before child, so every reference points forward.
    pub fn to_boc(&self) -> Result<Vec<u8>, DerivationError> {
        fn flatten(cell: &Cell, rows: &mut Vec<Vec<u8>>) -> Result<u16, DerivationError> {
            let index = u16::try_from(rows.len())
                .map_err(|_| DerivationError::Internal("TON: too many cells".into()))?;
            rows.push(Vec::new());
            let mut row = cell.encoded();
            for child in &cell.refs {
                row.extend_from_slice(&flatten(child, rows)?.to_be_bytes());
            }
            rows[usize::from(index)] = row;
            Ok(index)
        }
        let mut rows = Vec::new();
        flatten(self, &mut rows)?;
        let count = u16::try_from(rows.len())
            .map_err(|_| DerivationError::Internal("TON: too many cells".into()))?;
        let size = u32::try_from(rows.iter().map(Vec::len).sum::<usize>())
            .map_err(|_| DerivationError::Invalid("TON: BOC too large".into()))?;
        let mut out = vec![0xb5, 0xee, 0x9c, 0x72, 2, 4];
        out.extend_from_slice(&count.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes());
        for row in rows {
            out.extend(row);
        }
        Ok(out)
    }
    /// A cell from its padded data and descriptor, as a BOC stores it.
    pub(super) fn from_padded(
        data: Vec<u8>,
        descriptor: u8,
        refs: Vec<Cell>,
    ) -> Result<Self, DerivationError> {
        let bits = if descriptor & 1 != 0 {
            let last = *data
                .last()
                .ok_or_else(|| DerivationError::Invalid("TON: missing padding".into()))?;
            if last == 0 {
                return Err(DerivationError::Invalid("TON: invalid padding".into()));
            }
            data.len() * 8 - last.trailing_zeros() as usize - 1
        } else {
            data.len() * 8
        };
        if bits > 1023 || refs.len() > 4 {
            return Err(DerivationError::Invalid("TON: invalid cell".into()));
        }
        let mut cell = Cell::default();
        for bit in 0..bits {
            cell.uint(u64::from((data[bit / 8] >> (7 - bit % 8)) & 1), 1)?;
        }
        cell.refs = refs;
        Ok(cell)
    }
}

impl Cell {
    /// Read the cell from its first bit and reference.
    pub fn reader(&self) -> CellReader<'_> {
        CellReader {
            cell: self,
            bit: 0,
            reference: 0,
        }
    }

    fn bit_at(&self, index: usize) -> bool {
        (self.data[index / 8] >> (7 - index % 8)) & 1 == 1
    }
}

/// A cursor over a cell's bits and references, as a contract's
/// `begin_parse` reads them. Every read past the end is an error.
#[derive(Clone)]
pub(crate) struct CellReader<'a> {
    cell: &'a Cell,
    bit: usize,
    reference: usize,
}

impl<'a> CellReader<'a> {
    pub fn remaining_bits(&self) -> usize {
        self.cell.bits - self.bit
    }

    pub fn bit(&mut self) -> Result<bool, DerivationError> {
        if self.bit >= self.cell.bits {
            return Err(invalid("TON: read past the end of a cell"));
        }
        self.bit += 1;
        Ok(self.cell.bit_at(self.bit - 1))
    }

    pub fn uint(&mut self, width: usize) -> Result<u64, DerivationError> {
        if width > 64 {
            return Err(invalid("TON: integer wider than 64 bits"));
        }
        let mut value = 0u64;
        for _ in 0..width {
            value = value << 1 | u64::from(self.bit()?);
        }
        Ok(value)
    }

    pub fn bytes<const N: usize>(&mut self) -> Result<[u8; N], DerivationError> {
        let mut out = [0u8; N];
        for byte in &mut out {
            *byte = self.uint(8)? as u8;
        }
        Ok(out)
    }

    /// A 256-bit unsigned integer that must fit 64 bits.
    pub fn uint256_as_u64(&mut self) -> Result<u64, DerivationError> {
        let bytes: [u8; 32] = self.bytes()?;
        if bytes[..24].iter().any(|byte| *byte != 0) {
            return Err(invalid("TON: a 256-bit number beyond 64 bits"));
        }
        Ok(u64::from_be_bytes(
            bytes[24..].try_into().expect("eight bytes"),
        ))
    }

    /// `addr_std` without anycast: workchain and account.
    pub fn address(&mut self) -> Result<(i8, [u8; 32]), DerivationError> {
        if self.uint(3)? != 4 {
            return Err(invalid("TON: not a standard address"));
        }
        let workchain = self.uint(8)? as u8 as i8;
        Ok((workchain, self.bytes()?))
    }

    pub fn coins(&mut self) -> Result<u128, DerivationError> {
        let size = self.uint(4)? as usize;
        let mut value = 0u128;
        for _ in 0..size {
            value = value << 8 | u128::from(self.uint(8)?);
        }
        if size > 0 && value >> ((size - 1) * 8) == 0 {
            return Err(invalid("TON: coins not in their shortest form"));
        }
        Ok(value)
    }

    pub fn reference(&mut self) -> Result<&'a Cell, DerivationError> {
        let cell = self
            .cell
            .refs
            .get(self.reference)
            .ok_or_else(|| invalid("TON: read past a cell's references"))?;
        self.reference += 1;
        Ok(cell)
    }

    /// `Maybe ^Cell`.
    pub fn maybe_reference(&mut self) -> Result<Option<&'a Cell>, DerivationError> {
        if self.bit()? {
            self.reference().map(Some)
        } else {
            Ok(None)
        }
    }

    /// Refuse anything left unread.
    pub fn end(&self) -> Result<(), DerivationError> {
        if self.bit != self.cell.bits || self.reference != self.cell.refs.len() {
            return Err(invalid("TON: a cell holds more than its layout"));
        }
        Ok(())
    }
}

fn bits_for(max: usize) -> usize {
    (usize::BITS - max.leading_zeros()) as usize
}

/// A `Hashmap n X`'s entries, `n` at most 64: each key and a reader at its
/// value.
pub(crate) fn dictionary(
    root: &Cell,
    n: usize,
) -> Result<Vec<(u64, CellReader<'_>)>, DerivationError> {
    fn edge<'a>(
        cell: &'a Cell,
        n: usize,
        prefix: u64,
        out: &mut Vec<(u64, CellReader<'a>)>,
    ) -> Result<(), DerivationError> {
        let mut reader = cell.reader();
        let (length, label) = if !reader.bit()? {
            // hml_short: unary length, then the bits.
            let mut length = 0;
            while reader.bit()? {
                length += 1;
            }
            (length, reader.uint(length)?)
        } else if !reader.bit()? {
            // hml_long: the length in ⌈log2(n + 1)⌉ bits, then the bits.
            let length = reader.uint(bits_for(n))? as usize;
            (length, reader.uint(length)?)
        } else {
            // hml_same: one bit, repeated.
            let bit = reader.bit()?;
            let length = reader.uint(bits_for(n))? as usize;
            (
                length,
                if bit && length > 0 {
                    u64::MAX >> (64 - length)
                } else {
                    0
                },
            )
        };
        if length > n {
            return Err(invalid("TON: a dictionary label longer than its key"));
        }
        let key = if length == 64 {
            label
        } else {
            prefix << length | label
        };
        if length == n {
            out.push((key, reader));
            return Ok(());
        }
        let left = reader.reference()?;
        let right = reader.reference()?;
        reader.end()?;
        edge(left, n - length - 1, key << 1, out)?;
        edge(right, n - length - 1, key << 1 | 1, out)
    }
    if n > 64 {
        return Err(invalid("TON: dictionary keys wider than 64 bits"));
    }
    let mut out = Vec::new();
    edge(root, n, 0, &mut out)?;
    Ok(out)
}

/// A `Hashmap n X` of one entry, its value `value`'s bits and references,
/// its label in the shortest form, as TON's serializer writes it.
pub(crate) fn single_entry_dictionary(
    n: usize,
    key: u64,
    value: Cell,
) -> Result<Cell, DerivationError> {
    if n == 0 || n > 64 || (n < 64 && key >> n != 0) {
        return Err(invalid("TON: a key outside its dictionary"));
    }
    let width = bits_for(n);
    let short = 1 + n + 1 + n;
    let long = 2 + width + n;
    let same = (key == 0 || (n == 64 && key == u64::MAX) || (n < 64 && key == (1 << n) - 1))
        .then_some(3 + width);
    let mut cell = Cell::default();
    if same.is_some_and(|same| same < long.min(short)) {
        cell.uint(0b11, 2)?
            .uint(u64::from(key != 0), 1)?
            .uint(n as u64, width)?;
    } else if short <= long {
        cell.uint(0, 1)?;
        for _ in 0..n {
            cell.uint(1, 1)?;
        }
        cell.uint(0, 1)?.uint(key, n)?;
    } else {
        cell.uint(0b10, 2)?.uint(n as u64, width)?.uint(key, n)?;
    }
    cell.append(value)?;
    Ok(cell)
}
