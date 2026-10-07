use super::*;
use crate::derivation::setup::WalletSetupMethod;
use crate::derivation::setup::tests::{fixture, service};

/// On every network and for every way a signing wallet is added, a wallet
/// whose address has a scheme signs a message its address verifies, and one
/// without is refused; a watched wallet answers its scheme and signs nothing.
#[tokio::test]
async fn every_scheme_signs_what_its_address_verifies() {
    for chain in Chain::all() {
        let descriptor = crate::derivation::setup::wallet_setup_descriptor(chain);
        let (service, directory) = service().await;
        for method in [
            WalletSetupMethod::ImportPhrase,
            WalletSetupMethod::ImportPrivateKey,
            WalletSetupMethod::WatchAddresses,
        ] {
            if descriptor.option(method).is_none() {
                continue;
            }
            let wallet = service
                .import_wallets(fixture(chain, method))
                .await
                .unwrap()
                .wallets[0]
                .clone();
            let scheme = service
                .wallet_message_scheme(wallet.id.clone())
                .await
                .unwrap();
            let signed = service
                .sign_wallet_message(wallet.id.clone(), "Spectra proves this".into(), None)
                .await;
            match (method, scheme) {
                (WalletSetupMethod::WatchAddresses, _) | (_, None) => {
                    assert!(signed.is_err(), "{chain} {method:?} signed");
                }
                (_, Some(scheme)) => {
                    let signed =
                        signed.unwrap_or_else(|error| panic!("{chain} {method:?}: {error}"));
                    assert_eq!(signed.scheme, scheme);
                    assert_eq!(Some(signed.address.as_str()), wallet.primary_address());
                    assert!(
                        crate::send::message::verify_message(
                            chain,
                            signed.address.clone(),
                            signed.message.clone(),
                            signed.signature.clone()
                        ),
                        "{chain} {method:?} {scheme:?}"
                    );
                }
            }
            service
                .apply_state_command(StateCommand::RemoveWallet {
                    wallet_id: wallet.id,
                })
                .await
                .unwrap();
        }
        drop(service);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
