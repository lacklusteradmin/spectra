//! Hedera's consensus nodes, over HAPI gRPC: `proto.CryptoService/cryptoTransfer`
//! takes a signed `Transaction` and answers its precheck.
//!
//! An endpoint of this API is a mirror node's address book
//! (`/api/v1/network/nodes`), which names each node's account, its TLS
//! service endpoints (port 50212) and the SHA-384 of its certificate. A
//! node's certificate is self-signed, so the channel is pinned to that hash
//! (`api::grpc::pinned_channel`): the book comes over the web PKI from a
//! mirror node verified as the network's, and the node then proves it holds
//! the certificate the book names. Plaintext port 50211 is not used: the
//! payload is signed and cannot be altered there, but anyone on the path
//! could answer a precheck in the node's name — an acceptance for a
//! transaction they dropped, or a refusal for one they passed on, after
//! which a person would pay again.
//!
//! A precheck is the node's answer before consensus: `OK` takes the
//! transaction for consensus; a busy node or one that cannot submit passes
//! the same transaction to the next node it was signed for; any other code
//! refuses it, and so would every node.

use std::sync::Arc;

use prost::Message as _;
use prost::bytes::{Buf as _, BufMut as _};

use crate::api::error::ApiError;
use crate::api::hedera_mirror::HederaNode;
use crate::derivation::hedera::HederaAccountId;
use crate::registry::Chain;

/// `proto.TransactionResponse`.
#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct TransactionResponse {
    #[prost(int32, tag = "1")]
    pub node_transaction_precheck_code: i32,
    #[prost(uint64, tag = "2")]
    pub cost: u64,
}

/// The HAPI method a transfer is submitted to.
pub(crate) const CRYPTO_TRANSFER: &str = "/proto.CryptoService/cryptoTransfer";

/// What a node's precheck says of a transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Precheck {
    /// Taken for consensus: `OK`, or `DUPLICATE_TRANSACTION` for an id the
    /// network already holds, which only this payer's signature could have
    /// put there.
    Accepted,
    /// The node could not take it now; another node may.
    Busy,
    /// Refused, as every node would refuse it.
    Refused,
}

/// The precheck codes (`ResponseCodeEnum`) a transfer meets, by name.
pub(crate) fn precheck(code: i32) -> (Precheck, &'static str) {
    match code {
        0 => (Precheck::Accepted, "OK"),
        11 => (Precheck::Accepted, "DUPLICATE_TRANSACTION"),
        12 => (Precheck::Busy, "BUSY"),
        67 => (Precheck::Busy, "PLATFORM_NOT_ACTIVE"),
        69 => (Precheck::Busy, "PLATFORM_TRANSACTION_NOT_CREATED"),
        // The node is not the one the body names: the next body is.
        3 => (Precheck::Busy, "INVALID_NODE_ACCOUNT"),
        341 => (Precheck::Busy, "INVALID_NODE_ACCOUNT_ID"),
        1 => (Precheck::Refused, "INVALID_TRANSACTION"),
        2 => (Precheck::Refused, "PAYER_ACCOUNT_NOT_FOUND"),
        4 => (Precheck::Refused, "TRANSACTION_EXPIRED"),
        5 => (Precheck::Refused, "INVALID_TRANSACTION_START"),
        6 => (Precheck::Refused, "INVALID_TRANSACTION_DURATION"),
        7 => (Precheck::Refused, "INVALID_SIGNATURE"),
        8 => (Precheck::Refused, "MEMO_TOO_LONG"),
        9 => (Precheck::Refused, "INSUFFICIENT_TX_FEE"),
        10 => (Precheck::Refused, "INSUFFICIENT_PAYER_BALANCE"),
        15 => (Precheck::Refused, "INVALID_ACCOUNT_ID"),
        28 => (Precheck::Refused, "INSUFFICIENT_ACCOUNT_BALANCE"),
        48 => (Precheck::Refused, "INVALID_ACCOUNT_AMOUNTS"),
        64 => (Precheck::Refused, "TRANSACTION_OVERSIZE"),
        72 => (Precheck::Refused, "ACCOUNT_DELETED"),
        74 => (Precheck::Refused, "ACCOUNT_REPEATED_IN_ACCOUNT_AMOUNTS"),
        211 => (Precheck::Refused, "INVALID_ZERO_BYTE_IN_STRING"),
        256 => (Precheck::Refused, "PAYER_ACCOUNT_DELETED"),
        _ => (Precheck::Refused, "a refusal"),
    }
}

