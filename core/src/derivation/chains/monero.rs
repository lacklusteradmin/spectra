//! Monero: address derivation + chunked-base58 address encoding.
//!
//! Two seed formats are supported:
//!
//! **Native Monero Electrum (25 words)**
//!   Used by Cake Wallet, Monerujo, monero-cli, etc.  The 24 data words encode
//!   a 256-bit spend secret in base-1626 (little-endian, 3 words → 4 bytes),
//!   and the 25th word is a CRC-32 checksum.  No PBKDF2 is involved.
//!     private_spend = sc_reduce32(decoded_32_bytes)
//!     private_view  = sc_reduce32(Keccak256(private_spend))
//!
//! **Cross-chain BIP-39 (12/15/18/21/24 words)**
//!   Standard BIP-39 PBKDF2-HMAC-SHA512 with the same derivation:
//!     private_spend = sc_reduce32(BIP-39 seed[0..32])
//!     private_view  = sc_reduce32(Keccak256(private_spend))
//!
//! Address encoding uses Monero's chunked Base58 with the chain-specific
//! network byte (0x12 = mainnet, 0x18 = stagenet).

use bip39::{Language, Mnemonic};
use pbkdf2::pbkdf2_hmac;
use sha2::Sha512;
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

// ── Monero Electrum 25-word seed ─────────────────────────────────────────

