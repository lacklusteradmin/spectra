use super::*;

#[test]
fn peercoin_networks_use_native_precision_and_protocol_rules() {
    for (chain, id, symbol, path, maturity) in [
        (Chain::Peercoin, "peercoin", "PPC", "m/44'/6'/0'/0/0", 500),
        (
            Chain::PeercoinTestnet,
            "peercoin-testnet",
            "tPPC",
            "m/44'/1'/0'/0/0",
            60,
        ),
    ] {
        assert_eq!(chain.str_id(), id);
        assert_eq!(chain.mainnet_counterpart(), Chain::Peercoin);
        assert_eq!(chain.native_decimals(), 6);
        assert_eq!(chain.coin_symbol(), symbol);
        assert_eq!(
            crate::derivation::path::default_path_from_catalog(chain).unwrap(),
            path
        );
        assert_eq!(chain.entry().artwork_name, "peercoin");
        assert_eq!(chain.generated_output_maturity().unwrap(), maturity);
        assert_eq!(chain.peercoin_min_output_units().unwrap(), 10_000);
        assert_eq!(chain.peercoin_min_fee_units().unwrap(), 1_000);
        assert_eq!(chain.peercoin_fee_per_kb_units().unwrap(), 10_000);
        assert_eq!(chain.peercoin_max_money().unwrap(), 21_000_000_000_000);
        assert!(chain.uses_utxo_client());
        assert!(chain.uses_account_utxo());
        assert!(chain.supports_watch_only_import());
        assert!(chain.derives_from_private_key());
        assert!(chain.has_send_preview());
        assert!(chain.send_execution_shape().fee_fallback.is_none());
        assert!(chain.static_fee_units().is_none());
        assert!(!chain.supports_staking());
        assert!(!chain.hosts_tokens());
        assert!(!chain.is_evm());
        assert_eq!(chain.endpoint_apis(), &[EndpointApi::Blockbook]);
    }
    assert_eq!(Chain::Peercoin.coingecko_id(), "peercoin");
    assert_eq!(Chain::PeercoinTestnet.coingecko_id(), "");
    assert!(Chain::Bitcoin.peercoin_min_fee_units().is_err());
}

/// Qtum, Fractal Bitcoin, eCash and DigiByte: each node's own parameters,
/// the account wallet they make, and the indexers they are read through.
#[test]
fn the_bitcoin_derived_networks_carry_their_nodes_parameters() {
    use crate::api::endpoint::EndpointApi as Api;
    for (chain, symbol, decimals, path, apis, dust, fee_floor) in [
        (
            Chain::Qtum,
            "QTUM",
            8,
            "m/44'/88'/0'/0/0",
            &[Api::Blockbook][..],
            72_800,
            Some(400),
        ),
        (
            Chain::FractalBitcoin,
            "FB",
            8,
            "m/84'/0'/0'/0/0",
            &[Api::Esplora, Api::UnisatOpenApi][..],
            546,
            Some(1),
        ),
        (
            Chain::Ecash,
            "XEC",
            2,
            "m/44'/1899'/0'/0/0",
            &[Api::Chronik][..],
            546,
            None,
        ),
        (
            Chain::Digibyte,
            "DGB",
            8,
            "m/84'/20'/0'/0/0",
            &[Api::Esplora, Api::Blockbook][..],
            5_460,
            Some(100),
        ),
    ] {
        assert_eq!(chain.coin_symbol(), symbol);
        assert_eq!(chain.native_decimals(), decimals);
        assert_eq!(
            crate::derivation::path::default_path_from_catalog(chain).unwrap(),
            path
        );
        assert_eq!(chain.endpoint_apis(), apis, "{chain}");
        assert_eq!(chain.relay_dust_threshold().unwrap(), dust, "{chain}");
        assert_eq!(chain.min_fee_rate(), fee_floor, "{chain}");
        assert_eq!(chain.wif_version(), Some(0x80), "{chain}");
        assert!(
            chain.uses_account_utxo() && chain.uses_utxo_client(),
            "{chain}"
        );
        assert!(chain.supports_watch_only_import() && chain.accepts_account_xpub());
        assert!(chain.uses_generic_send_submit() && chain.has_send_preview());
        assert!(chain.payment_memo_kinds().is_empty());
        assert!(!chain.is_testnet() && !chain.hosts_tokens() && !chain.supports_staking());
    }
    assert_eq!(Chain::Ecash.sighash_fork_id().unwrap(), 0);
    assert_eq!(Chain::Ecash.cashaddr_prefix(), Some("ecash"));
    assert!(Chain::Ecash.fixed_utxo_address_versions().is_err());
    assert_eq!(Chain::Ecash.static_fee_units(), Some(1_000));
    assert_eq!(Chain::Qtum.generated_output_maturity().unwrap(), 2_000);
    assert_eq!(Chain::Ecash.generated_output_maturity().unwrap(), 100);
    assert!(Chain::Qtum.verifies_spent_outputs() && !Chain::Ecash.verifies_spent_outputs());
    assert!(!Chain::Qtum.indexers_estimate_fee_rate());
    // Taproot: spent on Fractal and DigiByte, not on Qtum.
    assert!(Chain::FractalBitcoin.account_spends_witness(1));
    assert!(Chain::Digibyte.account_spends_witness(1));
    assert!(Chain::Qtum.account_spends_witness(0) && !Chain::Qtum.account_spends_witness(1));
    // Fractal and eCash share Bitcoin's genesis block; each is told apart
    // by a later one, and Bitcoin's own check is its block 1.
    let genesis = Chain::Bitcoin.genesis_block_hash().unwrap();
    for chain in [Chain::Bitcoin, Chain::FractalBitcoin, Chain::Ecash] {
        let (height, hash) = chain.identity_block().unwrap();
        assert!(height > 0 && hash != genesis, "{chain}");
    }
    assert_ne!(
        Chain::Bitcoin.identity_block().unwrap(),
        Chain::FractalBitcoin.identity_block().unwrap()
    );
    assert!(Chain::FractalBitcoin.verifies_indexer_before_signing());
    assert!(!Chain::Bitcoin.verifies_indexer_before_signing());
    assert_eq!(
        Chain::Qtum.identity_block().unwrap(),
        (
            0,
            "000075aef83cf2853580f8ae8ce6f8c3096cfa21d98334d6e3f95e5582ed986c".into()
        )
    );
    // No payment request names Fractal's coin: a `bitcoin:` code would be
    // paid in BTC.
    assert!(Chain::FractalBitcoin.payment_uri_format().is_none());
}

