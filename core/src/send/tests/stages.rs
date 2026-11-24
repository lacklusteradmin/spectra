use super::*;

fn reseal(stored: &mut StoredSend) {
    stored.view.prepared_details = serde_json::to_string_pretty(&stored.prepared).unwrap();
    stored.view.review_digest = stored.digest().unwrap();
}

mod nft_tests {
    use super::*;
    use crate::registry::NftStandard;

    const OWNER: &str = "0x1111111111111111111111111111111111111111";
    const RECIPIENT: &str = "0x2222222222222222222222222222222222222222";
    const CONTRACT: &str = "0x3333333333333333333333333333333333333333";

    /// A stored NFT transfer exactly as `build_nft_transfer` stores one.
    fn transfer(standard: NftStandard, token_id: &str, quantity: &str) -> StoredSend {
        let prepared = PreparedPayload::Evm(crate::send::evm::PreparedEvmTransaction {
            chain_id: 1,
            nonce: 7,
            max_fee_per_gas: 3,
            max_priority_fee_per_gas: 1,
            gas_limit: 90_000,
            to: CONTRACT.into(),
            value_wei: 0,
            data: crate::send::evm::encode_nft_transfer(
                standard, OWNER, RECIPIENT, token_id, quantity,
            )
            .unwrap(),
            access_list: vec![],
            additional_fee_wei: 0,
        });
        let mut stored = StoredSend {
            view: SendArtifact {
                id: "nft".into(),
                revision: 0,
                stage: SendStage::Prepared,
                wallet_id: "wallet".into(),
                chain_id: crate::registry::Chain::Ethereum,
                sender: OWNER.into(),
                recipient: RECIPIENT.into(),
                amount: quantity.into(),
                asset: CONTRACT.into(),
                symbol: "APE".into(),
                staking: None,
                operation: Some(WalletOperation::TransferNft {
                    contract: CONTRACT.into(),
                    standard,
                    token_id: token_id.into(),
                    quantity: quantity.into(),
                    collection: "Apes".into(),
                }),
                created_at: 1.0,
                review_digest: String::new(),
                review: SendArtifactReview {
                    network_fee: Some(NetworkFee {
                        amount: "0.00000000000027".into(),
                        bound: FeeBound::AtMost,
                        value: None,
                    }),
                    ..SendArtifactReview::default()
                },
                prepared_details: String::new(),
                signing_payload_hex: String::new(),
                signed_payload: None,
                transaction_hash: None,
                attempts: vec![],
                selected_endpoints: vec![],
                memo: None,
            },
            request: crate::send::SendExecutionRequest {
                chain_id: crate::registry::Chain::Ethereum,
                wallet_id: "wallet".into(),
                password: None,
                to_address: RECIPIENT.into(),
                amount_str: quantity.into(),
                contract_address: None,
                token_standard: None,
                token_decimals: None,
                fee_rate_svb: None,
                fee_sat: None,
                gas_budget: None,
                fee_amount: None,
                evm_overrides: None,
                memo: None,
            },
            prepared,
            submission: None,
            signed_digest: None,
            substrate_verified_through: None,
            icp_staking_receipts: vec![],
        };
        reseal(&mut stored);
        stored
    }

    /// What a tamperer with the database would do after an edit: make the
    /// stored artifact consistent with itself again.
    fn evm(stored: &mut StoredSend) -> &mut crate::send::evm::PreparedEvmTransaction {
        match &mut stored.prepared {
            PreparedPayload::Evm(prepared) => prepared,
            _ => unreachable!(),
        }
    }

    fn operation(stored: &mut StoredSend) -> (&mut String, &mut String) {
        match stored.view.operation.as_mut().unwrap() {
            WalletOperation::TransferNft {
                token_id, quantity, ..
            } => (token_id, quantity),
            _ => unreachable!(),
        }
    }

