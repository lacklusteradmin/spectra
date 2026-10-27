//! Which APIs serve a network, and how its history and status are read.

use super::*;

impl Chain {
    /// The APIs the chain's own clients speak: what its balance, fee and
    /// broadcast reads use. Every URL speaking any of them is asked at once,
    /// whichever API it speaks; indexers and secondary services are asked for
    /// by API.
    ///
    /// More than one API is only possible where one client answers in all of
    /// them — the UTXO family, through `api::utxo`.
    pub fn endpoint_apis(self) -> &'static [EndpointApi] {
        use EndpointApi as Api;
        match self.mainnet_counterpart() {
            c if c.is_evm() => &[Api::EvmJsonRpc],
            Chain::Bitcoin => &[Api::Esplora, Api::Blockcypher],
            Chain::Litecoin => &[Api::Blockbook, Api::Esplora, Api::Blockcypher],
            Chain::Dogecoin => &[Api::Blockcypher, Api::Blockbook],
            Chain::Dash => &[Api::Blockbook, Api::Blockcypher],
            Chain::BitcoinCash => &[Api::Blockbook, Api::BchRestV2],
            // Zcash's transparent builder asks Blockbook for its consensus
            // branch, which no other indexer reports.
            Chain::BitcoinGold | Chain::Zcash | Chain::Peercoin => &[Api::Blockbook],
            Chain::BitcoinSV => &[Api::Whatsonchain],
            Chain::Solana => &[Api::SolanaJsonRpc],
            Chain::Tron => &[Api::TronHttp],
            Chain::Stellar => &[Api::Horizon],
            Chain::Xrp => &[Api::XrplJsonRpc],
            Chain::Cardano => &[Api::Koios],
            Chain::Polkadot | Chain::Bittensor => &[Api::SubstrateJsonRpc],
            Chain::Sui => &[Api::SuiJsonRpc],
            Chain::Aptos => &[Api::AptosRest],
            Chain::Ton => &[Api::ToncenterV2],
            Chain::Near => &[Api::NearJsonRpc],
            Chain::Icp => &[Api::IcpRosetta],
            Chain::Monero => &[Api::MoneroDaemonRpc],
            Chain::Decred => &[Api::Insight],
            Chain::Kaspa => &[Api::KaspaRest],
            _ => &[],
        }
    }

    /// Primary transports and implemented auxiliary indexers this network accepts.
    pub fn compatible_endpoint_apis(self) -> Vec<EndpointApi> {
        use EndpointApi as Api;
        let auxiliary: &[Api] = match self.mainnet_counterpart() {
            chain if chain.is_evm() => &[Api::Blockscout],
            Chain::Aptos => &[Api::AptosIndexer],
            Chain::Icp => &[Api::IcpReplica],
            Chain::Near => &[Api::Nearblocks, Api::Fastnear],
            Chain::Tron => &[Api::TrongridV1],
            Chain::Ton => &[Api::ToncenterV3],
            // Shielded funds are scanned from a lightwalletd server.
            Chain::Zcash => &[Api::Lightwalletd],
            // MWEB funds are scanned from a Litecoin node, peer to peer.
            Chain::Litecoin => &[Api::LitecoinP2p],
            _ => &[],
        };
        self.endpoint_apis()
            .iter()
            .chain(auxiliary)
            .copied()
            .collect()
    }

    /// How a configured URL the endpoint directory does not know is read.
    /// It picks nothing about which URLs are used.
    pub fn default_api(self) -> Option<EndpointApi> {
        self.endpoint_apis().first().copied()
    }

    /// The chain's reads go through `api::utxo::UtxoClient`, which answers
    /// the same way in every UTXO indexer API.
    pub fn uses_utxo_client(self) -> bool {
        let apis = self.endpoint_apis();
        !apis.is_empty() && apis.iter().all(|api| api.is_utxo_indexer())
    }

    /// Whether a wallet's coins can be listed address by address with their
    /// confirmations: an account on a UTXO indexer that reports both.
    /// Decred's Insight and Kaspa's REST API do not count confirmations
    /// against a tip, so their accounts list no coins.
    pub fn lists_account_coins(self) -> bool {
        self.uses_account_utxo() && self.uses_utxo_client()
    }

    /// The keyless explorer source for this EVM chain, if configured.
    pub fn evm_history_source(self) -> EvmHistorySource<'static> {
        crate::catalog::endpoints::catalog()
            .records
            .iter()
            .find(|record| {
                record.chain_id == self
                    && record.api == EndpointApi::Blockscout
                    && record.capabilities.contains(&EndpointCapability::History)
            })
            .map(|record| EvmHistorySource::Open(record.endpoint.as_str()))
            .unwrap_or(EvmHistorySource::Unavailable)
    }

    /// How this chain's pending transactions reach a final status.
    ///
    /// Resolved through the mainnet counterpart so a testnet cannot diverge.
    pub fn pending_status_poll(self) -> PendingStatusPoll {
        let chain = self.mainnet_counterpart();
        match chain {
            // Litecoin tracks receives too: its explorer confirms them on a
            // different cadence than the send path assumes.
            Chain::Litecoin => PendingStatusPoll::Utxo {
                require_send_kind: false,
            },
            Chain::Bitcoin
            | Chain::BitcoinCash
            | Chain::BitcoinSV
            | Chain::Dogecoin
            | Chain::Zcash
            | Chain::BitcoinGold
            | Chain::Decred
            | Chain::Kaspa
            | Chain::Dash
            | Chain::Peercoin => PendingStatusPoll::Utxo {
                require_send_kind: true,
            },
            Chain::Ton => PendingStatusPoll::TransactionStatus(EndpointApi::ToncenterV3),
            Chain::Tron
            | Chain::Solana
            | Chain::Cardano
            | Chain::Xrp
            | Chain::Stellar
            | Chain::Monero
            | Chain::Sui
            | Chain::Aptos
            | Chain::Icp
            | Chain::Near => PendingStatusPoll::TransactionStatus(
                chain.default_api().expect("Account transaction status API"),
            ),
            Chain::Polkadot | Chain::Bittensor => PendingStatusPoll::SubstrateFinality,
            other if other.is_evm() => PendingStatusPoll::EvmReceipt,
            _ => PendingStatusPoll::None,
        }
    }

    /// How incoming history merges with stored history.
    /// Exhaustive so each new chain must specify its merge rule.
    pub fn transaction_merge_strategy(
        self,
    ) -> crate::store::transaction_merge::TransactionMergeStrategy {
        use crate::store::transaction_merge::TransactionMergeStrategy as S;
        // Resolved through the mainnet counterpart so a testnet can never merge
        // differently from the chain it mirrors — listing them separately is
        // how `zcash-testnet` silently ended up account-based.
        match self.mainnet_counterpart() {
            Chain::Bitcoin
            | Chain::Dogecoin
            | Chain::BitcoinCash
            | Chain::BitcoinSV
            | Chain::Litecoin
            | Chain::BitcoinGold
            | Chain::Dash
            | Chain::Decred
            | Chain::Zcash
            | Chain::Kaspa
            | Chain::Peercoin => S::Utxo,
            other if other.is_evm() => S::Evm,
            _ => S::AccountBased,
        }
    }

    /// Whether a wallet's balance and history come from scanning blocks on
    /// the device through a daemon, rather than from a provider's index.
    pub fn scans_for_balance(self) -> bool {
        self.mainnet_counterpart() == Self::Monero
    }
    pub(crate) fn history_refresh_kind(self) -> HistoryRefreshKind {
        if self.is_evm() {
            HistoryRefreshKind::Evm
        } else if self.uses_account_utxo() {
            HistoryRefreshKind::Utxo
        } else {
            HistoryRefreshKind::Normalized
        }
    }
}
