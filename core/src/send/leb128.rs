//! Unsigned LEB128: seven bits a byte, least significant group first, the
//! high bit set on every byte but the last. BCS (Aptos, Sui), protobuf (ICP),
//! Monero's varints and Solana's compact-u16 all write their numbers this
//! way, and all write them here.

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

#[cfg(test)]
mod tests {
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
