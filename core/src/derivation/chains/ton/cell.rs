//! Ordinary level-zero TON cells used by V4R2 state initialization and sends.
use sha2::{Digest, Sha256};

#[derive(Clone, Default)]
pub(crate) struct Cell {
    data: Vec<u8>,
    bits: usize,
    refs: Vec<Cell>,
}

impl Cell {
    pub fn uint(&mut self, value: u64, width: usize) -> Result<&mut Self, String> {
        if width > 64 || (width < 64 && value >> width != 0) || self.bits + width > 1023 {
            return Err("TON: integer or cell bit capacity exceeded".into());
        }
        for shift in (0..width).rev() {
            if self.bits % 8 == 0 {
                self.data.push(0);
            }
            let i = self.bits / 8;
            self.data[i] |= (((value >> shift) & 1) as u8) << (7 - self.bits % 8);
            self.bits += 1;
        }
        Ok(self)
    }
    pub fn bytes(&mut self, bytes: &[u8]) -> Result<&mut Self, String> {
        for &byte in bytes {
            self.uint(u64::from(byte), 8)?;
        }
        Ok(self)
    }
    pub fn reference(&mut self, cell: Cell) -> Result<&mut Self, String> {
        if self.refs.len() == 4 {
            return Err("TON: too many cell references".into());
        }
        self.refs.push(cell);
        Ok(self)
    }
    pub fn append(&mut self, cell: Cell) -> Result<&mut Self, String> {
        for bit in 0..cell.bits {
            self.uint(u64::from((cell.data[bit / 8] >> (7 - bit % 8)) & 1), 1)?;
        }
        for r in cell.refs {
            self.reference(r)?;
        }
        Ok(self)
    }
    pub fn address(&mut self, workchain: i8, account: &[u8; 32]) -> Result<&mut Self, String> {
        self.uint(4, 3)?
            .uint(u64::from(workchain as u8), 8)?
            .bytes(account)
    }
    pub fn coins(&mut self, amount: u64) -> Result<&mut Self, String> {
        let size = (64 - amount.leading_zeros() as usize).div_ceil(8);
        self.uint(size as u64, 4)?;
        self.bytes(&amount.to_be_bytes()[8 - size..])
    }
    pub fn body(&mut self, body: Cell) -> Result<&mut Self, String> {
        if self.bits + 1 + body.bits <= 1023 && self.refs.len() + body.refs.len() <= 4 {
            self.uint(0, 1)?.append(body)
        } else {
            self.uint(1, 1)?.reference(body)
        }
    }
    fn encoded(&self) -> Vec<u8> {
        let mut out = vec![
            self.refs.len() as u8,
            (self.bits / 8 + self.bits.div_ceil(8)) as u8,
        ];
        out.extend_from_slice(&self.data);
        if self.bits % 8 != 0 {
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
    pub fn to_boc(&self) -> Result<Vec<u8>, String> {
        fn flatten(cell: &Cell, rows: &mut Vec<Vec<u8>>) -> Result<u16, String> {
            let index = u16::try_from(rows.len()).map_err(|_| "TON: too many cells")?;
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
        let count = u16::try_from(rows.len()).map_err(|_| "TON: too many cells")?;
        let size = u32::try_from(rows.iter().map(Vec::len).sum::<usize>())
            .map_err(|_| "TON: BOC too large")?;
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
    /// Only used for the embedded, hash-checked V4R2 code, never endpoint data.
    pub(super) fn from_padded(
        data: Vec<u8>,
        descriptor: u8,
        refs: Vec<Cell>,
    ) -> Result<Self, String> {
        let bits = if descriptor & 1 != 0 {
            let last = *data.last().ok_or("TON: missing padding")?;
            if last == 0 {
                return Err("TON: invalid padding".into());
            }
            data.len() * 8 - last.trailing_zeros() as usize - 1
        } else {
            data.len() * 8
        };
        if bits > 1023 || refs.len() > 4 {
            return Err("TON: invalid cell".into());
        }
        let mut cell = Cell::default();
        for bit in 0..bits {
            cell.uint(u64::from((data[bit / 8] >> (7 - bit % 8)) & 1), 1)?;
        }
        cell.refs = refs;
        Ok(cell)
    }
}