/// Raw protobuf messages through a gRPC call: the request is already
/// encoded, and the answer is decoded by the caller.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RawCodec;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RawEncoder;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RawDecoder;

impl tonic::codec::Codec for RawCodec {
    type Encode = Vec<u8>;
    type Decode = Vec<u8>;
    type Encoder = RawEncoder;
    type Decoder = RawDecoder;

    fn encoder(&mut self) -> Self::Encoder {
        RawEncoder
    }

    fn decoder(&mut self) -> Self::Decoder {
        RawDecoder
    }
}

impl tonic::codec::Encoder for RawEncoder {
    type Item = Vec<u8>;
    type Error = tonic::Status;

    fn encode(
        &mut self,
        item: Vec<u8>,
        dst: &mut tonic::codec::EncodeBuf<'_>,
    ) -> Result<(), tonic::Status> {
        dst.put_slice(&item);
        Ok(())
    }
}

impl tonic::codec::Decoder for RawDecoder {
    type Item = Vec<u8>;
    type Error = tonic::Status;

    fn decode(
        &mut self,
        src: &mut tonic::codec::DecodeBuf<'_>,
    ) -> Result<Option<Vec<u8>>, tonic::Status> {
        Ok(Some(src.copy_to_bytes(src.remaining()).to_vec()))
    }
}

pub struct HederaHapiClient {
    /// Address books, each a mirror node's `/api/v1/network/nodes`.
    books: Arc<Vec<String>>,
}

impl HederaHapiClient {
    pub fn new(books: Arc<Vec<String>>) -> Self {
        Self { books }
    }

    /// Refuse an address book whose mirror node does not serve `chain`.
    pub(crate) async fn verify_network(&self, chain: Chain) -> Result<(), ApiError> {
        let mirrors = self
            .books
            .iter()
            .map(|book| crate::api::hedera_mirror::mirror_of_address_book(book))
            .collect::<Result<Vec<_>, _>>()?;
        crate::api::hedera_mirror::HederaMirrorClient::new(Arc::new(mirrors))
            .verify_network(chain)
            .await
    }

    /// The consensus nodes the first address book to answer names.
    pub(crate) async fn fetch_nodes(&self) -> Result<Vec<HederaNode>, ApiError> {
        crate::api::http::race(&self.books, |book| async move {
            let nodes = crate::api::hedera_mirror::fetch_address_book(&book).await?;
            if nodes.is_empty() {
                return Err(ApiError::decode("Hedera address book names no TLS node"));
            }
            Ok(nodes)
        })
        .await
    }

    /// Submit each node's signed transaction to that node, in order, until
    /// one takes it, and answer `transaction_id`. A node the book no longer
    /// names, one that cannot be reached and one too busy pass it to the
    /// next; a refusal ends the submission, since every node would refuse.
    pub(crate) async fn submit(
        &self,
        transactions: &[(HederaAccountId, Vec<u8>)],
        transaction_id: &str,
    ) -> Result<String, ApiError> {
        let nodes = self.fetch_nodes().await?;
        let mut passed = Vec::new();
        for (account, transaction) in transactions {
            let Some(node) = nodes.iter().find(|node| node.account == *account) else {
                passed.push(format!("{account}: not in the address book"));
                continue;
            };
            for (host, port) in &node.tls_endpoints {
                match submit_to(host, *port, node.cert_hash, transaction.clone()).await {
                    Ok(code) => match precheck(code) {
                        (Precheck::Accepted, _) => return Ok(transaction_id.to_string()),
                        (Precheck::Busy, name) => {
                            passed.push(format!("{account}: {name}"));
                            break;
                        }
                        (Precheck::Refused, name) => {
                            return Err(ApiError::rejected(format!(
                                "Hedera node {account} refused the transaction ({name}, code {code})"
                            )));
                        }
                    },
                    Err(error) => passed.push(format!("{account} at {host}:{port}: {error}")),
                }
            }
        }
        Err(ApiError::Transport(format!(
            "No Hedera node took the transaction: {}",
            passed.join("; ")
        )))
    }
}