// The official 1626-word English word list.
// Source: https://github.com/monero-project/monero/blob/master/src/mnemonics/english.h
// Each word has a unique 3-character prefix; the decoder matches on that prefix.
const MONERO_WORDS: &[&str; 1626] = &[
    "abbey","abducts","ability","ablaze","abnormal","abort","abrasive","absorb",
    "abyss","academy","aces","aching","acidic","acoustic","acquire","across",
    "actress","acumen","adapt","addicted","adept","adhesive","adjust","adopt",
    "adrenalin","adult","adventure","aerial","afar","affair","afield","afloat",
    "afoot","afraid","after","against","agenda","aggravate","agile","aglow",
    "agnostic","agony","agreed","ahead","aided","ailments","aimless","airport",
    "aisle","ajar","akin","alarms","album","alchemy","alerts","algebra",
    "alkaline","alley","almost","aloof","alpine","already","also","altitude",
    "alumni","always","amaze","ambush","amended","amidst","ammo","amnesty",
    "among","amply","amused","anchor","android","anecdote","angled","ankle",
    "annoyed","answers","antics","anvil","anxiety","anybody","apart","apex",
    "aphid","aplomb","apology","apply","apricot","aptitude","aquarium","arbitrary",
    "archer","ardent","arena","argue","arises","army","around","arrow",
    "arsenic","artistic","ascend","ashtray","aside","asked","asleep","aspire",
    "assorted","asylum","athlete","atlas","atom","atrium","attire","auburn",
    "auctions","audio","august","aunt","austere","autumn","avatar","avidly",
    "avoid","awakened","awesome","awful","awkward","awning","awoken","axes",
    "axis","axle","aztec","azure","baby","bacon","badge","baffles",
    "bagpipe","bailed","bakery","balding","bamboo","banjo","baptism","basin",
    "batch","bawled","bays","because","beer","befit","begun","behind",
    "being","below","bemused","benches","berries","bested","betting","bevel",
    "beware","beyond","bias","bicycle","bids","bifocals","biggest","bikini",
    "bimonthly","binocular","biology","biplane","birth","biscuit","bite","biweekly",
    "blender","blip","bluntly","boat","bobsled","bodies","bogeys","boil",
    "boldly","bomb","border","boss","both","bounced","bovine","bowling",
    "boxes","boyfriend","broken","brunt","bubble","buckets","budget","buffet",
    "bugs","building","bulb","bumper","bunch","business","butter","buying",
    "buzzer","bygones","byline","bypass","cabin","cactus","cadets","cafe",
    "cage","cajun","cake","calamity","camp","candy","casket","catch",
    "cause","cavernous","cease","cedar","ceiling","cell","cement","cent",
    "certain","chlorine","chrome","cider","cigar","cinema","circle","cistern",
    "citadel","civilian","claim","click","clue","coal","cobra","cocoa",
    "code","coexist","coffee","cogs","cohesive","coils","colony","comb",
    "cool","copy","corrode","costume","cottage","cousin","cowl","criminal",
    "cube","cucumber","cuddled","cuffs","cuisine","cunning","cupcake","custom",
    "cycling","cylinder","cynical","dabbing","dads","daft","dagger","daily",
    "damp","dangerous","dapper","darted","dash","dating","dauntless","dawn",
    "daytime","dazed","debut","decay","dedicated","deepest","deftly","degrees",
    "dehydrate","deity","dejected","delayed","demonstrate","dented","deodorant","depth",
    "desk","devoid","dewdrop","dexterity","dialect","dice","diet","different",
    "digit","dilute","dime","dinner","diode","diplomat","directed","distance",
    "ditch","divers","dizzy","doctor","dodge","does","dogs","doing",
    "dolphin","domestic","donuts","doorway","dormant","dosage","dotted","double",
    "dove","down","dozen","dreams","drinks","drowning","drunk","drying",
    "dual","dubbed","duckling","dude","duets","duke","dullness","dummy",
    "dunes","duplex","duration","dusted","duties","dwarf","dwelt","dwindling",
    "dying","dynamite","dyslexic","each","eagle","earth","easy","eating",
    "eavesdrop","eccentric","echo","eclipse","economics","ecstatic","eden","edgy",
    "edited","educated","eels","efficient","eggs","egotistic","eight","either",
    "eject","elapse","elbow","eldest","eleven","elite","elope","else",
    "eluded","emails","ember","emerge","emit","emotion","empty","emulate",
    "energy","enforce","enhanced","enigma","enjoy","enlist","enmity","enough",
    "enraged","ensign","entrance","envy","epoxy","equip","erase","erected",
    "erosion","error","eskimos","espionage","essential","estate","etched","eternal",
    "ethics","etiquette","evaluate","evenings","evicted","evolved","examine","excess",
    "exhale","exit","exotic","exquisite","extra","exult","fabrics","factual",
    "fading","fainted","faked","fall","family","fancy","farming","fatal",
    "faulty","fawns","faxed","fazed","feast","february","federal","feel",
    "feline","females","fences","ferry","festival","fetches","fever","fewest",
    "fiat","fibula","fictional","fidget","fierce","fifteen","fight","films",
    "firm","fishing","fitting","five","fixate","fizzle","fleet","flippant",
    "flying","foamy","focus","foes","foggy","foiled","folding","fonts",
    "foolish","fossil","fountain","fowls","foxes","foyer","framed","friendly",
    "frown","fruit","frying","fudge","fuel","fugitive","fully","fuming",
    "fungal","furnished","fuselage","future","fuzzy","gables","gadget","gags",
    "gained","galaxy","gambit","gang","gasp","gather","gauze","gave",
    "gawk","gaze","gearbox","gecko","geek","gels","gemstone","general",
    "geometry","germs","gesture","getting","geyser","ghetto","ghost","giant",
    "giddy","gifts","gigantic","gills","gimmick","ginger","girth","giving",
    "glass","gleeful","glide","gnaw","gnome","goat","goblet","godfather",
    "goes","goggles","going","goldfish","gone","goodbye","gopher","gorilla",
    "gossip","gotten","gourmet","governing","gown","greater","grunt","guarded",
    "guest","guide","gulp","gumball","guru","gusts","gutter","guys",
    "gymnast","gypsy","gyrate","habitat","hacksaw","haggled","hairy","hamburger",
    "happens","hashing","hatchet","haunted","having","hawk","haystack","hazard",
    "hectare","hedgehog","heels","hefty","height","hemlock","hence","heron",
    "hesitate","hexagon","hickory","hiding","highway","hijack","hiker","hills",
    "himself","hinder","hippo","hire","history","hitched","hive","hoax",
    "hobby","hockey","hoisting","hold","honked","hookup","hope","hornet",
    "hospital","hotel","hounded","hover","howls","hubcaps","huddle","huge",
    "hull","humid","hunter","hurried","husband","huts","hybrid","hydrogen",
    "hyper","iceberg","icing","icon","identity","idiom","idled","idols",
    "igloo","ignore","iguana","illness","imagine","imbalance","imitate","impel",
    "inactive","inbound","incur","industrial","inexact","inflamed","ingested","initiate",
    "injury","inkling","inline","inmate","innocent","inorganic","input","inquest",
    "inroads","insult","intended","inundate","invoke","inwardly","ionic","irate",
    "iris","irony","irritate","island","isolated","issued","italics","itches",
    "items","itinerary","itself","ivory","jabbed","jackets","jaded","jagged",
    "jailed","jamming","january","jargon","jaunt","javelin","jaws","jazz",
    "jeans","jeers","jellyfish","jeopardy","jerseys","jester","jetting","jewels",
    "jigsaw","jingle","jittery","jive","jobs","jockey","jogger","joining",
    "joking","jolted","jostle","journal","joyous","jubilee","judge","juggled",
    "juicy","jukebox","july","jump","junk","jury","justice","juvenile",
    "kangaroo","karate","keep","kennel","kept","kernels","kettle","keyboard",
    "kickoff","kidneys","king","kiosk","kisses","kitchens","kiwi","knapsack",
    "knee","knife","knowledge","knuckle","koala","laboratory","ladder","lagoon",
    "lair","lakes","lamb","language","laptop","large","last","later",
    "launching","lava","lawsuit","layout","lazy","lectures","ledge","leech",
    "left","legion","leisure","lemon","lending","leopard","lesson","lettuce",
    "lexicon","liar","library","licks","lids","lied","lifestyle","light",
    "likewise","lilac","limits","linen","lion","lipstick","liquid","listen",
    "lively","loaded","lobster","locker","lodge","lofty","logic","loincloth",
    "long","looking","lopped","lordship","losing","lottery","loudly","love",
    "lower","loyal","lucky","luggage","lukewarm","lullaby","lumber","lunar",
    "lurk","lush","luxury","lymph","lynx","lyrics","macro","madness",
    "magically","mailed","major","makeup","malady","mammal","maps","masterful",
    "match","maul","maverick","maximum","mayor","maze","meant","mechanic",
    "medicate","meeting","megabyte","melting","memoir","menu","merger","mesh",
    "metro","mews","mice","midst","mighty","mime","mirror","misery",
    "mittens","mixture","moat","mobile","mocked","mohawk","moisture","molten",
    "moment","money","moon","mops","morsel","mostly","motherly","mouth",
    "movement","mowing","much","muddy","muffin","mugged","mullet","mumble",
    "mundane","muppet","mural","musical","muzzle","myriad","mystery","myth",
    "nabbing","nagged","nail","names","nanny","napkin","narrate","nasty",
    "natural","nautical","navy","nearby","necklace","needed","negative","neither",
    "neon","nephew","nerves","nestle","network","neutral","never","newt",
    "nexus","nibs","niche","niece","nifty","nightly","nimbly","nineteen",
    "nirvana","nitrogen","nobody","nocturnal","nodes","noises","nomad","noodles",
    "northern","nostril","noted","nouns","novelty","nowhere","nozzle","nuance",
    "nucleus","nudged","nugget","nuisance","null","number","nuns","nurse",
    "nutshell","nylon","oaks","oars","oasis","oatmeal","obedient","object",
    "obliged","obnoxious","observant","obtains","obvious","occur","ocean","october",
    "odds","odometer","offend","often","oilfield","ointment","okay","older",
    "olive","olympics","omega","omission","omnibus","onboard","oncoming","oneself",
    "ongoing","onion","online","onslaught","onto","onward","oozed","opacity",
    "opened","opposite","optical","opus","orange","orbit","orchid","orders",
    "organs","origin","ornament","orphans","oscar","ostrich","otherwise","otter",
    "ouch","ought","ounce","ourselves","oust","outbreak","oval","oven",
    "owed","owls","owner","oxidant","oxygen","oyster","ozone","pact",
    "paddles","pager","pairing","palace","pamphlet","pancakes","paper","paradise",
    "pastry","patio","pause","pavements","pawnshop","payment","peaches","pebbles",
    "peculiar","pedantic","peeled","pegs","pelican","pencil","people","pepper",
    "perfect","pests","petals","phase","pheasants","phone","phrases","physics",
    "piano","picked","pierce","pigment","piloted","pimple","pinched","pioneer",
    "pipeline","pirate","pistons","pitched","pivot","pixels","pizza","playful",
    "pledge","pliers","plotting","plus","plywood","poaching","pockets","podcast",
    "poetry","point","poker","polar","ponies","pool","popular","portents",
    "possible","potato","pouch","poverty","powder","pram","present","pride",
    "problems","pruned","prying","psychic","public","puck","puddle","puffin",
    "pulp","pumpkins","punch","puppy","purged","push","putty","puzzled",
    "pylons","pyramid","python","queen","quick","quote","rabbits","racetrack",
    "radar","rafts","rage","railway","raking","rally","ramped","randomly",
    "rapid","rarest","rash","rated","ravine","rays","razor","react",
    "rebel","recipe","reduce","reef","refer","regular","reheat","reinvest",
    "rejoices","rekindle","relic","remedy","renting","reorder","repent","request",
    "reruns","rest","return","reunion","revamp","rewind","rhino","rhythm",
    "ribbon","richly","ridges","rift","rigid","rims","ringing","riots",
    "ripped","rising","ritual","river","roared","robot","rockets","rodent",
    "rogue","roles","romance","roomy","roped","roster","rotate","rounded",
    "rover","rowboat","royal","ruby","rudely","ruffled","rugged","ruined",
    "ruling","rumble","runway","rural","rustled","ruthless","sabotage","sack",
    "sadness","safety","saga","sailor","sake","salads","sample","sanity",
    "sapling","sarcasm","sash","satin","saucepan","saved","sawmill","saxophone",
    "sayings","scamper","scenic","school","science","scoop","scrub","scuba",
    "seasons","second","sedan","seeded","segments","seismic","selfish","semifinal",
    "sensible","september","sequence","serving","session","setup","seventh","sewage",
    "shackles","shelter","shipped","shocking","shrugged","shuffled","shyness","siblings",
    "sickness","sidekick","sieve","sifting","sighting","silk","simplest","sincerely",
    "sipped","siren","situated","sixteen","sizes","skater","skew","skirting",
    "skulls","skydive","slackens","sleepless","slid","slower","slug","smash",
    "smelting","smidgen","smog","smuggled","snake","sneeze","sniff","snout",
    "snug","soapy","sober","soccer","soda","software","soggy","soil",
    "solved","somewhere","sonic","soothe","soprano","sorry","southern","sovereign",
    "sowed","soya","space","speedy","sphere","spiders","splendid","spout",
    "sprig","spud","spying","square","stacking","stellar","stick","stockpile",
    "strained","stunning","stylishly","subtly","succeed","suddenly","suede","suffice",
    "sugar","suitcase","sulking","summon","sunken","superior","surfer","sushi",
    "suture","swagger","swept","swiftly","sword","swung","syllabus","symptoms",
    "syndrome","syringe","system","taboo","tacit","tadpoles","tagged","tail",
    "taken","talent","tamper","tanks","tapestry","tarnished","tasked","tattoo",
    "taunts","tavern","tawny","taxi","teardrop","technical","tedious","teeming",
    "tell","template","tender","tepid","tequila","terminal","testing","tether",
    "textbook","thaw","theatrics","thirsty","thorn","threaten","thumbs","thwart",
    "ticket","tidy","tiers","tiger","tilt","timber","tinted","tipsy",
    "tirade","tissue","titans","toaster","tobacco","today","toenail","toffee",
    "together","toilet","token","tolerant","tomorrow","tonic","toolbox","topic",
    "torch","tossed","total","touchy","towel","toxic","toyed","trash",
    "trendy","tribal","trolling","truth","trying","tsunami","tubes","tucks",
    "tudor","tuesday","tufts","tugs","tuition","tulips","tumbling","tunnel",
    "turnip","tusks","tutor","tuxedo","twang","tweezers","twice","twofold",
    "tycoon","typist","tyrant","ugly","ulcers","ultimate","umbrella","umpire",
    "unafraid","unbending","uncle","under","uneven","unfit","ungainly","unhappy",
    "union","unjustly","unknown","unlikely","unmask","unnoticed","unopened","unplugs",
    "unquoted","unrest","unsafe","until","unusual","unveil","unwind","unzip",
    "upbeat","upcoming","update","upgrade","uphill","upkeep","upload","upon",
    "upper","upright","upstairs","uptight","upwards","urban","urchins","urgent",
    "usage","useful","usher","using","usual","utensils","utility","utmost",
    "utopia","uttered","vacation","vague","vain","value","vampire","vane",
    "vapidly","vary","vastness","vats","vaults","vector","veered","vegan",
    "vehicle","vein","velvet","venomous","verification","vessel","veteran","vexed",
    "vials","vibrate","victim","video","viewpoint","vigilant","viking","village",
    "vinegar","violin","vipers","virtual","visited","vitals","vivid","vixen",
    "vocal","vogue","voice","volcano","vortex","voted","voucher","vowels",
    "voyage","vulture","wade","waffle","wagtail","waist","waking","wallets",
    "wanted","warped","washing","water","waveform","waxing","wayside","weavers",
    "website","wedge","weekday","weird","welders","went","wept","were",
    "western","wetsuit","whale","when","whipped","whole","wickets","width",
    "wield","wife","wiggle","wildly","winter","wipeout","wiring","wise",
    "withdrawn","wives","wizard","wobbly","woes","woken","wolf","womanly",
    "wonders","woozy","worry","wounded","woven","wrap","wrist","wrong",
    "yacht","yahoo","yanks","yard","yawning","yearbook","yellow","yesterday",
    "yeti","yields","yodel","yoga","younger","yoyo","zapped","zeal",
    "zebra","zero","zesty","zigzags","zinger","zippers","zodiac","zombie",
    "zones","zoom",
];

