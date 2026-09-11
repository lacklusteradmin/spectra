//! TON send: WalletV4R2 message builder, signer, and sendBoc.

use serde_json::{json, Value};

use crate::http::{with_fallback, RetryProfile};

use crate::derivation::chains::ton::{cell::Cell, parse_ton_address, v4r2_state_init};
use crate::fetch::chains::ton::{TonClient, TonSendResult};

impl TonClient {
    /// Send a TON transfer via TonCenter sendBoc.
    pub async fn sign_and_send(
        &self,
        to_address: &str,
        nanotons: u64,
        seqno: u32,
        comment: Option<&str>,
        private_key_bytes: &[u8; 32],
        public_key_bytes: &[u8; 32],
        subwallet_id: Option<u32>,
        expiry_seconds: Option<u32>,
        send_mode: Option<u8>,
    ) -> Result<TonSendResult, String> {
        let boc = build_wallet_v4r2_transfer(
            to_address,
            nanotons,
            seqno,
            comment,
            private_key_bytes,
            public_key_bytes,
            subwallet_id,
            expiry_seconds,
            send_mode,
        )?;

        use base64::Engine;
        self.send_boc(&base64::engine::general_purpose::STANDARD.encode(&boc))
            .await
    }

    /// Send a pre-built BOC (for rebroadcast).
    pub async fn send_boc(&self, boc_b64: &str) -> Result<TonSendResult, String> {
        let body = json!({"boc": boc_b64});
        let api_key = self.api_key.clone();
        let boc_b64 = boc_b64.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let body = body.clone();
            let api_key = api_key.clone();
            let boc_b64 = boc_b64.clone();
            let url = if let Some(key) = api_key {
                format!(
                    "{}/sendBocReturnHash?api_key={}",
                    base.trim_end_matches('/'),
                    key
                )
            } else {
                format!("{}/sendBocReturnHash", base.trim_end_matches('/'))
            };
            async move {
                let resp: Value = client
                    .post_json(&url, &body, RetryProfile::ChainWrite)
                    .await?;
                if resp.get("ok").and_then(Value::as_bool) != Some(true) {
                    return Err(format!(
                        "TON broadcast rejected: {}",
                        resp.get("error").unwrap_or(&Value::Null)
                    ));
                }
                let hash = resp
                    .get("result")
                    .and_then(|r| r.get("hash"))
                    .and_then(Value::as_str)
                    .ok_or("TON broadcast: missing message hash")?
                    .to_string();
                use base64::Engine;
                if base64::engine::general_purpose::STANDARD
                    .decode(&hash)
                    .map_err(|_| "TON broadcast: invalid hash")?
                    .len()
                    != 32
                {
                    return Err("TON broadcast: invalid hash length".into());
                }
                Ok(TonSendResult {
                    message_hash: hash,
                    boc_b64,
                })
            }
        })
        .await
    }
}

/// Complete V4R2 external message. The signing hash is the cell representation
/// hash, and seqno zero includes StateInit so a funded undeployed wallet can send.
pub fn build_wallet_v4r2_transfer(
    to_address: &str,
    nanotons: u64,
    seqno: u32,
    comment: Option<&str>,
    private_key: &[u8; 32],
    public_key: &[u8; 32],
    subwallet_id: Option<u32>,
    expiry_seconds: Option<u32>,
    send_mode: Option<u8>,
) -> Result<Vec<u8>, String> {
    let expiry = expiry_seconds.unwrap_or(60);
    if expiry == 0 {
        return Err("TON: expiry must be positive".into());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "TON: invalid system clock")?
        .as_secs();
    let valid_until = u32::try_from(now + u64::from(expiry)).map_err(|_| "TON: expiry overflow")?;
    build_transfer_at(
        to_address,
        nanotons,
        seqno,
        comment,
        private_key,
        public_key,
        subwallet_id.unwrap_or(698_983_191),
        valid_until,
        send_mode.unwrap_or(3),
    )
}

fn build_transfer_at(
    to_address: &str,
    nanotons: u64,
    seqno: u32,
    comment: Option<&str>,
    private_key: &[u8; 32],
    public_key: &[u8; 32],
    wallet_id: u32,
    valid_until: u32,
    send_mode: u8,
) -> Result<Vec<u8>, String> {
    use ed25519_dalek::{Signer, SigningKey};
    let to = parse_ton_address(to_address)?.for_network(false)?;
    if nanotons == 0 {
        return Err("TON: amount must be positive".into());
    }
    // Fixed-value wallet transfers must not carry drain-balance or destroy modes.
    if send_mode != 3 {
        return Err("TON: only fixed-value send mode 3 is supported".into());
    }
    let key = SigningKey::from_bytes(private_key);
    if key.verifying_key().as_bytes() != public_key {
        return Err("TON: public key does not match signer".into());
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
            return Err("TON: comment exceeds 4096 UTF-8 bytes".into());
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
        message.body(tail.ok_or("TON: empty comment cell")?)?;
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
    external.to_boc()
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    #[test]
    fn ton_messages_match_official_sdk_vectors() {
        let fixtures: Value =
            serde_json::from_str(include_str!("../../../testdata/protocol/transactions.json"))
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
                hex::encode(crate::derivation::chains::ton::boc_root_hash(&boc).unwrap()),
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
            assert!(build_transfer_at(
                &to, 1, 7, None, &[1; 32], &public, 698983191, 1800000000, mode
            )
            .is_err());
        }
        assert!(
            build_transfer_at(&to, 1, 7, None, &[2; 32], &public, 698983191, 1800000000, 3)
                .is_err()
        );
        assert!(build_transfer_at(
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
        .is_err());
    }
    #[tokio::test]
    async fn ton_reads_real_seqno_and_refuses_failed_reads_and_submissions() {
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        let client = TonClient::new(std::sync::Arc::new(vec![server.uri()]), None);
        Mock::given(method("GET"))
            .and(path("/getAddressInformation"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"result":{"state":"active"}})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/runGetMethod"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"ok":true,"result":{"exit_code":0,"stack":[["num","0x2a"]]}}),
            ))
            .mount(&server)
            .await;
        assert_eq!(client.fetch_seqno("address").await.unwrap(), 42);
        server.reset().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":false})))
            .mount(&server)
            .await;
        assert!(client.fetch_seqno("address").await.is_err());
        Mock::given(method("POST"))
            .and(path("/sendBocReturnHash"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"ok":false,"error":"invalid boc"})),
            )
            .mount(&server)
            .await;
        assert!(client.send_boc("payload").await.is_err());
        server.reset().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"result":{"state":"uninitialized"}})),
            )
            .mount(&server)
            .await;
        assert_eq!(client.fetch_seqno("address").await.unwrap(), 0);
    }
}
