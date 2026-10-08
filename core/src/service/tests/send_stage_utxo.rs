//! Account input selection and each network's fee rule.

use super::*;

fn input(index: u8, value: u64, script: &[u8]) -> UtxoPreparedInput {
    UtxoPreparedInput {
        source: UtxoSendSource {
            address: format!("a{index}"),
            derivation_path: None,
            script_pubkey: script.to_vec(),
        },
        utxo: (
            hex::encode([index; 32]),
            u32::from(index),
            value,
            script.to_vec(),
        ),
    }
}

const P2PKH: [u8; 25] = {
    let mut script = [0u8; 25];
    script[0] = 0x76;
    script[1] = 0xa9;
    script[2] = 0x14;
    script[23] = 0x88;
    script[24] = 0xac;
    script
};

/// The largest outputs are spent first, only as many as the amount and
/// their fee need; change below the dust floor joins the fee; a reviewed fee
/// below what the inputs now take is refused, and too little is refused.
#[test]
fn selection_spends_the_largest_outputs_it_needs() {
    let policy = FeePolicy::PerKilobyte(1_000);
    let candidates = vec![
        input(1, 5_000, &P2PKH),
        input(2, 50_000, &P2PKH),
        input(3, 20_000, &P2PKH),
    ];
    let selection = select(
        candidates.clone(),
        60_000,
        &policy,
        &P2PKH,
        &P2PKH,
        546,
        None,
    )
    .unwrap();
    assert_eq!(
        selection
            .inputs
            .iter()
            .map(|input| input.utxo.2)
            .collect::<Vec<_>>(),
        [50_000, 20_000]
    );
    // Two inputs and two outputs fit one kilobyte.
    assert_eq!((selection.fee, selection.change), (1_000, 9_000));
    // 70,000 - 69,000 - 1,000 leaves nothing; 69,500 leaves 500, dust.
    let exact = select(
        candidates.clone(),
        69_000,
        &policy,
        &P2PKH,
        &P2PKH,
        546,
        None,
    )
    .unwrap();
    assert_eq!((exact.fee, exact.change), (1_000, 0));
    let dust = select(
        candidates.clone(),
        68_500,
        &policy,
        &P2PKH,
        &P2PKH,
        546,
        None,
    )
    .unwrap();
    assert_eq!((dust.fee, dust.change), (1_500, 0));
    assert!(
        select(
            candidates.clone(),
            74_500,
            &policy,
            &P2PKH,
            &P2PKH,
            546,
            None
        )
        .is_err()
    );
    // A reviewed fee: at least the policy's, and paid as reviewed.
    let fixed = select(
        candidates.clone(),
        60_000,
        &policy,
        &P2PKH,
        &P2PKH,
        546,
        Some(2_000),
    )
    .unwrap();
    assert_eq!((fixed.fee, fixed.change), (2_000, 8_000));
    assert!(select(candidates, 60_000, &policy, &P2PKH, &P2PKH, 546, Some(999)).is_err());
}

/// Each network's fee follows its size: Bitcoin's rate times the virtual
/// size, the legacy networks' fee per started kilobyte, ZIP-317's logical
/// actions, Decred's ten atoms a byte and Kaspa's mass, each at least the
/// network's static fee.
#[test]
fn fees_follow_each_networks_rule() {
    let shape = |inputs: usize| Shape {
        inputs: vec![P2PKH.as_slice(); inputs],
        outputs: vec![25, 25],
    };
    let size = |policy: &FeePolicy, inputs| policy.size(&shape(inputs)).unwrap();
    let fee = |policy: &FeePolicy, inputs| policy.fee(&shape(inputs)).unwrap();
    let rate = FeePolicy::Rate("2.5".into());
    assert_eq!(fee(&rate, 1), (size(&rate, 1) * 5).div_ceil(2));
    let legacy = FeePolicy::PerKilobyte(1_000_000);
    assert_eq!(size(&legacy, 1), 10 + 148 + 68);
    assert_eq!(fee(&legacy, 1), 1_000_000);
    assert_eq!(
        fee(&legacy, 7),
        2_000_000,
        "1,114 bytes start a second kilobyte"
    );
    assert_eq!(fee(&FeePolicy::Zip317, 1), 10_000);
    assert_eq!(fee(&FeePolicy::Zip317, 3), 15_000);
    let decred = FeePolicy::Decred(2_000);
    assert_eq!(fee(&decred, 1), (16 + 166 + 72) * 10);
    assert_eq!(
        FeePolicy::Decred(1_000_000).fee(&shape(1)).unwrap(),
        1_000_000
    );
    let kaspa = FeePolicy::Kaspa(1_000);
    assert_eq!(
        fee(&kaspa, 2),
        62 + 2 * 119 + 2 * 52 + 10 * 2 * 27 + 2 * 1_000
    );
}