/// A chain with several APIs needs one client answering in all of them —
/// the UTXO family's — or APIs that serve different capabilities, so each
/// read has one: Hedera's mirror node reads and its address book's nodes
/// take transactions.
#[test]
fn only_the_utxo_client_speaks_several_apis() {
    for chain in Chain::all() {
        let apis = chain.endpoint_apis();
        if apis.len() > 1 && !chain.uses_utxo_client() {
            assert!(chain.is_hedera(), "{}", chain.str_id());
            for capability in crate::api::endpoint::EndpointCapability::ALL {
                let serving = apis
                    .iter()
                    .filter(|api| {
                        crate::api::endpoint::endpoint_capability_options(chain, **api)
                            .contains(&capability)
                    })
                    .count();
                assert!(serving <= 1, "{chain} {capability:?}");
            }
        }
    }
}

/// Every EVM chain carries its EIP-155 id, and each id is the one its
/// network signs for.
#[test]
fn evm_chains_carry_their_eip155_ids() {
    for chain in Chain::all().filter(|chain| chain.is_evm()) {
        assert!(
            chain.evm_chain_id().unwrap() > 0,
            "{} has no chain id",
            chain.str_id()
        );
    }
    assert_eq!(Chain::Ethereum.evm_chain_id().unwrap(), 1);
    assert_eq!(Chain::EthereumSepolia.evm_chain_id().unwrap(), 11_155_111);
    assert_eq!(Chain::EthereumHoodi.evm_chain_id().unwrap(), 560_048);
    assert_eq!(Chain::Arbitrum.evm_chain_id().unwrap(), 42161);
    assert_eq!(Chain::Optimism.evm_chain_id().unwrap(), 10);
    assert_eq!(Chain::Avalanche.evm_chain_id().unwrap(), 43114);
    assert_eq!(Chain::Base.evm_chain_id().unwrap(), 8453);
    assert_eq!(Chain::EthereumClassic.evm_chain_id().unwrap(), 61);
    assert_eq!(Chain::BnbChain.evm_chain_id().unwrap(), 56);
    assert_eq!(Chain::Hyperliquid.evm_chain_id().unwrap(), 999);
    assert_eq!(Chain::Polygon.evm_chain_id().unwrap(), 137);
    assert_eq!(Chain::Linea.evm_chain_id().unwrap(), 59144);
    assert_eq!(Chain::Scroll.evm_chain_id().unwrap(), 534352);
    assert_eq!(Chain::Blast.evm_chain_id().unwrap(), 81457);
    assert_eq!(Chain::Mantle.evm_chain_id().unwrap(), 5000);
    assert_eq!(Chain::Sei.evm_chain_id().unwrap(), 1329);
    assert_eq!(Chain::Celo.evm_chain_id().unwrap(), 42220);
    assert_eq!(Chain::Cronos.evm_chain_id().unwrap(), 25);
    assert_eq!(Chain::OpBnb.evm_chain_id().unwrap(), 204);
    assert_eq!(Chain::ZkSyncEra.evm_chain_id().unwrap(), 324);
    assert_eq!(Chain::Sonic.evm_chain_id().unwrap(), 146);
    assert_eq!(Chain::Berachain.evm_chain_id().unwrap(), 80094);
    assert_eq!(Chain::Unichain.evm_chain_id().unwrap(), 130);
    assert_eq!(Chain::Ink.evm_chain_id().unwrap(), 57073);
    assert_eq!(Chain::XLayer.evm_chain_id().unwrap(), 196);
    assert_eq!(Chain::Plasma.evm_chain_id().unwrap(), 9745);
    assert_eq!(Chain::Monad.evm_chain_id().unwrap(), 143);
    assert_eq!(Chain::WorldChain.evm_chain_id().unwrap(), 480);
    for (chain, id) in [
        (Chain::Kaia, 8217),
        (Chain::Xdc, 50),
        (Chain::RobinhoodChain, 4663),
        (Chain::KavaEvm, 2222),
        (Chain::Chiliz, 88888),
        (Chain::Stable, 988),
        (Chain::Arc, 5042),
        (Chain::MantaPacific, 169),
        (Chain::Metis, 1088),
        (Chain::Flare, 14),
        (Chain::Katana, 747474),
        (Chain::MegaEth, 4326),
        (Chain::Merlin, 4200),
        (Chain::Peaq, 3338),
        (Chain::Somnia, 5031),
        (Chain::ZeroG, 16661),
    ] {
        assert_eq!(chain.evm_chain_id().unwrap(), id, "{chain}");
        assert_eq!(Chain::from_evm_chain_id(id), Some(chain));
    }
}