    /// An NFT transfer signs exactly the reviewed call: the collection's
    /// `safeTransferFrom` of that token and quantity, from the wallet to the
    /// reviewed recipient, with no value. Any other call under its name, even
    /// one consistent with itself, is refused.
    #[test]
    fn only_the_reviewed_transfer_is_signed() {
        for (standard, id, quantity) in [
            (NftStandard::Erc721, "1234", "1"),
            (NftStandard::Erc1155, "7", "3"),
        ] {
            transfer(standard, id, quantity).validate().unwrap();
            let tampers: Vec<(&str, Box<dyn Fn(&mut StoredSend)>)> = vec![
                (
                    "another token",
                    Box::new(|s| *operation(s).0 = "1235".into()),
                ),
                (
                    "another collection",
                    Box::new(|s| evm(s).to = RECIPIENT.into()),
                ),
                ("ether attached", Box::new(|s| evm(s).value_wei = 1)),
                (
                    "another call",
                    Box::new(|s| {
                        evm(s).data = crate::send::evm::encode_erc20_transfer(RECIPIENT, 1).unwrap()
                    }),
                ),
                (
                    "another recipient",
                    Box::new(|s| {
                        s.view.recipient = CONTRACT.into();
                        s.request.to_address = CONTRACT.into();
                    }),
                ),
                ("another fee", Box::new(|s| evm(s).max_fee_per_gas = 4)),
            ];
            for (what, tamper) in tampers {
                let mut stored = transfer(standard, id, quantity);
                tamper(&mut stored);
                reseal(&mut stored);
                let error = stored.validate().unwrap_err().to_string();
                assert!(error.contains("operation was altered"), "{what}: {error}");
            }
            // The amount on screen is the quantity in the call.
            let mut stored = transfer(standard, id, quantity);
            stored.view.amount = "2".into();
            stored.request.amount_str = "2".into();
            reseal(&mut stored);
            assert!(stored.validate().is_err());
            // An operation dropped from the artifact leaves an NFT call unnamed:
            // the digest that bound it fails.
            let mut stored = transfer(standard, id, quantity);
            stored.view.operation = None;
            assert!(stored.validate().is_err());
        }
        // An ERC-721 transfer is one token: a quantity of two encodes no call.
        let mut stored = transfer(NftStandard::Erc721, "1", "1");
        *operation(&mut stored).1 = "2".into();
        stored.view.amount = "2".into();
        stored.request.amount_str = "2".into();
        reseal(&mut stored);
        assert!(stored.validate().is_err());
    }
}

mod zcash_shielded_tests {
    use super::*;
    use crate::send::zcash_shielded::{PreparedZcashShielded, ZcashPayment};

    const SENDER: &str = "t1XVXWCvpMgBvUaed4XDqWtgQgJSu1Ghz7F";
    const RECIPIENT: &str = "u1hekqggren38vzwhnny8vewspf9j59nxaly64ese097lmedgnyzv979k9rh00wj7lyhfxjkvwee5rw44vppm656k4qyp6fxq99588xm68mryat0nejep54e2g2ge23vpn0u5u23gpnzc63teksttlk455q8xxm0j9m8e76mg4lqh752r4";
    const OWN: &str = "u14hxpxnwfzfujl6n36gfpr2e90mxzupmaksy3ly7ztgcey8qa06703yutk3m3u46tfkeywk9w69atu6zmdlp0lekh8jpsxmpzngrspf379dtgyvyfggtqlyvcq3wjr5y5xngex2qtsxheywdyzdfalk5clnms8wy3fje0j560wvsjm6d8";