// Decode a 25-word Monero Electrum mnemonic into the raw 32-byte spend secret.
// The 25th word is a CRC-32 checksum over the 3-char prefixes of the first 24.
// No PBKDF2: the decoded bytes go directly into sc_reduce32.
pub(crate) fn decode_monero_electrum_seed(phrase: &str) -> Result<Zeroizing<[u8; 32]>, String> {
    const PREFIX: usize = 3;
    const N: u32 = 1626;

    let words: Vec<&str> = phrase.split_whitespace().collect();
    if words.len() != 25 {
        return Err(format!("Monero Electrum seed must be 25 words, got {}", words.len()));
    }

    // Look up each word by its 3-char prefix.
    let mut indices = [0u32; 25];
    for (i, &word) in words.iter().enumerate() {
        let w = word.to_ascii_lowercase();
        let pfx: &str = if w.len() >= PREFIX { &w[..PREFIX] } else { &w[..] };
        let idx = MONERO_WORDS
            .iter()
            .position(|&wl| wl.starts_with(pfx))
            .ok_or_else(|| format!("Unknown Monero word at position {}: '{}'", i + 1, word))?;
        indices[i] = idx as u32;
    }

    // Verify the CRC-32 checksum (word 25 must equal words[crc32(prefixes) % 24]).
    let prefix_str: String = words[..24]
        .iter()
        .map(|w| {
            let w = w.to_ascii_lowercase();
            w.chars().take(PREFIX).collect::<String>()
        })
        .collect();
    const CRC: crc::Crc<u32> = crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);
    let checksum = CRC.checksum(prefix_str.as_bytes());
    let expected_idx = (checksum % 24) as usize;
    if indices[24] != indices[expected_idx] {
        return Err("Invalid Monero mnemonic: checksum word does not match".to_string());
    }

    // Decode 8 groups of 3 words into 4 LE bytes each.
    let mut seed = Zeroizing::new([0u8; 32]);
    for g in 0..8 {
        let w1 = indices[g * 3];
        let w2 = indices[g * 3 + 1];
        let w3 = indices[g * 3 + 2];
        let x: u32 = w1 + N * ((N + w2 - w1) % N) + N * N * ((N + w3 - w2) % N);
        seed[g * 4..g * 4 + 4].copy_from_slice(&x.to_le_bytes());
    }

    Ok(seed)
}

