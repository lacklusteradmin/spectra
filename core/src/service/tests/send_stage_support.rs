//! What the staged-send tests share: a service holding one imported wallet,
//! its network's one endpoint a mock node, and the build, sign and broadcast
//! stages run against it.
use super::*;
use crate::derivation::import::{WalletImportCommit, WalletImportKind, WalletImportRequest};
use crate::send::SendExecutionRequest;
use crate::send::stages::SendArtifact;
use serde_json::Value;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

/// What a wallet is imported from.
pub(crate) enum Secret<'a> {
    Key(&'a str),
    Phrase(&'a str),
}

/// A mock node answering each request from its path and JSON body (`Null`
/// for a body that is none): `None` is a 404.
pub(crate) async fn node(
    answer: impl Fn(&str, &Value) -> Option<Value> + Send + Sync + 'static,
) -> MockServer {
    node_at(move |url, body| answer(url.path(), body)).await
}

/// [`node`] answering from the whole URL, its query included.
pub(crate) async fn node_at(
    answer: impl Fn(&reqwest::Url, &Value) -> Option<Value> + Send + Sync + 'static,
) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let body = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            match answer(&request.url, &body) {
                Some(value) => ResponseTemplate::new(200).set_body_json(value),
                None => {
                    ResponseTemplate::new(404).set_body_json(serde_json::json!({"status": 404}))
                }
            }
        })
        .mount(&server)
        .await;
    server
}

/// The JSON-RPC method of each request `server` received, in order.
pub(crate) async fn methods(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter_map(|request| {
            serde_json::from_slice::<Value>(&request.body).ok()?["method"]
                .as_str()
                .map(str::to_string)
        })
        .collect()
}

/// The path of each request `server` received, in order.
pub(crate) async fn paths(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.url.path().to_string())
        .collect()
}

pub(crate) struct Wallet {
    pub service: Arc<WalletService>,
    pub id: String,
    pub address: String,
    pub chain: Chain,
    database: std::path::PathBuf,
}

impl Wallet {
    /// A fresh store holding one wallet imported from `secret`, sealed with
    /// no password, whose network's only endpoint is `endpoint`.
    pub(crate) async fn import(chain: Chain, endpoint: &str, secret: Secret<'_>) -> Self {
        let service = WalletService::new(vec![ChainEndpoints {
            chain_id: chain,
            endpoints: vec![endpoint.to_string()],
            capabilities: EndpointCapability::ALL.to_vec(),
        }])
        .unwrap();
        service.set_secret_store(Arc::new(
            crate::store::secret_backends::InMemorySecretStore::new(),
        ));
        let database = std::env::temp_dir().join(format!(
            "staged-send-{}.sqlite",
            crate::store::new_event_id()
        ));
        service
            .open_state(database.to_string_lossy().into())
            .await
            .unwrap();
        let (kind, seed_phrase, private_key) = match secret {
            Secret::Key(key) => (WalletImportKind::PrivateKey, None, Some(key.to_string())),
            Secret::Phrase(phrase) => (WalletImportKind::Phrase, Some(phrase.to_string()), None),
        };
        let wallet = service
            .import_wallets(WalletImportCommit {
                password: None,
                request: WalletImportRequest {
                    wallet_name: "Sender".into(),
                    chain,
                    kind,
                },
                derivation_path: None,
                derivation_overrides: Default::default(),
                seed_phrase,
                private_key,
                restore_height: None,
                named_account: None,
                ton_wallet_version: None,
                upgrade_wallet_id: None,
            })
            .await
            .unwrap()
            .wallets
            .remove(0);
        let address = wallet.addresses.values().next().unwrap().clone();
        Self {
            service,
            id: wallet.id,
            address,
            chain,
            database,
        }
    }

    /// Add `url` as the user's endpoint for `api`, with every capability the
    /// API offers, and use only the user's endpoints on this network: what a
    /// service asked for by API reaches the mock, never the catalog's hosts.
    pub(crate) async fn use_only(&self, api: &str, url: &str) {
        use crate::store::state::{AppSettingUpdate, StateCommand};
        let api_kind = self
            .chain
            .compatible_endpoint_apis()
            .into_iter()
            .find(|candidate| candidate.as_str() == api)
            .unwrap();
        for update in [
            AppSettingUpdate::AddCustomEndpoint {
                chain_id: self.chain,
                api: api.into(),
                endpoint: url.into(),
                capabilities: crate::endpoint_api::endpoint_capability_options(
                    self.chain, api_kind,
                ),
            },
            AppSettingUpdate::CustomEndpointsOnly {
                chain_id: self.chain,
                value: true,
            },
        ] {
            self.service
                .apply_state_command(StateCommand::SetAppSetting { update })
                .await
                .unwrap();
        }
        assert_eq!(
            self.service
                .custom_api_endpoints(self.chain, &[api_kind], &[])
                .await,
            [url.trim_end_matches('/')]
        );
        assert!(self.service.uses_custom_endpoints_only(self.chain).await);
    }