#[test]
fn non_evm_chains_have_no_eip155_identity() {
    for chain in Chain::all().filter(|chain| !chain.is_evm()) {
        assert!(chain.evm_chain_id().is_err(), "{}", chain.str_id());
    }
}

#[test]
fn new_evm_mainnets_have_native_assets_and_shared_wallet_rules() {
    for (chain, id, chain_id, symbol, token_id, artwork) in [
        (Chain::Plasma, "plasma", 9745, "XPL", "plasma", "plasma"),
        (Chain::Monad, "monad", 143, "MON", "monad", "monad"),
        (
            Chain::WorldChain,
            "world-chain",
            480,
            "ETH",
            "ethereum",
            "worldcoin",
        ),
        (Chain::Kaia, "kaia", 8217, "KAIA", "kaia", "kaia"),
        (Chain::Xdc, "xdc", 50, "XDC", "xdce-crowd-sale", "xdc"),
        (
            Chain::RobinhoodChain,
            "robinhood-chain",
            4663,
            "ETH",
            "ethereum",
            "robinhood",
        ),
        (Chain::KavaEvm, "kava-evm", 2222, "KAVA", "kava", "kava"),
        (Chain::Chiliz, "chiliz", 88888, "CHZ", "chiliz", "chiliz"),
        (Chain::Stable, "stable", 988, "USDT0", "usdt0", "stable"),
        (Chain::Arc, "arc", 5042, "USDC", "usd-coin", "arc"),
        (
            Chain::MantaPacific,
            "manta-pacific",
            169,
            "ETH",
            "ethereum",
            "manta",
        ),
        (Chain::Metis, "metis", 1088, "METIS", "metis-token", "metis"),
        (Chain::Flare, "flare", 14, "FLR", "flare-networks", "flare"),
        (Chain::Katana, "katana", 747474, "ETH", "ethereum", "katana"),
        (
            Chain::MegaEth,
            "megaeth",
            4326,
            "ETH",
            "ethereum",
            "megaeth",
        ),
        (Chain::Merlin, "merlin", 4200, "BTC", "bitcoin", "merlin"),
        (Chain::Peaq, "peaq", 3338, "PEAQ", "peaq-2", "peaq"),
        (Chain::Somnia, "somnia", 5031, "SOMI", "somnia", "somnia"),
        (Chain::ZeroG, "0g", 16661, "0G", "zero-gravity", "0g"),
    ] {
        assert_eq!(Chain::parse(id).unwrap(), chain);
        assert!(!chain.is_testnet());
        assert_eq!(chain.mainnet_counterpart(), chain);
        assert_eq!(chain.evm_chain_id().unwrap(), chain_id);
        assert_eq!(chain.address_slot(), "ethereum");
        assert!(chain.derives_from_private_key());
        assert!(chain.supports_watch_only_import());
        assert!(chain.sends_tokens());
        assert!(chain.allows_token_standard(crate::registry::TokenStandard::Erc20));
        assert_eq!(chain.coin_symbol(), symbol);
        assert_eq!(chain.native_decimals(), 18);
        assert_eq!(chain.entry().artwork_name, artwork);
        assert_eq!(
            chain.entry().derivation_path[0].path,
            "m/44'/60'/{account}'/0/0"
        );
        let deployment = crate::catalog::tokens::deployment(&format!("{id}:native")).unwrap();
        assert!(deployment.is_native());
        assert_eq!(deployment.token_id, token_id);
        assert_eq!(deployment.chain_id, chain);
        assert_eq!(deployment.symbol, symbol);
        assert_eq!(deployment.decimals, 18);
        assert!(deployment.contract.is_empty());
        assert!(
            crate::catalog::endpoints::catalog()
                .records
                .iter()
                .any(|record| {
                    record.chain_id == chain
                        && record.api == EndpointApi::EvmJsonRpc
                        && record.capabilities.contains(&EndpointCapability::Balance)
                        && record.capabilities.contains(&EndpointCapability::Fee)
                        && record.capabilities.contains(&EndpointCapability::Broadcast)
                })
        );
    }
}