/// Derive (private_spend, public_spend, private_view, public_view) from
/// a 32-byte BIP-39 seed prefix.
/// Expand a 32-byte spend seed into (private_spend, public_spend, private_view, public_view) via sc_reduce32 + Keccak256.
pub(crate) fn derive_monero_keys_from_spend_seed(
    spend_seed: &[u8; 32],
) -> Result<([u8; 32], [u8; 32], [u8; 32], [u8; 32]), String> {
    use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
    use curve25519_dalek::scalar::Scalar as DalekScalar;

    let private_spend = DalekScalar::from_bytes_mod_order(*spend_seed).to_bytes();

    use sha3::{Digest, Keccak256};
    let spend_hash: [u8; 32] = Keccak256::digest(&private_spend).into();
    let private_view = DalekScalar::from_bytes_mod_order(spend_hash).to_bytes();

    let public_spend = (DalekScalar::from_bytes_mod_order(private_spend)
        * ED25519_BASEPOINT_POINT)
        .compress()
        .to_bytes();
    let public_view = (DalekScalar::from_bytes_mod_order(private_view)
        * ED25519_BASEPOINT_POINT)
        .compress()
        .to_bytes();

    Ok((private_spend, public_spend, private_view, public_view))
}

/// Encode a Monero standard address: `network_byte || public_spend (32) ||
/// public_view (32) || keccak256(prev)[0..4]`, then chunked Base58.
/// Network byte: 0x12 for `Chain::Monero`, 0x18 for `Chain::MoneroStagenet`.
/// Encode a Monero standard address from public spend + view keys with the given network byte.
pub(crate) fn encode_monero_main_address(
    public_spend: &[u8; 32],
    public_view: &[u8; 32],
    is_mainnet: bool,
) -> Result<String, String> {
    let network_byte: u8 = if is_mainnet { 0x12 } else { 0x18 };
    let mut payload = Vec::with_capacity(69);
    payload.push(network_byte);
    payload.extend_from_slice(public_spend);
    payload.extend_from_slice(public_view);
    use sha3::{Digest, Keccak256};
    let digest: [u8; 32] = Keccak256::digest(&payload).into();
    payload.extend_from_slice(&digest[..4]);
    Ok(monero_base58_encode(&payload))
}

