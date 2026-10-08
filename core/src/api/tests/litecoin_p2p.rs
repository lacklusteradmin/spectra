use super::*;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/mweb-mainnet.json")).unwrap()
}

fn bytes(value: &serde_json::Value) -> Vec<u8> {
    hex::decode(value.as_str().unwrap()).unwrap()
}

fn header(value: &serde_json::Value) -> BlockHeader {
    BlockHeader::parse(bytes(value).try_into().unwrap())
}

fn mainnet() -> Params {
    params(Chain::Litecoin).unwrap()
}

/// A node's answers at its tip, checked as a light client checks them: the
/// MWEB header proved through the HogEx, the leafset by its root, and a page
/// of 64 outputs by the output root.
#[test]
fn live_answers_prove_into_the_tip() {
    let fixture = fixture();
    let light = &fixture["light_client"];
    let proved = parse_mweb_header(&bytes(&light["mwebheader"])).unwrap();
    assert_eq!(display(&proved.block.hash), light["tip"].as_str().unwrap());
    assert!(proved.block.meets_proof_of_work());
    let leafset = parse_leafset(&bytes(&light["mwebleafset"]), &proved).unwrap();
    assert_eq!(leafset.size, proved.mweb.output_mmr_size);
    assert_eq!(leafset.unspent_from(370_000), Some(370_003));
    let outputs = parse_unspent_outputs(
        &bytes(&light["mwebutxos_compact"]),
        &proved,
        &leafset,
        370_003,
    )
    .unwrap();
    assert_eq!(outputs.len(), 64);
    assert_eq!(outputs[0].leaf, 370_003);
    // The ids the node gives for the same page, asked for by hash alone.
    let hashes = bytes(&light["mwebutxos_hashes"]);
    let mut reader = Reader(&hashes[32..]);
    assert_eq!(reader.compact_size().unwrap(), 370_003);
    assert_eq!(reader.u8().unwrap(), 1);
    assert_eq!(reader.compact_size().unwrap(), 64);
    for output in &outputs {
        assert_eq!(reader.compact_size().unwrap(), output.leaf);
        assert_eq!(reader.array::<32>().unwrap(), output.output.id());
    }
    // An answer by hash alone is not what was asked for.
    assert!(parse_unspent_outputs(&hashes, &proved, &leafset, 370_003).is_err());
}

/// A changed answer is refused: a proof hash, a leaf of the leafset, the
/// HogEx's commitment, an output, a page that starts elsewhere.
#[test]
fn changed_answers_are_refused() {
    let fixture = fixture();
    let light = &fixture["light_client"];
    let header_bytes = bytes(&light["mwebheader"]);
    let proved = parse_mweb_header(&header_bytes).unwrap();
    let leafset_bytes = bytes(&light["mwebleafset"]);
    let leafset = parse_leafset(&leafset_bytes, &proved).unwrap();
    let utxos = bytes(&light["mwebutxos_compact"]);

    let mut changed = utxos.clone();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    assert!(parse_unspent_outputs(&changed, &proved, &leafset, 370_003).is_err());
    // A byte of an output: its id changes, so its leaf no longer proves.
    let mut changed = utxos.clone();
    changed[60] ^= 1;
    assert!(parse_unspent_outputs(&changed, &proved, &leafset, 370_003).is_err());
    assert!(parse_unspent_outputs(&utxos, &proved, &leafset, 370_005).is_err());

    let mut changed = leafset_bytes.clone();
    changed[100] ^= 0x80;
    assert!(parse_leafset(&changed, &proved).is_err());
    // A leafset that claims a spent output within the page is unspent
    // fails the page too (leaf 370,004 was spent).
    assert!(!leafset.contains(370_004));
    let mut bits = leafset_bytes[35..].to_vec();
    bits[(370_004 / 8) as usize] |= 0x80 >> (370_004 % 8);
    let forged = Leafset::from_bits(bits, leafset.size);
    assert!(parse_unspent_outputs(&utxos, &proved, &forged, 370_003).is_err());

    // The HogEx's commitment: the header that follows it in the answer.
    let mut changed = header_bytes.clone();
    let end = changed.len() - 1;
    changed[end] ^= 1;
    assert!(parse_mweb_header(&changed).is_err());
}