    /// Track the token `contract` under `standard` and give the wallet a
    /// holding of `amount` of it, as a refresh would have: what an owned
    /// preview quotes from. Returns the holding's key.
    pub(crate) async fn hold_token(
        &self,
        standard: &str,
        contract: &str,
        decimals: u32,
        amount: &str,
    ) -> String {
        use crate::store::state::StateCommand;
        self.service
            .apply_state_command(StateCommand::AddCustomToken {
                standard: Some(standard.into()),
                chain_id: self.chain,
                symbol: "TEST".into(),
                name: "Test".into(),
                contract: contract.into(),
                coingecko_id: String::new(),
                coinpaprika_id: String::new(),
                decimals,
            })
            .await
            .unwrap();
        let holding = crate::store::wallet_domain::AssetHolding {
            id: String::new(),
            name: "Test".into(),
            symbol: "TEST".into(),
            coingecko_id: String::new(),
            chain_id: self.chain,
            token_standard: standard.into(),
            contract_address: Some(contract.into()),
            amount: amount.into(),
        };
        let mut wallet = self.service.stored_wallet(&self.id).await.unwrap();
        wallet.holdings = vec![holding.clone()];
        self.service
            .apply_state_command(StateCommand::UpsertWallet { wallet })
            .await
            .unwrap();
        holding.deployment_id()
    }

    /// A send of `amount` of the network's coin to `to`.
    pub(crate) fn request(&self, to: &str, amount: &str) -> SendExecutionRequest {
        SendExecutionRequest {
            chain_id: self.chain,
            wallet_id: self.id.clone(),
            password: None,
            to_address: to.into(),
            amount_str: amount.into(),
            contract_address: None,
            token_standard: None,
            token_decimals: None,
            fee_rate_svb: None,
            fee_sat: None,
            gas_budget: None,
            fee_amount: None,
            evm_overrides: None,
            sign_only: false,
            memo: None,
        }
    }

    /// [`Self::request`] for `amount` of the token `contract`.
    pub(crate) fn token_request(
        &self,
        to: &str,
        amount: &str,
        contract: &str,
        decimals: u32,
    ) -> SendExecutionRequest {
        SendExecutionRequest {
            contract_address: Some(contract.into()),
            token_decimals: Some(decimals),
            ..self.request(to, amount)
        }
    }

    pub(crate) async fn build(
        &self,
        request: SendExecutionRequest,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        self.service.build_send(request).await
    }

    pub(crate) async fn sign(
        &self,
        artifact: &SendArtifact,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        self.service
            .sign_send(artifact.id.clone(), artifact.review_digest.clone(), None)
            .await
    }

    pub(crate) async fn broadcast(
        &self,
        artifact: &SendArtifact,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let endpoints = self.service.send_endpoints(self.chain).await.unwrap();
        self.service
            .broadcast_send(artifact.id.clone(), endpoints)
            .await
    }

    /// The artifact as stored now.
    pub(crate) async fn stored(&self, artifact: &SendArtifact) -> SendArtifact {
        self.service
            .inspect_send(artifact.id.clone())
            .await
            .unwrap()
    }

    /// Whether the store holds any built transaction.
    pub(crate) async fn built_nothing(&self) -> bool {
        self.service.list_sends().await.unwrap().is_empty()
    }

    /// Edit the stored artifact's JSON in place, as anything that can write
    /// the database file could.
    pub(crate) fn rewrite(&self, artifact: &SendArtifact, edit: impl FnOnce(&mut Value)) {
        let db = rusqlite::Connection::open(&self.database).unwrap();
        let payload: String = db
            .query_row(
                "SELECT payload FROM send_artifacts WHERE id=?1",
                [&artifact.id],
                |row| row.get(0),
            )
            .unwrap();
        let mut payload: Value = serde_json::from_str(&payload).unwrap();
        edit(&mut payload);
        db.execute(
            "UPDATE send_artifacts SET payload=?2 WHERE id=?1",
            rusqlite::params![artifact.id, payload.to_string()],
        )
        .unwrap();
    }
}

/// What a refusal says, for an assertion on its words.
pub(crate) fn refusal<T: std::fmt::Debug>(result: Result<T, SpectraBridgeError>) -> String {
    result.unwrap_err().to_string()
}