/// Monero's chunked Base58: split the input into 8-byte blocks, each
/// encoding to a fixed-width 11-char chunk. The alphabet differs in
/// ordering from BIP-58 but uses the same 58 characters.
/// Monero chunked Base58: split input into 8-byte blocks, encode each to a fixed-width 11-char chunk.
pub(crate) fn monero_base58_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 58] =
        b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    const FULL_BLOCK_SIZE: usize = 8;
    const FULL_ENCODED_BLOCK_SIZE: usize = 11;
    const ENCODED_BLOCK_SIZES: [usize; FULL_BLOCK_SIZE + 1] = [0, 2, 3, 5, 6, 7, 9, 10, 11];

    let mut out = String::new();
    let full_blocks = data.len() / FULL_BLOCK_SIZE;
    let remainder = data.len() % FULL_BLOCK_SIZE;

    for i in 0..full_blocks {
        let start = i * FULL_BLOCK_SIZE;
        let block = &data[start..start + FULL_BLOCK_SIZE];
        let mut value: u64 = 0;
        for &b in block {
            value = (value << 8) | u64::from(b);
        }
        let mut chars = [b'1'; FULL_ENCODED_BLOCK_SIZE];
        for j in (0..FULL_ENCODED_BLOCK_SIZE).rev() {
            chars[j] = ALPHABET[(value % 58) as usize];
            value /= 58;
        }
        out.push_str(std::str::from_utf8(&chars).unwrap());
    }
    if remainder > 0 {
        let block = &data[full_blocks * FULL_BLOCK_SIZE..];
        let mut value: u64 = 0;
        for &b in block {
            value = (value << 8) | u64::from(b);
        }
        let encoded_len = ENCODED_BLOCK_SIZES[remainder];
        let mut chars = vec![b'1'; encoded_len];
        for j in (0..encoded_len).rev() {
            chars[j] = ALPHABET[(value % 58) as usize];
            value /= 58;
        }
        out.push_str(std::str::from_utf8(&chars).unwrap());
    }
    out
}

