//! What a wallet's page offers, for every front end.
//!
//! One answer from the wallet's network, its signing kind and what core
//! holds for it: the app's wallet page renders it, and `spectra wallet
//! actions` prints it. An action is listed only where core performs it, and
//! a watch-only wallet lists only what needs no key. Each action carries its
//! note, an English sentence a front end translates by its text, so every
//! front end says the same thing about it.

use super::*;
use crate::derivation::setup::{WalletSetupMethod, wallet_setup_descriptor};
use crate::store::state::{WalletSigning, WalletState};

/// Something a wallet's page offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum WalletAction {
    /// Send from this wallet.
    Send,
    /// A multisig account's signers and its spends: transactions to build,
    /// read, sign and submit with its other signers.
    Multisig,
    /// Show the address this wallet receives at.
    Receive,
    /// This wallet's transactions.
    History,
    /// Open the wallet's address on its network's explorer.
    OpenInExplorer,
    /// Give a watched wallet its keys from the phrase or key that holds its
    /// address, keeping its name and history.
    AddKeys,
    /// This wallet's staking positions, and staking from it.
    Stake,
    /// Scan blocks on this device for the wallet's funds (Monero).
    ScanBlocks,
    /// The addresses and coins behind the balance of an account-discovery
    /// UTXO wallet.
    Coins,
    /// The ERC-20 allowances an EVM wallet has granted, and revoking them.
    TokenApprovals,
    /// The ERC-721 and ERC-1155 tokens an EVM wallet holds, and sending one.
    Nfts,
    /// A Zcash wallet's shielded funds: scanning for them, the address that
    /// receives them, shielding transparent funds and sending.
    ShieldedFunds,
    /// A Litecoin wallet's MWEB funds: scanning for them, the stealth
    /// address that receives them, pegging transparent funds in and sending.
    MwebFunds,
    /// What the wallet's account on its network holds and needs beyond a
    /// balance: resources, a reserve, a contract, storage.
    NetworkAccount,
    /// A NEAR account's access keys, and deleting a function-call key.
    AccessKeys,
    /// The storage deposits NEAR token contracts hold for the account, and
    /// getting them back from tokens it no longer holds.
    TokenStorage,
    /// A Sui wallet's coin objects, and merging a type's objects into one.
    CoinObjects,
    /// A Solana wallet's empty token accounts, and closing them for their
    /// rent.
    TokenAccounts,
    /// An XRP Ledger or Stellar wallet's trust lines: the issued assets it
    /// can hold, trusting another, and removing an empty one.
    TrustLines,
    /// A test network's faucet page.
    GetTestCoins,
    /// Sign a message with the wallet's key to prove it holds its address,
    /// or check a signature.
    SignMessage,
    /// Check that a message was signed by the watched address.
    VerifyMessage,
    /// Add the wallet's key, or watched address, to another network as a
    /// wallet of its own.
    AddToNetwork,
    /// Change the wallet's name.
    Rename,
    /// Show the recovery phrase the wallet was created or restored from.
    RevealPhrase,
    /// Export the wallet's keys as other wallets import them.
    ExportKeys,
    /// Remove the wallet, its history and its keys.
    Delete,
}

/// Where on the page an action belongs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum WalletActionSection {
    /// Sending, receiving and the wallet's history: what every wallet does.
    Everyday,
    /// What the wallet's network adds.
    Network,
    /// The wallet itself: its name, its keys, deleting it.
    Manage,
}

impl WalletAction {
    pub fn section(self) -> WalletActionSection {
        match self {
            Self::Send | Self::Receive | Self::History => WalletActionSection::Everyday,
            Self::Multisig
            | Self::OpenInExplorer
            | Self::AddKeys
            | Self::Stake
            | Self::ScanBlocks
            | Self::Coins
            | Self::TokenApprovals
            | Self::Nfts
            | Self::ShieldedFunds
            | Self::MwebFunds
            | Self::NetworkAccount
            | Self::AccessKeys
            | Self::TokenStorage
            | Self::CoinObjects
            | Self::GetTestCoins
            | Self::TokenAccounts
            | Self::TrustLines
            | Self::SignMessage
            | Self::VerifyMessage => WalletActionSection::Network,
            Self::AddToNetwork
            | Self::Rename
            | Self::RevealPhrase
            | Self::ExportKeys
            | Self::Delete => WalletActionSection::Manage,
        }
    }

