use super::keys::{StealthAddress, ViewKeys};
use super::output::{self, OwnedOutput};
use super::primitives::{self, Commitment};
use super::transaction::{self, Plan, Spend, verify_input, verify_kernel, verify_output};
use crate::api::litecoin_p2p::wire::{
    self as wire, HogEx, MwebHeader, Output, OutputMessage, PegOut, RangeProof, Reader, TxBody,
};
use crate::registry::Chain;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/mweb-mainnet.json")).unwrap()
}

fn bytes(value: &serde_json::Value) -> Vec<u8> {
    hex::decode(value.as_str().unwrap()).unwrap()
}

/// Every input, output and kernel of six mainnet blocks verifies — each
/// signature, each output's bulletproof — and re-encodes byte for byte;
/// each peg-in kernel is the one its canonical output names, and each
/// block's HogEx commits to its MWEB header.
#[test]
fn mainnet_blocks_verify() {
    let fixture = fixture();
    let (mut inputs, mut outputs, mut kernels, mut pegins) = (0, 0, 0, 0);
    for block in fixture["blocks"].as_array().unwrap() {
        let raw = bytes(&block["mweb_body"]);
        let mut reader = Reader(&raw);
        let body = TxBody::decode(&mut reader).unwrap();
        reader.finished().unwrap();
        let mut encoded = Vec::new();
        body.encode(&mut encoded);
        assert_eq!(encoded, raw);
        assert!(body.inputs.iter().all(verify_input), "{}", block["height"]);
        assert!(
            body.outputs.iter().all(verify_output),
            "{}",
            block["height"]
        );
        assert!(
            body.kernels.iter().all(verify_kernel),
            "{}",
            block["height"]
        );
        let mut sorted = body.clone();
        sorted.sort();
        assert_eq!(sorted, body, "a block's parts are in Litecoin Core's order");
        inputs += body.inputs.len();
        outputs += body.outputs.len();
        kernels += body.kernels.len();
        let canonical: Vec<bitcoin::Transaction> = block["pegin_txs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tx| bitcoin::consensus::deserialize(&bytes(tx)).unwrap())
            .collect();
        for kernel in body.kernels.iter().filter(|k| k.pegin.is_some()) {
            let script = wire::pegin_script(&kernel.id());
            assert!(canonical.iter().any(|tx| tx.output.iter().any(|out| {
                out.script_pubkey.as_bytes() == script.as_slice()
                    && out.value.to_sat() == kernel.pegin.unwrap()
            })));
            pegins += 1;
        }
        let raw = bytes(&block["hogex"]);
        let mut reader = Reader(&raw);
        let hogex = HogEx::decode(&mut reader).unwrap();
        reader.finished().unwrap();
        let raw = bytes(&block["mweb_header"]);
        let mut reader = Reader(&raw);
        let header = MwebHeader::decode(&mut reader).unwrap();
        reader.finished().unwrap();
        assert_eq!(header.height, block["height"].as_u64().unwrap());
        assert_eq!(
            hogex.outputs[0].script_pubkey.as_bytes(),
            wire::hogaddr_script(&header.hash()).as_slice()
        );
    }
    assert_eq!((inputs, outputs, kernels, pegins), (9, 9, 8, 3));
}

/// A bit changed anywhere a signature or proof covers is caught.
#[test]
fn a_changed_part_fails_verification() {
    let fixture = fixture();
    let block = &fixture["blocks"][0];
    let raw = bytes(&block["mweb_body"]);
    let body = TxBody::decode(&mut Reader(&raw)).unwrap();
    let mut output = body.outputs[0].clone();
    let fields = output.message.standard.as_mut().unwrap();
    fields.masked_value ^= 1;
    assert!(!verify_output(&output));
    let mut kernel = body.kernels[0].clone();
    kernel.fee = kernel.fee.map(|fee| fee + 1);
    assert!(!verify_kernel(&kernel));
    let mut output = body.outputs[1].clone();
    output.signature[63] ^= 1;
    assert!(!verify_output(&output));
}

/// The kernels' fees are ltcd's estimate: 100 litoshis a unit of weight
/// (three for a stealth kernel, eighteen an output, one per 42 bytes of a
/// peg-out script) and the peg-outs' bytes at the fee rate.
#[test]
fn fees_are_what_mainnet_kernels_paid() {
    let pegout = |script: usize| PegOut {
        amount: 1,
        script: vec![0; script],
    };
    // 3191349: two outputs, no peg-out.
    assert_eq!(transaction::fee(2, &[], 0).unwrap(), 3_900);
    // 3191353: a 22-byte peg-out and no output, at 2,000 litoshis a kB.
    assert_eq!(transaction::fee(0, &[pegout(22)], 2_000).unwrap(), 462);
    // 3191346: one output and a 22-byte peg-out, at 10,000 a kB.
    assert_eq!(transaction::fee(1, &[pegout(22)], 10_000).unwrap(), 2_510);
}

/// ltcd's vector: the deterministic sender signature, rewinding with the
/// scan key, the address at index 0 and its testnet spelling.
#[test]
fn ltcd_vector_rewinds_to_address_zero() {
    let fixture = fixture();
    let vector = &fixture["ltcd_vector"];
    let raw = bytes(&vector["output"]);
    assert_eq!(raw.len(), 286);
    // ltcd writes a hash-only proof as 32 zero bytes, then the hash.
    let mut head = raw[..158].to_vec();
    head.extend(&raw[190..222]);
    head.extend(&raw[222..]);
    let output = Output::decode_compact(&mut Reader(&head)).unwrap();
    assert!(raw[158..190].iter().all(|byte| *byte == 0));
    let key = |name: &str| primitives::secret(bytes(&vector[name]).try_into().unwrap()).unwrap();
    let sender = key("sender_secret");
    assert_eq!(primitives::public(&sender), output.sender);
    assert_eq!(
        primitives::sign(&sender, &output.signature_message()).unwrap(),
        output.signature
    );
    assert!(verify_output(&output));
    let view = ViewKeys {
        scan: key("scan_secret"),
        spend: primitives::public(&key("spend_secret")),
    };
    let addresses = view.spend_keys().unwrap();
    let owned = output::rewind(&output, &view, &addresses).unwrap();
    assert_eq!(owned.value, vector["value"].as_u64().unwrap());
    assert_eq!(owned.address_index, 0);
    let address = view.address(0).unwrap();
    let spelled = vector["address_index_0_testnet"].as_str().unwrap();
    assert_eq!(address.encode(Chain::LitecoinTestnet).unwrap(), spelled);
    assert_eq!(
        StealthAddress::decode(Chain::LitecoinTestnet, spelled),
        Some(address)
    );
    assert_eq!(StealthAddress::decode(Chain::Litecoin, spelled), None);
    assert_eq!(
        StealthAddress::decode(Chain::LitecoinTestnet, &spelled.to_uppercase()),
        Some(address)
    );
    // Another wallet's scan key finds nothing; so does this one's when the
    // output pays a spend key it does not hold.
    let stranger = ViewKeys {
        scan: key("sender_secret"),
        spend: view.spend,
    };
    assert!(output::rewind(&output, &stranger, &addresses).is_none());
    let mut fewer = addresses.clone();
    fewer.remove(&address.spend);
    assert!(output::rewind(&output, &view, &fewer).is_none());
}

fn wallet(seed_byte: u8) -> (ViewKeys, [u8; 32]) {
    let (view, spend) = ViewKeys::from_seed(&[seed_byte; 64]).unwrap();
    (view, *spend)
}

/// Fund `view`'s address `index` with `value`, as another wallet would.
fn funded(view: &ViewKeys, spend: &[u8; 32], index: u32, value: u64) -> Spend {
    let created = output::create_output(
        &view.address(index).unwrap(),
        value,
        &primitives::secret([0x42; 32]).unwrap(),
    )
    .unwrap();
    let coin = output::rewind(&created.output, view, &view.spend_keys().unwrap()).unwrap();
    let address_secret =
        super::keys::address_spend_secret(view, spend, coin.address_index).unwrap();
    Spend {
        output_secret: coin.output_secret(&address_secret).unwrap(),
        coin,
    }
}

/// A payment inside MWEB: two outputs spent, the recipient paid, change
/// returned; it verifies as a node checks it, and each side rewinds what
/// it was paid.
#[test]
fn a_payment_balances_and_each_side_finds_its_output() {
    let (alice, alice_spend) = wallet(1);
    let (bob, _) = wallet(2);
    let spends = vec![
        funded(&alice, &alice_spend, 2, 60_000_000),
        funded(&alice, &alice_spend, 3, 50_000_000),
    ];
    let fee = transaction::fee(2, &[], 0).unwrap();
    let paid = 100_000_000;
    let plan = Plan {
        spends,
        recipients: vec![
            (bob.address(2).unwrap(), paid),
            (alice.address(0).unwrap(), 110_000_000 - paid - fee),
        ],
        fee,
        pegin: 0,
        pegouts: vec![],
    };
    let built = transaction::build(&plan).unwrap();
    transaction::verify(&built).unwrap();
    let found = |view: &ViewKeys| -> Vec<OwnedOutput> {
        let keys = view.spend_keys().unwrap();
        built
            .body
            .outputs
            .iter()
            .filter_map(|o| output::rewind(o, view, &keys))
            .collect()
    };
    let bob_found = found(&bob);
    assert_eq!(
        bob_found
            .iter()
            .map(|c| (c.value, c.address_index))
            .collect::<Vec<_>>(),
        [(paid, 2)]
    );
    let alice_found = found(&alice);
    assert_eq!(
        alice_found
            .iter()
            .map(|c| (c.value, c.address_index))
            .collect::<Vec<_>>(),
        [(110_000_000 - paid - fee, 0)]
    );
    // The serialization round-trips.
    let mut raw = Vec::new();
    built.encode(&mut raw);
    assert_eq!(wire::MwebTx::decode(&mut Reader(&raw)).unwrap(), built);

    // What a node refuses: a changed fee, a dropped output, an offset off by one.
    let mut tampered = built.clone();
    tampered.body.kernels[0].fee = Some(fee + 1);
    assert!(transaction::verify(&tampered).is_err());
    let mut tampered = built.clone();
    tampered.body.outputs.pop();
    assert!(transaction::verify(&tampered).is_err());
    let mut tampered = built.clone();
    tampered.kernel_offset[31] ^= 1;
    assert!(transaction::verify(&tampered).is_err());
    let mut tampered = built.clone();
    tampered.stealth_offset[31] ^= 1;
    assert!(transaction::verify(&tampered).is_err());
}

/// A peg-in brings value in with no input; a peg-out takes it to a script.
#[test]
fn peg_ins_and_peg_outs_balance() {
    let (alice, alice_spend) = wallet(3);
    let fee = transaction::fee(1, &[], 0).unwrap();
    let pegin = Plan {
        spends: vec![],
        recipients: vec![(alice.address(1).unwrap(), 5_000_000)],
        fee,
        pegin: 5_000_000 + fee,
        pegouts: vec![],
    };
    let built = transaction::build(&pegin).unwrap();
    transaction::verify(&built).unwrap();
    assert_eq!(built.body.kernels[0].pegin, Some(5_000_000 + fee));

    let pegouts = vec![PegOut {
        amount: 20_000_000,
        script: vec![0x00, 0x14].into_iter().chain([7; 20]).collect(),
    }];
    let fee = transaction::fee(1, &pegouts, 10_000).unwrap();
    let pegout = Plan {
        spends: vec![funded(&alice, &alice_spend, 2, 30_000_000)],
        recipients: vec![(alice.address(0).unwrap(), 10_000_000 - fee)],
        fee,
        pegin: 0,
        pegouts,
    };
    let built = transaction::build(&pegout).unwrap();
    transaction::verify(&built).unwrap();
    // Every input and output gone: a peg-out of the whole balance.
    let fee = transaction::fee(0, &pegout.pegouts, 10_000).unwrap();
    let all_out = Plan {
        spends: vec![funded(&alice, &alice_spend, 5, 20_000_000 + fee)],
        recipients: vec![],
        fee,
        pegin: 0,
        pegouts: pegout.pegouts.clone(),
    };
    transaction::verify(&transaction::build(&all_out).unwrap()).unwrap();
    // Amounts that do not balance build nothing.
    let unbalanced = Plan {
        spends: vec![funded(&alice, &alice_spend, 2, 1_000)],
        recipients: vec![(alice.address(0).unwrap(), 1_000)],
        fee: 1,
        pegin: 0,
        pegouts: vec![],
    };
    assert!(transaction::build(&unbalanced).is_err());
}

/// A commitment's prefix is y's residuosity, and parsing restores the point.
#[test]
fn commitments_round_trip() {
    for seed in 1u8..20 {
        let blind = primitives::secret([seed; 32]).unwrap();
        let commitment = primitives::commit(&blind, u64::from(seed) * 1_000).unwrap();
        assert!(matches!(commitment.0[0], 8 | 9));
        assert_eq!(
            primitives::commitment_of(&primitives::commitment_point(&commitment).unwrap()),
            commitment
        );
    }
    assert!(primitives::commitment_point(&Commitment([2; 33])).is_err());
    let _ = OutputMessage {
        standard: None,
        extra: None,
    }
    .encode();
    let _ = RangeProof::Hash([0; 32]).hash();
}

fn decode_signed(raw: &[u8]) -> wire::ExtendedTransaction {
    let mut reader = Reader(raw);
    let tx = wire::ExtendedTransaction::decode(&mut reader).unwrap();
    reader.finished().unwrap();
    tx
}

/// A reviewed payment out of MWEB funds, signed: the transaction is MWEB
/// only, its id its kernel's, and it pays the recipient, returns the change
/// and pays at least its weight's fee; a peg-out takes the amount to the
/// recipient's script.
#[test]
fn a_reviewed_payment_signs_what_it_says() {
    use super::prepared::{plan_spend, sign_spend};
    let chain = Chain::LitecoinTestnet;
    let (alice, alice_spend) = wallet(4);
    let (bob, _) = wallet(5);
    let coins: Vec<OwnedOutput> = [(2, 70_000_000), (3, 40_000_000), (7, 1_000)]
        .into_iter()
        .map(|(index, value)| funded(&alice, &alice_spend, index, value).coin)
        .collect();
    let by_id = coins
        .iter()
        .map(|coin| (coin.output_id, coin.clone()))
        .collect();
    let to_bob = bob.address(2).unwrap().encode(chain).unwrap();

    let prepared = plan_spend(chain, &coins, &to_bob, 100_000_000).unwrap();
    assert_eq!(prepared.inputs.len(), 2, "largest first");
    assert_eq!(prepared.fee, transaction::fee(2, &[], 0).unwrap());
    assert_eq!(prepared.change, 110_000_000 - 100_000_000 - prepared.fee);
    let signed = sign_spend(chain, &prepared, &by_id, &alice, &alice_spend).unwrap();
    let tx = decode_signed(&signed.raw);
    assert!(tx.canonical.input.is_empty() && tx.canonical.output.is_empty());
    let mweb = tx.mweb.unwrap();
    transaction::verify(&mweb).unwrap();
    let mut id = mweb.body.kernels[0].id();
    id.reverse();
    assert_eq!(signed.txid, hex::encode(id));
    assert_eq!(
        transaction::mweb_outputs(&signed.raw).unwrap(),
        mweb.body.outputs
    );
    let rewound = |view: &ViewKeys| -> Vec<(u64, u32)> {
        let keys = view.spend_keys().unwrap();
        mweb.body
            .outputs
            .iter()
            .filter_map(|o| output::rewind(o, view, &keys))
            .map(|coin| (coin.value, coin.address_index))
            .collect()
    };
    assert_eq!(rewound(&bob), [(100_000_000, 2)]);
    assert_eq!(rewound(&alice), [(prepared.change, 0)]);

    // A peg-out: the amount leaves by the kernel to the recipient's script.
    let to_script = crate::derivation::litecoin::encode_litecoin_address(
        chain,
        crate::derivation::types::BitcoinScriptType::P2wpkh,
        &secp256k1::PublicKey::from_secret_key(
            secp256k1::SECP256K1,
            &secp256k1::SecretKey::from_slice(&[9; 32]).unwrap(),
        ),
    )
    .unwrap();
    let script = crate::derivation::utxo_address::parse_utxo_address(chain, &to_script)
        .unwrap()
        .script_pubkey();
    let pegout = plan_spend(chain, &coins, &to_script, 50_000_000).unwrap();
    let signed = sign_spend(chain, &pegout, &by_id, &alice, &alice_spend).unwrap();
    let mweb = decode_signed(&signed.raw).mweb.unwrap();
    transaction::verify(&mweb).unwrap();
    assert_eq!(
        mweb.body.kernels[0].pegouts,
        [PegOut {
            amount: 50_000_000,
            script
        }]
    );
    assert!(pegout.fee >= transaction::fee(1, &mweb.body.kernels[0].pegouts, 10_000).unwrap());

    // What is refused: more than the balance, dust to a script, a payment
    // whose review was changed, and an output that is no longer the wallet's.
    assert!(plan_spend(chain, &coins, &to_bob, 110_000_000).is_err());
    assert!(plan_spend(chain, &coins, &to_script, 100).is_err());
    assert!(plan_spend(chain, &coins, &to_bob, 0).is_err());
    let mut changed = prepared.clone();
    changed.fee -= 1;
    changed.change += 1;
    assert!(sign_spend(chain, &changed, &by_id, &alice, &alice_spend).is_err());
    let mut doubled = prepared.clone();
    doubled.inputs.push(doubled.inputs[0].clone());
    assert!(doubled.check(chain).is_err());
    let mut elsewhere = prepared.clone();
    elsewhere.recipient = bob.address(2).unwrap().encode(Chain::Litecoin).unwrap();
    assert!(elsewhere.check(chain).is_err());
    let mut gone = by_id.clone();
    gone.remove(&prepared.inputs[0].output_id);
    assert!(sign_spend(chain, &prepared, &gone, &alice, &alice_spend).is_err());
}

/// A peg-in, signed: its canonical output is the script its kernel makes,
/// carrying the amount and the kernel's fee, as Litecoin Core relays a
/// transaction whose peg-in kernels are exactly its MWEB kernels; its id is
/// the canonical part's.
#[test]
fn a_peg_in_pays_its_kernel_script() {
    use super::prepared::{CanonicalRecipient, PreparedPegIn, sign_pegin};
    use crate::derivation::types::BitcoinScriptType;
    let chain = Chain::Litecoin;
    let (alice, _) = wallet(6);
    let address = alice
        .address(super::keys::PEGIN_INDEX)
        .unwrap()
        .encode(chain)
        .unwrap();
    let key = secp256k1::SecretKey::from_slice(&[1; 32]).unwrap();
    let sender = crate::derivation::litecoin::encode_litecoin_address(
        chain,
        BitcoinScriptType::P2wpkh,
        &secp256k1::PublicKey::from_secret_key(secp256k1::SECP256K1, &key),
    )
    .unwrap();
    let script = crate::derivation::utxo_address::parse_utxo_address(chain, &sender)
        .unwrap()
        .script_pubkey();
    let utxo = ("11".repeat(32), 0, 10_000_000, script.clone());
    let prepared = PreparedPegIn {
        inputs: vec![crate::send::stages::UtxoPreparedInput {
            source: crate::send::stages::UtxoSendSource {
                address: sender.clone(),
                derivation_path: None,
                script_pubkey: script,
            },
            utxo: utxo.clone(),
        }],
        recipient: address.clone(),
        amount: 3_000_000,
        mweb_fee: PreparedPegIn::kernel_fee().unwrap(),
        canonical_fee: 500,
    };
    assert_eq!(prepared.mweb_fee, 2_100);
    let signed = sign_pegin(chain, &prepared, |to, value| {
        crate::send::litecoin::sign_ltc_with_output_script(
            chain,
            std::slice::from_ref(&utxo),
            to,
            value,
            prepared.canonical_fee,
            &sender,
            &key.secret_bytes(),
        )
    })
    .unwrap();
    let tx = decode_signed(&signed.raw);
    let mweb = tx.mweb.clone().unwrap();
    transaction::verify(&mweb).unwrap();
    let kernel = &mweb.body.kernels[0];
    assert_eq!(kernel.pegin, Some(3_002_100));
    let pegins: Vec<_> = tx
        .canonical
        .output
        .iter()
        .filter(|out| out.script_pubkey.as_bytes().starts_with(&[0x59, 0x20]))
        .collect();
    assert_eq!(pegins.len(), 1);
    assert_eq!(
        pegins[0].script_pubkey.as_bytes(),
        wire::pegin_script(&kernel.id())
    );
    assert_eq!(pegins[0].value.to_sat(), 3_002_100);
    assert_eq!(signed.txid, tx.canonical.compute_txid().to_string());
    let paid: u64 = tx
        .canonical
        .output
        .iter()
        .map(|out| out.value.to_sat())
        .sum();
    assert_eq!(10_000_000 - paid, prepared.canonical_fee);
    // The peg-in's output is the wallet's, at the address it keeps for them.
    let keys = alice.spend_keys().unwrap();
    let coin = output::rewind(&mweb.body.outputs[0], &alice, &keys).unwrap();
    assert_eq!(
        (coin.value, coin.address_index),
        (3_000_000, super::keys::PEGIN_INDEX)
    );
    // The canonical output is as long as the quote assumed, and at least the
    // dust threshold for the least amount a quote allows.
    let recipient = CanonicalRecipient::parse(chain, &address).unwrap();
    assert_eq!(recipient.script_len(), pegins[0].script_pubkey.len());
    let least = recipient.minimum_amount(chain).unwrap();
    assert_eq!(
        least + prepared.mweb_fee,
        crate::send::litecoin::litecoin_dust_threshold(chain, pegins[0].script_pubkey.as_bytes())
            .unwrap()
    );
    // A peg-in to another network's address, or one whose kernel pays less
    // than its weight, is refused.
    let mut other = prepared.clone();
    other.recipient = alice
        .address(1)
        .unwrap()
        .encode(Chain::LitecoinTestnet)
        .unwrap();
    assert!(other.check(chain).is_err());
    let mut cheap = prepared.clone();
    cheap.mweb_fee -= 1;
    assert!(cheap.check(chain).is_err());
}
