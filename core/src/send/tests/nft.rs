use super::*;
use crate::api::evm_nft::NftStandard;

const OWNER: &str = "0x1111111111111111111111111111111111111111";
const RECIPIENT: &str = "0x2222222222222222222222222222222222222222";
const CONTRACT: &str = "0x3333333333333333333333333333333333333333";

/// A stored NFT transfer exactly as `build_nft_transfer` stores one.
fn transfer(standard: NftStandard, token_id: &str, quantity: &str) -> StoredSend {
    let prepared = PreparedPayload::Evm(super::super::evm::PreparedEvmTransaction {
        chain_id: 1,
        nonce: 7,
        max_fee_per_gas: 3,
        max_priority_fee_per_gas: 1,
        gas_limit: 90_000,
        to: CONTRACT.into(),
        value_wei: 0,
        data: super::super::evm::encode_nft_transfer(
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
                network_fee: "0.00000000000027".into(),
            }),
            created_at: 1.0,
            review_digest: String::new(),
            review: SendArtifactReview::default(),
            prepared_details: String::new(),
            signing_payload_hex: String::new(),
            signed_payload: None,
            transaction_hash: None,
            attempts: vec![],
            selected_endpoints: vec![],
        },
        request: super::super::SendExecutionRequest {
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
            sign_only: false,
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
fn reseal(stored: &mut StoredSend) {
    stored.view.prepared_details = serde_json::to_string_pretty(&stored.prepared).unwrap();
    stored.view.review_digest = stored.digest().unwrap();
}

fn evm(stored: &mut StoredSend) -> &mut super::super::evm::PreparedEvmTransaction {
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
                    evm(s).data = super::super::evm::encode_erc20_transfer(RECIPIENT, 1).unwrap()
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
