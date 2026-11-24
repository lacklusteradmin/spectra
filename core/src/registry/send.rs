//! How a send on a network is previewed, shaped and requested.

use super::*;

impl Chain {
    /// The chain's send preview comes from the shared path: its coin's fee
    /// and balance, through one entry point.
    ///
    /// The UTXO family, Tron and the EVM family each have a preview path of
    /// their own, which take different inputs. A testnet quotes like its
    /// mainnet; which network is reached is the chain id's business.
    pub fn has_shared_send_preview(self) -> bool {
        self.is_cosmos()
            || self.is_substrate()
            || matches!(
                self.mainnet_counterpart(),
                Chain::Solana
                    | Chain::Xrp
                    | Chain::Stellar
                    | Chain::Monero
                    | Chain::Cardano
                    | Chain::Sui
                    | Chain::Aptos
                    | Chain::Ton
                    | Chain::Icp
                    | Chain::Near
                    | Chain::Algorand
                    | Chain::Vechain
                    | Chain::Filecoin
                    | Chain::Tezos
                    | Chain::Starknet
                    | Chain::Hedera
            )
    }

    pub fn has_send_preview(self) -> bool {
        // The EVM family and the chains with a preview path of their own —
        // everything that does not go through the generic submit — always have
        // one. The rest need either a shared-path shape or a fee fallback.
        self.is_evm()
            || self.uses_account_utxo()
            || !self.uses_generic_send_submit()
            || self.has_shared_send_preview()
            || self.send_execution_shape().fee_fallback.is_some()
    }

    /// This chain's send builder can sign a transaction and stop, without
    /// putting it on the chain.
    ///
    /// Capabilities follow the staged builder; unsupported protocols refuse early.
    pub fn supports_sign_only(self) -> bool {
        self.has_send_preview()
    }

    /// How a payment request to an address on this network is written for
    /// a QR code: the format wallets read, with the amount and memo in it.
    /// `None` where no format is widely read, and the address is shared alone.
    pub fn payment_uri_format(self) -> Option<PaymentUriFormat> {
        use PaymentUriFormat::*;
        if self.is_evm() {
            return self.evm_chain_id().is_ok().then_some(Eip681);
        }
        match self.mainnet_counterpart() {
            Self::Bitcoin => Some(Bip21("bitcoin")),
            Self::Litecoin => Some(Bip21("litecoin")),
            Self::Dogecoin => Some(Bip21("dogecoin")),
            Self::BitcoinCash => Some(Bip21("bitcoincash")),
            Self::Dash => Some(Bip21("dash")),
            Self::Zcash => Some(Bip21("zcash")),
            Self::Peercoin => Some(Bip21("peercoin")),
            Self::Qtum => Some(Bip21("qtum")),
            Self::Ecash => Some(Bip21("ecash")),
            Self::Digibyte => Some(Bip21("digibyte")),
            Self::Solana => Some(SolanaPay),
            Self::Xrp => Some(Xrpl),
            Self::Stellar => Some(Sep7),
            Self::Monero => Some(MoneroUri),
            Self::Ton => Some(TonTransfer),
            _ => None,
        }
    }