/// Kusama and Astar are Substrate networks like Polkadot: their identity
/// and coin as read from their own nodes, sr25519 keys along junctions,
/// raw keys and watch-only addresses, the shared transfer model and
/// finality, verified keyless endpoints, and no tokens or staking.
#[test]
fn kusama_and_astar_share_the_substrate_wallet_rules() {
    use crate::registry::SubstrateFeePayment;
    for (chain, id, symbol, decimals, prefix, genesis, fee_payment) in [
        (
            Chain::Kusama,
            "kusama",
            "KSM",
            12,
            2,
            "0x48239ef607d7928874027a43a67689209727dfb3d3dc5e5b03a39bdc2eda771a",
            SubstrateFeePayment::Asset,
        ),
        (
            Chain::Astar,
            "astar",
            "ASTR",
            18,
            5,
            "0x9eb76c5184c4ab8679d2d5d819fdf90b9c001403e9e17da2e14b6d8aec4029c6",
            SubstrateFeePayment::Native,
        ),
    ] {
        assert_eq!(Chain::parse(id).unwrap(), chain);
        assert!(!chain.is_testnet() && !chain.is_evm());
        assert!(chain.is_substrate());
        assert_eq!(chain.coin_symbol(), symbol);
        assert_eq!(chain.native_decimals(), decimals);
        assert_eq!(chain.ss58_prefix(), Some(prefix));
        assert_eq!(chain.substrate_genesis_hash(), Some(genesis));
        assert_eq!(chain.substrate_balance_bytes(), Some(16));
        assert_eq!(chain.substrate().unwrap().fee_payment, fee_payment);
        assert_eq!(chain.key_scheme(), Some(KeyScheme::Sr25519));
        assert!(chain.derives_along_junctions() && !chain.uses_derivation_path());
        assert!(!chain.supports_derivation_hmac_override());
        assert!(chain.supports_watch_only_import());
        assert!(chain.has_shared_send_preview() && chain.uses_generic_send_submit());
        assert_eq!(chain.endpoint_apis(), &[EndpointApi::SubstrateJsonRpc]);
        assert!(matches!(
            chain.pending_status_poll(),
            PendingStatusPoll::SubstrateFinality
        ));
        assert!(!chain.hosts_tokens() && !chain.supports_staking());
        assert!(chain.multisig_policy_format().is_none());
        assert!(chain.payment_memo_kinds().is_empty());
        let deployment = crate::catalog::tokens::deployment(&format!("{id}:native")).unwrap();
        assert_eq!(deployment.token_id, id);
        assert_eq!(deployment.decimals, u32::from(decimals));
        let endpoints = crate::catalog::endpoints::catalog()
            .records
            .iter()
            .filter(|record| record.chain_id == chain)
            .filter(|record| {
                [
                    EndpointCapability::Balance,
                    EndpointCapability::Fee,
                    EndpointCapability::Broadcast,
                    EndpointCapability::Verification,
                ]
                .iter()
                .all(|capability| record.capabilities.contains(capability))
            })
            .count();
        assert!(endpoints >= 2, "{chain}");
    }
}

#[test]
fn verified_new_chain_tokens_have_exact_deployment_identity() {
    for (chain, token_id, contract, decimals) in [
        (
            Chain::Plasma,
            "usd-coin",
            "0x2d661c89d812261039af9764eceaaee884f5f67f",
            6,
        ),
        (
            Chain::Plasma,
            "euro-coin",
            "0x3ee196e78d4d4248b849b8e1c7f44c5457fafd2c",
            6,
        ),
        (
            Chain::Monad,
            "pancakeswap-token",
            "0xf59d81cd43f620e722e07f9cb3f6e41b031017a3",
            18,
        ),
        (
            Chain::Monad,
            "usd-coin",
            "0x754704bc059f8c67012fed69bc8a327a5aafb603",
            6,
        ),
        (
            Chain::WorldChain,
            "usd-coin",
            "0x79a02482a880bce3f13e09da970dc34db4cd24d1",
            6,
        ),
        (
            Chain::WorldChain,
            "worldcoin-wld",
            "0x2cfc85d8e48f8eab294be644d9e25c3030863003",
            18,
        ),
        (
            Chain::Kaia,
            "tether",
            "0xd077a400968890eacc75cdc901f0356c943e4fdb",
            6,
        ),
        (
            Chain::Xdc,
            "usd-coin",
            "0xfa2958cb79b0491cc627c1557f441ef849ca8eb1",
            6,
        ),
        (
            Chain::RobinhoodChain,
            "global-dollar",
            "0x5fc5360d0400a0fd4f2af552add042d716f1d168",
            6,
        ),
        (
            Chain::KavaEvm,
            "tether",
            "0x919c1c267bc06a7039e03fcc2ef738525769109c",
            6,
        ),
        (
            Chain::Stable,
            "stable-2",
            "0x0000000000000000000000000000000000001003",
            18,
        ),
        (
            Chain::Stable,
            "tether-gold-tokens",
            "0xb8ce59fc3717ada4c02eadf9682a9e934f625ebb",
            6,
        ),
        (
            Chain::Arc,
            "euro-coin",
            "0xbef5f6d51cb62b58e6a8f77868681825c6fe21c1",
            6,
        ),
        (
            Chain::MantaPacific,
            "manta-network",
            "0x95cef13441be50d20ca4558cc0a27b601ac544e5",
            18,
        ),
        (
            Chain::Flare,
            "usdt0",
            "0xe7cd86e13ac4309349f30b3435a9d337750fc82d",
            6,
        ),
        (
            Chain::Katana,
            "katana-network-token",
            "0x7f1f4b4b29f5058fa32cc7a97141b8d7e5abdc2d",
            18,
        ),
        (
            Chain::MegaEth,
            "megaeth",
            "0x28b7e77f82b25b95953825f1e3ea0e36c1c29861",
            18,
        ),
        (
            Chain::MegaEth,
            "usdt0",
            "0xb8ce59fc3717ada4c02eadf9682a9e934f625ebb",
            6,
        ),
        (
            Chain::MegaEth,
            "ethena-usde",
            "0x5d3a1ff2b6bab83b63cd9ad0787074081a52ef34",
            18,
        ),
        (
            Chain::Merlin,
            "merlin-chain",
            "0x5c46bff4b38dc1eae09c5bac65872a1d8bc87378",
            18,
        ),
    ] {
        let id = crate::catalog::tokens::deployment_id_for(chain, Some(contract)).unwrap();
        let deployment = crate::catalog::tokens::deployment(&id).unwrap();
        assert_eq!(deployment.token_id, token_id);
        assert_eq!(deployment.chain_id, chain);
        assert_eq!(deployment.contract, contract);
        assert_eq!(deployment.decimals, decimals);
        assert_eq!(
            deployment.token_standard,
            Some(crate::registry::TokenStandard::Erc20)
        );
        assert!(!deployment.is_native());
    }
}