/// The node's headers after the anchor chain on from it, each with its
/// proof of work; a header changed, dropped or of another difficulty is
/// refused.
#[test]
fn headers_chain_from_the_anchor() {
    let fixture = fixture();
    let light = &fixture["light_client"];
    let anchor = header(&light["anchor_header"]);
    assert_eq!(display(&anchor.hash), light["anchor"].as_str().unwrap());
    let raw = bytes(&light["headers"]);
    let mut reader = Reader(&raw);
    let count = reader.compact_size().unwrap();
    let headers: Vec<BlockHeader> = (0..count)
        .map(|_| {
            let header = BlockHeader::parse(reader.array().unwrap());
            assert_eq!(reader.compact_size().unwrap(), 0);
            header
        })
        .collect();
    let height = light["anchor_height"].as_u64().unwrap();
    let no_window = |_: u64| None;
    verify_headers(&mainnet(), &anchor, height, &headers, &no_window).unwrap();
    assert_eq!(
        display(&headers.last().unwrap().hash),
        light["tip"].as_str().unwrap()
    );

    let mut changed = headers.clone();
    changed[5].raw[76] ^= 1;
    changed[5] = BlockHeader::parse(changed[5].raw);
    assert!(verify_headers(&mainnet(), &anchor, height, &changed, &no_window).is_err());
    let mut dropped = headers.clone();
    dropped.remove(3);
    assert!(verify_headers(&mainnet(), &anchor, height, &dropped, &no_window).is_err());
    // From another anchor, the first header does not follow.
    assert!(verify_headers(&mainnet(), &headers[0], height, &headers, &no_window).is_err());
}

/// Litecoin's retarget, against two mainnet retargets: the window's last
/// bits, and its last block's time less its first's.
#[test]
fn retargets_are_mainnets() {
    let fixture = fixture();
    let retargets = &fixture["headers"]["retargets"];
    let at = |height: &str| header(&retargets[height]);
    for (first, last, next) in [
        ("3187295", "3189311", "3189312"),
        ("3189311", "3191327", "3191328"),
    ] {
        let timespan = u64::from(at(last).time()) - u64::from(at(first).time());
        assert_eq!(
            retarget(at(last).bits(), timespan),
            Some(at(next).bits()),
            "{next}"
        );
    }
    // The chain across 3,191,328's retarget, with the window's first time.
    let across = &fixture["headers"]["across_retarget"];
    let chain: Vec<BlockHeader> = across["headers"]
        .as_array()
        .unwrap()
        .iter()
        .map(header)
        .collect();
    let window = |height: u64| (height == 3_189_311).then(|| at("3189311").time());
    let anchor_height = across["anchor_height"].as_u64().unwrap();
    verify_headers(&mainnet(), &chain[0], anchor_height, &chain[1..], &window).unwrap();
    // Without the window's start the retarget cannot be checked, and a
    // retarget block with its predecessor's bits is not the one Litecoin gives.
    assert!(verify_headers(&mainnet(), &chain[0], anchor_height, &chain[1..], &|_| None).is_err());
    assert_ne!(chain[1].bits(), chain[2].bits());
    // Compact bits survive a round trip through the target.
    for header in &chain {
        assert_eq!(
            target_to_compact(&compact_to_target(header.bits()).unwrap()),
            header.bits()
        );
    }
}

#[test]
fn endpoints_name_a_tcp_host_and_port() {
    assert_eq!(
        host_port("tcp://seed.example:9333", &mainnet()).unwrap(),
        ("seed.example".into(), 9333)
    );
    assert_eq!(
        host_port("tcp://[::1]", &mainnet()).unwrap(),
        ("::1".into(), 9333)
    );
    assert_eq!(
        host_port("tcp://127.0.0.1", &params(Chain::LitecoinTestnet).unwrap())
            .unwrap()
            .1,
        19335
    );
    for bad in ["https://seed.example", "seed.example:9333"] {
        assert!(host_port(bad, &mainnet()).is_err(), "{bad}");
    }
}