// ── BIP-39 ───────────────────────────────────────────────────────────────

// Map locale string ("en", "zh-cn", etc.) to BIP-39 wordlist; defaults to English.
fn resolve_bip39_language(name: Option<&str>) -> Result<Language, String> {
    let value = match name {
        Some(value) if !value.trim().is_empty() => value.trim().to_ascii_lowercase(),
        _ => return Ok(Language::English),
    };
    match value.as_str() {
        "english" | "en" => Ok(Language::English),
        "czech" | "cs" => Ok(Language::Czech),
        "french" | "fr" => Ok(Language::French),
        "italian" | "it" => Ok(Language::Italian),
        "japanese" | "ja" | "jp" => Ok(Language::Japanese),
        "korean" | "ko" | "kr" => Ok(Language::Korean),
        "portuguese" | "pt" => Ok(Language::Portuguese),
        "spanish" | "es" => Ok(Language::Spanish),
        "simplified-chinese" | "chinese-simplified" | "simplified_chinese" | "zh-hans"
        | "zh-cn" | "zh" => Ok(Language::SimplifiedChinese),
        "traditional-chinese" | "chinese-traditional" | "traditional_chinese" | "zh-hant"
        | "zh-tw" => Ok(Language::TraditionalChinese),
        other => Err(format!("Unsupported mnemonic wordlist: {other}")),
    }
}