#[test]
fn new_mainnet_indexers_declare_their_verified_account_methods() {
    for (chain, endpoint) in [
        (
            Chain::Plasma,
            "https://api.routescan.io/v2/network/mainnet/evm/9745/etherscan",
        ),
        (
            Chain::WorldChain,
            "https://worldchain-mainnet.explorer.alchemy.com",
        ),
        (Chain::Chiliz, "https://scan-api.chiliz.com"),
        (
            Chain::MantaPacific,
            "https://pacific-explorer.manta.network",
        ),
        (Chain::Metis, "https://andromeda-explorer.metis.io"),
        (Chain::Flare, "https://flare-explorer.flare.network"),
        (Chain::MegaEth, "https://megaeth.blockscout.com"),
        (Chain::Merlin, "https://scan.merlinchain.io"),
        (Chain::Somnia, "https://explorer.somnia.network"),
    ] {
        assert_eq!(chain.evm_history_source(), EvmHistorySource::Open(endpoint));
        let record = crate::catalog::endpoints::catalog()
            .records
            .iter()
            .find(|record| record.chain_id == chain && record.endpoint == endpoint)
            .unwrap();
        assert_eq!(record.api, EndpointApi::Blockscout);
        for capability in [
            EndpointCapability::History,
            EndpointCapability::TokenHistory,
            EndpointCapability::TokenDiscovery,
        ] {
            assert!(record.capabilities.contains(&capability));
        }
    }
}

#[test]
fn rollup_fees_follow_deployed_oracle_methods() {
    for chain in [
        Chain::Optimism,
        Chain::OptimismSepolia,
        Chain::Base,
        Chain::BaseSepolia,
        Chain::Celo,
        Chain::Unichain,
        Chain::Ink,
        Chain::InkSepolia,
        Chain::WorldChain,
        Chain::MantaPacific,
        Chain::Katana,
        Chain::MegaEth,
    ] {
        assert_eq!(
            chain.evm_rollup_fee_model(),
            Some(OpStackFeeModel::FjordWithOperator),
            "{chain}"
        );
    }
    assert_eq!(
        Chain::CeloSepolia.evm_rollup_fee_model(),
        Some(OpStackFeeModel::Fjord)
    );
    assert_eq!(
        Chain::OpBnb.evm_rollup_fee_model(),
        Some(OpStackFeeModel::Fjord)
    );
    for chain in [Chain::Blast, Chain::Metis] {
        assert_eq!(chain.evm_rollup_fee_model(), Some(OpStackFeeModel::Bedrock));
    }
    // Arbitrum Orbit folds the L1 cost into gas, as Arbitrum does.
    assert_eq!(Chain::RobinhoodChain.evm_rollup_fee_model(), None);
    assert_eq!(Chain::Plasma.evm_rollup_fee_model(), None);
    assert_eq!(Chain::Monad.evm_rollup_fee_model(), None);
    assert_eq!(Chain::Monad.evm_gas_buffer_bps(), 750);
    assert_eq!(Chain::WorldChain.evm_gas_buffer_bps(), 2000);
}

/// Three networks' nodes decode no typed envelope; two need a tip the fee
/// history's lowest sample does not show; five give their coin an ERC-20
/// face over the same balance.
#[test]
fn per_network_evm_transaction_rules() {
    let legacy: Vec<_> = Chain::all()
        .filter(|c| c.evm_legacy_transactions())
        .collect();
    assert_eq!(legacy, [Chain::KavaEvm, Chain::Metis, Chain::Merlin]);
    let floors: Vec<_> = Chain::all()
        .filter(|c| c.evm_min_priority_fee_wei() > 0)
        .map(|c| (c, c.evm_min_priority_fee_wei()))
        .collect();
    assert_eq!(
        floors,
        [
            (Chain::Chiliz, 1_000_000_000),
            (Chain::ZeroG, 4_000_000_000)
        ]
    );
    for chain in Chain::all().filter(|c| c.evm_native_coin_contract().is_some()) {
        assert!(chain.is_evm(), "{chain}");
        let contract = chain.evm_native_coin_contract().unwrap();
        assert!(
            crate::catalog::tokens::list_token_deployments(Some(chain))
                .iter()
                .all(|t| !t.contract.eq_ignore_ascii_case(contract)),
            "{chain} catalogues its coin's ERC-20 face as a token"
        );
    }
    assert_eq!(
        Chain::all()
            .filter(|c| c.evm_native_coin_contract().is_some())
            .collect::<Vec<_>>(),
        [
            Chain::Celo,
            Chain::Stable,
            Chain::Arc,
            Chain::Metis,
            Chain::Peaq
        ]
    );
}

