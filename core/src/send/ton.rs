//! TON send: WalletV4R2 message builder and signer.

use crate::send::error::SendError;

#[cfg(test)]
use crate::derivation::ton::parse_ton_address;
use crate::derivation::ton::v4r2_state_init;
use crate::derivation::ton_cell::Cell;

#[cfg(test)]
pub(crate) fn build_transfer_at(
    to_address: &str,
    nanotons: u64,
    seqno: u32,
    comment: Option<&str>,
    private_key: &[u8; 32],
    public_key: &[u8; 32],
    wallet_id: u32,
    valid_until: u32,
    send_mode: u8,
) -> Result<Vec<u8>, SendError> {
    build_transfer_for_address(
        parse_ton_address(to_address)?.for_network(false)?,
        nanotons,
        seqno,
        comment,
        private_key,
        public_key,
        wallet_id,
        valid_until,
        send_mode,
    )
}

pub(crate) fn build_transfer_for_address(
    to: crate::derivation::ton::TonAddress,
    nanotons: u64,
    seqno: u32,
    comment: Option<&str>,
    private_key: &[u8; 32],
    public_key: &[u8; 32],
    wallet_id: u32,
    valid_until: u32,
    send_mode: u8,
) -> Result<Vec<u8>, SendError> {
    use ed25519_dalek::{Signer, SigningKey};
    if nanotons == 0 {
        return Err(SendError::Invalid("TON: amount must be positive".into()));
    }
    // Fixed-value wallet transfers must not carry drain-balance or destroy modes.
    if send_mode != 3 {
        return Err(SendError::Invalid(
            "TON: only fixed-value send mode 3 is supported".into(),
        ));
    }
    let key = SigningKey::from_bytes(private_key);
    if key.verifying_key().as_bytes() != public_key {
        return Err(SendError::Invalid(
            "TON: public key does not match signer".into(),
        ));
    }
    let init = v4r2_state_init(public_key, wallet_id)?;
    let sender = init.hash_depth().0;

    let mut message = Cell::default();
    // int_msg_info: ihr_disabled, bounce, bounced, absent source, destination.
    message
        .uint(0, 1)?
        .uint(1, 1)?
        .uint(u64::from(to.bounceable), 1)?
        .uint(0, 1)?
        .uint(0, 2)?
        .address(to.workchain, &to.account_id)?
        .coins(nanotons)?
        .uint(0, 1)?
        .coins(0)?
        .coins(0)?
        .uint(0, 64)?
        .uint(0, 32)?
        .uint(0, 1)?;
    if let Some(text) = comment.filter(|t| !t.is_empty()) {
        if text.len() > 4096 {
            return Err(SendError::Invalid(
                "TON: comment exceeds 4096 UTF-8 bytes".into(),
            ));
        }
        let mut payload = vec![0u8; 4]; // text-comment opcode
        payload.extend_from_slice(text.as_bytes());
        let mut tail = None;
        for chunk in payload.chunks(127).rev() {
            let mut cell = Cell::default();
            cell.bytes(chunk)?;
            if let Some(next) = tail {
                cell.reference(next)?;
            }
            tail = Some(cell);
        }
        message.body(tail.ok_or_else(|| SendError::Invalid("TON: empty comment cell".into()))?)?;
    } else {
        message.uint(0, 1)?;
    }

    let mut signing = Cell::default();
    signing
        .uint(u64::from(wallet_id), 32)?
        .uint(
            u64::from(if seqno == 0 { u32::MAX } else { valid_until }),
            32,
        )?
        .uint(u64::from(seqno), 32)?
        .uint(0, 8)?
        .uint(u64::from(send_mode), 8)?
        .reference(message)?;
    let signature = key.sign(&signing.hash_depth().0);
    let mut body = Cell::default();
    body.bytes(&signature.to_bytes())?.append(signing)?;
    let mut external = Cell::default();
    external
        .uint(2, 2)?
        .uint(0, 2)?
        .address(0, &sender)?
        .coins(0)?;
    if seqno == 0 {
        external.uint(3, 2)?.reference(init)?;
    } else {
        external.uint(0, 1)?;
    }
    external.uint(1, 1)?.reference(body)?;
    Ok(external.to_boc()?)
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    use serde_json::Value;
    #[test]
    fn ton_messages_match_official_sdk_vectors() {
        let fixtures: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/protocol-transactions.json"
        ))
        .unwrap();
        let public: [u8; 32] = hex::decode(fixtures["public_key"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        for vector in fixtures["ton"].as_array().unwrap() {
            let boc = build_transfer_at(
                vector["address"].as_str().unwrap(),
                123456789,
                vector["seqno"].as_u64().unwrap() as u32,
                Some(vector["comment"].as_str().unwrap()),
                &[1; 32],
                &public,
                698983191,
                1800000000,
                3,
            )
            .unwrap();
            assert_eq!(
                hex::encode(crate::derivation::ton::boc_root_hash(&boc).unwrap()),
                vector["root_hash"].as_str().unwrap(),
                "{}",
                vector["name"]
            );
            // Optional export allows the independent SDK to decode actual Rust output.
            if let Ok(dir) = std::env::var("SPECTRA_PROTOCOL_OUTPUT") {
                std::fs::write(
                    std::path::Path::new(&dir)
                        .join(format!("ton-{}.boc", vector["name"].as_str().unwrap())),
                    boc,
                )
                .unwrap();
            }
        }
    }
    #[test]
    fn ton_refuses_wrong_signer_network_and_drain_modes() {
        let public = ed25519_dalek::SigningKey::from_bytes(&[1; 32])
            .verifying_key()
            .to_bytes();
        let to = format!("0:{}", "22".repeat(32));
        for mode in [0, 128, 160, 255] {
            assert!(
                build_transfer_at(
                    &to, 1, 7, None, &[1; 32], &public, 698983191, 1800000000, mode
                )
                .is_err()
            );
        }
        assert!(
            build_transfer_at(&to, 1, 7, None, &[2; 32], &public, 698983191, 1800000000, 3)
                .is_err()
        );
        assert!(
            build_transfer_at(
                "kQDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff-W72r5gqPgpP",
                1,
                7,
                None,
                &[1; 32],
                &public,
                698983191,
                1800000000,
                3
            )
            .is_err()
        );
    }
}
