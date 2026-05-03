use ed25519_dalek::SigningKey;
    use crate::derivation::chains::cardano::derive_cardano_icarus_xprv_root;
    use crate::derivation::chains::monero::monero_base58_encode;
    use crate::derivation::chains::ton::{crc16_xmodem, v4r2_code_hash_and_depth};
    use crate::derivation::engine::{
        derive, parse_uniffi_request, AddressAlgorithm, Chain, CurveFamily, DerivationAlgorithm,
        ParsedRequest, PublicKeyFormat, ScriptType, UniFFIDerivationRequest,
    };
    use crate::derivation::engine::*;

    fn base_request(chain: Chain, curve: CurveFamily) -> ParsedRequest {
        // Per-chain presets — selecting the derivation algorithm / address
        // algorithm / default path from the same table that production
        // callers would hit via `chain_defaults_from_name`.
        let (derivation_algorithm, address_algorithm, derivation_path) = match chain {
            Chain::Near => (
                DerivationAlgorithm::DirectSeedEd25519,
                AddressAlgorithm::NearHex,
                "m",
            ),
            Chain::Ton => (
                DerivationAlgorithm::TonMnemonic,
                AddressAlgorithm::TonRawAccountId,
                "m",
            ),
            Chain::Cardano => (
                DerivationAlgorithm::Bip32Ed25519Icarus,
                AddressAlgorithm::CardanoShelleyEnterprise,
                "m/1852'/1815'/0'/0/0",
            ),
            Chain::Polkadot => (
                DerivationAlgorithm::SubstrateBip39,
                AddressAlgorithm::Ss58,
                "m",
            ),
            Chain::Monero => (
                DerivationAlgorithm::MoneroBip39,
                AddressAlgorithm::MoneroMain,
                "m",
            ),
            _ => match curve {
                CurveFamily::Secp256k1 => (
                    DerivationAlgorithm::Bip32Secp256k1,
                    AddressAlgorithm::Bitcoin,
                    "m/44'/0'/0'/0/0",
                ),
                CurveFamily::Ed25519 => (
                    DerivationAlgorithm::Slip10Ed25519,
                    AddressAlgorithm::Solana,
                    "m/44'/501'/0'/0'",
                ),
                CurveFamily::Sr25519 => (
                    DerivationAlgorithm::SubstrateBip39,
                    AddressAlgorithm::Ss58,
                    "m",
                ),
            },
        };
        let address_algorithm = match curve {
            CurveFamily::Secp256k1 if !matches!(chain, Chain::Ethereum | Chain::EthereumClassic
                | Chain::Arbitrum | Chain::Optimism | Chain::Avalanche | Chain::Hyperliquid | Chain::Tron) => address_algorithm,
            CurveFamily::Secp256k1 => AddressAlgorithm::Evm,
            CurveFamily::Ed25519 => address_algorithm,
            CurveFamily::Sr25519 => address_algorithm,
        };
        let public_key_format = match curve {
            CurveFamily::Secp256k1 => {
                if matches!(chain, Chain::Ethereum | Chain::EthereumClassic
                    | Chain::Arbitrum | Chain::Optimism | Chain::Avalanche
                    | Chain::Hyperliquid | Chain::Tron)
                {
                    PublicKeyFormat::Uncompressed
                } else {
                    PublicKeyFormat::Compressed
                }
            }
            CurveFamily::Ed25519 | CurveFamily::Sr25519 => PublicKeyFormat::Raw,
        };
        let script_type = if matches!(chain, Chain::Bitcoin) {
            ScriptType::P2wpkh
        } else {
            ScriptType::Account
        };

        ParsedRequest {
            chain,
            curve,
            requested_outputs: OUTPUT_ADDRESS | OUTPUT_PUBLIC_KEY | OUTPUT_PRIVATE_KEY,
            derivation_algorithm,
            address_algorithm,
            public_key_format,
            script_type,
            seed_phrase: "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".to_string(),
            derivation_path: Some(derivation_path.to_string()),
            passphrase: String::new(),
            hmac_key: None,
            mnemonic_wordlist: Some("english".to_string()),
            iteration_count: 0,
            salt_prefix: None,
        }
    }

    #[test]
    fn derives_all_supported_chains() {
        let chains = [
            (Chain::Bitcoin, CurveFamily::Secp256k1),
            (Chain::BitcoinCash, CurveFamily::Secp256k1),
            (Chain::BitcoinSv, CurveFamily::Secp256k1),
            (Chain::Litecoin, CurveFamily::Secp256k1),
            (Chain::Dogecoin, CurveFamily::Secp256k1),
            (Chain::Ethereum, CurveFamily::Secp256k1),
            (Chain::EthereumClassic, CurveFamily::Secp256k1),
            (Chain::Arbitrum, CurveFamily::Secp256k1),
            (Chain::Optimism, CurveFamily::Secp256k1),
            (Chain::Avalanche, CurveFamily::Secp256k1),
            (Chain::Hyperliquid, CurveFamily::Secp256k1),
            (Chain::Tron, CurveFamily::Secp256k1),
            (Chain::Xrp, CurveFamily::Secp256k1),
            (Chain::Solana, CurveFamily::Ed25519),
            (Chain::Stellar, CurveFamily::Ed25519),
            (Chain::Cardano, CurveFamily::Ed25519),
            (Chain::Sui, CurveFamily::Ed25519),
            (Chain::Aptos, CurveFamily::Ed25519),
            (Chain::Ton, CurveFamily::Ed25519),
            (Chain::InternetComputer, CurveFamily::Ed25519),
            (Chain::Near, CurveFamily::Ed25519),
            (Chain::Polkadot, CurveFamily::Sr25519),
            (Chain::Monero, CurveFamily::Ed25519),
        ];

        for (chain, curve) in chains {
            let result = derive(base_request(chain, curve))
                .unwrap_or_else(|e| panic!("failed to derive {}: {e}", chain_name(chain)));
            assert!(
                result
                    .address
                    .as_deref()
                    .is_some_and(|v| !v.trim().is_empty()),
                "missing address for {}",
                chain_name(chain)
            );
            assert!(
                result
                    .public_key_hex
                    .as_deref()
                    .is_some_and(|v| !v.trim().is_empty()),
                "missing public key for {}",
                chain_name(chain)
            );
            assert!(
                result
                    .private_key_hex
                    .as_deref()
                    .is_some_and(|v| !v.trim().is_empty()),
                "missing private key for {}",
                chain_name(chain)
            );
        }
    }

    #[test]
    fn custom_hmac_key_changes_secp_derivation() {
        let baseline = derive(base_request(Chain::Bitcoin, CurveFamily::Secp256k1))
            .expect("baseline secp derivation");
        let mut customized = base_request(Chain::Bitcoin, CurveFamily::Secp256k1);
        customized.hmac_key = Some("Nostr seed".to_string());
        let tweaked = derive(customized).expect("customized secp derivation");
        assert_ne!(baseline.private_key_hex, tweaked.private_key_hex);
        assert_ne!(baseline.address, tweaked.address);
    }

    #[test]
    fn custom_hmac_key_changes_slip10_derivation() {
        let baseline = derive(base_request(Chain::Solana, CurveFamily::Ed25519))
            .expect("baseline slip10 derivation");
        let mut customized = base_request(Chain::Solana, CurveFamily::Ed25519);
        customized.hmac_key = Some("custom ed25519 seed".to_string());
        let tweaked = derive(customized).expect("customized slip10 derivation");
        assert_ne!(baseline.private_key_hex, tweaked.private_key_hex);
        assert_ne!(baseline.address, tweaked.address);
    }

    #[test]
    fn custom_salt_prefix_changes_seed() {
        let baseline = derive(base_request(Chain::Bitcoin, CurveFamily::Secp256k1))
            .expect("baseline seed derivation");
        let mut customized = base_request(Chain::Bitcoin, CurveFamily::Secp256k1);
        customized.salt_prefix = Some("electrum".to_string());
        let tweaked = derive(customized).expect("customized seed derivation");
        assert_ne!(baseline.private_key_hex, tweaked.private_key_hex);
    }

    #[test]
    fn custom_iteration_count_changes_seed() {
        let baseline = derive(base_request(Chain::Bitcoin, CurveFamily::Secp256k1))
            .expect("baseline iteration derivation");
        let mut customized = base_request(Chain::Bitcoin, CurveFamily::Secp256k1);
        customized.iteration_count = 4096;
        let tweaked = derive(customized).expect("customized iteration derivation");
        assert_ne!(baseline.private_key_hex, tweaked.private_key_hex);
    }

    #[test]
    fn default_hmac_key_matches_standard_seed() {
        // Explicitly passing "Bitcoin seed" / "ed25519 seed" must match the
        // None default — the old API explicitly rejected them, so regressions
        // here would break any caller still sending the canonical constants.
        let standard = derive(base_request(Chain::Bitcoin, CurveFamily::Secp256k1))
            .expect("standard secp");
        let mut explicit = base_request(Chain::Bitcoin, CurveFamily::Secp256k1);
        explicit.hmac_key = Some("Bitcoin seed".to_string());
        let explicit_derived = derive(explicit).expect("explicit secp");
        assert_eq!(standard.private_key_hex, explicit_derived.private_key_hex);

        let standard_ed = derive(base_request(Chain::Solana, CurveFamily::Ed25519))
            .expect("standard slip10");
        let mut explicit_ed = base_request(Chain::Solana, CurveFamily::Ed25519);
        explicit_ed.hmac_key = Some("ed25519 seed".to_string());
        let explicit_ed_derived = derive(explicit_ed).expect("explicit slip10");
        assert_eq!(
            standard_ed.private_key_hex,
            explicit_ed_derived.private_key_hex
        );
    }

    #[test]
    fn unknown_wordlist_is_rejected() {
        let mut request = base_request(Chain::Bitcoin, CurveFamily::Secp256k1);
        request.mnemonic_wordlist = Some("klingon".to_string());
        let error = match derive(request) {
            Err(err) => err,
            Ok(_) => panic!("klingon wordlist should not resolve"),
        };
        assert!(error.to_lowercase().contains("wordlist"), "got: {error}");
    }

    #[test]
    fn near_direct_seed_vector() {
        // NEAR uses the MyNearWallet / near-seed-phrase convention:
        // priv = BIP-39 PBKDF2 seed[0..32]. For "abandon abandon … about"
        // with EMPTY passphrase, the 64-byte seed begins with the publicly
        // documented constant 5eb00bbd… (Trezor's c55257… constant is for
        // passphrase="TREZOR" — a different vector).
        let request = base_request(Chain::Near, CurveFamily::Ed25519);
        let result = derive(request).expect("near direct-seed derive");
        let priv_hex = result.private_key_hex.expect("near priv");
        let pub_hex = result.public_key_hex.expect("near pub");
        let address = result.address.expect("near address");

        assert_eq!(
            priv_hex,
            "5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc1"
        );

        // Public key must be the ed25519 derivative of this private scalar.
        let priv_bytes = hex::decode(&priv_hex).expect("decode priv");
        let mut priv_arr = [0u8; 32];
        priv_arr.copy_from_slice(&priv_bytes);
        let expected_pub = hex::encode(
            SigningKey::from_bytes(&priv_arr)
                .verifying_key()
                .to_bytes(),
        );
        assert_eq!(pub_hex, expected_pub);
        // NEAR implicit account id = hex(public_key).
        assert_eq!(address, expected_pub);
    }

    #[test]
    fn near_direct_seed_ignores_path() {
        // DirectSeedEd25519 does no BIP-32 walk — changing the derivation_path
        // must not change the private key.
        let mut a = base_request(Chain::Near, CurveFamily::Ed25519);
        a.derivation_path = Some("m/44'/397'/0'".to_string());
        let mut b = base_request(Chain::Near, CurveFamily::Ed25519);
        b.derivation_path = Some("m/999'/888/0".to_string());
        let a_priv = derive(a).unwrap().private_key_hex;
        let b_priv = derive(b).unwrap().private_key_hex;
        assert_eq!(a_priv, b_priv);
    }

    #[test]
    fn ton_mnemonic_structure() {
        // TON mnemonic scheme: entropy = HMAC-SHA512(mnemonic, passphrase);
        // seed = PBKDF2(entropy, "TON default seed", 100_000, 64); priv = seed[0..32].
        // Cross-wallet vectors aren't pinned yet — this asserts structural
        // invariants + keypair consistency as a regression floor.
        let request = base_request(Chain::Ton, CurveFamily::Ed25519);
        let result = derive(request).expect("ton derive");
        let priv_hex = result.private_key_hex.expect("ton priv");
        let pub_hex = result.public_key_hex.expect("ton pub");
        let address = result.address.expect("ton address");

        assert_eq!(priv_hex.len(), 64);
        let priv_bytes = hex::decode(&priv_hex).expect("decode priv");
        let mut priv_arr = [0u8; 32];
        priv_arr.copy_from_slice(&priv_bytes);
        let expected_pub = hex::encode(
            SigningKey::from_bytes(&priv_arr)
                .verifying_key()
                .to_bytes(),
        );
        assert_eq!(pub_hex, expected_pub);
        // Raw account id format: "0:<64 hex>".
        assert!(address.starts_with("0:"));
        assert_eq!(address.len(), 2 + 64);
        assert_eq!(&address[2..], expected_pub);
    }

    #[test]
    fn ton_mnemonic_diverges_from_slip10() {
        // Sanity check that the new TonMnemonic algorithm is actually
        // different from the old SLIP-0010 pipeline it replaced.
        let mut ton_req = base_request(Chain::Ton, CurveFamily::Ed25519);
        let ton_priv = derive(ton_req.clone_for_test())
            .expect("ton priv")
            .private_key_hex;
        ton_req.derivation_algorithm = DerivationAlgorithm::Slip10Ed25519;
        ton_req.derivation_path = Some("m/44'/607'/0'".to_string());
        let slip_priv = derive(ton_req).expect("slip10 priv").private_key_hex;
        assert_ne!(ton_priv, slip_priv);
    }

    #[test]
    fn ton_mnemonic_honors_iteration_count() {
        let baseline = derive(base_request(Chain::Ton, CurveFamily::Ed25519))
            .expect("baseline ton")
            .private_key_hex;
        let mut tweaked = base_request(Chain::Ton, CurveFamily::Ed25519);
        tweaked.iteration_count = 50_000;
        let tweaked_priv = derive(tweaked).expect("tweaked ton").private_key_hex;
        assert_ne!(baseline, tweaked_priv);
    }

    #[test]
    fn cardano_icarus_enterprise_address_structure() {
        // CIP-1852 / CIP-3 Icarus + CIP-19 Shelley enterprise address:
        // header byte is 0x61 (type 6 = enterprise + payment-key hash, network 1 = mainnet).
        // Bech32 HRP = "addr"; payload is 29 bytes (1 header + 28 Blake2b-224).
        let request = base_request(Chain::Cardano, CurveFamily::Ed25519);
        let result = derive(request).expect("cardano derive");
        let priv_hex = result.private_key_hex.expect("cardano priv");
        let pub_hex = result.public_key_hex.expect("cardano pub");
        let address = result.address.expect("cardano address");

        assert_eq!(priv_hex.len(), 64);
        assert_eq!(pub_hex.len(), 64);
        assert!(address.starts_with("addr1"), "address: {address}");
        let (hrp, data) =
            bech32::decode(&address).expect("bech32 decode must succeed");
        assert_eq!(hrp.as_str(), "addr");
        assert_eq!(data.len(), 29);
        assert_eq!(data[0], 0x61);
    }

    #[test]
    fn cardano_icarus_diverges_from_slip10() {
        // Icarus BIP-32-Ed25519 with Khovratovich-Law clamping produces a
        // different scalar than SLIP-0010 ed25519 for the same path.
        let icarus = derive(base_request(Chain::Cardano, CurveFamily::Ed25519))
            .expect("icarus")
            .private_key_hex;
        let mut slip = base_request(Chain::Cardano, CurveFamily::Ed25519);
        slip.derivation_algorithm = DerivationAlgorithm::Slip10Ed25519;
        slip.derivation_path = Some("m/1852'/1815'/0'/0'/0'".to_string());
        let slip_priv = derive(slip).expect("slip").private_key_hex;
        assert_ne!(icarus, slip_priv);
    }

    #[test]
    fn cardano_icarus_passphrase_changes_root() {
        let baseline = derive(base_request(Chain::Cardano, CurveFamily::Ed25519))
            .expect("baseline cardano")
            .private_key_hex;
        let mut tweaked = base_request(Chain::Cardano, CurveFamily::Ed25519);
        tweaked.passphrase = "TREZOR".to_string();
        let tweaked_priv = derive(tweaked).expect("tweaked cardano").private_key_hex;
        assert_ne!(baseline, tweaked_priv);
    }

    #[test]
    fn cardano_icarus_root_is_clamped() {
        // Khovratovich-Law clamping on the root xprv: kL[31] top 3 bits are
        // cleared then bit 6 is set — so byte 31 & 0b1110_0000 == 0b0100_0000.
        // (Child derivation can carry through byte 31, so we only check the
        // root here.)
        let root = derive_cardano_icarus_xprv_root(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            "",
            Some("english"),
            0,
        )
        .expect("icarus root");
        assert_eq!(root[0] & 0b0000_0111, 0, "kL[0] low 3 bits must be clear");
        assert_eq!(
            root[31] & 0b1110_0000,
            0b0100_0000,
            "kL[31] top 3 bits must be 010"
        );
    }

    #[test]
    fn polkadot_substrate_address_structure() {
        // SS58 Polkadot mainnet (network prefix 0): 1-byte prefix + 32-byte
        // pubkey + 2-byte Blake2b-512("SS58PRE"||…) checksum, base58-encoded.
        // The 35-byte payload base58s to a string starting with '1' for prefix 0.
        let request = base_request(Chain::Polkadot, CurveFamily::Sr25519);
        let result = derive(request).expect("polkadot derive");
        let priv_hex = result.private_key_hex.expect("polkadot mini-secret");
        let pub_hex = result.public_key_hex.expect("polkadot pub");
        let address = result.address.expect("polkadot address");

        assert_eq!(priv_hex.len(), 64, "mini-secret = 32 bytes");
        assert_eq!(pub_hex.len(), 64, "sr25519 pub = 32 bytes");
        assert_ne!(priv_hex, pub_hex, "mini-secret must differ from pub");
        assert!(address.starts_with('1'), "Polkadot mainnet starts with '1', got {address}");
        // Polkadot SS58 addresses are 47–48 base58 chars for the 35-byte payload.
        assert!(
            (47..=48).contains(&address.len()),
            "polkadot address length out of range: {} ({address})",
            address.len()
        );
    }

    #[test]
    fn polkadot_substrate_passphrase_changes_mini_secret() {
        let baseline = derive(base_request(Chain::Polkadot, CurveFamily::Sr25519))
            .expect("baseline polkadot")
            .private_key_hex;
        let mut tweaked = base_request(Chain::Polkadot, CurveFamily::Sr25519);
        tweaked.passphrase = "TREZOR".to_string();
        let tweaked_priv = derive(tweaked).expect("tweaked polkadot").private_key_hex;
        assert_ne!(baseline, tweaked_priv);
    }

    #[test]
    fn polkadot_substrate_diverges_from_bip39_seed_prefix() {
        // substrate-bip39 uses the BIP-39 *entropy* (16 bytes for 12 words)
        // as the PBKDF2 password — NOT the mnemonic string. So the resulting
        // mini-secret must differ from the first 32 bytes of the standard
        // BIP-39 PBKDF2 seed (which uses the mnemonic string as password).
        let result = derive(base_request(Chain::Polkadot, CurveFamily::Sr25519))
            .expect("polkadot");
        let mini_secret = result.private_key_hex.expect("mini-secret");
        let bip39_seed_prefix =
            "5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc1";
        assert_ne!(mini_secret, bip39_seed_prefix);
    }

    #[test]
    fn polkadot_path_with_junction_is_rejected() {
        let mut request = base_request(Chain::Polkadot, CurveFamily::Sr25519);
        request.derivation_path = Some("//Alice".to_string());
        let error = match derive(request) {
            Err(err) => err,
            Ok(_) => panic!("junction path must be rejected"),
        };
        assert!(
            error.to_lowercase().contains("junction"),
            "error should mention junctions, got: {error}"
        );
    }

    #[test]
    fn polkadot_ss58_round_trip() {
        // Re-decode the SS58 address and verify the embedded pubkey + checksum.
        use blake2::digest::consts::U64;
        use blake2::digest::Digest;
        use blake2::Blake2b;
        type Blake2b512 = Blake2b<U64>;

        let result = derive(base_request(Chain::Polkadot, CurveFamily::Sr25519))
            .expect("polkadot");
        let address = result.address.expect("polkadot address");
        let pub_hex = result.public_key_hex.expect("polkadot pub");

        let decoded = bs58::decode(&address).into_vec().expect("base58 decode");
        // 1-byte prefix + 32-byte pubkey + 2-byte checksum = 35 bytes.
        assert_eq!(decoded.len(), 35);
        assert_eq!(decoded[0], 0x00, "Polkadot mainnet prefix is 0");
        assert_eq!(hex::encode(&decoded[1..33]), pub_hex);

        let mut hasher = Blake2b512::new();
        hasher.update(b"SS58PRE");
        hasher.update(&decoded[..33]);
        let checksum = hasher.finalize();
        assert_eq!(&decoded[33..35], &checksum[..2]);
    }

    #[test]
    fn monero_address_structure() {
        // Monero mainnet standard address: 0x12 || spend(32) || view(32) ||
        // keccak256(prev)[..4] = 69 bytes → 95 chars chunked Base58.
        // Network byte 0x12 forces the first character to be '4'.
        let request = base_request(Chain::Monero, CurveFamily::Ed25519);
        let result = derive(request).expect("monero derive");
        let priv_hex = result.private_key_hex.expect("monero priv");
        let pub_hex = result.public_key_hex.expect("monero pub");
        let address = result.address.expect("monero address");

        assert_eq!(priv_hex.len(), 64, "spend key = 32 bytes hex");
        assert_eq!(pub_hex.len(), 128, "spend+view pubs = 64 bytes hex");
        assert_eq!(address.len(), 95, "Monero mainnet address is 95 chars");
        assert!(address.starts_with('4'), "Monero mainnet starts with '4', got {address}");
    }

    #[test]
    fn monero_keys_match_reduced_bip39_seed_prefix() {
        // The BIP-39 seed prefix for "abandon … about" with empty passphrase
        // is 5eb0…fc1; that 32-byte LE integer exceeds the curve25519 group
        // order ℓ, so sc_reduce32 reduces it to the value below. Pinning the
        // exact reduction guards against regressions in the scalar-mod path.
        use curve25519_dalek::scalar::Scalar as DalekScalar;
        let bip39_prefix =
            hex::decode("5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc1")
                .expect("decode bip39 prefix");
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&bip39_prefix);
        let expected = hex::encode(DalekScalar::from_bytes_mod_order(bytes).to_bytes());

        let result = derive(base_request(Chain::Monero, CurveFamily::Ed25519))
            .expect("monero");
        let priv_hex = result.private_key_hex.expect("priv");
        assert_eq!(priv_hex, expected);
    }

    #[test]
    fn monero_view_key_is_keccak_of_spend() {
        // Reconstruct view = sc_reduce32(Keccak256(spend)) and check that
        // public_view inside the address payload matches.
        use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
        use curve25519_dalek::scalar::Scalar as DalekScalar;

        let result = derive(base_request(Chain::Monero, CurveFamily::Ed25519))
            .expect("monero");
        let pub_hex = result.public_key_hex.expect("pub");
        let priv_bytes =
            hex::decode(result.private_key_hex.expect("priv")).expect("decode priv");
        let mut spend = [0u8; 32];
        spend.copy_from_slice(&priv_bytes);

        use sha3::{Digest, Keccak256};
        let spend_hash: [u8; 32] = Keccak256::digest(&spend).into();
        let view_scalar = DalekScalar::from_bytes_mod_order(spend_hash);
        let public_view = (view_scalar * ED25519_BASEPOINT_POINT).compress().to_bytes();

        assert_eq!(&pub_hex[64..128], hex::encode(public_view));
    }

    #[test]
    fn monero_passphrase_changes_keys() {
        let baseline = derive(base_request(Chain::Monero, CurveFamily::Ed25519))
            .expect("baseline monero")
            .private_key_hex;
        let mut tweaked = base_request(Chain::Monero, CurveFamily::Ed25519);
        tweaked.passphrase = "TREZOR".to_string();
        let tweaked_priv = derive(tweaked).expect("tweaked monero").private_key_hex;
        assert_ne!(baseline, tweaked_priv);
    }

    #[test]
    fn monero_base58_encodes_known_length_pattern() {
        // Sanity check the chunked Base58: a 69-byte input must produce 95
        // characters (8 full blocks * 11 chars + 1 5-byte trailing block * 7).
        let payload = [0u8; 69];
        let encoded = monero_base58_encode(&payload);
        assert_eq!(encoded.len(), 95);
        // All-zero input encodes as all '1's (the alphabet's index-0 char).
        assert!(encoded.chars().all(|c| c == '1'));
    }

    impl ParsedRequest {
        fn clone_for_test(&self) -> ParsedRequest {
            ParsedRequest {
                chain: self.chain,
                curve: self.curve,
                requested_outputs: self.requested_outputs,
                derivation_algorithm: self.derivation_algorithm,
                address_algorithm: self.address_algorithm,
                public_key_format: self.public_key_format,
                script_type: self.script_type,
                seed_phrase: self.seed_phrase.clone(),
                derivation_path: self.derivation_path.clone(),
                passphrase: self.passphrase.clone(),
                hmac_key: self.hmac_key.clone(),
                mnemonic_wordlist: self.mnemonic_wordlist.clone(),
                iteration_count: self.iteration_count,
                salt_prefix: self.salt_prefix.clone(),
            }
        }
    }

    fn chain_name(chain: Chain) -> &'static str {
        match chain {
            Chain::Bitcoin => "bitcoin",
            Chain::BitcoinCash => "bitcoin_cash",
            Chain::BitcoinSv => "bitcoin_sv",
            Chain::Litecoin => "litecoin",
            Chain::Dogecoin => "dogecoin",
            Chain::Ethereum => "ethereum",
            Chain::EthereumClassic => "ethereum_classic",
            Chain::Arbitrum => "arbitrum",
            Chain::Optimism => "optimism",
            Chain::Avalanche => "avalanche",
            Chain::Hyperliquid => "hyperliquid",
            Chain::Tron => "tron",
            Chain::Solana => "solana",
            Chain::Stellar => "stellar",
            Chain::Xrp => "xrp",
            Chain::Cardano => "cardano",
            Chain::Sui => "sui",
            Chain::Aptos => "aptos",
            Chain::Ton => "ton",
            Chain::InternetComputer => "internet_computer",
            Chain::Near => "near",
            Chain::Polkadot => "polkadot",
            Chain::Monero => "monero",
            Chain::Zcash => "zcash",
            Chain::BitcoinGold => "bitcoin_gold",
            Chain::Decred => "decred",
            Chain::Kaspa => "kaspa",
            Chain::Dash => "dash",
            Chain::Bittensor => "bittensor",
            Chain::BitcoinTestnet => "bitcoin_testnet",
            Chain::BitcoinTestnet4 => "bitcoin_testnet4",
            Chain::BitcoinSignet => "bitcoin_signet",
            Chain::LitecoinTestnet => "litecoin_testnet",
            Chain::BitcoinCashTestnet => "bitcoin_cash_testnet",
            Chain::BitcoinSvTestnet => "bitcoin_sv_testnet",
            Chain::DogecoinTestnet => "dogecoin_testnet",
            Chain::ZcashTestnet => "zcash_testnet",
            Chain::DecredTestnet => "decred_testnet",
            Chain::KaspaTestnet => "kaspa_testnet",
            Chain::DashTestnet => "dash_testnet",
            Chain::EthereumSepolia => "ethereum_sepolia",
            Chain::EthereumHoodi => "ethereum_hoodi",
            Chain::ArbitrumSepolia => "arbitrum_sepolia",
            Chain::OptimismSepolia => "optimism_sepolia",
            Chain::BaseSepolia => "base_sepolia",
            Chain::BnbTestnet => "bnb_testnet",
            Chain::AvalancheFuji => "avalanche_fuji",
            Chain::PolygonAmoy => "polygon_amoy",
            Chain::HyperliquidTestnet => "hyperliquid_testnet",
            Chain::EthereumClassicMordor => "ethereum_classic_mordor",
            Chain::TronNile => "tron_nile",
            Chain::SolanaDevnet => "solana_devnet",
            Chain::XrpTestnet => "xrp_testnet",
            Chain::StellarTestnet => "stellar_testnet",
            Chain::CardanoPreprod => "cardano_preprod",
            Chain::SuiTestnet => "sui_testnet",
            Chain::AptosTestnet => "aptos_testnet",
            Chain::TonTestnet => "ton_testnet",
            Chain::NearTestnet => "near_testnet",
            Chain::PolkadotWestend => "polkadot_westend",
            Chain::MoneroStagenet => "monero_stagenet",
        }
    }

    #[test]
    fn ton_v4r2_code_hash_matches_known_constant() {
        // Ensures the embedded BOC + our parser produce the canonical v4R2
        // root hash. If either changes, every v4R2 address we emit is wrong,
        // so this is the trip-wire.
        let (hash, _depth) = v4r2_code_hash_and_depth().expect("v4r2 code hash");
        assert_eq!(
            hex::encode(hash),
            "feb5ff6820e2ff0d9483e7e0d62c817d846789fb4ae580c878866d959dabd5c0"
        );
    }

    #[test]
    fn ton_v4r2_address_structure_and_determinism() {
        // User-friendly base64url addresses are exactly 48 chars (36 bytes
        // → 48 chars under base64url-no-pad). v4R2 bounceable mainnet
        // addresses all start with "EQ" because tag=0x11, workchain=0x00
        // decodes to base64 `EQ`.
        let mut request = base_request(Chain::Ton, CurveFamily::Ed25519);
        request.address_algorithm = AddressAlgorithm::TonV4R2;
        let a = derive(request.clone_for_test()).expect("ton v4r2").address.expect("address");
        let b = derive(request).expect("ton v4r2 again").address.expect("address");
        assert_eq!(a, b, "v4r2 address must be deterministic");
        assert_eq!(a.len(), 48, "v4r2 address must be 48 chars: {a}");
        assert!(a.starts_with("EQ"), "v4r2 bounceable-mainnet prefix: {a}");
        // Must not contain '+' or '/' (base64url-specific).
        assert!(!a.contains('+'));
        assert!(!a.contains('/'));
    }

    #[test]
    fn ton_v4r2_diverges_from_raw_account_id() {
        let mut raw = base_request(Chain::Ton, CurveFamily::Ed25519);
        raw.address_algorithm = AddressAlgorithm::TonRawAccountId;
        let raw_addr = derive(raw).expect("raw").address.expect("raw address");
        let mut v4 = base_request(Chain::Ton, CurveFamily::Ed25519);
        v4.address_algorithm = AddressAlgorithm::TonV4R2;
        let v4_addr = derive(v4).expect("v4r2").address.expect("v4r2 address");
        assert_ne!(raw_addr, v4_addr);
    }

    #[test]
    fn ton_v4r2_changes_with_mnemonic() {
        let mut a = base_request(Chain::Ton, CurveFamily::Ed25519);
        a.address_algorithm = AddressAlgorithm::TonV4R2;
        let addr_a = derive(a).expect("a").address.expect("a addr");
        let mut b = base_request(Chain::Ton, CurveFamily::Ed25519);
        b.address_algorithm = AddressAlgorithm::TonV4R2;
        b.seed_phrase = "legal winner thank year wave sausage worth useful legal winner thank yellow".to_string();
        let addr_b = derive(b).expect("b").address.expect("b addr");
        assert_ne!(addr_a, addr_b);
    }

    #[test]
    fn crc16_xmodem_known_vector() {
        // Standard CRC-16/XMODEM test vector: "123456789" → 0x31C3.
        assert_eq!(crc16_xmodem(b"123456789"), 0x31C3);
    }

    #[test]
    fn chain_inference_covers_every_per_chain_variant() {
        // Each per-chain AddressAlgorithm variant must resolve to the
        // matching Chain. This is what lets callers omit `chain` from
        // the request and still reach the right derivation pipeline.
        let cases: &[(AddressAlgorithm, Chain)] = &[
            (AddressAlgorithm::Bitcoin, Chain::Bitcoin),
            (AddressAlgorithm::Litecoin, Chain::Litecoin),
            (AddressAlgorithm::Dogecoin, Chain::Dogecoin),
            (AddressAlgorithm::BitcoinCashLegacy, Chain::BitcoinCash),
            (AddressAlgorithm::BitcoinSvLegacy, Chain::BitcoinSv),
            (AddressAlgorithm::Evm, Chain::Ethereum),
            (AddressAlgorithm::TronBase58Check, Chain::Tron),
            (AddressAlgorithm::XrpBase58Check, Chain::Xrp),
            (AddressAlgorithm::Solana, Chain::Solana),
            (AddressAlgorithm::StellarStrKey, Chain::Stellar),
            (AddressAlgorithm::SuiKeccak, Chain::Sui),
            (AddressAlgorithm::AptosKeccak, Chain::Aptos),
            (AddressAlgorithm::IcpPrincipal, Chain::InternetComputer),
            (AddressAlgorithm::NearHex, Chain::Near),
            (AddressAlgorithm::TonRawAccountId, Chain::Ton),
            (AddressAlgorithm::TonV4R2, Chain::Ton),
            (AddressAlgorithm::CardanoShelleyEnterprise, Chain::Cardano),
            (AddressAlgorithm::Ss58, Chain::Polkadot),
            (AddressAlgorithm::MoneroMain, Chain::Monero),
        ];
        for (alg, expected_chain) in cases {
            let inferred = match chain_from_address_algorithm(*alg) {
                Ok(c) => c,
                Err(e) => panic!("chain_from_address_algorithm failed: {e}"),
            };
            assert_eq!(
                std::mem::discriminant(&inferred),
                std::mem::discriminant(expected_chain),
                "chain inference mismatch for address algorithm"
            );
        }
    }

    #[test]
    fn chain_inference_auto_is_rejected() {
        let err = match chain_from_address_algorithm(AddressAlgorithm::Auto) {
            Err(e) => e,
            Ok(_) => panic!("Auto should not infer a chain"),
        };
        assert!(err.contains("explicit"));
    }

    #[test]
    fn parse_uniffi_request_infers_chain_when_omitted() {
        // Construct a minimal request with chain=None and verify it
        // reaches the same ParsedRequest shape as an explicit chain.
        let req_json_no_chain = serde_json::json!({
            "curve": CURVE_ED25519,
            "requestedOutputs": OUTPUT_ADDRESS | OUTPUT_PUBLIC_KEY,
            "derivationAlgorithm": DERIVATION_SLIP10_ED25519,
            "addressAlgorithm": ADDRESS_SOLANA,
            "publicKeyFormat": PUBLIC_KEY_RAW,
            "scriptType": SCRIPT_ACCOUNT,
            "seedPhrase": "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            "derivationPath": "m/44'/501'/0'/0'",
            "iterationCount": 0,
        });
        let req: UniFFIDerivationRequest = serde_json::from_value(req_json_no_chain)
            .expect("parse JSON");
        assert!(req.chain.is_none());
        let parsed = parse_uniffi_request(req).expect("parse");
        assert!(matches!(parsed.chain, Chain::Solana));
    }

    #[test]
    fn explicit_chain_override_beats_inference() {
        // When the caller sends `chain` AND an AddressAlgorithm that
        // would infer a different chain, the explicit value wins — this
        // preserves back-compat for callers that haven't migrated.
        let req_json = serde_json::json!({
            "chain": CHAIN_TRON, // explicit Tron
            "curve": CURVE_SECP256K1,
            "requestedOutputs": OUTPUT_ADDRESS,
            "derivationAlgorithm": DERIVATION_BIP32_SECP256K1,
            "addressAlgorithm": ADDRESS_EVM, // would infer Ethereum
            "publicKeyFormat": PUBLIC_KEY_UNCOMPRESSED,
            "scriptType": SCRIPT_ACCOUNT,
            "seedPhrase": "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            "derivationPath": "m/44'/195'/0'/0/0",
            "iterationCount": 0,
        });
        let req: UniFFIDerivationRequest = serde_json::from_value(req_json).expect("parse JSON");
        let parsed = parse_uniffi_request(req).expect("parse");
        assert!(matches!(parsed.chain, Chain::Tron));
    }

    // ──────────────────────────────────────────────────────────────────
    //  canonical external golden vectors.
    //
    // Each entry pins our derivation output against an address emitted
    // by a reference wallet / official spec for the same mnemonic +
    // path. If our code ever drifts from the rest of the ecosystem,
    // one of these asserts fails loudly before a caller can send
    // funds to a wrong address.
    // ──────────────────────────────────────────────────────────────────

    struct GoldenVector {
        label: &'static str,
        source: &'static str,
        mnemonic: &'static str,
        path: &'static str,
        chain: Chain,
        curve: CurveFamily,
        derivation_algorithm: DerivationAlgorithm,
        address_algorithm: AddressAlgorithm,
        public_key_format: PublicKeyFormat,
        script_type: ScriptType,
        expected: &'static str,
        case_insensitive: bool,
    }

    fn run_golden_vector(v: &GoldenVector) -> Result<String, String> {
        let request = ParsedRequest {
            chain: v.chain,
            curve: v.curve,
            requested_outputs: OUTPUT_ADDRESS,
            derivation_algorithm: v.derivation_algorithm,
            address_algorithm: v.address_algorithm,
            public_key_format: v.public_key_format,
            script_type: v.script_type,
            seed_phrase: v.mnemonic.to_string(),
            derivation_path: Some(v.path.to_string()),
            passphrase: String::new(),
            hmac_key: None,
            mnemonic_wordlist: Some("english".to_string()),
            iteration_count: 0,
            salt_prefix: None,
        };
        let out = derive(request)?;
        out.address.ok_or_else(|| "no address returned".to_string())
    }

    fn canonical_golden_vectors() -> Vec<GoldenVector> {
        // Three reference mnemonics appear below:
        //   - ABANDON: BIP-39 test vector; used by BIP-84 / BIP-86.
        //   - ALL_ALL: Trezor firmware canonical test seed; used by the
        //     trezor-firmware fixtures for Bitcoin/Ethereum/Tron/Solana
        //     /Cardano/XRP/Litecoin.
        //   - SEP_0005: Stellar SEP-0005 §Test 1 mnemonic.
        const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        const ALL_ALL: &str = "all all all all all all all all all all all all";
        const SEP_0005: &str = "illness spike retreat truth genius clock brain pass fit cave bargain toe";

        vec![
            GoldenVector {
                label: "Bitcoin P2WPKH (BIP-84)",
                source: "github.com/bitcoin/bips/blob/master/bip-0084.mediawiki",
                mnemonic: ABANDON,
                path: "m/84'/0'/0'/0/0",
                chain: Chain::Bitcoin,
                curve: CurveFamily::Secp256k1,
                derivation_algorithm: DerivationAlgorithm::Bip32Secp256k1,
                address_algorithm: AddressAlgorithm::Bitcoin,
                public_key_format: PublicKeyFormat::Compressed,
                script_type: ScriptType::P2wpkh,
                expected: "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu",
                case_insensitive: false,
            },
            GoldenVector {
                label: "Bitcoin P2TR (BIP-86)",
                source: "github.com/bitcoin/bips/blob/master/bip-0086.mediawiki",
                mnemonic: ABANDON,
                path: "m/86'/0'/0'/0/0",
                chain: Chain::Bitcoin,
                curve: CurveFamily::Secp256k1,
                derivation_algorithm: DerivationAlgorithm::Bip32Secp256k1,
                address_algorithm: AddressAlgorithm::Bitcoin,
                public_key_format: PublicKeyFormat::XOnly,
                script_type: ScriptType::P2tr,
                expected: "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr",
                case_insensitive: false,
            },
            GoldenVector {
                label: "Bitcoin P2PKH legacy",
                source: "trezor-firmware/tests/device_tests/bitcoin/test_getaddress.py",
                mnemonic: ALL_ALL,
                path: "m/44'/0'/0'/0/0",
                chain: Chain::Bitcoin,
                curve: CurveFamily::Secp256k1,
                derivation_algorithm: DerivationAlgorithm::Bip32Secp256k1,
                address_algorithm: AddressAlgorithm::Bitcoin,
                public_key_format: PublicKeyFormat::Compressed,
                script_type: ScriptType::P2pkh,
                expected: "1JAd7XCBzGudGpJQSDSfpmJhiygtLQWaGL",
                case_insensitive: false,
            },
            GoldenVector {
                label: "Bitcoin P2WPKH (all-all seed)",
                source: "trezor-firmware/tests/device_tests/bitcoin/test_getaddress_segwit_native.py",
                mnemonic: ALL_ALL,
                path: "m/84'/0'/0'/0/0",
                chain: Chain::Bitcoin,
                curve: CurveFamily::Secp256k1,
                derivation_algorithm: DerivationAlgorithm::Bip32Secp256k1,
                address_algorithm: AddressAlgorithm::Bitcoin,
                public_key_format: PublicKeyFormat::Compressed,
                script_type: ScriptType::P2wpkh,
                expected: "bc1qannfxke2tfd4l7vhepehpvt05y83v3qsf6nfkk",
                case_insensitive: false,
            },
            GoldenVector {
                label: "Litecoin P2PKH legacy",
                source: "trezor-firmware/tests/device_tests/bitcoin/test_getaddress.py",
                mnemonic: ALL_ALL,
                path: "m/44'/2'/0'/0/0",
                chain: Chain::Litecoin,
                curve: CurveFamily::Secp256k1,
                derivation_algorithm: DerivationAlgorithm::Bip32Secp256k1,
                address_algorithm: AddressAlgorithm::Litecoin,
                public_key_format: PublicKeyFormat::Compressed,
                script_type: ScriptType::P2pkh,
                expected: "LcubERmHD31PWup1fbozpKuiqjHZ4anxcL",
                case_insensitive: false,
            },
            GoldenVector {
                label: "Ethereum",
                source: "trezor-firmware/common/tests/fixtures/ethereum/getaddress.json",
                mnemonic: ALL_ALL,
                path: "m/44'/60'/0'/0/0",
                chain: Chain::Ethereum,
                curve: CurveFamily::Secp256k1,
                derivation_algorithm: DerivationAlgorithm::Bip32Secp256k1,
                address_algorithm: AddressAlgorithm::Evm,
                public_key_format: PublicKeyFormat::Uncompressed,
                script_type: ScriptType::Account,
                expected: "0x73d0385F4d8E00C5e6504C6030F47BF6212736A8",
                case_insensitive: true,
            },
            GoldenVector {
                label: "Ethereum Classic",
                source: "trezor-firmware/common/tests/fixtures/ethereum/getaddress.json",
                mnemonic: ALL_ALL,
                path: "m/44'/61'/0'/0/0",
                chain: Chain::EthereumClassic,
                curve: CurveFamily::Secp256k1,
                derivation_algorithm: DerivationAlgorithm::Bip32Secp256k1,
                address_algorithm: AddressAlgorithm::Evm,
                public_key_format: PublicKeyFormat::Uncompressed,
                script_type: ScriptType::Account,
                expected: "0xF410e37E9C8BCf8CF319c84Ae9dCEbe057804a04",
                case_insensitive: true,
            },
            GoldenVector {
                label: "Tron",
                source: "trezor-firmware/common/tests/fixtures/tron/get_address.json",
                mnemonic: ALL_ALL,
                path: "m/44'/195'/0'/0/0",
                chain: Chain::Tron,
                curve: CurveFamily::Secp256k1,
                derivation_algorithm: DerivationAlgorithm::Bip32Secp256k1,
                address_algorithm: AddressAlgorithm::TronBase58Check,
                public_key_format: PublicKeyFormat::Uncompressed,
                script_type: ScriptType::Account,
                expected: "TY72iA3SBtrds3QLYsS7LwYfkzXwAXCRWT",
                case_insensitive: false,
            },
            GoldenVector {
                label: "XRP Ledger",
                source: "trezor-firmware/tests/device_tests/ripple/test_get_address.py",
                mnemonic: ALL_ALL,
                path: "m/44'/144'/0'/0/0",
                chain: Chain::Xrp,
                curve: CurveFamily::Secp256k1,
                derivation_algorithm: DerivationAlgorithm::Bip32Secp256k1,
                address_algorithm: AddressAlgorithm::XrpBase58Check,
                public_key_format: PublicKeyFormat::Compressed,
                script_type: ScriptType::P2pkh,
                expected: "rNaqKtKrMSwpwZSzRckPf7S96DkimjkF4H",
                case_insensitive: false,
            },
            GoldenVector {
                label: "Solana",
                source: "trezor-firmware/common/tests/fixtures/solana/get_address.json",
                mnemonic: ALL_ALL,
                path: "m/44'/501'/0'/0'",
                chain: Chain::Solana,
                curve: CurveFamily::Ed25519,
                derivation_algorithm: DerivationAlgorithm::Slip10Ed25519,
                address_algorithm: AddressAlgorithm::Solana,
                public_key_format: PublicKeyFormat::Raw,
                script_type: ScriptType::Account,
                expected: "14CCvQzQzHCVgZM3j9soPnXuJXh1RmCfwLVUcdfbZVBS",
                case_insensitive: false,
            },
            GoldenVector {
                label: "Stellar (SEP-0005 Test 1 #0)",
                source: "trezor-firmware/common/tests/fixtures/stellar/get_address.json",
                mnemonic: SEP_0005,
                path: "m/44'/148'/0'",
                chain: Chain::Stellar,
                curve: CurveFamily::Ed25519,
                derivation_algorithm: DerivationAlgorithm::Slip10Ed25519,
                address_algorithm: AddressAlgorithm::StellarStrKey,
                public_key_format: PublicKeyFormat::Raw,
                script_type: ScriptType::Account,
                expected: "GDRXE2BQUC3AZNPVFSCEZ76NJ3WWL25FYFK6RGZGIEKWE4SOOHSUJUJ6",
                case_insensitive: false,
            },
            GoldenVector {
                label: "Cardano Shelley enterprise",
                source: "trezor-firmware/common/tests/fixtures/cardano/get_enterprise_address.json",
                mnemonic: ALL_ALL,
                path: "m/1852'/1815'/0'/0/0",
                chain: Chain::Cardano,
                curve: CurveFamily::Ed25519,
                derivation_algorithm: DerivationAlgorithm::Bip32Ed25519Icarus,
                address_algorithm: AddressAlgorithm::CardanoShelleyEnterprise,
                public_key_format: PublicKeyFormat::Raw,
                script_type: ScriptType::Account,
                expected: "addr1vxq0nckg3ekgzuqg7w5p9mvgnd9ym28qh5grlph8xd2z92su77c6m",
                case_insensitive: false,
            },
        ]
    }

    #[test]
    fn canonical_external_golden_vectors_all_pass() {
        // Runs every golden vector and collects failures so we see the
        // full picture in one test run. Any mismatch → test fails with
        // a readable per-chain diff.
        let vectors = canonical_golden_vectors();
        let mut failures: Vec<String> = Vec::new();
        for v in &vectors {
            match run_golden_vector(v) {
                Err(e) => failures.push(format!(
                    "{} — derivation error: {} (source: {}, path: {})",
                    v.label, e, v.source, v.path
                )),
                Ok(addr) => {
                    let ok = if v.case_insensitive {
                        addr.eq_ignore_ascii_case(v.expected)
                    } else {
                        addr == v.expected
                    };
                    if !ok {
                        failures.push(format!(
                            "{} mismatch\n    expected: {}\n    actual:   {}\n    source:   {}\n    path:     {}",
                            v.label, v.expected, addr, v.source, v.path
                        ));
                    }
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} canonical vector(s) failed:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    #[test]
    fn evm_replica_chains_share_ethereum_golden() {
        // Every EVM-compatible chain reuses Ethereum's BIP-44 derivation
        // (m/44'/60'/…) and Keccak address algorithm, so the same
        // mnemonic must produce the same address on Arbitrum / Optimism
        // / BNB Chain / Avalanche / Hyperliquid as on Ethereum. Pin this
        // so a future per-chain override can't silently diverge.
        const ALL_ALL: &str = "all all all all all all all all all all all all";
        let expected = "0x73d0385F4d8E00C5e6504C6030F47BF6212736A8";
        let replicas = [
            Chain::Arbitrum,
            Chain::Optimism,
            Chain::Avalanche,
            Chain::Hyperliquid,
        ];
        for chain in replicas {
            let v = GoldenVector {
                label: "EVM replica",
                source: "Ethereum golden",
                mnemonic: ALL_ALL,
                path: "m/44'/60'/0'/0/0",
                chain,
                curve: CurveFamily::Secp256k1,
                derivation_algorithm: DerivationAlgorithm::Bip32Secp256k1,
                address_algorithm: AddressAlgorithm::Evm,
                public_key_format: PublicKeyFormat::Uncompressed,
                script_type: ScriptType::Account,
                expected,
                case_insensitive: true,
            };
            let addr = run_golden_vector(&v).expect("evm replica derivation");
            assert!(
                addr.eq_ignore_ascii_case(expected),
                "EVM replica drifted: chain={:?} got {}",
                std::mem::discriminant(&chain),
                addr
            );
        }
    }