// BIP-39 mnemonic → 64-byte seed via NFKD normalization and PBKDF2-HMAC-SHA512.
fn derive_bip39_seed(
    seed_phrase: &str,
    passphrase: &str,
    iteration_count: u32,
    mnemonic_wordlist: Option<&str>,
    salt_prefix: Option<&str>,
) -> Result<Zeroizing<[u8; 64]>, String> {
    let language = resolve_bip39_language(mnemonic_wordlist)?;
    let mnemonic =
        Mnemonic::parse_in_normalized(language, seed_phrase).map_err(|e| e.to_string())?;
    let iterations = if iteration_count == 0 { 2048 } else { iteration_count };
    let prefix = salt_prefix.unwrap_or("mnemonic");
    let normalized_mnemonic = Zeroizing::new(mnemonic.to_string().nfkd().collect::<String>());
    let normalized_passphrase = Zeroizing::new(passphrase.nfkd().collect::<String>());
    let normalized_prefix = Zeroizing::new(prefix.nfkd().collect::<String>());
    let salt = Zeroizing::new(format!(
        "{}{}",
        normalized_prefix.as_str(),
        normalized_passphrase.as_str()
    ));
    let mut seed = Zeroizing::new([0u8; 64]);
    pbkdf2_hmac::<Sha512>(
        normalized_mnemonic.as_bytes(),
        salt.as_bytes(),
        iterations,
        &mut *seed,
    );
    Ok(seed)
}

/// Dispatch on word count: 25 words → Monero Electrum (no PBKDF2), otherwise BIP-39.
pub(crate) fn derive_from_seed_phrase(
    is_mainnet: bool,
    seed_phrase: &str,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let word_count = seed_phrase.split_whitespace().count();
    let mut spend_seed = [0u8; 32];
    if word_count == 25 {
        let raw = decode_monero_electrum_seed(seed_phrase)?;
        spend_seed.copy_from_slice(&*raw);
    } else {
        let seed = derive_bip39_seed(seed_phrase, "", 0, None, None)?;
        spend_seed.copy_from_slice(&seed[..32]);
    }
    let (private_spend, public_spend, private_view, public_view) =
        derive_monero_keys_from_spend_seed(&spend_seed)?;

    let address = if want_address {
        Some(encode_monero_main_address(&public_spend, &public_view, is_mainnet)?)
    } else {
        None
    };

    let public_key_hex = if want_public_key {
        let mut both = [0u8; 64];
        both[..32].copy_from_slice(&public_spend);
        both[32..].copy_from_slice(&public_view);
        Some(hex::encode(both))
    } else {
        None
    };

    let private_key_hex = if want_private_key {
        let mut both = [0u8; 64];
        both[..32].copy_from_slice(&private_spend);
        both[32..].copy_from_slice(&private_view);
        Some(hex::encode(both))
    } else {
        None
    };

    Ok((address, public_key_hex, private_key_hex))
}

// ── UniFFI exports ────────────────────────────────────────────────────────

use crate::derivation::types::DerivationResult;
use crate::SpectraBridgeError;

/// UniFFI export: derive Monero mainnet keys from a BIP-39 seed phrase.
#[uniffi::export]
pub fn derive_monero(
    seed_phrase: String,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (address, public_key_hex, private_key_hex) =
        derive_from_seed_phrase(true, &seed_phrase, want_address, want_public_key, want_private_key)?;
    Ok(DerivationResult { address, public_key_hex, private_key_hex, account: 0, branch: 0, index: 0 })
}

/// UniFFI export: derive Monero stagenet keys from a BIP-39 seed phrase.
#[uniffi::export]
pub fn derive_monero_stagenet(
    seed_phrase: String,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (address, public_key_hex, private_key_hex) =
        derive_from_seed_phrase(false, &seed_phrase, want_address, want_public_key, want_private_key)?;
    Ok(DerivationResult { address, public_key_hex, private_key_hex, account: 0, branch: 0, index: 0 })
}
