//! TON send: external messages for the v4R2 and W5 wallet contracts.

use crate::derivation::ton::{TonAddress, TonWalletVersion};
use crate::derivation::ton_cell::Cell;
use crate::registry::Chain;
use crate::send::error::SendError;

/// The wallet a message is sent from: the contract version, the network it
/// signs for and its key pair.
pub(crate) struct TonSigner<'a> {
    pub version: TonWalletVersion,
    pub chain: Chain,
    pub private_key: &'a [u8; 32],
    pub public_key: &'a [u8; 32],
}

impl<'a> TonSigner<'a> {
    /// The signer for the wallet stored at `sender`: the version whose
    /// account that address is for this key. Refuses an address no version
    /// gives the key rather than signing as another account.
    pub(crate) fn for_sender(
        chain: Chain,
        sender: &str,
        private_key: &'a [u8; 32],
        public_key: &'a [u8; 32],
    ) -> Result<Self, SendError> {
        let version = TonWalletVersion::of_address(public_key, sender, chain).ok_or_else(|| {
            SendError::invalid("TON: the sender is no wallet version of the signing key")
        })?;
        Ok(Self {
            version,
            chain,
            private_key,
            public_key,
        })
    }
}

pub(crate) fn build_transfer_for_address(
    signer: &TonSigner<'_>,
    to: TonAddress,
    nanotons: u64,
    seqno: u32,
    comment: Option<&str>,
    valid_until: u32,
    send_mode: u8,
) -> Result<Vec<u8>, SendError> {
    let payload = comment
        .filter(|t| !t.is_empty())
        .map(comment_cell)
        .transpose()?;
    build_transfer_with_body(signer, to, nanotons, seqno, payload, valid_until, send_mode)
}

