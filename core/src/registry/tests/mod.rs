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
        assert_eq!(
            chain.peercoin_generated_output_maturity().unwrap(),
            maturity
        );
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

/// A chain with several APIs needs one client answering in all of them,
/// and only the UTXO family has one.
#[test]
fn only_the_utxo_client_speaks_several_apis() {
    for chain in Chain::all() {
        if chain.endpoint_apis().len() > 1 {
            assert!(chain.uses_utxo_client(), "{}", chain.str_id());
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
    assert_eq!(
        Chain::Blast.evm_rollup_fee_model(),
        Some(OpStackFeeModel::Bedrock)
    );
    assert_eq!(Chain::Plasma.evm_rollup_fee_model(), None);
    assert_eq!(Chain::Monad.evm_rollup_fee_model(), None);
    assert_eq!(Chain::Monad.evm_gas_buffer_bps(), 750);
    assert_eq!(Chain::WorldChain.evm_gas_buffer_bps(), 2000);
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

#[test]
fn testnet_derivation_metadata_names_the_concrete_network() {
    for chain in Chain::all().filter(|c| c.is_testnet()) {
        assert_eq!(
            crate::send::flow::seed_derivation_chain_raw(chain).as_deref(),
            Some(chain.chain_display_name())
        );
    }
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
    /// The six exceptions are the whole list of places where the enum and
    /// `chains.toml` spell a chain differently, which is worth being able to
    /// read in one place.
    fn expected_id(chain: Chain) -> String {
        const EXCEPTIONS: &[(Chain, &str)] = &[
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