/// One node's precheck code for `transaction`.
async fn submit_to(
    host: &str,
    port: u16,
    cert_hash: [u8; 48],
    transaction: Vec<u8>,
) -> Result<i32, ApiError> {
    let channel = crate::api::grpc::pinned_channel(host, port, cert_hash).await?;
    let mut grpc = tonic::client::Grpc::new(channel);
    grpc.ready()
        .await
        .map_err(|error| ApiError::Transport(format!("gRPC: {error}")))?;
    let path = http::uri::PathAndQuery::from_static(CRYPTO_TRANSFER);
    let response = grpc
        .unary(tonic::Request::new(transaction), path, RawCodec)
        .await
        .map_err(|status| {
            ApiError::Transport(format!("gRPC {}: {}", status.code(), status.message()))
        })?;
    let response = TransactionResponse::decode(response.into_inner().as_slice())
        .map_err(|error| ApiError::decode(format!("Hedera precheck: {error}")))?;
    Ok(response.node_transaction_precheck_code)
}

/// A consensus node on loopback for tests: it records each transaction it is
/// sent and answers the precheck `answer` gives it.
#[cfg(test)]
pub(crate) mod test_node {
    use super::*;
    use std::sync::Mutex;

    type Answer = Arc<dyn Fn(&[u8]) -> i32 + Send + Sync>;

    #[derive(Clone)]
    struct Node {
        answer: Answer,
        received: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    struct Unary(Node);

    impl tonic::server::UnaryService<Vec<u8>> for Unary {
        type Response = Vec<u8>;
        type Future = std::future::Ready<Result<tonic::Response<Vec<u8>>, tonic::Status>>;

        fn call(&mut self, request: tonic::Request<Vec<u8>>) -> Self::Future {
            let transaction = request.into_inner();
            let code = (self.0.answer)(&transaction);
            self.0.received.lock().unwrap().push(transaction);
            std::future::ready(Ok(tonic::Response::new(
                TransactionResponse {
                    node_transaction_precheck_code: code,
                    cost: 0,
                }
                .encode_to_vec(),
            )))
        }
    }

    /// A node on a loopback port, and what it has been sent.
    pub(crate) struct TestNode {
        pub port: u16,
        pub received: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    /// A node that answers each `cryptoTransfer` with the precheck `answer`
    /// gives the transaction, over plaintext HTTP/2 on loopback.
    pub(crate) async fn start(answer: impl Fn(&[u8]) -> i32 + Send + Sync + 'static) -> TestNode {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let received = Arc::new(Mutex::new(Vec::new()));
        let node = Node {
            answer: Arc::new(answer),
            received: received.clone(),
        };
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let node = node.clone();
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(
                        move |request: http::Request<hyper::body::Incoming>| {
                            let node = node.clone();
                            async move {
                                assert_eq!(request.uri().path(), CRYPTO_TRANSFER);
                                Ok::<_, std::convert::Infallible>(
                                    tonic::server::Grpc::new(RawCodec)
                                        .unary(Unary(node), request)
                                        .await,
                                )
                            }
                        },
                    );
                    let _ = hyper::server::conn::http2::Builder::new(
                        hyper_util::rt::TokioExecutor::new(),
                    )
                    .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                    .await;
                });
            }
        });
        TestNode { port, received }
    }
}

#[cfg(test)]
#[path = "tests/hedera_hapi.rs"]
mod tests;