/// Address slots: every chain has one, and the EVM family shares Ethereum's.
#[test]
fn address_slots_are_shared_across_the_evm_family_only() {
    for chain in Chain::all() {
        let slot = chain.address_slot();
        assert!(!slot.is_empty(), "{} has no slot", chain.str_id());
        if chain.is_evm() {
            assert_eq!(
                slot,
                Chain::Ethereum.str_id(),
                "{} should share the ethereum slot",
                chain.str_id()
            );
        } else {
            assert_eq!(
                slot,
                chain.str_id(),
                "{} should own its slot",
                chain.str_id()
            );
        }
    }
}

/// Both networks can watch their validated addresses. Monero additionally
/// requires a private view key, so an address is insufficient on either.
#[test]
fn watch_only_support_excludes_only_the_monero_family() {
    // Every account network reads account keys, each version spelling
    // its prefix and a test network's no mainnet's.
    for chain in Chain::all() {
        assert_eq!(
            chain.accepts_account_xpub(),
            chain.uses_account_utxo(),
            "{chain}"
        );
        for version in chain.account_key_versions() {
            let encoded = crate::derivation::bitcoin::base58check_encode(
                &[version.version.as_slice(), &[0u8; 74]].concat(),
            );
            assert!(
                encoded.starts_with(version.prefix),
                "{chain} {}",
                version.prefix
            );
            if chain.is_testnet() {
                assert!(
                    !chain
                        .mainnet_counterpart()
                        .account_key_versions()
                        .contains(version),
                    "{chain} {}",
                    version.prefix
                );
            }
        }
    }
    for chain in Chain::all() {
        assert!(
            chain.supports_watch_only_import() == (chain.mainnet_counterpart() != Chain::Monero),
            "{} has inconsistent watch support",
            chain.str_id()
        );
    }
    let excluded: Vec<&str> = Chain::all()
        .filter(|c| !c.is_testnet() && !c.supports_watch_only_import())
        .map(|c| c.str_id())
        .collect();
    assert_eq!(excluded, vec!["monero"]);
}

/// Hosting follows each network’s protocol set, with no default protocol.
#[test]
fn token_hosting_follows_the_token_protocol_set() {
    let hosting: Vec<&str> = Chain::all()
        .filter(|c| c.hosts_tokens())
        .map(Chain::str_id)
        .collect();
    let with_standard: Vec<&str> = crate::catalog::chains::catalog()
        .iter()
        .filter(|c| !c.token_standards.is_empty())
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(hosting, with_standard);
    for chain in Chain::all().filter(|c| c.is_testnet()) {
        assert_eq!(
            chain.token_standards(),
            chain.mainnet_counterpart().token_standards(),
            "{}",
            chain.str_id()
        );
    }
    assert!(!Chain::Bitcoin.hosts_tokens() && !Chain::Monero.hosts_tokens());
    for chain in Chain::all().filter(|c| c.hosts_tokens()) {
        for standard in chain.token_standards() {
            assert!(chain.allows_token_standard(*standard), "{}", chain.str_id());
        }
    }
}

#[test]
fn protocol_sets_keep_legacy_and_current_protocols_on_one_network() {
    for chain in [Chain::Tron, Chain::TronNile] {
        assert_eq!(
            chain.token_standards(),
            [
                crate::registry::TokenStandard::Trc10,
                crate::registry::TokenStandard::Trc20
            ]
        );
        let entries = &chain.entry().token_standards;
        let prompts: Vec<(&str, &str)> = entries
            .iter()
            .map(|s| (s.standard.as_str(), s.identifier_prompt.as_str()))
            .collect();
        assert_eq!(
            prompts,
            [("TRC-10", "Token ID"), ("TRC-20", "Contract Address")]
        );
    }
    for chain in [Chain::Aptos, Chain::AptosTestnet] {
        assert_eq!(
            chain.token_standards(),
            [
                crate::registry::TokenStandard::AptosCoin,
                crate::registry::TokenStandard::Aip21
            ]
        );
        assert_eq!(
            chain.token_standard_for_identifier("0x1::coin::T"),
            Some(crate::registry::TokenStandard::AptosCoin)
        );
        assert_eq!(
            chain.token_standard_for_identifier("0x123"),
            Some(crate::registry::TokenStandard::Aip21)
        );
    }
}

