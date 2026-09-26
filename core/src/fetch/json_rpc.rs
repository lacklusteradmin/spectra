//! Shared wire handling; chain clients still own methods and domain decoding.
use std::sync::Arc;

use serde_json::{Value, json};

use crate::EndpointApi;
use crate::fetch::http::{HttpClient, RetryProfile, with_fallback};

pub(crate) async fn call(
    api: EndpointApi,
    client: &Arc<HttpClient>,
    endpoints: &[String],
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let body = match api {
        EndpointApi::XrplJsonRpc => json!({"method": method, "params": [params]}),
        EndpointApi::MoneroWalletRpc => {
            json!({"jsonrpc": "2.0", "id": "0", "method": method, "params": params})
        }
        EndpointApi::NearJsonRpc => {
            json!({"jsonrpc": "2.0", "id": "1", "method": method, "params": params})
        }
        EndpointApi::EvmJsonRpc
        | EndpointApi::SolanaJsonRpc
        | EndpointApi::SuiJsonRpc
        | EndpointApi::SubstrateJsonRpc
        | EndpointApi::TronJsonRpc => {
            json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
        }
        _ => return Err(format!("{} is not a JSON-RPC API", api.as_str())),
    };
    let body = Arc::new(body);
    with_fallback(endpoints, |url| {
        let client = Arc::clone(client);
        let body = Arc::clone(&body);
        async move {
            let response: Value = client
                .post_json(&url, &*body, RetryProfile::ChainRead)
                .await?;
            decode(api, response)
        }
    })
    .await
}

fn decode(api: EndpointApi, response: Value) -> Result<Value, String> {
    if let Some(error) = response.get("error").filter(|error| !error.is_null()) {
        return Err(format!("{} rpc error: {error}", api.as_str()));
    }
    let result = response
        .get("result")
        .ok_or_else(|| format!("{}: missing result", api.as_str()))?;
    if api == EndpointApi::XrplJsonRpc
        && result.get("status").and_then(Value::as_str) == Some("error")
    {
        return Err(format!(
            "xrp rpc error: {}",
            result
                .get("error_message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
        ));
    }
    Ok(result.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method},
    };

    #[tokio::test]
    async fn retries_protocol_errors_and_preserves_rpc_dialects() {
        for (api, request) in [
            (
                EndpointApi::EvmJsonRpc,
                json!({"jsonrpc":"2.0","id":1,"method":"probe","params":{}}),
            ),
            (
                EndpointApi::NearJsonRpc,
                json!({"jsonrpc":"2.0","id":"1","method":"probe","params":{}}),
            ),
            (
                EndpointApi::MoneroWalletRpc,
                json!({"jsonrpc":"2.0","id":"0","method":"probe","params":{}}),
            ),
            (
                EndpointApi::XrplJsonRpc,
                json!({"method":"probe","params":[{}]}),
            ),
        ] {
            let bad = MockServer::start().await;
            let good = MockServer::start().await;
            Mock::given(method("POST"))
                .and(body_json(&request))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"error":{"code":-1}})),
                )
                .expect(1)
                .mount(&bad)
                .await;
            Mock::given(method("POST"))
                .and(body_json(&request))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"result":{"value":42},"error":null})),
                )
                .expect(1)
                .mount(&good)
                .await;
            assert_eq!(
                call(
                    api,
                    &HttpClient::shared(),
                    &[bad.uri(), good.uri()],
                    "probe",
                    json!({})
                )
                .await
                .unwrap(),
                json!({"value":42})
            );
        }
    }

    #[test]
    fn malformed_and_xrpl_error_responses_are_not_successes() {
        assert!(decode(EndpointApi::EvmJsonRpc, json!({})).is_err());
        assert!(
            decode(
                EndpointApi::XrplJsonRpc,
                json!({"result":{"status":"error","error_message":"refused"}})
            )
            .unwrap_err()
            .contains("refused")
        );
    }
}