    /// A stored shielded artifact as `store_zcash_shielded` stores one: a
    /// payment of 0.3 ZEC with a memo, or shielding 0.49985 ZEC.
    fn stored(shielding: bool) -> StoredSend {
        let prepared = if shielding {
            PreparedZcashShielded {
                proposal_hex: "00".into(),
                payments: vec![],
                fee_zat: 15_000,
                transparent_in_zat: 50_000_000,
                shielded_in_zat: 0,
                change_zat: 49_985_000,
                spends_sapling: false,
                uses_sapling: false,
            }
        } else {
            PreparedZcashShielded {
                proposal_hex: "00".into(),
                payments: vec![ZcashPayment {
                    address: RECIPIENT.into(),
                    zatoshis: 30_000_000,
                    memo: Some("thanks".into()),
                }],
                fee_zat: 10_000,
                transparent_in_zat: 0,
                shielded_in_zat: 150_000_000,
                change_zat: 119_990_000,
                spends_sapling: false,
                uses_sapling: false,
            }
        };
        let (recipient, amount) = if shielding {
            (OWN, "0.49985")
        } else {
            (RECIPIENT, "0.3")
        };
        let mut stored = StoredSend {
            view: SendArtifact {
                id: "zec".into(),
                revision: 0,
                stage: SendStage::Prepared,
                wallet_id: "wallet".into(),
                chain_id: crate::registry::Chain::Zcash,
                sender: SENDER.into(),
                recipient: recipient.into(),
                amount: amount.into(),
                asset: "ZEC".into(),
                symbol: "ZEC".into(),
                staking: None,
                operation: Some(if shielding {
                    WalletOperation::ShieldTransparent {
                        amount: amount.into(),
                    }
                } else {
                    WalletOperation::ShieldedPayment {
                        memo: Some("thanks".into()),
                    }
                }),
                created_at: 1.0,
                review_digest: String::new(),
                review: SendArtifactReview {
                    network_fee: Some(NetworkFee {
                        amount: (if shielding { "0.00015" } else { "0.0001" }).into(),
                        bound: FeeBound::Exact,
                        value: None,
                    }),
                    ..SendArtifactReview::default()
                },
                prepared_details: String::new(),
                signing_payload_hex: String::new(),
                signed_payload: None,
                transaction_hash: None,
                attempts: vec![],
                selected_endpoints: vec![],
                memo: None,
            },
            request: crate::send::SendExecutionRequest {
                chain_id: crate::registry::Chain::Zcash,
                wallet_id: "wallet".into(),
                password: None,
                to_address: recipient.into(),
                amount_str: amount.into(),
                contract_address: None,
                token_standard: None,
                token_decimals: None,
                fee_rate_svb: None,
                fee_sat: None,
                gas_budget: None,
                fee_amount: None,
                evm_overrides: None,
                memo: None,
            },
            prepared: PreparedPayload::ZcashShielded(prepared),
            submission: None,
            signed_digest: None,
            substrate_verified_through: None,
            icp_staking_receipts: vec![],
        };
        reseal(&mut stored);
        stored
    }

    /// What a tamperer with the database would do after an edit: make the
    /// stored artifact consistent with itself again.
    fn prepared(stored: &mut StoredSend) -> &mut PreparedZcashShielded {
        match &mut stored.prepared {
            PreparedPayload::ZcashShielded(prepared) => prepared,
            _ => unreachable!(),
        }
    }

    fn memo(stored: &mut StoredSend) -> &mut Option<String> {
        match stored.view.operation.as_mut().unwrap() {
            WalletOperation::ShieldedPayment { memo, .. } => memo,
            _ => unreachable!(),
        }
    }

    fn refused(shielding: bool, what: &str, tamper: impl Fn(&mut StoredSend)) {
        let mut stored = stored(shielding);
        tamper(&mut stored);
        reseal(&mut stored);
        let error = stored.validate().unwrap_err().to_string();
        assert!(error.contains("operation was altered"), "{what}: {error}");
    }

