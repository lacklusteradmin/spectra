//! Bitcoin's legacy wire format, shared by every chain that inherited it.
//!
//! `varint` had nine identical copies across this folder, `dsha256` six, and
//! the txid decoder five under three names. The copies agreed on the bytes and
//! disagreed on what to do when the input was wrong: two swallowed a bad txid
//! into an empty prevout, which builds a structurally invalid transaction and
//! reports success. Here it is one error.

use sha2::{Digest, Sha256};

/// Bitcoin `CompactSize`. Values above `u32::MAX` do not occur in any field
/// this crate writes — script lengths, input and output counts.
pub(crate) fn varint(n: usize) -> Vec<u8> {
    match n {
        0..=0xfc => vec![n as u8],
        0xfd..=0xffff => {
            let mut v = vec![0xfd];
            v.extend_from_slice(&(n as u16).to_le_bytes());
            v
        }
        _ => {
            let mut v = vec![0xfe];
            v.extend_from_slice(&(n as u32).to_le_bytes());
            v
        }
    }
}

/// SHA-256 applied twice — Bitcoin's `HASH256`.
pub(crate) fn dsha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(Sha256::digest(data)).into()
}

/// Decode a display-order txid into the little-endian bytes an outpoint
/// carries.
pub(crate) fn decode_txid_le(txid: &str) -> Result<Vec<u8>, String> {
    let mut bytes = hex::decode(txid).map_err(|e| format!("txid decode: {e}"))?;
    if bytes.len() != 32 {
        return Err("txid must contain exactly 32 bytes".into());
    }
    bytes.reverse();
    Ok(bytes)
}

/// `OP_DUP OP_HASH160 <20-byte hash> OP_EQUALVERIFY OP_CHECKSIG`.
pub(crate) fn p2pkh_script(hash: &[u8; 20]) -> Vec<u8> {
    let mut script = vec![0x76u8, 0xa9, 0x14];
    script.extend_from_slice(hash);
    script.push(0x88);
    script.push(0xac);
    script
}

/// `<sig> <pubkey>`, each behind its own length byte. Both are well under the
/// 76-byte direct-push limit.
pub(crate) fn p2pkh_script_sig(der_sig: &[u8], pubkey: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + der_sig.len() + pubkey.len());
    out.push(der_sig.len() as u8);
    out.extend_from_slice(der_sig);
    out.push(pubkey.len() as u8);
    out.extend_from_slice(pubkey);
    out
}

/// One serialized transaction input: outpoint, script sig, sequence.
pub(crate) fn build_input(
    txid: &str,
    vout: u32,
    script_sig: &[u8],
    sequence: u32,
) -> Result<Vec<u8>, String> {
    let mut out = decode_txid_le(txid)?;
    out.extend_from_slice(&vout.to_le_bytes());
    out.extend_from_slice(&varint(script_sig.len()));
    out.extend_from_slice(script_sig);
    out.extend_from_slice(&sequence.to_le_bytes());
    Ok(out)
}

/// A version-1 transaction with zero locktime, from already-serialized inputs
/// and `(script_pubkey, value)` outputs.
pub(crate) fn build_tx(inputs: &[Vec<u8>], outputs: &[(Vec<u8>, u64)]) -> Vec<u8> {
    let mut raw = Vec::new();
    raw.extend_from_slice(&1u32.to_le_bytes()); // version
    raw.extend_from_slice(&varint(inputs.len()));
    for input in inputs {
        raw.extend_from_slice(input);
    }
    raw.extend_from_slice(&varint(outputs.len()));
    for (script, value) in outputs {
        raw.extend_from_slice(&value.to_le_bytes());
        raw.extend_from_slice(&varint(script.len()));
        raw.extend_from_slice(script);
    }
    raw.extend_from_slice(&0u32.to_le_bytes()); // locktime
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_cover_each_width() {
        assert_eq!(varint(0), vec![0x00]);
        assert_eq!(varint(0xfc), vec![0xfc]);
        assert_eq!(varint(0xfd), vec![0xfd, 0xfd, 0x00]);
        assert_eq!(varint(0xffff), vec![0xfd, 0xff, 0xff]);
        assert_eq!(varint(0x1_0000), vec![0xfe, 0x00, 0x00, 0x01, 0x00]);
    }

    #[test]
    fn dsha256_matches_the_known_empty_digest() {
        assert_eq!(
            hex::encode(dsha256(b"")),
            "5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456"
        );
    }

    /// A txid that is not hex is an error, not an empty outpoint. Two of the
    /// copies this replaced built a transaction around the empty case.
    #[test]
    fn a_bad_txid_is_refused() {
        assert!(decode_txid_le("not hex").is_err());
        assert!(build_input("not hex", 0, &[], 0xffff_ffff).is_err());
        for bad in [
            String::new(),
            "0102".into(),
            "00".repeat(31),
            "00".repeat(33),
        ] {
            assert!(decode_txid_le(&bad).is_err());
        }
        let mut expected = vec![0; 32];
        expected[0] = 1;
        assert_eq!(
            decode_txid_le(&format!("{}01", "00".repeat(31))).unwrap(),
            expected
        );
    }

    #[test]
    fn a_p2pkh_script_is_twenty_five_bytes() {
        let script = p2pkh_script(&[0x11; 20]);
        assert_eq!(script.len(), 25);
        assert_eq!(&script[..3], &[0x76, 0xa9, 0x14]);
        assert_eq!(&script[23..], &[0x88, 0xac]);
    }

    #[test]
    fn a_transaction_frames_its_inputs_and_outputs() {
        let input = build_input(
            "00000000000000000000000000000000000000000000000000000000000000ff",
            1,
            &[0xaa],
            0xffff_ffff,
        )
        .unwrap();
        let raw = build_tx(&[input], &[(p2pkh_script(&[0x22; 20]), 1_000)]);
        assert_eq!(&raw[..4], &1u32.to_le_bytes());
        assert_eq!(raw[4], 1, "one input");
        assert_eq!(&raw[raw.len() - 4..], &0u32.to_le_bytes(), "locktime");
    }
}
