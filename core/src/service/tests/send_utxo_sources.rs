use super::*;

#[test]
fn source_paths_obey_the_selected_network_catalog() {
    for (chain, coin) in [
        (Chain::Litecoin, 2),
        (Chain::LitecoinTestnet, 1),
        (Chain::Peercoin, 6),
        (Chain::PeercoinTestnet, 1),
    ] {
        for purpose in [44, 49, 84, 86] {
            if purpose == 86 && chain.mainnet_counterpart() != Chain::Peercoin {
                assert!(
                    account_utxo_account_path(chain, &format!("m/86'/{coin}'/3'/0/4")).is_err()
                );
                continue;
            }
            for branch in [0, 1] {
                assert!(
                    account_utxo_account_path(
                        chain,
                        &format!("m/{purpose}'/{coin}'/3'/{branch}/4")
                    )
                    .is_ok()
                );
            }
        }
        for path in [
            "m/84'/0'/3'/0/4".to_string(),
            format!("m/84'/{coin}'/3'/0'/4"),
            format!("m/84'/{coin}'/3'/0/4'"),
            format!("m/84'/{coin}'/3'/2/4"),
            format!("m/84'/{coin}'/3/0/4"),
        ] {
            assert!(
                account_utxo_account_path(chain, &path).is_err(),
                "{chain:?} {path}"
            );
        }
    }
}

#[test]
fn account_sources_allow_peercoin_taproot_and_refuse_unsupported_litecoin_scripts() {
    const SEED: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    for (chain, coin) in [(Chain::Peercoin, 6), (Chain::PeercoinTestnet, 1)] {
        let path = format!("m/86'/{coin}'/3'/0/4");
        let context =
            crate::service::address_discovery::UtxoDerivation::new(chain, SEED, path.clone())
                .unwrap();
        let root = context.derive(4).unwrap().0;
        let (address, source_path) = context.derive_on_branch(1, 7).unwrap();
        let source = account_utxo_source(chain, &root, Some(&path), address, source_path).unwrap();
        assert_eq!(source.script_pubkey.len(), 34);
        assert_eq!(&source.script_pubkey[..2], &[0x51, 0x20]);
    }
    for chain in [Chain::Litecoin, Chain::LitecoinTestnet] {
        let address = bech32::segwit::encode_v1(
            bech32::Hrp::parse(chain.fixed_utxo_segwit_hrp().unwrap()).unwrap(),
            &[1; 32],
        )
        .unwrap();
        assert!(account_utxo_source(chain, &address, None, address.clone(), None).is_err());
    }
}