    /// A shielded payment shows what its proposal pays: the one recipient, the
    /// amount, the memo and the fee, from shielded notes alone.
    #[test]
    fn a_shielded_payment_shows_what_it_pays() {
        stored(false).validate().unwrap();
        refused(false, "another recipient", |s| {
            s.view.recipient = OWN.into();
            s.request.to_address = OWN.into();
        });
        refused(false, "another amount", |s| {
            s.view.amount = "0.31".into();
            s.request.amount_str = "0.31".into();
        });
        refused(false, "another memo", |s| {
            *memo(s) = Some("thank you".into())
        });
        refused(false, "a memo hidden", |s| *memo(s) = None);
        refused(false, "another fee", |s| prepared(s).fee_zat = 15_000);
        refused(false, "a second payment", |s| {
            let payment = prepared(s).payments[0].clone();
            prepared(s).payments.push(payment);
        });
        refused(false, "transparent funds spent", |s| {
            prepared(s).transparent_in_zat = 1
        });
        refused(false, "named a shielding", |s| {
            s.view.operation = Some(WalletOperation::ShieldTransparent {
                amount: "0.3".into(),
            })
        });
        refused(false, "unnamed", |s| s.view.operation = None);
    }

    /// Shielding pays no one: every transparent input moves to the wallet's own
    /// shielded pool, and the amount shown is what arrives there.
    #[test]
    fn shielding_moves_only_the_wallets_own_funds() {
        stored(true).validate().unwrap();
        refused(true, "a payment", |s| {
            prepared(s).payments.push(ZcashPayment {
                address: RECIPIENT.into(),
                zatoshis: 1,
                memo: None,
            })
        });
        refused(true, "another amount", |s| {
            s.view.amount = "0.5".into();
            s.request.amount_str = "0.5".into();
            if let Some(WalletOperation::ShieldTransparent { amount, .. }) = &mut s.view.operation {
                *amount = "0.5".into();
            }
        });
        refused(true, "another fee", |s| prepared(s).fee_zat = 10_000);
        refused(true, "shielded notes spent", |s| {
            prepared(s).shielded_in_zat = 1
        });
        refused(true, "nothing to shield", |s| {
            prepared(s).transparent_in_zat = 0
        });
        refused(true, "named a payment", |s| {
            s.view.operation = Some(WalletOperation::ShieldedPayment { memo: None })
        });
    }
}

mod near_storage_refund_tests {
    use super::*;
    use crate::send::near::PreparedNearFunctionCall;

    const ACCOUNT: &str = "alice.near";
    const TOKEN: &str = "usdt.tether-token.near";

    /// A stored storage refund exactly as `build_token_storage_refund` stores
    /// one: 0.00125 NEAR back, at most 0.0003 NEAR in fees.
    fn refund() -> StoredSend {
        let prepared =
            PreparedPayload::NearFunctionCall(PreparedNearFunctionCall::storage_unregister(
                ACCOUNT,
                [1; 32],
                43,
                TOKEN,
                30_000_000_000_000,
                [2; 32],
                300_000_000_000_000_000_000,
            ));
        let mut stored = StoredSend {
            view: SendArtifact {
                id: "refund".into(),
                revision: 0,
                stage: SendStage::Prepared,
                wallet_id: "wallet".into(),
                chain_id: crate::registry::Chain::Near,
                sender: ACCOUNT.into(),
                recipient: ACCOUNT.into(),
                amount: "0.00125".into(),
                asset: "NEAR".into(),
                symbol: "NEAR".into(),
                staking: None,
                operation: Some(WalletOperation::RefundTokenStorage {
                    contract: TOKEN.into(),
                    refund: "0.00125".into(),
                }),
                created_at: 1.0,
                review_digest: String::new(),
                review: SendArtifactReview {
                    network_fee: Some(NetworkFee {
                        amount: "0.0003".into(),
                        bound: FeeBound::AtMost,
                        value: None,
                    }),
                    ..SendArtifactReview::default()
                },
                prepared_details: String::new(),
                signing_payload_hex: String::new(),
                signed_payload: None,
                transaction_hash: None,
                attempts: vec![],
                selected_endpoints: vec![],
                memo: None,
            },
            request: crate::send::SendExecutionRequest {
                chain_id: crate::registry::Chain::Near,
                wallet_id: "wallet".into(),
                password: None,
                to_address: ACCOUNT.into(),
                amount_str: "0.00125".into(),
                contract_address: None,
                token_standard: None,
                token_decimals: None,
                fee_rate_svb: None,
                fee_sat: None,
                gas_budget: None,
                fee_amount: Some("0.0003".into()),
                evm_overrides: None,
                memo: None,
            },
            prepared,
            submission: None,
            signed_digest: None,
            substrate_verified_through: None,
            icp_staking_receipts: vec![],
        };
        reseal(&mut stored);
        stored
    }

