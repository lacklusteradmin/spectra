use super::*;

/// EIP-137's own namehash vectors.
#[test]
fn namehash_is_eip137s() {
    let hash = |name| hex::encode(crate::api::evm_json_rpc::ens_namehash(name));
    assert_eq!(hash(""), "0".repeat(64));
    assert_eq!(
        hash("eth"),
        "93cdeb708b7545dc668eb9280176169d1c33cfd8ed6f04690a0bcc88a93fc4ae"
    );
    assert_eq!(
        hash("foo.eth"),
        "de9b09fd7c5f901e23a3f19fecc54828e9c848539801e86591bd9801b019f84f"
    );
}

/// A revocation is `approve(spender, 0)`: the selector, the spender as a
/// word, and a zero word.
#[test]
fn a_revocation_approves_nothing() {
    let spender = "0x1111111254eeb25477b68fb85ed929f73a960582";
    let data = crate::send::evm::encode_erc20_approve(spender, 0).unwrap();
    assert_eq!(
        hex::encode(data),
        format!("095ea7b3{:0>64}{}", &spender[2..], "0".repeat(64))
    );
}

/// Neither a network without EVM approvals nor a wallet that is gone is
/// asked anything.
#[tokio::test]
async fn only_an_evm_wallet_has_approvals() {
    let (service, directory) = crate::derivation::setup::tests::service().await;
    let wallet = service
        .import_wallets(crate::derivation::setup::tests::fixture(
            Chain::Solana,
            crate::derivation::setup::WalletSetupMethod::ImportPhrase,
        ))
        .await
        .unwrap()
        .wallets[0]
        .id
        .clone();
    assert!(matches!(
        service.wallet_token_approvals(wallet.clone()).await,
        Err(SpectraBridgeError::InvalidInput { .. })
    ));
    assert!(service.wallet_ens_name("missing".into()).await.is_err());
    drop(service);
    std::fs::remove_dir_all(directory).unwrap();
}