/// A text comment: op 0, then the text, 127 bytes to a cell, each cell
/// referencing the next.
pub(crate) fn comment_cell(text: &str) -> Result<Cell, SendError> {
    if text.len() > 4096 {
        return Err(SendError::invalid("TON: comment exceeds 4096 UTF-8 bytes"));
    }
    let mut bytes = vec![0u8; 4];
    bytes.extend_from_slice(text.as_bytes());
    let mut tail = None;
    for chunk in bytes.chunks(127).rev() {
        let mut cell = Cell::default();
        cell.bytes(chunk)?;
        if let Some(next) = tail {
            cell.reference(next)?;
        }
        tail = Some(cell);
    }
    Ok(tail.expect("at least the op"))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PreparedJettonTransfer {
    pub master: String,
    pub source_wallet: String,
    pub attached_nanotons: u64,
}

/// TEP-74 transfer request sent to the sender's jetton wallet. The receiver
/// in the body is the reviewed owner address, never the master contract.
pub(crate) fn build_jetton_transfer(
    signer: &TonSigner<'_>,
    plan: &PreparedJettonTransfer,
    recipient: TonAddress,
    response: TonAddress,
    amount: u128,
    seqno: u32,
    valid_until: u32,
) -> Result<Vec<u8>, SendError> {
    if amount == 0 {
        return Err(SendError::invalid("Jetton amount must be positive"));
    }
    let mut payload = Cell::default();
    payload
        .uint(0x0f8a7ea5, 32)?
        .uint(u64::from(seqno), 64)?
        .coins_u128(amount)?
        .address(recipient.workchain, &recipient.account_id)?
        .address(response.workchain, &response.account_id)?
        .uint(0, 1)?
        .coins(1)?
        .uint(0, 1)?;
    let mut to = crate::derivation::ton::parse_ton_address(&plan.source_wallet)?
        .for_network(signer.chain.is_testnet())?;
    to.bounceable = true;
    build_transfer_with_body(
        signer,
        to,
        plan.attached_nanotons,
        seqno,
        Some(payload),
        valid_until,
        3,
    )
}

/// An external message from the signer's wallet sending `nanotons` and
/// `payload` to `to`, bounceable as `to` says.
pub(crate) fn build_transfer_with_body(
    signer: &TonSigner<'_>,
    to: TonAddress,
    nanotons: u64,
    seqno: u32,
    payload: Option<Cell>,
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
    let key = SigningKey::from_bytes(signer.private_key);
    if key.verifying_key().as_bytes() != signer.public_key {
        return Err(SendError::Invalid(
            "TON: public key does not match signer".into(),
        ));
    }
    let init = signer.version.state_init(signer.public_key, signer.chain)?;
    let sender = init.hash_depth().0;
    let wallet_id = signer.version.wallet_id(signer.chain)?;

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
    if let Some(payload) = payload {
        message.body(payload)?;
    } else {
        message.uint(0, 1)?;
    }

    // A deployment signs the all-ones expiry.
    let valid_until = if seqno == 0 { u32::MAX } else { valid_until };
    let mut signing = Cell::default();
    let mut body = Cell::default();
    match signer.version {
        // wallet id, expiry, seqno, op 0 (simple send), then one
        // (mode, message) pair; the signature goes in front.
        TonWalletVersion::V4R2 => {
            signing
                .uint(u64::from(wallet_id), 32)?
                .uint(u64::from(valid_until), 32)?
                .uint(u64::from(seqno), 32)?
                .uint(0, 8)?
                .uint(u64::from(send_mode), 8)?
                .reference(message)?;
            let signature = key.sign(&signing.hash_depth().0);
            body.bytes(&signature.to_bytes())?.append(signing)?;
        }
        // The signed-external opcode, wallet id, expiry and seqno, then the
        // out-action list by reference (one `action_send_msg` on an empty
        // list) and no extended actions; the signature goes at the tail.
        TonWalletVersion::W5 => {
            let mut actions = Cell::default();
            actions
                .reference(Cell::default())?
                .uint(0x0ec3_c86d, 32)?
                .uint(u64::from(send_mode), 8)?
                .reference(message)?;
            signing
                .uint(0x7369_676e, 32)?
                .uint(u64::from(wallet_id), 32)?
                .uint(u64::from(valid_until), 32)?
                .uint(u64::from(seqno), 32)?
                .uint(1, 1)?
                .reference(actions)?
                .uint(0, 1)?;
            let signature = key.sign(&signing.hash_depth().0);
            body.append(signing)?.bytes(&signature.to_bytes())?;
        }
    }
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
    use crate::derivation::ton::{boc_root_hash, parse_ton_address};
    use serde_json::Value;

    const KEY: [u8; 32] = [1; 32];

    fn public() -> [u8; 32] {
        ed25519_dalek::SigningKey::from_bytes(&KEY)
            .verifying_key()
            .to_bytes()
    }

    fn signer(version: TonWalletVersion, chain: Chain, public: &[u8; 32]) -> TonSigner<'_> {
        TonSigner {
            version,
            chain,
            private_key: &KEY,
            public_key: public,
        }
    }

    fn transfer(
        signer: &TonSigner<'_>,
        to: &str,
        seqno: u32,
        comment: Option<&str>,
        send_mode: u8,
    ) -> Result<Vec<u8>, SendError> {
        build_transfer_for_address(
            signer,
            parse_ton_address(to)?.for_network(signer.chain.is_testnet())?,
            123456789,
            seqno,
            comment,
            1800000000,
            send_mode,
        )
    }

    fn jetton(signer: &TonSigner<'_>, amount: u128) -> Result<Vec<u8>, SendError> {
        let plan = PreparedJettonTransfer {
            master: format!("0:{}", "44".repeat(32)),
            source_wallet: format!("0:{}", "33".repeat(32)),
            attached_nanotons: 100000000,
        };
        let sender = signer.version.address(signer.public_key, signer.chain)?;
        build_jetton_transfer(
            signer,
            &plan,
            parse_ton_address(&format!("0:{}", "22".repeat(32)))?,
            parse_ton_address(&sender)?,
            amount,
            7,
            1800000000,
        )
    }

    #[test]
    fn jetton_transfers_match_official_sdk_message_hashes() {
        let public = public();
        let v4r2: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/token-send-vectors.json"))
                .unwrap();
        let w5: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/ton-w5.json")).unwrap();
        for (version, vectors) in [
            (TonWalletVersion::V4R2, &v4r2["ton"]),
            (TonWalletVersion::W5, &w5["jettons"]),
        ] {
            let signer = signer(version, Chain::Ton, &public);
            for vector in vectors.as_array().unwrap() {
                let raw =
                    jetton(&signer, vector["amount"].as_str().unwrap().parse().unwrap()).unwrap();
                assert_eq!(
                    hex::encode(boc_root_hash(&raw).unwrap()),
                    vector["root_hash"].as_str().unwrap(),
                    "{version:?}"
                );
            }
            assert!(
                jetton(&signer, 1u128 << 120)
                    .unwrap_err()
                    .to_string()
                    .contains("120-bit")
            );
        }
    }

    #[test]
    fn ton_messages_match_official_sdk_vectors() {
        let v4r2: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/protocol-transactions.json"
        ))
        .unwrap();
        let public: [u8; 32] = hex::decode(v4r2["public_key"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(public, self::public());
        for vector in v4r2["ton"].as_array().unwrap() {
            let boc = transfer(
                &signer(TonWalletVersion::V4R2, Chain::Ton, &public),
                vector["address"].as_str().unwrap(),
                vector["seqno"].as_u64().unwrap() as u32,
                Some(vector["comment"].as_str().unwrap()),
                3,
            )
            .unwrap();
            assert_eq!(
                hex::encode(boc_root_hash(&boc).unwrap()),
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
        let w5: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/ton-w5.json")).unwrap();
        for vector in w5["transfers"].as_array().unwrap() {
            let chain = if vector["network"] == "testnet" {
                Chain::TonTestnet
            } else {
                Chain::Ton
            };
            let boc = transfer(
                &signer(TonWalletVersion::W5, chain, &public),
                vector["address"].as_str().unwrap(),
                vector["seqno"].as_u64().unwrap() as u32,
                Some(vector["comment"].as_str().unwrap()),
                3,
            )
            .unwrap();
            assert_eq!(
                hex::encode(boc_root_hash(&boc).unwrap()),
                vector["root_hash"].as_str().unwrap(),
                "W5 {}",
                vector["name"]
            );
        }
    }

    #[test]
    fn ton_refuses_wrong_signer_sender_network_and_drain_modes() {
        let public = public();
        let to = format!("0:{}", "22".repeat(32));
        for version in TonWalletVersion::ALL {
            let wallet = signer(version, Chain::Ton, &public);
            for mode in [0, 128, 160, 255] {
                assert!(transfer(&wallet, &to, 7, None, mode).is_err());
            }
            let other = ed25519_dalek::SigningKey::from_bytes(&[2; 32])
                .verifying_key()
                .to_bytes();
            assert!(
                transfer(
                    &TonSigner {
                        public_key: &other,
                        ..signer(version, Chain::Ton, &public)
                    },
                    &to,
                    7,
                    None,
                    3
                )
                .is_err()
            );
            assert!(
                transfer(
                    &wallet,
                    "kQDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff-W72r5gqPgpP",
                    7,
                    None,
                    3
                )
                .is_err()
            );
            // The stored sender names the version; another key's names none.
            let sender = version.address(&public, Chain::Ton).unwrap();
            assert_eq!(
                TonSigner::for_sender(Chain::Ton, &sender, &KEY, &public)
                    .unwrap()
                    .version,
                version
            );
            assert!(TonSigner::for_sender(Chain::Ton, &sender, &[2; 32], &other).is_err());
        }
    }
}