    /// What a tamperer with the database would do after an edit: make the
    /// stored artifact consistent with itself again.
    fn call(stored: &mut StoredSend) -> &mut PreparedNearFunctionCall {
        match &mut stored.prepared {
            PreparedPayload::NearFunctionCall(prepared) => prepared,
            _ => unreachable!(),
        }
    }

    /// A storage refund signs exactly the reviewed call: the account
    /// unregistering itself from the token it names, without `force`, with one
    /// yoctoNEAR, its deposit coming back to it. Any other call under its name,
    /// even one consistent with itself, is refused.
    #[test]
    fn only_the_reviewed_unregistration_is_signed() {
        refund().validate().unwrap();
        let tampers: Vec<(&str, Box<dyn Fn(&mut StoredSend)>)> = vec![
            (
                "force",
                Box::new(|s| call(s).args = br#"{"force":true}"#.to_vec()),
            ),
            ("no yoctoNEAR", Box::new(|s| call(s).deposit = 0)),
            (
                "another contract",
                Box::new(|s| call(s).receiver = "wrap.near".into()),
            ),
            (
                "another method",
                Box::new(|s| call(s).method = "storage_withdraw".into()),
            ),
            ("another fee", Box::new(|s| call(s).fee_budget = 1)),
            (
                "another refund",
                Box::new(|s| {
                    s.view.amount = "1".into();
                    s.request.amount_str = "1".into();
                }),
            ),
            (
                "another recipient",
                Box::new(|s| {
                    s.view.recipient = "bob.near".into();
                    s.request.to_address = "bob.near".into();
                }),
            ),
        ];
        for (what, tamper) in tampers {
            let mut stored = refund();
            tamper(&mut stored);
            reseal(&mut stored);
            let error = stored.validate().unwrap_err().to_string();
            assert!(error.contains("operation was altered"), "{what}: {error}");
        }
        // A NEAR call with its operation dropped is no staking and no
        // operation: refused, however consistent.
        let mut stored = refund();
        stored.view.operation = None;
        reseal(&mut stored);
        assert!(stored.validate().is_err());
    }
}

mod exact_amount_tests {
    use super::PreparedPayload;

    /// Every big integer a prepared payload stores is an exact decimal
    /// string, which a JSON reader keeps whole, and reads back as written.
    #[test]
    fn prepared_big_integers_are_exact_strings() {
        let yocto = 10u128.pow(24) + 1;
        let near = PreparedPayload::Near {
            public_key: [1; 32],
            nonce: 7,
            block_hash: [2; 32],
            amount: yocto,
            token_contract: Some("usdt.near".into()),
            fee_budget: yocto + 1,
            registration_deposit: Some(yocto + 2),
        };
        let json = serde_json::to_value(&near).unwrap();
        let fields = &json["Near"];
        assert_eq!(fields["amount"], yocto.to_string());
        assert_eq!(fields["fee_budget"], (yocto + 1).to_string());
        assert_eq!(fields["registration_deposit"], (yocto + 2).to_string());
        let read: PreparedPayload = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&read).unwrap(), json);

        let unregistered = PreparedPayload::Near {
            public_key: [1; 32],
            nonce: 7,
            block_hash: [2; 32],
            amount: 1,
            token_contract: None,
            fee_budget: 1,
            registration_deposit: None,
        };
        let json = serde_json::to_value(&unregistered).unwrap();
        assert!(json["Near"]["registration_deposit"].is_null());
        assert!(serde_json::from_value::<PreparedPayload>(json).is_ok());
    }
}