    /// What a payment on this network can say about whose deposit it is,
    /// the first the default: XRP's destination tag, Stellar's text and ID
    /// memos, TON's text comment, a Cosmos or Hedera transaction's memo. Empty where
    /// payments carry none.
    pub fn payment_memo_kinds(self) -> &'static [PaymentMemoKind] {
        match self.mainnet_counterpart() {
            cosmos if cosmos.is_cosmos() => &[PaymentMemoKind::MemoText],
            Self::Xrp => &[PaymentMemoKind::DestinationTag],
            Self::Stellar => &[PaymentMemoKind::MemoText, PaymentMemoKind::MemoId],
            Self::Ton => &[PaymentMemoKind::Comment],
            Self::Hedera => &[PaymentMemoKind::MemoText],
            _ => &[],
        }
    }

    /// Whether this chain's native send needs nothing beyond a destination, an
    /// amount and the fee its preview already supplied.
    ///
    /// The mainnets that answer no are the EVM family, which needs a nonce and
    /// gas overrides, and five that each need something only they have: a UTXO
    /// selection (Bitcoin, Dogecoin), a resolved source account (Internet
    /// Computer), a resource model (Tron) and a mint account (Solana).
    ///
    /// It is a chain fact and it lived as two lists of names in
    /// `AppState+SendExecution`, next to a comment saying the lists should not
    /// be there. A chain reaching the shared path had to be added to whichever
    /// of the two the author happened to be looking at.
    pub fn uses_generic_send_submit(self) -> bool {
        self.is_cosmos()
            || self.is_substrate()
            || matches!(
                self.mainnet_counterpart(),
                Chain::Sui
                    | Chain::Aptos
                    | Chain::Ton
                    | Chain::Xrp
                    | Chain::Stellar
                    | Chain::Cardano
                    | Chain::Near
                    | Chain::BitcoinCash
                    | Chain::BitcoinSV
                    | Chain::Litecoin
                    | Chain::Zcash
                    | Chain::BitcoinGold
                    | Chain::Decred
                    | Chain::Kaspa
                    | Chain::Dash
                    | Chain::Peercoin
                    | Chain::Algorand
                    | Chain::Qtum
                    | Chain::FractalBitcoin
                    | Chain::Ecash
                    | Chain::Digibyte
                    | Chain::Vechain
                    | Chain::Filecoin
                    | Chain::Tezos
                    | Chain::Starknet
                    | Chain::Hedera
                    // It has a shared-path preview like the rest.
                    | Chain::Monero
            )
    }

    /// How this chain's fee enters a send, and the fee to assume without a
    /// preview.
    pub fn send_execution_shape(self) -> SendExecutionShape {
        let chain = self.mainnet_counterpart();
        match chain {
            // The fee a simulation priced at the network's gas price.
            cosmos if cosmos.is_cosmos() => SendExecutionShape {
                fee_field: SendFeeField::FeeAmount,
                fee_fallback: None,
            },
            Chain::Sui => SendExecutionShape {
                fee_field: SendFeeField::GasBudget,
                fee_fallback: None,
            },
            // Hedera's is the transaction's `transactionFee`: the most it
            // pays, at the network's exchange rate.
            // Starknet's is the most its resource bounds let the network
            // charge, for the deployment and the transfer together.
            // Tezos's fee amount is what the batch pays bakers and burns for
            // storage at most, from a node's simulation.
            // VeChainThor's is the VTHO a simulation's gas costs at the
            // block's base fee and the suggested tip.
            Chain::Cardano
            | Chain::Aptos
            | Chain::Near
            | Chain::Algorand
            | Chain::Vechain
            | Chain::Filecoin
            | Chain::Tezos
            | Chain::Starknet
            | Chain::Hedera => SendExecutionShape {
                fee_field: SendFeeField::FeeAmount,
                fee_fallback: None,
            },
            // The fee a node quoted for the extrinsic itself.
            substrate if substrate.is_substrate() => SendExecutionShape {
                fee_field: SendFeeField::FeeAmount,
                fee_fallback: None,
            },
            Chain::Bitcoin | Chain::BitcoinCash | Chain::BitcoinSV => SendExecutionShape {
                fee_field: SendFeeField::FeeSats,
                fee_fallback: Some("0.00001"),
            },
            Chain::Peercoin
            | Chain::Qtum
            | Chain::FractalBitcoin
            | Chain::Ecash
            | Chain::Digibyte => SendExecutionShape {
                fee_field: SendFeeField::FeeSats,
                fee_fallback: None,
            },
            Chain::Litecoin => SendExecutionShape {
                fee_field: SendFeeField::FeeSats,
                fee_fallback: Some("0.0001"),
            },
            // The five whose send existed but was unroutable. `fee_fallback`
            // is the default `build_send` already applies when the request
            // carries no `fee_sat`, in the chain's own units — so the fee the
            // sheet shows and validates against is the fee core will use.
            // None of them has a shared-path preview, and without a fallback
            // the generic submit refuses for want of an estimate.
            Chain::Zcash | Chain::BitcoinGold | Chain::Kaspa => SendExecutionShape {
                fee_field: SendFeeField::FeeSats,
                fee_fallback: Some("0.00001"),
            },
            Chain::Decred | Chain::Dash => SendExecutionShape {
                fee_field: SendFeeField::FeeSats,
                fee_fallback: Some("0.00002"),
            },
            _ => SendExecutionShape {
                fee_field: SendFeeField::None,
                fee_fallback: None,
            },
        }
    }

    /// The most bytes of UTF-8 a text memo on this network holds: Stellar's
    /// `MEMO_TEXT`, a Cosmos network's `max_memo_characters` (which the SDK
    /// counts in bytes) and a Hedera transaction's memo. `None` where
    /// payments carry no text memo.
    pub fn memo_text_limit(self) -> Option<usize> {
        if let Some(network) = self.cosmos() {
            return Some(network.memo_limit);
        }
        match self.mainnet_counterpart() {
            Self::Stellar => Some(28),
            Self::Hedera => Some(super::hedera::MEMO_MAX_BYTES),
            _ => None,
        }
    }

    /// The token this network's fees are paid in, where that is not its own
    /// coin: VTHO on VeChainThor, which pays for the gas a VET transfer
    /// uses. `None` wherever a coin pays its own fees.
    pub(crate) fn fee_token_contract(self) -> Option<&'static str> {
        self.thor().map(|network| network.energy)
    }

    /// Whether sending the asset `contract` names (`None`, the network's own
    /// coin) pays the fee out of the same balance: a coin that pays its own
    /// fees, or VTHO.
    pub(crate) fn pays_own_fee(self, contract: Option<&str>) -> bool {
        match (self.fee_token_contract(), contract) {
            (None, contract) => contract.is_none(),
            (Some(_), None) => false,
            (Some(fee), Some(contract)) => {
                crate::catalog::tokens::normalize_token_identifier(Some(contract.into()), self)
                    .as_deref()
                    == Some(fee)
            }
        }
    }

    /// Whether the ledger creates an account only once it holds the
    /// network's reserve, so a first payment below it fails: XRP's and
    /// Stellar's reserves, Algorand's minimum balance.
    pub fn requires_account_reserve(self) -> bool {
        matches!(
            self.mainnet_counterpart(),
            Self::Xrp | Self::Stellar | Self::Algorand
        )
    }

    /// Protocol fee estimates in native smallest units. Testnets share the
    /// family's estimate; endpoints still belong to the concrete network.
    pub fn static_fee_units(self) -> Option<u128> {
        if let Some(network) = self.cosmos() {
            return network
                .gas_price
                .fee_for(super::cosmos::TRANSFER_GAS_ESTIMATE);
        }
        match self.mainnet_counterpart() {
            Chain::Solana => Some(5_000),
            Chain::Tron => Some(1_000_000),
            Chain::Cardano => Some(170_000),
            Chain::Sui => Some(1_000),
            Chain::Ton => Some(7_000_000),
            Chain::Icp => Some(10_000),
            Chain::Zcash => Some(10_000),
            Chain::Monero => Some(500_000_000),
            Chain::Dogecoin => Some(1_000_000),
            Chain::Litecoin | Chain::BitcoinSV | Chain::BitcoinGold | Chain::Kaspa => Some(1_000),
            Chain::BitcoinCash | Chain::Decred | Chain::Dash => Some(2_000),
            // Per started kilobyte: Bitcoin ABC's minimum relay rate, which
            // Cashtab pays.
            Chain::Ecash => Some(1_000),
            _ => None,
        }
    }
}