/// VeChainThor's fees are VTHO, a VIP-180 deployment counted at the
/// network's own 18 places; every other network pays in its own coin.
#[test]
fn vechain_pays_its_fees_in_vtho() {
    let vtho = Chain::Vechain.fee_holding_template();
    let energy = Chain::Vechain.thor().unwrap().energy;
    assert_eq!(vtho.symbol, "VTHO");
    assert_eq!(Chain::Vechain.entry().fee_symbol, "VTHO");
    assert_eq!(vtho.token_standard, Some(TokenStandard::Vip180));
    assert_eq!(vtho.contract_address.as_deref(), Some(energy));
    let deployment = crate::catalog::tokens::deployment(&vtho.deployment_id()).unwrap();
    assert_eq!(
        deployment.decimals,
        u32::from(Chain::Vechain.native_decimals())
    );
    assert!(!Chain::Vechain.pays_own_fee(None));
    assert!(Chain::Vechain.pays_own_fee(Some("0x0000000000000000000000000000456E65726779")));
    assert!(!Chain::Vechain.pays_own_fee(Some("0x5ef79995fe8a89e0812330e4378eb2660cede699")));
    for chain in Chain::all().filter(|chain| !chain.is_vechain()) {
        assert!(chain.pays_own_fee(None), "{chain}");
        assert!(!chain.pays_own_fee(Some(energy)), "{chain}");
        assert_eq!(
            chain.fee_holding_template(),
            chain.native_holding_template(),
            "{chain}"
        );
        assert_eq!(chain.entry().fee_symbol, chain.coin_symbol(), "{chain}");
    }
    // Ethereum's addresses, but its own network and slot.
    assert!(!Chain::Vechain.is_evm());
    assert_eq!(Chain::Vechain.address_slot(), "vechain");
    assert_eq!(Chain::Vechain.coin_symbol(), "VET");
    assert_eq!(Chain::Vechain.endpoint_apis(), &[EndpointApi::ThorRest]);
}

#[test]
fn str_id_roundtrips() {
    for chain in Chain::all() {
        let id = chain.str_id();
        let back = Chain::from_str_id(id).expect("str_id must round-trip");
        assert_eq!(chain, back, "round-trip failed for {id}");
    }
    assert!(Chain::from_str_id("not-a-chain").is_none());
}

#[test]
fn testnet_mainnet_counterparts_are_mainnets() {
    for testnet in Chain::all().filter(|c| c.is_testnet()) {
        let counterpart = testnet.mainnet_counterpart();
        assert!(
            !counterpart.is_testnet(),
            "{:?} mainnet_counterpart returned testnet {:?}",
            testnet,
            counterpart
        );
    }
}

mod catalog_agreement_tests {
    use super::*;

    /// The genesis hashes an indexer is checked against are the networks'
    /// published ones.
    #[test]
    fn genesis_hashes_are_the_networks() {
        for (chain, hash) in [
            (
                Chain::Bitcoin,
                "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f",
            ),
            (
                Chain::BitcoinTestnet,
                "000000000933ea01ad0ee984209779baaec3ced90fa3f408719526f8d77f4943",
            ),
            (
                Chain::BitcoinSignet,
                "00000008819873e925422c1ff0f99f7cc9bbb232af63a077a480a3633bee1ef6",
            ),
            (
                Chain::Litecoin,
                "12a765e31ffd4059bada1e25190f6e98c99d9714d334efa41a195a7e7e04bfe2",
            ),
        ] {
            assert_eq!(chain.genesis_block_hash().unwrap(), hash, "{chain}");
        }
        assert!(Chain::Dogecoin.genesis_block_hash().is_err());
    }

    /// A chain id spelled from the variant's own name, so the check below has
    /// a source independent of the catalog it is checking.
    ///
    /// The exceptions are the whole list of places where the enum and
    /// `chains.toml` spell a chain differently, which is worth being able to
    /// read in one place.
    fn expected_id(chain: Chain) -> String {
        const EXCEPTIONS: &[(Chain, &str)] = &[
            (Chain::MegaEth, "megaeth"),
            (Chain::ZeroG, "0g"),
            (Chain::Icp, "internet-computer"),
            (Chain::BnbChain, "bnb"),
            (Chain::OpBnb, "opbnb"),
            (Chain::ZkSyncEra, "zksync-era"),
            (Chain::ZkSyncEraSepolia, "zksync-era-sepolia"),
            (Chain::BitcoinTestnet4, "bitcoin-testnet-4"),
            (Chain::BnbChainTestnet, "bnb-testnet"),
        ];
        if let Some((_, id)) = EXCEPTIONS.iter().find(|(c, _)| *c == chain) {
            return (*id).to_string();
        }
        let name = format!("{chain:?}");
        let mut out = String::with_capacity(name.len() + 4);
        let bytes: Vec<char> = name.chars().collect();
        for (i, ch) in bytes.iter().enumerate() {
            let starts_word = ch.is_uppercase()
                && i > 0
                && (bytes[i - 1].is_lowercase()
                    || bytes[i - 1].is_ascii_digit()
                    || bytes.get(i + 1).is_some_and(|n| n.is_lowercase()));
            if starts_word {
                out.push('-');
            }
            out.extend(ch.to_lowercase());
        }
        out
    }

    /// The enum is an index into `chains.toml`, so the two orders must match
    /// exactly — position by position, not merely as sets.
    ///
    /// Asserting `chain.str_id() == entry.id` would prove nothing: `str_id`
    /// *reads* the catalog, so the two agree by construction. The variant
    /// name is the independent source, which is why [`expected_id`] exists.
    /// If the index is not sound, every chain silently becomes a different
    /// chain, and unlike a rename that is invisible from the outside.
    #[test]
    fn chain_order_matches_the_catalog() {
        let catalog = crate::catalog::chains::list_all_chains();
        assert_eq!(
            ALL_CHAINS.len(),
            catalog.len(),
            "{} chains in the enum, {} in chains.toml",
            ALL_CHAINS.len(),
            catalog.len()
        );
        for (index, entry) in catalog.iter().enumerate() {
            let chain = ALL_CHAINS[index];
            assert_eq!(
                chain as usize, index,
                "ALL_CHAINS[{index}] is {chain:?}, whose discriminant is {}",
                chain as usize
            );
            assert_eq!(
                expected_id(chain),
                entry.id,
                "position {index}: the enum has {chain:?}, chains.toml has \"{}\"",
                entry.id
            );
        }
    }