    /// What the action does, in the words every front end shows. The text is
    /// the key a front end translates it by.
    pub fn note(self) -> &'static str {
        match self {
            Self::Send => "Send from this wallet.",
            Self::Multisig => {
                "This account's signers and threshold, and spending from it with their signatures: build a transaction or read one a signer sent, sign it, and submit it once enough have signed."
            }
            Self::Receive => "Show the address this wallet receives at.",
            Self::History => "This wallet's transactions.",
            Self::OpenInExplorer => "Open this wallet's address on the network's explorer.",
            Self::AddKeys => {
                "Add the phrase or key that holds this address, so the wallet can sign. It keeps its name and history."
            }
            Self::Stake => "This wallet's staking positions, and staking from it.",
            Self::ScanBlocks => "Scan blocks on this device for this wallet's funds.",
            Self::TokenAccounts => {
                "Close the empty token accounts this wallet keeps and get their rent back."
            }
            Self::TrustLines => {
                "The issued assets this wallet can hold, trusting another so it can receive it, and removing an empty line to free its reserve."
            }
            Self::GetTestCoins => {
                "Open this test network's faucet, where its coins are free. Some faucets ask you to sign in."
            }
            Self::CoinObjects => {
                "How many objects hold each coin, and merging them so sends cost less gas."
            }
            Self::AccessKeys => {
                "The keys that can act for this account, and removing the ones dapps were given."
            }
            Self::TokenStorage => {
                "Get back the NEAR token contracts hold for this account's storage, from tokens it no longer holds."
            }
            Self::NetworkAccount => {
                "What this wallet's account on its network holds and needs beyond its balance."
            }
            Self::TokenApprovals => {
                "Contracts this wallet has let spend its tokens, and taking that back."
            }
            Self::Nfts => "The NFTs this wallet holds, and sending one.",
            Self::ShieldedFunds => {
                "Scan for shielded ZEC, receive it privately, move transparent ZEC into the shielded pool, and send from it."
            }
            Self::MwebFunds => {
                "Scan for MWEB LTC, receive it privately, move transparent LTC into MWEB, and send from it."
            }
            Self::Coins => "The addresses this wallet has handed out and the coins they hold.",
            Self::SignMessage => {
                "Sign a message with this wallet's key to prove it holds this address, or check a signature."
            }
            Self::VerifyMessage => "Check that a message was signed by this address.",
            Self::AddToNetwork => {
                "Add this wallet's key or address to another network, as a wallet of its own."
            }
            Self::Rename => "Change the name this wallet shows.",
            Self::RevealPhrase => {
                "Show the recovery phrase this wallet was created or restored from."
            }
            Self::ExportKeys => {
                "Export this wallet's keys in the form other wallets import, or its public key to watch it elsewhere."
            }
            Self::Delete => "Remove this wallet, its history and its keys from this device.",
        }
    }
}

/// One action a wallet offers, with what it does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletActionOffer {
    pub action: WalletAction,
    pub section: WalletActionSection,
    pub note: String,
}

/// Everything a wallet's page offers, in the order to offer it, and the
/// network's capabilities and limits the setup page showed before it was
/// added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletActions {
    pub wallet_id: String,
    pub chain: Chain,
    pub actions: Vec<WalletActionOffer>,
    pub summary: WalletSetupSummary,
}

