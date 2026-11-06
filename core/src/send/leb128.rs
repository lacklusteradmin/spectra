//! Unsigned LEB128: seven bits a byte, least significant group first, the
//! high bit set on every byte but the last. BCS (Aptos, Sui), protobuf (ICP,
//! Tron), Monero's varints and Solana's compact-u16 all spell their numbers
//! this way, and all read and write them here.

/// Append `value` to `out`.
pub(crate) fn write(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// `value` alone.
pub(crate) fn encode(value: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(10);
    write(value, &mut out);
    out
}

/// Why [`read`] found no number.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReadError {
    /// The bytes ended inside the number.
    EndsEarly,
    /// The number does not fit 64 bits.
    Overlong,
}

/// The number at the front of `bytes`, which is left starting after it.
pub(crate) fn read(bytes: &mut &[u8]) -> Result<u64, ReadError> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let (&byte, rest) = bytes.split_first().ok_or(ReadError::EndsEarly)?;
        *bytes = rest;
        let group = u64::from(byte & 0x7f);
        if (group << shift) >> shift != group {
            return Err(ReadError::Overlong);
        }
        value |= group << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(ReadError::Overlong)
}

#[cfg(test)]
mod tests {
    use super::{ReadError, read};

    #[test]
    fn a_number_reads_back_and_leaves_what_follows() {
        for value in [0, 127, 128, 300, 16_383, 16_384, 624_485, u64::MAX] {
            let mut bytes = super::encode(value);
            bytes.push(0xaa);
            let mut rest = bytes.as_slice();
            assert_eq!(read(&mut rest), Ok(value));
            assert_eq!(rest, [0xaa], "{value}");
        }
    }

    #[test]
    fn a_number_that_ends_early_is_refused() {
        for bytes in [&[][..], &[0x80], &[0xff, 0xff]] {
            let mut rest = bytes;
            assert_eq!(read(&mut rest), Err(ReadError::EndsEarly), "{bytes:?}");
        }
    }

    /// Past 64 bits, whether by an eleventh byte or by the tenth holding more
    /// than the one bit left, rather than silently dropping the excess.
    #[test]
    fn a_number_past_64_bits_is_refused() {
        let eleven = [0x80; 11];
        let mut tenth = [0xff; 10];
        tenth[9] = 0x02;
        for bytes in [&eleven[..], &tenth[..]] {
            let mut rest = bytes;
            assert_eq!(read(&mut rest), Err(ReadError::Overlong), "{bytes:?}");
        }
    }

    /// The boundaries where a number takes another byte, against the bytes
    /// the LEB128 specification gives.
    #[test]
    fn each_seven_bits_take_a_byte() {
        for (value, bytes) in [
            (0, &[0x00][..]),
            (127, &[0x7f]),
            (128, &[0x80, 0x01]),
            (300, &[0xac, 0x02]),
            (16_383, &[0xff, 0x7f]),
            (16_384, &[0x80, 0x80, 0x01]),
            (624_485, &[0xe5, 0x8e, 0x26]),
            (
                u64::MAX,
                &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01],
            ),
        ] {
            assert_eq!(super::encode(value), bytes, "{value}");
        }
    }
}