    /// Every catalog entry is reachable from the name it publishes, with no
    /// special case in the resolver.
    #[test]
    fn every_catalog_name_resolves() {
        for entry in crate::catalog::chains::list_all_chains() {
            assert_eq!(
                Chain::from_display_name(&entry.name).map(Chain::str_id),
                Some(entry.id.as_str()),
                "{} does not resolve from \"{}\"",
                entry.id,
                entry.name
            );
        }
    }
}

mod the_post_send_refresh_set_is_the_registrys {
    use super::{Chain, PendingStatusPoll};

    /// After a send, a chain either polls for a pending status or refreshes
    /// history. Which it does is `pending_status_poll`, and a testnet does what
    /// its mainnet does.
    #[test]
    fn every_utxo_testnet_polls_the_way_its_mainnet_does() {
        let utxo = |c: Chain| matches!(c.pending_status_poll(), PendingStatusPoll::Utxo { .. });
        assert!(utxo(Chain::Bitcoin));
        for chain in Chain::all() {
            assert_eq!(
                utxo(chain),
                utxo(chain.mainnet_counterpart()),
                "{chain:?} must poll like its mainnet"
            );
        }
    }
}

mod utxo_discovery_is_the_registrys_chain_set {
    use crate::registry::Chain;

    /// Every UTXO testnet is in the discovery set its mainnet is in.
    #[test]
    fn the_testnets_are_in_the_set_their_mainnets_are_in() {
        assert!(Chain::Bitcoin.uses_account_utxo());
        for chain in Chain::all() {
            assert_eq!(
                chain.uses_account_utxo(),
                chain.mainnet_counterpart().uses_account_utxo(),
                "{chain:?} walks the same addresses its mainnet does"
            );
        }
    }
}

mod history_family {
    use crate::registry::Chain;
    use crate::registry::HistoryFamily as S;

    fn strategy(chain_id: crate::registry::Chain) -> S {
        chain_id.history_family()
    }

    #[test]
    fn history_protocols_select_their_merge_semantics() {
        for name in [
            crate::registry::Chain::Bitcoin,
            crate::registry::Chain::BitcoinCash,
            crate::registry::Chain::BitcoinSV,
            crate::registry::Chain::Litecoin,
            crate::registry::Chain::Dogecoin,
            crate::registry::Chain::Dash,
        ] {
            assert_eq!(strategy(name), S::Utxo, "{name}");
        }
        for name in [
            crate::registry::Chain::Tron,
            crate::registry::Chain::Solana,
            crate::registry::Chain::Cardano,
            crate::registry::Chain::Xrp,
            crate::registry::Chain::Stellar,
            crate::registry::Chain::Monero,
            crate::registry::Chain::Sui,
            crate::registry::Chain::Aptos,
            crate::registry::Chain::Ton,
            crate::registry::Chain::Icp,
            crate::registry::Chain::Near,
            crate::registry::Chain::Polkadot,
        ] {
            assert_eq!(strategy(name), S::Account, "{name}");
        }
        for name in [
            crate::registry::Chain::Ethereum,
            crate::registry::Chain::Arbitrum,
            crate::registry::Chain::Base,
            crate::registry::Chain::Polygon,
        ] {
            assert_eq!(strategy(name), S::Evm, "{name}");
        }
    }

    #[test]
    fn a_testnet_merges_the_same_way_as_its_mainnet() {
        for chain in Chain::all() {
            let mainnet = chain.mainnet_counterpart();
            if mainnet == chain {
                continue;
            }
            assert_eq!(
                chain.history_family(),
                mainnet.history_family(),
                "{} diverges from {}",
                chain.str_id(),
                mainnet.str_id()
            );
        }
    }
}

mod zcash_schedule {
    use super::*;

    /// The branch at each upgrade's first block, NU6.3 (Ironwood) included,
    /// and nothing to build for before NU5.
    #[test]
    fn branches_follow_librustzcash() {
        for (chain, height, branch) in [
            (Chain::Zcash, 1_687_104, 0xc2d6_d0b4),
            (Chain::Zcash, 3_364_600, 0x5437_f330),
            (Chain::Zcash, 3_428_142, 0x5437_f330),
            (Chain::Zcash, 3_428_143, 0x37a5_165b),
            (Chain::ZcashTestnet, 4_134_000, 0x37a5_165b),
        ] {
            assert_eq!(
                chain.zcash_consensus_branch(height),
                Ok(branch),
                "{chain} {height}"
            );
        }
        assert_eq!(
            Chain::Zcash.zcash_consensus_branch(1_687_103),
            Err(RegistryError::ZcashV5Inactive(1_687_103))
        );
        assert!(Chain::Bitcoin.zcash_consensus_branch(1).is_err());
    }
}