/// The actions `wallet` offers, in page order.
pub(crate) fn offered_actions(wallet: &WalletState) -> Vec<WalletAction> {
    let chain = wallet.chain_id;
    let signs = !wallet.signing.is_watch_only();
    let message_scheme = wallet
        .address_on(chain)
        .and_then(|address| crate::send::message::scheme_for(chain, address));
    [
        (
            signs && wallet.multisig_policy.is_none(),
            WalletAction::Send,
        ),
        (
            super::multisig::MultisigScheme::of(wallet).is_some(),
            WalletAction::Multisig,
        ),
        (true, WalletAction::Receive),
        (true, WalletAction::History),
        (
            wallet
                .address_on(chain)
                .and_then(|address| crate::address_explorer_link(chain, address.to_string()))
                .is_some(),
            WalletAction::OpenInExplorer,
        ),
        (!upgrade_methods(wallet).is_empty(), WalletAction::AddKeys),
        // Positions are read from the address, so a watched wallet sees them;
        // every staking action still needs the key.
        (
            !chain.is_testnet() && chain.supports_staking(),
            WalletAction::Stake,
        ),
        // The scan reads with the wallet's own keys, or a view-only
        // wallet's view key.
        (chain.scans_for_balance(), WalletAction::ScanBlocks),
        (chain.lists_account_coins(), WalletAction::Coins),
        (chain.is_evm(), WalletAction::TokenApprovals),
        (chain.is_evm(), WalletAction::Nfts),
        (
            super::zcash_shielded::shielded_account(wallet).is_ok(),
            WalletAction::ShieldedFunds,
        ),
        (
            super::litecoin_mweb::holds_mweb_keys(wallet).is_ok(),
            WalletAction::MwebFunds,
        ),
        (
            super::wallet_network_account::has_network_account(wallet),
            WalletAction::NetworkAccount,
        ),
        (
            chain.mainnet_counterpart() == Chain::Near,
            WalletAction::AccessKeys,
        ),
        (
            chain.mainnet_counterpart() == Chain::Near,
            WalletAction::TokenStorage,
        ),
        (
            chain.mainnet_counterpart() == Chain::Sui,
            WalletAction::CoinObjects,
        ),
        (
            chain.mainnet_counterpart() == Chain::Solana,
            WalletAction::TokenAccounts,
        ),
        (
            matches!(chain.mainnet_counterpart(), Chain::Xrp | Chain::Stellar),
            WalletAction::TrustLines,
        ),
        (chain.faucet_url().is_some(), WalletAction::GetTestCoins),
        (signs && message_scheme.is_some(), WalletAction::SignMessage),
        (
            !signs && message_scheme.is_some(),
            WalletAction::VerifyMessage,
        ),
        (
            !super::wallet_copy::copy_targets(wallet).is_empty(),
            WalletAction::AddToNetwork,
        ),
        (true, WalletAction::Rename),
        (
            matches!(wallet.signing, WalletSigning::SeedPhrase { .. }),
            WalletAction::RevealPhrase,
        ),
        (
            !super::wallet_keys::exportable_keys(wallet).is_empty(),
            WalletAction::ExportKeys,
        ),
        (true, WalletAction::Delete),
    ]
    .into_iter()
    .filter_map(|(offered, action)| offered.then_some(action))
    .collect()
}

/// The setup methods that give a watched wallet its keys: a phrase always,
/// and a key where the wallet watches one address rather than an account,
/// each as the network offers it. Empty for a wallet that has its keys.
pub(crate) fn upgrade_methods(wallet: &WalletState) -> Vec<WalletSetupMethod> {
    // Only a UTXO multisig account holds a cosigner's phrase itself; the
    // others' members sign from wallets of their own.
    if !wallet.signing.is_watch_only()
        || (wallet.multisig_policy.is_some() && wallet.chain_id.utxo_multisig_script().is_none())
    {
        return Vec::new();
    }
    let descriptor = wallet_setup_descriptor(wallet.chain_id);
    [
        WalletSetupMethod::ImportPhrase,
        WalletSetupMethod::ImportPrivateKey,
    ]
    .into_iter()
    .filter(|method| descriptor.option(*method).is_some())
    // A key holds one address; a watched account, single-key or multisig,
    // takes a phrase.
    .filter(|method| {
        *method == WalletSetupMethod::ImportPhrase
            || (wallet.xpub.is_none() && wallet.multisig_policy.is_none())
    })
    .collect()
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The ways a watched wallet can be given its keys, for the setup page
    /// its Add Keys action opens; empty once it has them.
    pub async fn wallet_upgrade_methods(
        &self,
        wallet_id: String,
    ) -> Result<Vec<WalletSetupMethod>, SpectraBridgeError> {
        let wallet = self
            .wallet_state
            .read()
            .await
            .wallets
            .iter()
            .find(|wallet| wallet.id == wallet_id)
            .cloned()
            .ok_or_else(|| SpectraBridgeError::failure("Wallet removed"))?;
        Ok(upgrade_methods(&wallet))
    }

    /// What the wallet's page offers, with each action's note, and its
    /// network's capabilities and limits. Contacts nothing.
    pub async fn wallet_actions(
        &self,
        wallet_id: String,
    ) -> Result<WalletActions, SpectraBridgeError> {
        let wallet = self
            .wallet_state
            .read()
            .await
            .wallets
            .iter()
            .find(|wallet| wallet.id == wallet_id)
            .cloned()
            .ok_or_else(|| SpectraBridgeError::failure("Wallet removed"))?;
        let summary = self.wallet_setup_summary(wallet.chain_id).await;
        Ok(WalletActions {
            wallet_id,
            chain: wallet.chain_id,
            actions: offered_actions(&wallet)
                .into_iter()
                .map(|action| WalletActionOffer {
                    action,
                    section: action.section(),
                    note: action.note().to_string(),
                })
                .collect(),
            summary,
        })
    }
}

#[cfg(test)]
#[path = "tests/wallet_actions.rs"]
mod tests;
