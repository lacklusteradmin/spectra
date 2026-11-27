"""Account-model APIs, part B: Cosmos SDK REST, Substrate JSON-RPC, Koios,
algod and Algorand's indexer, ICP Rosetta and the IC's HTTP interface,
VeChainThor REST, Lotus JSON-RPC and Filfox, Tezos RPC and TzKT, Starknet
JSON-RPC, and a Hedera mirror node and its address book.

Each probe makes the requests core's adapter (`core/src/api/<api>.rs`) makes
for a capability and checks the fields it reads. A network's sample (an
active address, a recent transaction, a token holder) is read from the
network itself and shared by every endpoint of that network.

Network facts (chain ids, denominations, genesis hashes, fee routes) are read
from `core/src/registry/` so the probe checks what core binds, not a copy.
Storage hashers, SCALE layouts and ABI selectors core reads from runtime
metadata or derives are fixed here; the selectors are checked against
`core/tests/fixtures/starknet-vectors.json` at import.

Only reads and refusals: a fee is quoted by simulations carrying no
signature (Cosmos) or a fixed, invalid one (Tezos, Starknet, as core sends
them), or by a fee query for an extrinsic already on chain (Substrate); a
broadcast route gets a payload that decodes to no transaction.
"""
import base64
import concurrent.futures
import hashlib
import json
import re
import struct
import threading
import time
import urllib.parse
import zlib
from collections import namedtuple

import common
from common import fail, ok, unchecked

SRC = common.ROOT / "core" / "src"


# ── Checks ─────────────────────────────────────────────────────────────────

class Bad(Exception):
    """A check that failed; the message is the recorded reason."""


def need(value, message):
    if not value:
        raise Bad(message)
    return value


def attempt(check):
    """A check's result: its own `ok`, or `fail` with why it stopped."""
    try:
        return check()
    except Bad as error:
        return fail(str(error))
    except Exception as error:  # noqa: BLE001 — an answer the probe did not expect is evidence
        return fail(f"unexpected answer ({type(error).__name__}: {error})")


def body_of(reply, what, statuses=(200,)):
    """A reply's JSON, or `Bad` naming what was asked and the answer."""
    if reply.error or reply.status not in statuses or reply.json is None:
        raise Bad(f"{what}: {reply.brief()}")
    return reply.json


def digits(value):
    return isinstance(value, str) and value.isdigit()


def no_sample(row, error, capabilities=None):
    reason = f"no sample for {row['chain_id']}: {error}"
    return {"capabilities": {c: fail(reason) for c in capabilities or row["capabilities"]}, "sample": {}}


# ── HTTP, paced for operators that rate-limit per second ───────────────────

_PACE = {"api.zan.top": 0.3}
_last_request = {}
_lock = threading.Lock()


def _pace(url):
    host = urllib.parse.urlsplit(url).hostname
    gap = _PACE.get(host)
    if not gap:
        return
    with _lock:
        now = time.monotonic()
        start = max(now, _last_request.get(host, 0) + gap)
        _last_request[host] = start
    if start > now:
        time.sleep(start - now)


def _answer(url, send):
    """`send()`'s reply; a transport failure common.request lets escape (a
    truncated error body raises from inside its HTTPError handler) becomes a
    reply with no answer."""
    _pace(url)
    try:
        return send()
    except Exception as error:  # noqa: BLE001
        return common.Reply(url, error=f"{type(error).__name__}: {error}")


def GET(url, **options):
    return _answer(url, lambda: common.get(url, **options))


def POST(url, body=None, **options):
    return _answer(url, lambda: common.post(url, body, **options))


def RAW(url, data, content_type, **options):
    return _answer(url, lambda: common.request("POST", url, data=data, headers={"Content-Type": content_type},
                                               **options))


def RPC(url, method, params=None, **options):
    return _answer(url, lambda: common.rpc(url, method, params, **options))


def rpc_value(url, method, params=None, what=None):
    """A JSON-RPC result, or `Bad` with the error object or the answer."""
    reply = RPC(url, method, params)
    envelope = reply.json
    if not isinstance(envelope, dict):
        raise Bad(f"{what or method}: {reply.brief()}")
    if envelope.get("error") is not None:
        raise Bad(f"{what or method}: error {json.dumps(envelope['error'])[:300]}")
    if "result" not in envelope:
        raise Bad(f"{what or method}: no result; {reply.brief()}")
    return envelope["result"]


# ── Broadcast refusals ─────────────────────────────────────────────────────

# What a route that is not served, or served only with a key, answers.
DENIED = ("method not found", "not allowed", "disabled", "unauthorized", "unauthorised",
          "forbidden", "api key", "apikey", "api-key", "access denied", "not permitted")
# What a service says when the payload decodes to no transaction.
UNDECODABLE = ("decode", "deserial", "parse", "unmarshal", "invalid", "malformed", "rlp",
               "cbor", "could not", "cannot", "unexpected end", "eof", "wrong param",
               "missing field", "bad request", "too short", "failed to")


def refusal(reply, words=UNDECODABLE):
    """`ok` when the route answered and refused the payload as undecodable;
    `fail` when it is not served, demands a key, or answered otherwise."""
    if reply.error:
        return fail(reply.brief())
    text = reply.text.lower()
    if reply.status in (401, 402, 403, 404, 405, 407, 429, 501, 502, 503, 504):
        return fail(reply.brief())
    for word in DENIED:
        if word in text:
            return fail(f"refused as {word!r}: {reply.brief()}")
    if any(word in text for word in words):
        return ok(f"refused the undecodable payload: {reply.brief(220)}")
    return fail(f"unrecognised answer: {reply.brief()}")


def rpc_refusal(reply, codes=()):
    """A JSON-RPC route's refusal: an error object that is neither
    method-not-found nor a key demand passes."""
    if reply.error or not isinstance(reply.json, dict):
        return refusal(reply)
    error = reply.json.get("error")
    if error is None:
        return fail(f"no error for an undecodable payload: {reply.brief()}")
    code = error.get("code") if isinstance(error, dict) else None
    message = json.dumps(error).lower()
    if code == -32601 or any(word in message for word in DENIED):
        return fail(f"refused as not served: {json.dumps(error)[:300]}")
    if reply.status in (401, 403, 429):
        return fail(reply.brief())
    if code in codes or code in (-32602, -32700, -32600, -32000) or any(w in message for w in UNDECODABLE):
        return ok(f"refused the undecodable payload: {json.dumps(error)[:240]}")
    return fail(f"unrecognised error: {json.dumps(error)[:300]}")


# ── Shared samples ─────────────────────────────────────────────────────────

_samples = {}
_sample_locks = {}


def shared_sample(key, derive):
    """`derive()` once per key for the whole run; a failure is remembered."""
    with _lock:
        lock = _sample_locks.setdefault(key, threading.Lock())
    with lock:
        if key not in _samples:
            try:
                _samples[key] = (True, derive())
            except Exception as error:  # noqa: BLE001
                _samples[key] = (False, str(error) if isinstance(error, Bad) else f"{type(error).__name__}: {error}")
        good, value = _samples[key]
    if not good:
        raise Bad(value)
    return value


def from_any(row, rows, derive, extra=()):
    """`derive(base)` on this endpoint first, then the network's others,
    then `extra` (label, base) sources outside the catalog."""
    problems = []
    candidates = [(r["id"], r["endpoint"]) for r in [row] + [r for r in rows if r["id"] != row["id"]]]
    for label, base in candidates + list(extra):
        try:
            value = derive(base.rstrip("/"))
            value["read_from"] = label
            return value
        except Exception as error:  # noqa: BLE001
            problems.append(f"{label}: {error}")
    raise Bad("; ".join(problems)[:800])


# ── Registry facts ─────────────────────────────────────────────────────────

def _registry(name):
    return (SRC / "registry" / name).read_text()


def _found(pattern, text, what):
    match = re.search(pattern, text, re.S)
    if not match:
        raise RuntimeError(f"registry drift: {what} not found")
    return match.group(1)


def _kebab(variant):
    return re.sub(r"(?<!^)(?=[A-Z])", "-", variant).lower()


def _networks(file, struct_name, fields):
    """Each `const X: S = S { … }` of a registry file, keyed by the chain id
    its `Self::Variant => &X` names."""
    text = _registry(file)
    bodies = dict(re.findall(rf"const ([A-Z_]+): {struct_name} = {struct_name} \{{(.*?)\n\}};", text, re.S))
    networks = {}
    for variant, const in re.findall(r"Self::(\w+) => &([A-Z_]+),", text):
        if const in bodies:
            networks[_kebab(variant)] = {field: read(bodies[const]) for field, read in fields.items()}
    return networks


COSMOS = _networks("cosmos.rs", "CosmosNetwork", {
    "chain_id": lambda body: _found(r'chain_id: "([^"]+)"', body, "cosmos chain_id"),
    "denom": lambda body: _found(r'denom: "([^"]+)"', body, "cosmos denom"),
    "fee_market": lambda body: re.search(r"fee_market: (?:None|Some\(CosmosFeeMarket::(\w+)\))", body).group(1),
    "tx_search": lambda body: _found(r"tx_search: CosmosTxSearch::(\w+)", body, "tx_search").lower(),
})
SUBSTRATE = _networks("substrate.rs", "SubstrateNetwork", {
    "ss58": lambda body: int(_found(r"ss58_prefix: (\d+)", body, "ss58_prefix")),
    "genesis": lambda body: _found(r'genesis_hash: "(0x[0-9a-f]{64})"', body, "genesis_hash"),
    "balance_bytes": lambda body: int(_found(r"balance_bytes: (\d+)", body, "balance_bytes")),
})
_ALGORAND = _registry("algorand.rs")
ALGORAND_GENESIS_ID = _found(r'genesis_id: "([^"]+)"', _ALGORAND, "algorand genesis_id")
ALGORAND_GENESIS_HASH = base64.b64encode(bytes(int(b, 16) for b in re.findall(
    r"0x([0-9a-f]{2})", _found(r"genesis_hash: \[\s*(0x.*?)\]", _ALGORAND, "algorand genesis_hash")))).decode()
assert len(base64.b64decode(ALGORAND_GENESIS_HASH)) == 32
_NETWORKS = _registry("networks.rs")
ICP_LEDGER_HEX = _found(r'Self::Icp => Ok\("([0-9a-f]{20})"\)', _NETWORKS, "icp ledger id")
ICP_GOVERNANCE = _found(r'Self::Icp => Ok\("([a-z0-9-]+-cai)"\)', _NETWORKS, "icp governance id")
CARDANO_MAGIC = {
    "cardano": int(_found(r"Self::Cardano => Ok\(([\d_]+)\)", _NETWORKS, "cardano magic").replace("_", "")),
    "cardano-preprod": int(_found(r"Self::CardanoPreprod => Ok\(([\d_]+)\)", _NETWORKS, "preprod magic").replace("_", "")),
}
FILECOIN_NETWORK = _found(r'fn filecoin_network_name.*?Self::Filecoin => Ok\("([^"]+)"\)', _NETWORKS, "filecoin name")
FILECOIN_GENESIS = _found(r'fn filecoin_genesis_cid.*?Self::Filecoin => Ok\("([^"]+)"\)', _NETWORKS, "filecoin genesis")
_VECHAIN = _registry("vechain.rs")
VECHAIN_GENESIS = _found(r'genesis_id: "(0x[0-9a-f]{64})"', _VECHAIN, "vechain genesis")
TEZOS_CHAIN_ID = _found(r'chain_id: "(Net\w+)"', _registry("tezos.rs"), "tezos chain id")
_STARKNET = _registry("starknet.rs")
STARKNET_CHAIN_ID = _found(r'chain_id: "([A-Z_]+)"', _STARKNET, "starknet chain id")
STARKNET_FEE_TOKEN = _found(r'fee_token: "(0x[0-9a-f]+)"', _STARKNET, "starknet fee token")
STARKNET_ACCOUNT_CLASS = _found(r'account_class_hash: "(0x[0-9a-f]+)"', _STARKNET, "starknet account class")
HEDERA_GENESIS = _found(r'genesis_block_hash: "(0x[0-9a-f]+)"', _registry("hedera.rs"), "hedera genesis")


# ── Encodings: base58, SS58, xxHash64, Keccak-256, protobuf, CBOR ──────────

B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def b58encode(data):
    number = int.from_bytes(data, "big")
    text = ""
    while number:
        number, rest = divmod(number, 58)
        text = B58[rest] + text
    return "1" * (len(data) - len(data.lstrip(b"\0"))) + text


def b58check(payload):
    return b58encode(payload + hashlib.sha256(hashlib.sha256(payload).digest()).digest()[:4])


def ss58(account, prefix):
    data = bytes([prefix]) + account
    return b58encode(data + hashlib.blake2b(b"SS58PRE" + data, digest_size=64).digest()[:2])


_M64 = (1 << 64) - 1
_P1, _P2, _P3, _P4, _P5 = (0x9E3779B185EBCA87, 0xC2B2AE3D27D4EB4F, 0x165667B19E3779F9,
                           0x85EBCA77C2B2AE63, 0x27D4EB2F165667C5)


def _rotl(value, bits):
    return ((value << bits) | (value >> (64 - bits))) & _M64


def xxh64(data, seed=0):
    def round_(acc, lane):
        return (_rotl((acc + lane * _P2) & _M64, 31) * _P1) & _M64

    length, i = len(data), 0
    if length >= 32:
        v = [(seed + _P1 + _P2) & _M64, (seed + _P2) & _M64, seed, (seed - _P1) & _M64]
        while i + 32 <= length:
            for lane in range(4):
                v[lane] = round_(v[lane], int.from_bytes(data[i + 8 * lane:i + 8 * lane + 8], "little"))
            i += 32
        h = (_rotl(v[0], 1) + _rotl(v[1], 7) + _rotl(v[2], 12) + _rotl(v[3], 18)) & _M64
        for lane in v:
            h = ((h ^ round_(0, lane)) * _P1 + _P4) & _M64
    else:
        h = (seed + _P5) & _M64
    h = (h + length) & _M64
    while i + 8 <= length:
        h = (_rotl(h ^ round_(0, int.from_bytes(data[i:i + 8], "little")), 27) * _P1 + _P4) & _M64
        i += 8
    if i + 4 <= length:
        h = (_rotl(h ^ (int.from_bytes(data[i:i + 4], "little") * _P1 & _M64), 23) * _P2 + _P3) & _M64
        i += 4
    while i < length:
        h = (_rotl(h ^ (data[i] * _P5 & _M64), 11) * _P1) & _M64
        i += 1
    h ^= h >> 33
    h = (h * _P2) & _M64
    h ^= h >> 29
    h = (h * _P3) & _M64
    return h ^ (h >> 32)


def twox128(name):
    data = name.encode()
    return xxh64(data, 0).to_bytes(8, "little") + xxh64(data, 1).to_bytes(8, "little")


def blake2_128_concat(data):
    return hashlib.blake2b(data, digest_size=16).digest() + data


def twox64_concat(data):
    return xxh64(data, 0).to_bytes(8, "little") + data


def storage_key(pallet, item, *keys):
    return "0x" + (twox128(pallet) + twox128(item) + b"".join(keys)).hex()


assert storage_key("System", "Events") == "0x26aa394eea5630e07c48ae0c9558cef780d41e5e16056765bc8461851072c9d7"


def _keccak_f(lanes):
    def rol(value, bits):
        bits %= 64
        return ((value >> (64 - bits)) + (value << bits)) & _M64 if bits else value

    r = 1
    for _ in range(24):
        c = [lanes[x][0] ^ lanes[x][1] ^ lanes[x][2] ^ lanes[x][3] ^ lanes[x][4] for x in range(5)]
        d = [c[(x + 4) % 5] ^ rol(c[(x + 1) % 5], 1) for x in range(5)]
        lanes = [[lanes[x][y] ^ d[x] for y in range(5)] for x in range(5)]
        x, y = 1, 0
        current = lanes[x][y]
        for t in range(24):
            x, y = y, (2 * x + 3 * y) % 5
            current, lanes[x][y] = lanes[x][y], rol(current, (t + 1) * (t + 2) // 2)
        for y in range(5):
            row = [lanes[x][y] for x in range(5)]
            for x in range(5):
                lanes[x][y] = row[x] ^ ((~row[(x + 1) % 5]) & row[(x + 2) % 5])
        for j in range(7):
            r = ((r << 1) ^ ((r >> 7) * 0x71)) % 256
            if r & 2:
                lanes[0][0] ^= 1 << ((1 << j) - 1)
    return lanes


def keccak256(data):
    rate = 136
    padded = bytearray(data) + b"\x01"
    while len(padded) % rate:
        padded.append(0)
    padded[-1] |= 0x80
    lanes = [[0] * 5 for _ in range(5)]
    for offset in range(0, len(padded), rate):
        for i in range(rate // 8):
            lanes[i % 5][i // 5] ^= int.from_bytes(padded[offset + 8 * i:offset + 8 * i + 8], "little")
        lanes = _keccak_f(lanes)
    return b"".join(lanes[i % 5][i // 5].to_bytes(8, "little") for i in range(4))


def starknet_selector(name):
    return hex(int.from_bytes(keccak256(name.encode()), "big") & ((1 << 250) - 1))


assert keccak256(b"").hex() == "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
_STARKNET_VECTORS = json.loads((common.ROOT / "core" / "tests" / "fixtures" / "starknet-vectors.json").read_text())
for _name, _selector in _STARKNET_VECTORS["selectors"].items():
    assert int(starknet_selector(_name), 16) == int(_selector, 16), _name


def _varint(number):
    out = bytearray()
    while True:
        byte, number = number & 0x7F, number >> 7
        out.append(byte | (0x80 if number else 0))
        if not number:
            return bytes(out)


def _pb_bytes(field, data):
    return _varint(field << 3 | 2) + _varint(len(data)) + data


def _pb_text(field, text):
    return _pb_bytes(field, text.encode()) if text else b""


def _pb_uint(field, number):
    return _varint(field << 3) + _varint(number) if number else b""


Tag = namedtuple("Tag", "tag value")


class CborMap(dict):
    duplicates = 0


def cbor_encode(value):
    def head(major, number):
        if number < 24:
            return bytes([major << 5 | number])
        for info, size in ((24, 1), (25, 2), (26, 4), (27, 8)):
            if number < 1 << (8 * size):
                return bytes([major << 5 | info]) + number.to_bytes(size, "big")
        raise ValueError("CBOR head too large")

    if isinstance(value, Tag):
        return head(6, value.tag) + cbor_encode(value.value)
    if isinstance(value, int):
        return head(0, value) if value >= 0 else head(1, -1 - value)
    if isinstance(value, bytes):
        return head(2, len(value)) + value
    if isinstance(value, str):
        return head(3, len(value.encode())) + value.encode()
    if isinstance(value, list):
        return head(4, len(value)) + b"".join(map(cbor_encode, value))
    if isinstance(value, dict):
        return head(5, len(value)) + b"".join(cbor_encode(k) + cbor_encode(v) for k, v in value.items())
    raise TypeError(type(value))


def cbor_decode(data, i=0):
    first = data[i]
    i += 1
    major, info = first >> 5, first & 31
    if info < 24:
        number = info
    elif info in (24, 25, 26, 27):
        size = 1 << (info - 24)
        raw = data[i:i + size]
        if len(raw) != size:
            raise ValueError("truncated CBOR")
        number, i = int.from_bytes(raw, "big"), i + size
    elif info == 31:
        number = None
    else:
        raise ValueError(f"reserved CBOR info {info}")
    if major == 0:
        return number, i
    if major == 1:
        return -1 - number, i
    if major in (2, 3):
        if number is None:
            chunks = []
            while data[i] != 0xFF:
                chunk, i = cbor_decode(data, i)
                chunks.append(chunk)
            joined = b"".join(chunks) if major == 2 else "".join(chunks)
            return joined, i + 1
        raw = bytes(data[i:i + number])
        if len(raw) != number:
            raise ValueError("truncated CBOR string")
        return (raw if major == 2 else raw.decode()), i + number
    if major == 4:
        items = []
        while (number is None and data[i] != 0xFF) or (number is not None and len(items) < number):
            item, i = cbor_decode(data, i)
            items.append(item)
        return items, i + (1 if number is None else 0)
    if major == 5:
        mapping = CborMap()
        count = 0
        while (number is None and data[i] != 0xFF) or (number is not None and count < number):
            key, i = cbor_decode(data, i)
            value, i = cbor_decode(data, i)
            if key in mapping:
                mapping.duplicates += 1
            mapping[key] = value
            count += 1
        return mapping, i + (1 if number is None else 0)
    if major == 6:
        value, i = cbor_decode(data, i)
        return Tag(number, value), i
    simple = {20: False, 21: True, 22: None, 23: None}
    if info in simple:
        return simple[info], i
    if info in (25, 26, 27):
        size = 1 << (info - 24)
        raw = number.to_bytes(size, "big")
        return (struct.unpack(">e", raw) if size == 2 else struct.unpack(">f" if size == 4 else ">d", raw))[0], i
    return number, i


def cbor_whole(data):
    """The one CBOR value `data` holds, without the self-describing tag;
    `ValueError` on trailing bytes."""
    value, end = cbor_decode(data, 0)
    if end != len(data):
        raise ValueError(f"{len(data) - end} trailing bytes after the CBOR value")
    while isinstance(value, Tag) and value.tag == 55799:
        value = value.value
    return value


# ══ Cosmos SDK REST (core/src/api/cosmos_rest.rs) ══════════════════════════

COSMOS_TRANSFER_GAS = 100_000  # registry::COSMOS_TRANSFER_GAS_ESTIMATE, the placeholder limit a simulation carries


def cosmos_search(base, network, query, limit=50):
    parameter = "events" if network["tx_search"] == "events" else "query"
    return GET(f"{base}/cosmos/tx/v1beta1/txs?{parameter}={urllib.parse.quote_plus(query)}"
               f"&page=1&limit={limit}&order_by=ORDER_BY_DESC")


def cosmos_height(base):
    block = body_of(GET(f"{base}/cosmos/base/tendermint/v1beta1/blocks/latest"), "latest block")
    header = block.get("sdk_block", {}).get("header") or block.get("block", {}).get("header") or {}
    need(digits(header.get("height")), f"latest block: no height in {json.dumps(block)[:200]}")
    return int(header["height"])


def cosmos_sends(txs, denom):
    """(sender, hash, height) of each successful MsgSend of `denom` in a search answer, then any."""
    native, other = [], []
    for tx in txs:
        if tx.get("code", 0) != 0:
            continue
        for message in tx.get("tx", {}).get("body", {}).get("messages", []):
            if message.get("@type") != "/cosmos.bank.v1beta1.MsgSend":
                continue
            found = (message.get("from_address"), tx.get("txhash"), tx.get("height"))
            if any(coin.get("denom") == denom for coin in message.get("amount", [])):
                native.append(found)
            else:
                other.append(found)
    return native + other


def derive_cosmos(network):
    """The sender of a recent MsgSend whose account can still spend the
    network's coin, so a node can simulate a transfer from it."""
    denom = network["denom"]

    def funded(base, sender):
        held = GET(f"{base}/cosmos/bank/v1beta1/spendable_balances/{sender}/by_denom?denom={urllib.parse.quote_plus(denom)}")
        amount = ((held.json or {}).get("balance") or {}).get("amount") if isinstance(held.json, dict) else None
        return digits(amount) and int(amount) > 1000

    def derive(base):
        problems, checked = [], set()
        reply = cosmos_search(base, network, "message.action='/cosmos.bank.v1beta1.MsgSend'", limit=50)
        if reply.status == 200 and isinstance(reply.json, dict):
            for sender, txhash, height in cosmos_sends(reply.json.get("tx_responses") or [], denom):
                if sender in checked or len(checked) >= 12:
                    continue
                checked.add(sender)
                if funded(base, sender):
                    return {"address": sender, "tx": txhash, "tx_height": height,
                            "how": "the newest MsgSend sender a message.action search answered that can still spend"}
            problems.append(f"message.action search: none of {len(checked)} senders can spend")
        else:
            problems.append(f"message.action search: {reply.brief(160)}")
        # Recent blocks, newest first, for nodes whose search is slow or empty.
        height = cosmos_height(base)
        for back in range(2, 60):
            block = GET(f"{base}/cosmos/tx/v1beta1/txs/block/{height - back}")
            if block.status != 200 or not isinstance(block.json, dict):
                problems.append(f"txs/block: {block.brief(120)}")
                break
            raw = block.json.get("block", {}).get("data", {}).get("txs") or []
            for index, tx in enumerate(block.json.get("txs") or []):
                for message in tx.get("body", {}).get("messages", []):
                    sender = message.get("from_address")
                    if (message.get("@type") == "/cosmos.bank.v1beta1.MsgSend" and index < len(raw)
                            and sender not in checked):
                        checked.add(sender)
                        if funded(base, sender):
                            txhash = hashlib.sha256(base64.b64decode(raw[index])).hexdigest().upper()
                            return {"address": sender, "tx": txhash, "tx_height": str(height - back),
                                    "how": "the sender, able to spend, of a MsgSend in a recent block"}
        raise Bad("; ".join(problems) or "no funded MsgSend sender in 60 recent blocks")
    return derive


def cosmos_simulation(denom, address, sequence, height):
    """`PreparedCosmosTransfer::simulation_bytes` for a 1-unit self-send:
    no public key, one empty signature, no fee coin."""
    coin = _pb_text(1, denom) + _pb_text(2, "1")
    send = _pb_text(1, address) + _pb_text(2, address) + _pb_bytes(3, coin)
    message = _pb_text(1, "/cosmos.bank.v1beta1.MsgSend") + _pb_bytes(2, send)
    body = _pb_bytes(1, message) + _pb_uint(3, height + 1)
    signer = _pb_bytes(2, _pb_bytes(1, _pb_uint(1, 1))) + _pb_uint(3, sequence)
    auth = _pb_bytes(1, signer) + _pb_bytes(2, _pb_uint(2, COSMOS_TRANSFER_GAS))
    return _pb_bytes(1, body) + _pb_bytes(2, auth) + _pb_bytes(3, b"")


def cosmos_market_price(base, network):
    market, denom = network["fee_market"], network["denom"]
    if market is None:
        return "no fee market"
    if market == "Skip":
        body = body_of(GET(f"{base}/feemarket/v1/gas_price/{denom}"), "fee market")
        price = body.get("price") or {}
        need(price.get("denom") == denom, f"fee market: another denomination {price}")
        amount = price.get("amount")
    elif market == "OsmosisTxFees":
        amount = body_of(GET(f"{base}/osmosis/txfees/v1beta1/cur_eip_base_fee"), "txfees").get("base_fee")
    else:
        amount = (body_of(GET(f"{base}/injective/txfees/v1beta1/cur_eip_base_fee"), "txfees")
                  .get("base_fee") or {}).get("base_fee")
    need(isinstance(amount, str) and re.fullmatch(r"\d*(\.\d+)?", amount.strip()) and amount.strip(),
         f"fee market price is not a decimal: {amount!r}")
    return f"{market} price {amount}"


def probe_cosmos(row):
    chain, base = row["chain_id"], row["endpoint"].rstrip("/")
    network = COSMOS[chain]
    denom = network["denom"]
    try:
        # Read from the catalog's first endpoint for the network, not whichever
        # runs first, so a stale node cannot set the sample.
        rows = common.endpoints("cosmos-rest", chain)
        sample = shared_sample(("cosmos", chain), lambda: from_any(rows[0], rows, derive_cosmos(network)))
    except Bad as error:
        return no_sample(row, error)
    address, txhash = sample["address"], sample["tx"]
    info = GET(f"{base}/cosmos/base/tendermint/v1beta1/node_info")
    named = (info.json or {}).get("default_node_info", {}).get("network") if isinstance(info.json, dict) else None
    identity = (ok(f"node_info names {named}") if named == network["chain_id"]
                else fail(f"node_info: expected {network['chain_id']}, got {named!r}; {info.brief(160)}"))

    try:
        head = cosmos_height(base)
    except Exception as error:  # noqa: BLE001
        head = None
    lag = int(sample["tx_height"]) - head if head is not None else 0
    # A node more than a few minutes behind the sample answers stale state.
    stale = (f"the node is stale: its latest block {head} is {lag} blocks behind the sample transaction's "
             f"height {sample['tx_height']}") if lag > 100 else None

    def needs_identity():
        if not identity["ok"]:
            raise Bad(f"network check: {identity['detail']}")

    def fresh():
        if stale:
            raise Bad(stale)

    def balance():
        fresh()
        body = body_of(GET(f"{base}/cosmos/bank/v1beta1/balances/{address}/by_denom?denom="
                           f"{urllib.parse.quote_plus(denom)}"), "balance")
        held = body.get("balance")
        if held is None:
            return ok("balance null, read as 0")
        need(held.get("denom") == denom, f"another denomination: {held}")
        need(digits(held.get("amount")), f"amount is not a number: {held}")
        return ok(f"{held['amount']} {denom}")

    def history():
        fresh()
        counts, decoded = {}, 0
        for label, query in (("sent", f"message.sender='{address}'"), ("received", f"transfer.recipient='{address}'")):
            body = body_of(cosmos_search(base, network, query), f"{label} search")
            txs = body.get("tx_responses")
            need(isinstance(txs, list), f"{label} search: no tx_responses list in {json.dumps(body)[:200]}")
            for tx in txs:
                need(isinstance(tx.get("txhash"), str), f"{label} search: a result without txhash")
                need(digits(tx.get("height")), f"{label} search: a result without height")
                need(isinstance(tx.get("code", 0), int), f"{label} search: code is not a number")
                flat = [e for e in tx.get("events") or [] if e.get("type") == "transfer"
                        and any(a.get("key") == "msg_index" for a in e.get("attributes") or [])]
                logs = [e for log in tx.get("logs") or [] for e in log.get("events") or [] if e.get("type") == "transfer"]
                decoded += bool(flat or logs)
            counts[label] = len(txs)
        need(counts["sent"] > 0, f"sent search answered no transactions for {address}, which sent {txhash}")
        return ok(f"first page: {counts['sent']} sent, {counts['received']} received; "
                  f"{decoded} carry transfer events")

    def fee():
        needs_identity()
        body = body_of(GET(f"{base}/cosmos/auth/v1beta1/accounts/{address}"), "account")
        account = body.get("account") or {}
        candidates = [account, account.get("base_account") or {},
                      (account.get("base_vesting_account") or {}).get("base_account") or {}]
        held = need(next((c for c in candidates if isinstance(c.get("account_number"), str)), None),
                    f"account: no base account in {json.dumps(account)[:200]}")
        need(str(held.get("address", "")).lower() == address.lower(), "account: another address")
        need(digits(held.get("account_number")) and digits(held.get("sequence")),
             f"account: number or sequence missing in {json.dumps(held)[:200]}")
        retried = ""
        for attempt_number in range(2):
            height = cosmos_height(base)
            tx_bytes = cosmos_simulation(denom, address.lower(), int(held["sequence"]), height)
            simulated = POST(f"{base}/cosmos/tx/v1beta1/simulate", {"tx_bytes": base64.b64encode(tx_bytes).decode()})
            # Core times a quote out one block past the height it read; a node
            # whose latest block lags its state refuses that as expired.
            if attempt_number == 0 and "timeout height" in simulated.text:
                retried = f" (first simulation refused: {simulated.brief(140)}; retried)"
                continue
            break
        gas = (body_of(simulated, "simulate").get("gas_info") or {}).get("gas_used")
        need(digits(gas) and int(gas) > 0, f"simulate: no gas used in {simulated.brief(200)}")
        market = cosmos_market_price(base, network)
        spendable = body_of(GET(f"{base}/cosmos/bank/v1beta1/spendable_balances/{address}/by_denom?denom="
                                f"{urllib.parse.quote_plus(denom)}"), "spendable balance").get("balance")
        need(spendable is None or digits(spendable.get("amount")), f"spendable balance: {spendable}")
        return ok(f"account {held['account_number']}/{held['sequence']}, height {height}, "
                  f"simulated self-send used {gas} gas; {market}; spendable "
                  f"{(spendable or {}).get('amount', '0')}{retried}")

    def broadcast():
        reply = POST(f"{base}/cosmos/tx/v1beta1/txs", {"tx_bytes": "AA==", "mode": "BROADCAST_MODE_SYNC"})
        if reply.status == 200 and isinstance(reply.json, dict):
            tx = reply.json.get("tx_response") or {}
            code = tx.get("code")
            if isinstance(code, int) and code != 0:
                return ok(f"refused with code {code} ({tx.get('codespace')}): {str(tx.get('raw_log'))[:160]}")
            return fail(f"no refusal for an undecodable transaction: {reply.brief()}")
        return refusal(reply)

    def verification():
        needs_identity()
        body = body_of(GET(f"{base}/cosmos/tx/v1beta1/txs/{txhash}"), f"transaction {txhash}")
        tx = body.get("tx_response") or {}
        need(str(tx.get("txhash", "")).lower() == txhash.lower(), f"another transaction: {tx.get('txhash')}")
        need(digits(tx.get("height")) and int(tx["height"]) > 0, "no height")
        need(isinstance(tx.get("code"), int), "no code")
        return ok(f"{txhash} at height {tx['height']}, code {tx['code']}")

    checks = {"balance": balance, "history": history, "fee": fee, "broadcast": broadcast,
              "verification": verification}
    record = {"capabilities": {name: attempt(check) for name, check in checks.items()},
              "sample": sample, "identity": identity, "latest_height": head}
    if stale:
        record["notes"] = [stale]
    return record


# ══ Substrate JSON-RPC (core/src/api/substrate_json_rpc/) ══════════════════

def scale_compact(data, i):
    mode = data[i] & 3
    if mode == 0:
        return data[i] >> 2, i + 1
    if mode == 1:
        return int.from_bytes(data[i:i + 2], "little") >> 2, i + 2
    if mode == 2:
        return int.from_bytes(data[i:i + 4], "little") >> 2, i + 4
    size = (data[i] >> 2) + 4
    return int.from_bytes(data[i + 1:i + 1 + size], "little"), i + 1 + size


def substrate_signer(extrinsic):
    """The `MultiAddress::Id` signer of a signed v4 extrinsic, or None."""
    _, i = scale_compact(extrinsic, 0)
    version = extrinsic[i]
    if version != 0x84 or extrinsic[i + 1] != 0:
        return None
    return extrinsic[i + 2:i + 34]


def hexbytes(text):
    need(isinstance(text, str) and text.startswith("0x"), f"not hex: {str(text)[:80]}")
    return bytes.fromhex(text[2:])


def derive_substrate(network):
    """The signer of a signed extrinsic in a recent finalized block whose
    account holds a balance there. Blocks are read sixteen at a time, newest
    first: Astar's carry a signed extrinsic only every few dozen."""
    def block_at(base, number):
        hash_ = rpc_value(base, "chain_getBlockHash", [number])
        return number, hash_, rpc_value(base, "chain_getBlock", [hash_])["block"]

    def derive(base):
        finalized = rpc_value(base, "chain_getFinalizedHead")
        head = int(rpc_value(base, "chain_getHeader", [finalized])["number"], 16)
        with concurrent.futures.ThreadPoolExecutor(8) as pool:
            for start in range(head, head - 1200, -16):
                batch = pool.map(lambda n: block_at(base, n), range(start, start - 16, -1))
                for number, hash_, block in batch:
                    for extrinsic in block["extrinsics"]:
                        raw = hexbytes(extrinsic)
                        signer = substrate_signer(raw)
                        if not signer:
                            continue
                        account = rpc_value(base, "state_getStorage", [
                            storage_key("System", "Account", blake2_128_concat(signer)), finalized])
                        if not account or not int.from_bytes(hexbytes(account)[16:16 + network["balance_bytes"]], "little"):
                            continue
                        return {"address": ss58(signer, network["ss58"]), "account": "0x" + signer.hex(),
                                "extrinsic": extrinsic,
                                "tx": "0x" + hashlib.blake2b(raw, digest_size=32).hexdigest(),
                                "block": number, "block_hash": hash_,
                                "how": "the signer of a signed extrinsic in a recent finalized block, holding a balance at the finalized head"}
        raise Bad("no signed extrinsic from a funded account in 1,200 finalized blocks")
    return derive


def substrate_keys(base, prefix, at, count=20):
    keys = rpc_value(base, "state_getKeysPaged", [prefix, count, None, at])
    need(isinstance(keys, list) and keys, f"state_getKeysPaged answered no keys under {prefix[:40]}…")
    return keys


def decode_asset_account(raw):
    need(len(raw) >= 18, f"Assets.Account record of {len(raw)} bytes")
    balance, status, reason = int.from_bytes(raw[:16], "little"), raw[16], raw[17]
    need(status in (0, 1, 2), f"Assets.Account status {status}")
    extra = {0: 0, 1: 0, 2: 16, 3: 0, 4: 48}.get(reason)
    need(extra is not None and len(raw) == 18 + extra, f"Assets.Account reason {reason}, {len(raw)} bytes")
    return balance, ("Liquid", "Frozen", "Blocked")[status]


def decode_asset_details(raw):
    need(len(raw) == 190, f"Assets.Asset record of {len(raw)} bytes, not 190")
    need(raw[176] in (0, 1), "Assets.Asset is_sufficient is not a bool")
    need(raw[189] in (0, 1, 2), f"Assets.Asset status {raw[189]}")
    return int.from_bytes(raw[160:176], "little"), ("Live", "Frozen", "Destroying")[raw[189]]


def decode_asset_metadata(raw):
    i = 16
    name_length, i = scale_compact(raw, i)
    i += name_length
    symbol_length, i = scale_compact(raw, i)
    symbol = raw[i:i + symbol_length].decode("utf-8", "replace")
    i += symbol_length
    need(len(raw) == i + 2 and raw[i + 1] in (0, 1), f"Assets.Metadata record of {len(raw)} bytes")
    return symbol, raw[i]


def derive_asset_holder(network, ids):
    """A holder of the first of `ids` that has one, or of the network's first
    asset when `ids` is empty, read from the `Assets` pallet's keys."""
    def derive(base):
        at = rpc_value(base, "chain_getFinalizedHead")
        candidates = ids
        if not candidates:
            keys = substrate_keys(base, storage_key("Assets", "Asset"), at, 50)
            candidates = [str(int.from_bytes(hexbytes(key)[-4:], "little")) for key in keys]
        for asset in candidates:
            prefix = storage_key("Assets", "Account", blake2_128_concat(int(asset).to_bytes(4, "little")))
            keys = rpc_value(base, "state_getKeysPaged", [prefix, 20, None, at]) or []
            for key in keys:
                value = rpc_value(base, "state_getStorage", [key, at])
                if value and decode_asset_account(hexbytes(value))[0] > 0:
                    holder = hexbytes(key)[-32:]
                    return {"asset": asset, "holder": ss58(holder, network["ss58"]), "holder_account": "0x" + holder.hex(),
                            "how": "a key of the Assets pallet's Account map holding a balance"}
        raise Bad(f"no holder found for assets {candidates[:5]}")
    return derive


def derive_pool_member(base):
    at = rpc_value(base, "chain_getFinalizedHead")
    keys = substrate_keys(base, storage_key("NominationPools", "PoolMembers"), at, 5)
    member = hexbytes(keys[0])[-32:]
    return {"member": ss58(member, 0), "member_account": "0x" + member.hex(),
            "how": "the first key of NominationPools.PoolMembers"}


def state_call(base, method, data, at):
    return hexbytes(rpc_value(base, "state_call", [method, "0x" + data.hex(), at]))


def probe_substrate(row):
    chain, base = row["chain_id"], row["endpoint"].rstrip("/")
    network = SUBSTRATE[chain]
    rows = common.endpoints("substrate-json-rpc", chain)
    try:
        sample = dict(shared_sample(("substrate", chain), lambda: from_any(row, rows, derive_substrate(network))))
    except Bad as error:
        return no_sample(row, error)
    account = hexbytes(sample["account"])
    notes = []
    context = {}

    def pinned():
        """`pinned_runtime`: genesis, best block, runtime version and
        metadata at it, finalized head."""
        if "error" in context:
            raise Bad(context["error"])
        if context:
            return context
        try:
            genesis = rpc_value(base, "chain_getBlockHash", [0])
            need(genesis == network["genesis"], f"genesis {genesis}, not {network['genesis']}")
            best = rpc_value(base, "chain_getBlockHash", [])
            version = rpc_value(base, "state_getRuntimeVersion", [best])
            need(version.get("specVersion") and version.get("transactionVersion"),
                 f"runtime version: {json.dumps(version)[:160]}")
            metadata = hexbytes(rpc_value(base, "state_getMetadata", [best]))
            need(metadata[:4] == b"meta" and metadata[4] >= 14, f"metadata: magic {metadata[:5].hex()}")
            finalized = rpc_value(base, "chain_getFinalizedHead")
            number = int(rpc_value(base, "chain_getHeader", [finalized])["number"], 16)
            context.update(best=best, spec=version["specVersion"], tx_version=version["transactionVersion"],
                           metadata=f"v{metadata[4]}, {len(metadata)} bytes", finalized=number)
            return context
        except Bad as error:
            context["error"] = f"runtime context: {error}"
            raise Bad(context["error"])

    def balance():
        at = pinned()["best"]
        value = rpc_value(base, "state_getStorage", [storage_key("System", "Account", blake2_128_concat(account)), at])
        if value is None:
            return ok("System.Account absent, read as 0")
        raw = hexbytes(value)
        width = network["balance_bytes"]
        need(len(raw) == 16 + 3 * width + 16, f"System.Account record of {len(raw)} bytes, not {16 + 4 * width}")
        free = int.from_bytes(raw[16:16 + width], "little")
        return ok(f"free {free} at spec {context['spec']} (metadata {context['metadata']})")

    def fee():
        at = pinned()["best"]
        nonce = rpc_value(base, "system_accountNextIndex", [sample["address"]])
        need(isinstance(nonce, int), f"system_accountNextIndex: {nonce!r}")
        quote = rpc_value(base, "payment_queryInfo", [sample["extrinsic"], at])
        fee_ = quote.get("partialFee") if isinstance(quote, dict) else None
        need(digits(fee_) and int(fee_) > 0, f"payment_queryInfo: partialFee {fee_!r} in {json.dumps(quote)[:160]}")
        return ok(f"partialFee {fee_} for the sample extrinsic; next nonce {nonce}")

    def broadcast():
        return rpc_refusal(RPC(base, "author_submitExtrinsic", ["0x00"]), codes=(1000, 1001, 1002, 1010, 1011, 1012))

    def verification():
        pinned()
        number = sample["block"]
        hash_ = rpc_value(base, "chain_getBlockHash", [number])
        need(hash_ == sample["block_hash"], f"block {number} is {hash_}, not {sample['block_hash']}")
        block = rpc_value(base, "chain_getBlock", [hash_])["block"]
        need(int(block["header"]["number"], 16) == number, "block number does not match")
        hashes = ["0x" + hashlib.blake2b(hexbytes(x), digest_size=32).hexdigest() for x in block["extrinsics"]]
        need(sample["tx"] in hashes, f"{sample['tx']} not among block {number}'s extrinsics")
        events = rpc_value(base, "state_getStorage", [storage_key("System", "Events"), hash_])
        need(isinstance(events, str) and len(events) > 4, f"System.Events at block {number}: {events!r}")
        metadata = hexbytes(rpc_value(base, "state_getMetadata", [block["header"]["parentHash"]]))
        need(metadata[:4] == b"meta", "metadata at the parent block is not runtime metadata")
        return ok(f"{sample['tx']} in finalized block {number}; events {len(events) // 2 - 1} bytes, "
                  f"parent metadata {len(metadata)} bytes")

    def token_balance():
        at = pinned()["best"]
        ids = [token["contract"] for token in common.tokens(chain)]
        holding = shared_sample(("substrate-assets", chain), lambda: from_any(
            row, rows, derive_asset_holder(network, ids)))
        sample["token"] = holding
        holder = hexbytes(holding["holder_account"])
        decimals_by_id = {token["contract"]: token["decimals"] for token in common.tokens(chain)}
        answers = []
        for asset in ids or [holding["asset"]]:
            encoded = int(asset).to_bytes(4, "little")
            details = rpc_value(base, "state_getStorage", [storage_key("Assets", "Asset", blake2_128_concat(encoded)), at])
            need(details, f"Assets.Asset({asset}) absent")
            _, status = decode_asset_details(hexbytes(details))
            metadata = rpc_value(base, "state_getStorage", [storage_key("Assets", "Metadata", blake2_128_concat(encoded)), at])
            need(metadata, f"Assets.Metadata({asset}) absent")
            symbol, decimals = decode_asset_metadata(hexbytes(metadata))
            if asset in decimals_by_id:
                need(decimals == decimals_by_id[asset], f"asset {asset}: {decimals} decimals, catalog says {decimals_by_id[asset]}")
            held = rpc_value(base, "state_getStorage", [storage_key(
                "Assets", "Account", blake2_128_concat(encoded) + blake2_128_concat(holder)), at])
            amount = decode_asset_account(hexbytes(held))[0] if held else 0
            if asset == holding["asset"]:
                need(amount > 0, f"the sample holder holds none of asset {asset}")
            answers.append(f"{asset} {symbol} {status} {decimals}dp: {amount}")
        return ok("; ".join(answers))

    def staking():
        at = pinned()["best"]
        member = shared_sample(("substrate-pools", chain), lambda: from_any(row, rows, derive_pool_member))
        sample["staking"] = member
        owner = hexbytes(member["member_account"])
        minimum = rpc_value(base, "state_getStorage", [storage_key("NominationPools", "MinJoinBond"), at])
        need(minimum and len(hexbytes(minimum)) == 16, f"MinJoinBond: {minimum!r}")
        value = rpc_value(base, "state_getStorage", [storage_key("NominationPools", "PoolMembers", twox64_concat(owner)), at])
        need(value, "PoolMembers: the sample member is absent")
        raw = hexbytes(value)
        pool, points = int.from_bytes(raw[:4], "little"), int.from_bytes(raw[4:20], "little")
        eras, i = scale_compact(raw, 36)
        need(len(raw) == i + 20 * eras, f"PoolMember record of {len(raw)} bytes")
        active = state_call(base, "NominationPoolsApi_points_to_balance",
                            pool.to_bytes(4, "little") + points.to_bytes(16, "little"), at)
        need(len(active) == 16, f"points_to_balance: {active.hex()}")
        problems = []
        pending = state_call(base, "NominationPoolsApi_pending_rewards", owner, at)
        if len(pending) != 16:
            problems.append(f"NominationPoolsApi_pending_rewards answered {len(pending)} bytes (0x{pending.hex()}), "
                            "not the u128 core's nomination_pool_snapshot decodes")
        for method, data in (("member_needs_delegate_migration", owner),
                             ("pool_needs_delegate_migration", pool.to_bytes(4, "little"))):
            flag = state_call(base, f"NominationPoolsApi_{method}", data, at)
            need(flag in (b"\x00", b"\x01"), f"{method}: {flag.hex()}")
        slash = state_call(base, "NominationPoolsApi_member_pending_slash", owner, at)
        need(len(slash) == 16, f"member_pending_slash: {slash.hex()}")
        era = rpc_value(base, "state_getStorage", [storage_key("Staking", "ActiveEra"), at])
        need(era, "Staking.ActiveEra absent")
        if eras:
            subpools = rpc_value(base, "state_getStorage", [storage_key(
                "NominationPools", "SubPoolsStorage", twox64_concat(pool.to_bytes(4, "little"))), at])
            need(subpools, "SubPoolsStorage absent for a member that is unbonding")
        read = (f"member of pool {pool}, {points} points, active {int.from_bytes(active, 'little')}, "
                f"{eras} unbonding eras; active era {int.from_bytes(hexbytes(era)[:4], 'little')}")
        if problems:
            return fail("; ".join(problems) + f" (the other reads answered: {read})")
        return ok(read)

    checks = {"balance": balance, "fee": fee, "broadcast": broadcast, "verification": verification}
    if "token-balance" in row["capabilities"]:
        checks["token-balance"] = token_balance
    if "staking" in row["capabilities"]:
        checks["staking"] = staking
    capabilities = {name: attempt(check) for name, check in checks.items()}
    notes.append("Storage hashers and SCALE layouts are the probe's own (core reads them from metadata); "
                 "the fee is payment_queryInfo for an extrinsic already on chain, not a built transfer.")
    sample.pop("extrinsic", None)
    return {"capabilities": capabilities, "sample": sample, "runtime": {k: v for k, v in context.items() if k != "error"},
            "notes": notes}


# ══ Koios (core/src/api/koios.rs) ═══════════════════════════════════════════

def koios_post(base, path, body):
    return POST(f"{base}{path}", body)


KOIOS_LEAN = {"_inputs": False, "_metadata": False, "_assets": True, "_withdrawals": False, "_certs": False,
              "_scripts": False, "_bytecode": False}
# A network's catalog endpoints may all be down; its sample then comes from
# Koios's own public instance for that network, named in the sample.
KOIOS_SAMPLE_FALLBACK = {"cardano-preprod": ("preprod.koios.rest (outside the catalog)", "https://preprod.koios.rest/api/v1")}


def derive_koios(chain):
    """A key-payment address that received the catalog's first token on
    `chain` in a recent block, or, where the catalog lists none, any native
    asset."""
    wanted = common.tokens(chain)[0]["contract"] if common.tokens(chain) else None
    tried = set()

    def ordinary(base, address):
        """Whether the address's UTXO set is small enough to list quickly:
        an exchange's hot wallet holds thousands, which no endpoint lists in time."""
        if address in tried or len(tried) >= 8:
            return False
        tried.add(address)
        listed = _answer(base, lambda: common.request(
            "POST", f"{base}/address_utxos", json_body={"_addresses": [address], "_extended": True},
            timeout=15, retries=0))
        return listed.status == 200 and isinstance(listed.json, list) and len(listed.json) <= 300

    def derive(base):
        tip = body_of(GET(f"{base}/tip"), "tip")[0]["block_height"]
        blocks = body_of(GET(f"{base}/blocks?block_height=gte.{tip - 90}&select=hash,tx_count,block_height"), "blocks")
        hashes = [b["hash"] for b in sorted(blocks, key=lambda b: -b["block_height"]) if b.get("tx_count")]
        need(hashes, "no recent block carries transactions")
        for start in range(0, len(hashes), 5):
            refs = body_of(koios_post(base, "/block_txs", {"_block_hashes": hashes[start:start + 5]}), "block_txs")
            tx_hashes = [ref["tx_hash"] for ref in refs]
            for offset in range(0, len(tx_hashes), 40):
                infos = body_of(koios_post(base, "/tx_info", {"_tx_hashes": tx_hashes[offset:offset + 40],
                                                              **KOIOS_LEAN}), "tx_info")
                for info in infos:
                    for output in info.get("outputs", []):
                        address = output.get("payment_addr", {}).get("bech32", "")
                        if not address.startswith(("addr1q", "addr_test1q")):
                            continue
                        held = [f"{a['policy_id']}.{a['asset_name']}" for a in output.get("asset_list") or []]
                        asset = wanted if wanted in held else (None if wanted else (held or [None])[0])
                        if asset and ordinary(base, address):
                            return {"address": address, "tx": info["tx_hash"], "asset": asset,
                                    "how": "a key-payment output receiving the asset in the last 90 blocks, "
                                           "with at most 300 UTXOs"}
        raise Bad(f"no key-payment output received {wanted or 'a native asset'} in the last 90 blocks")
    return derive


def probe_koios(row):
    chain, base = row["chain_id"], row["endpoint"].rstrip("/")
    rows = common.endpoints("koios", chain)
    extra = [KOIOS_SAMPLE_FALLBACK[chain]] if chain in KOIOS_SAMPLE_FALLBACK else []
    try:
        sample = shared_sample(("koios", chain), lambda: from_any(row, rows, derive_koios(chain), extra))
    except Bad as error:
        return no_sample(row, error)
    address, txhash, asset = sample["address"], sample["tx"], sample.get("asset")
    decimals_by_asset = {token["contract"]: token["decimals"] for token in common.tokens(chain)}
    cache = {}

    def utxos():
        if "utxos" not in cache:
            rows_ = body_of(koios_post(base, "/address_utxos", {"_addresses": [address], "_extended": True}),
                            "address_utxos")
            need(isinstance(rows_, list), "address_utxos: not a list")
            for utxo in rows_:
                need(isinstance(utxo.get("tx_hash"), str) and isinstance(utxo.get("tx_index"), int),
                     f"address_utxos: a row without tx_hash/tx_index: {json.dumps(utxo)[:160]}")
                need(digits(utxo.get("value")), "address_utxos: value is not a number")
                need(isinstance(utxo.get("asset_list"), list), "address_utxos: no asset_list (is _extended honoured?)")
                need(isinstance(utxo.get("is_spent", False), bool), "address_utxos: is_spent is not a bool")
            cache["utxos"] = [u for u in rows_ if not u.get("is_spent")]
        return cache["utxos"]

    def held_assets():
        held = {}
        for utxo in utxos():
            for item in utxo["asset_list"]:
                need(isinstance(item.get("policy_id"), str) and digits(item.get("quantity")),
                     f"asset_list entry: {json.dumps(item)[:160]}")
                key = f"{item['policy_id']}.{item.get('asset_name') or ''}"
                held[key] = held.get(key, 0) + int(item["quantity"])
        return held

    def asset_info(assets):
        listed = [list(a.split(".", 1)) for a in assets]
        rows_ = body_of(koios_post(base, "/asset_info", {"_asset_list": listed}), "asset_info")
        found = {}
        for info in rows_:
            need(isinstance(info.get("policy_id"), str), "asset_info: a row without policy_id")
            cip68 = (((info.get("cip68_metadata") or {}).get("333") or {}).get("fields") or [{}])[0].get("map") or []
            places = next((entry.get("v", {}).get("int") for entry in cip68
                           if entry.get("k", {}).get("bytes") == "646563696d616c73"), None)
            if places is None:
                places = (info.get("token_registry_metadata") or {}).get("decimals")
            need(places is None or (isinstance(places, int) and 0 <= places <= 38), f"asset_info: decimals {places!r}")
            found[f"{info['policy_id']}.{info.get('asset_name') or ''}"] = places or 0
        return found

    def balance():
        rows_ = body_of(koios_post(base, "/address_info", {"_addresses": [address]}), "address_info")
        need(rows_ and digits(rows_[0].get("balance")), f"address_info: {json.dumps(rows_)[:160]}")
        return ok(f"{rows_[0]['balance']} lovelace")

    def utxo():
        return ok(f"{len(utxos())} unspent outputs")

    def token_discovery():
        held = held_assets()
        if asset:
            need(asset in held, f"the sample's asset {asset} is not among its UTXOs' assets")
        decimals = asset_info(list(held)[:20]) if held else {}
        return ok(f"{len(held)} assets held; asset_info answered {len(decimals)}")

    def token_balance():
        need(asset, "no native asset found for a sample on this network")
        decimals = asset_info([asset])
        need(asset in decimals, f"asset_info did not answer for {asset}")
        if asset in decimals_by_asset:
            need(decimals[asset] == decimals_by_asset[asset],
                 f"{asset}: {decimals[asset]} decimals, catalog says {decimals_by_asset[asset]}")
        amount = held_assets().get(asset, 0)
        need(amount > 0, f"the sample holds none of {asset}")
        return ok(f"{amount} of {asset} at {decimals[asset]} decimals")

    def transactions():
        if "txs" not in cache:
            refs = body_of(koios_post(base, "/address_txs?order=block_height.desc,tx_hash.desc&limit=20&offset=0",
                                      {"_addresses": [address]}), "address_txs")
            need(refs and all(isinstance(r.get("tx_hash"), str) for r in refs), f"address_txs: {json.dumps(refs)[:160]}")
            infos = body_of(koios_post(base, "/tx_info", {"_tx_hashes": [r["tx_hash"] for r in refs],
                                                          "_inputs": True, "_assets": True}), "tx_info")
            need(len(infos) == len(refs), f"tx_info answered {len(infos)} of {len(refs)} transactions")
            for info in infos:
                need(digits(info.get("fee")), f"tx_info: fee {info.get('fee')!r}")
                need(isinstance(info.get("tx_timestamp"), int), "tx_info: no tx_timestamp")
                need(isinstance(info.get("block_height"), int), "tx_info: no block_height")
                for io in info.get("inputs", []) + info.get("outputs", []):
                    need(isinstance(io.get("payment_addr", {}).get("bech32"), str) and digits(io.get("value")),
                         f"tx_info: an input or output without address or value: {json.dumps(io)[:160]}")
                    need(isinstance(io.get("asset_list", []), list), "tx_info: asset_list is not a list")
            cache["txs"] = infos
        return cache["txs"]

    def history():
        infos = transactions()
        mine = sum(any(io["payment_addr"]["bech32"] == address for io in i.get("inputs", []) + i.get("outputs", []))
                   for i in infos)
        need(mine, "no transaction on the first page moves the sample's address")
        return ok(f"{len(infos)} transactions on the first page, {mine} touch the address")

    def token_history():
        moved = 0
        for info in transactions():
            for io in info.get("inputs", []) + info.get("outputs", []):
                if io["payment_addr"]["bech32"] == address:
                    for item in io.get("asset_list", []):
                        need(isinstance(item.get("policy_id"), str) and digits(item.get("quantity")),
                             f"tx_info asset_list entry: {json.dumps(item)[:160]}")
                        moved += 1
        need(moved or not asset, "the first page carries no asset movement for an address that holds one")
        return ok(f"{moved} asset entries on the address's inputs and outputs")

    def fee():
        params = body_of(GET(f"{base}/epoch_params?order=epoch_no.desc&limit=1"), "epoch_params")
        need(params, "epoch_params: empty")
        values = {}
        for name in ("min_fee_a", "min_fee_b", "coins_per_utxo_size", "max_tx_size", "max_val_size"):
            value = params[0].get(name)
            need(isinstance(value, int) or digits(value), f"epoch_params: {name} {value!r}")
            values[name] = int(value)
        tip = body_of(GET(f"{base}/tip"), "tip")
        need(tip and isinstance(tip[0].get("abs_slot"), int), f"tip: {json.dumps(tip)[:120]}")
        return ok(f"epoch {params[0].get('epoch_no')}: a={values['min_fee_a']} b={values['min_fee_b']}; "
                  f"slot {tip[0]['abs_slot']}")

    def broadcast():
        return refusal(RAW(f"{base}/submittx", b"\x00", "application/cbor"))

    def verification():
        genesis = body_of(GET(f"{base}/genesis"), "genesis")
        magic = genesis[0].get("networkmagic") if genesis else None
        need(str(magic) == str(CARDANO_MAGIC[chain]), f"genesis network magic {magic!r}, not {CARDANO_MAGIC[chain]}")
        rows_ = body_of(koios_post(base, "/tx_info", {"_tx_hashes": [txhash], "_scripts": True}), "tx_info")
        need(len(rows_) == 1 and rows_[0].get("tx_hash", "").lower() == txhash.lower(), f"tx_info: {json.dumps(rows_)[:160]}")
        contracts = rows_[0].get("plutus_contracts")
        need(isinstance(contracts, list) and all(isinstance(c.get("valid_contract"), bool) for c in contracts),
             f"tx_info's plutus_contracts give no phase-2 result: {json.dumps(contracts)[:160]}")
        need(isinstance(rows_[0].get("block_height"), int), "tx_info: block_height is not a number")
        return ok(f"{txhash} at block {rows_[0]['block_height']}, {len(contracts)} Plutus contracts, "
                  f"succeeded {all(c['valid_contract'] for c in contracts)}")

    checks = {"balance": balance, "utxo": utxo, "token-discovery": token_discovery, "token-balance": token_balance,
              "history": history, "token-history": token_history, "fee": fee, "broadcast": broadcast,
              "verification": verification}
    return {"capabilities": {name: attempt(check) for name, check in checks.items()}, "sample": sample}


# ══ algod and Algorand's indexer (core/src/api/algod.rs, algorand_indexer.rs) ══

def algorand_genesis(value):
    return value.get("genesis-id") == ALGORAND_GENESIS_ID and value.get("genesis-hash") == ALGORAND_GENESIS_HASH


def derive_algorand(base):
    round_ = body_of(GET(f"{base}/health"), "indexer health").get("round")
    need(isinstance(round_, int), "indexer health: no round")
    body = body_of(GET(f"{base}/v2/transactions?tx-type=pay&limit=20&min-round={round_ - 10}"), "recent payments")
    for tx in body.get("transactions", []):
        payment = tx.get("payment-transaction", {})
        if (payment.get("amount", 0) > 0 and payment.get("receiver") != tx.get("sender")
                and not payment.get("close-remainder-to")):
            return {"address": tx["sender"], "tx": tx["id"], "round": tx.get("confirmed-round"),
                    "how": "the sender of a payment in the indexer's last ten rounds"}
    raise Bad("no payment in the last ten rounds")


def algorand_sample(row):
    return shared_sample(("algorand", row["chain_id"]), lambda: from_any(
        row if row["api"] == "algorand-indexer" else common.endpoints("algorand-indexer", row["chain_id"])[0],
        common.endpoints("algorand-indexer", row["chain_id"]), derive_algorand))


def probe_algod(row):
    base = row["endpoint"].rstrip("/")
    try:
        sample = algorand_sample(row)
    except Bad as error:
        return no_sample(row, error)
    address, txid = sample["address"], sample["tx"]

    def fee():
        params = body_of(GET(f"{base}/v2/transactions/params"), "params")
        need(algorand_genesis(params), f"genesis {params.get('genesis-id')} {params.get('genesis-hash')}")
        need(isinstance(params.get("last-round"), int) and params["last-round"] > 0, "params: no last-round")
        need(isinstance(params.get("min-fee"), int) and params["min-fee"] > 0, "params: no min-fee")
        need(isinstance(params.get("fee"), int), "params: no fee")
        return ok(f"round {params['last-round']}, min-fee {params['min-fee']}, fee/byte {params['fee']}")

    def balance():
        account = body_of(GET(f"{base}/v2/accounts/{address}?exclude=all"), "account")
        need(account.get("address") == address, "account: another address")
        need(isinstance(account.get("amount"), int) and isinstance(account.get("min-balance"), int),
             f"account: amount or min-balance missing: {json.dumps(account)[:160]}")
        return ok(f"{account['amount']} microAlgos, min-balance {account['min-balance']}")

    def broadcast():
        return refusal(RAW(f"{base}/v2/transactions", b"\x00", "application/x-binary"))

    def verification():
        params = body_of(GET(f"{base}/v2/transactions/params"), "params")
        need(algorand_genesis(params), "params: another network")
        reply = GET(f"{base}/v2/transactions/pending/{txid}")
        if reply.status == 404:
            message = (reply.json or {}).get("message", "") if isinstance(reply.json, dict) else ""
            need("txn" in message.lower() or "transaction" in message.lower(), f"pending: {reply.brief()}")
            return ok(f"{txid} not held (404 '{message[:80]}'); core then asks the indexer")
        pending = body_of(reply, "pending")
        need(isinstance(pending.get("txn"), dict), "pending: no txn")
        need(isinstance(pending.get("confirmed-round"), int) and pending["confirmed-round"] > 0,
             f"pending: confirmed-round {pending.get('confirmed-round')!r}")
        return ok(f"{txid} confirmed in round {pending['confirmed-round']}")

    checks = {"balance": balance, "fee": fee, "broadcast": broadcast, "verification": verification}
    return {"capabilities": {name: attempt(check) for name, check in checks.items()}, "sample": sample}


def probe_algorand_indexer(row):
    base = row["endpoint"].rstrip("/")
    try:
        sample = algorand_sample(row)
    except Bad as error:
        return no_sample(row, error)
    address, txid = sample["address"], sample["tx"]

    def history():
        body = body_of(GET(f"{base}/v2/accounts/{address}/transactions?limit=50"), "account transactions")
        txs = need(body.get("transactions") if isinstance(body.get("transactions"), list) else None, "no transactions list")
        need(txs, "no transactions for an address that just paid")
        for tx in txs:
            need(algorand_genesis(tx), f"a transaction without the network's genesis id and hash: {tx.get('id')}")
            need(isinstance(tx.get("id"), str) and isinstance(tx.get("tx-type"), str), "a transaction without id or type")
            if tx["tx-type"] == "pay":
                payment = tx.get("payment-transaction", {})
                need(isinstance(tx.get("sender"), str) and isinstance(payment.get("receiver"), str)
                     and isinstance(payment.get("amount"), int), f"payment {tx['id']}: sender, receiver or amount missing")
        lookup = body_of(GET(f"{base}/v2/transactions/{txid}"), "transaction by id").get("transaction", {})
        need(lookup.get("id") == txid and algorand_genesis(lookup), f"transaction by id: {json.dumps(lookup)[:120]}")
        need(isinstance(lookup.get("confirmed-round"), int) and lookup["confirmed-round"] > 0, "no confirmed-round")
        return ok(f"{len(txs)} transactions, next-token {'present' if body.get('next-token') else 'absent'}; "
                  f"{txid} confirmed in round {lookup['confirmed-round']}")

    return {"capabilities": {"history": attempt(history)}, "sample": sample}


# ══ ICP Rosetta (core/src/api/icp_rosetta.rs) ═══════════════════════════════

ICP_NETWORK = {"blockchain": "Internet Computer", "network": ICP_LEDGER_HEX}
ICP_CURRENCY = {"symbol": "ICP", "decimals": 8}


def rosetta(base, path, body):
    return POST(f"{base}{path}", {"network_identifier": ICP_NETWORK, **body})


def derive_icp(base):
    status = body_of(rosetta(base, "/network/status", {}), "network status")
    head = status["current_block_identifier"]["index"]
    for index in range(head, head - 40, -1):
        block = body_of(rosetta(base, "/block", {"block_identifier": {"index": index}}), f"block {index}")["block"]
        for tx in block.get("transactions", []):
            operations = tx.get("operations", [])
            senders = [op for op in operations if op.get("type") == "TRANSACTION"
                       and str(op.get("amount", {}).get("value", "")).startswith("-")]
            receivers = [op for op in operations if op.get("type") == "TRANSACTION"
                         and not str(op.get("amount", {}).get("value", "-")).startswith("-")]
            if senders and receivers:
                held = rosetta(base, "/account/balance", {"account_identifier": {"address": senders[0]["account"]["address"]}})
                value = ((held.json or {}).get("balances") or [{}])[0].get("value") if isinstance(held.json, dict) else None
                if not (digits(value) and int(value) > 0):
                    continue
                return {"address": senders[0]["account"]["address"], "recipient": receivers[0]["account"]["address"],
                        "tx": tx["transaction_identifier"]["hash"], "block": index,
                        "how": "the sender, still holding ICP, of a ledger transfer in a recent block"}
    raise Bad("no transfer in the last 40 blocks")


def probe_icp_rosetta(row):
    base = row["endpoint"].rstrip("/")
    try:
        sample = shared_sample(("icp", "rosetta"), lambda: from_any(row, [row], derive_icp))
    except Bad as error:
        return no_sample(row, error)
    address = sample["address"]

    def balance():
        body = body_of(rosetta(base, "/account/balance", {"account_identifier": {"address": address}}), "balance")
        value = ((body.get("balances") or [{}])[0]).get("value")
        need(digits(value), f"balance: {json.dumps(body)[:160]}")
        return ok(f"{value} e8s")

    def history():
        body = body_of(rosetta(base, "/search/transactions",
                               {"account_identifier": {"address": address}, "limit": 50, "offset": 0}), "search")
        txs = body.get("transactions")
        need(isinstance(txs, list) and txs, f"search: no transactions for {address}")
        for tx in txs:
            need(isinstance(tx.get("block_identifier", {}).get("index"), int), "a result without a block index")
            need(isinstance(tx.get("transaction", {}).get("metadata", {}).get("timestamp"), int), "a result without a timestamp")
            for op in tx["transaction"].get("operations", []):
                if op.get("type") in ("TRANSACTION", "MINT", "BURN", "FEE"):
                    need(re.fullmatch(r"-?\d+", str(op.get("amount", {}).get("value"))), "an operation without an amount")
        if len(txs) == 50 and body.get("next_offset") is None:
            need(isinstance(body.get("total_count"), int), "a full page with neither next_offset nor total_count")
        return ok(f"{len(txs)} transactions; next_offset {body.get('next_offset')}, total_count {body.get('total_count')}")

    def fee():
        operations = [
            {"operation_identifier": {"index": 0}, "type": "TRANSACTION", "account": {"address": address},
             "amount": {"value": "-1", "currency": ICP_CURRENCY}},
            {"operation_identifier": {"index": 1}, "type": "TRANSACTION", "account": {"address": sample["recipient"]},
             "amount": {"value": "1", "currency": ICP_CURRENCY}},
            {"operation_identifier": {"index": 2}, "type": "FEE", "account": {"address": address},
             "amount": {"value": "-10000", "currency": ICP_CURRENCY}},
        ]
        preprocess = body_of(rosetta(base, "/construction/preprocess", {"operations": operations}), "preprocess")
        options = preprocess.get("options")
        request = {} if options is None else {"options": options}
        metadata = body_of(rosetta(base, "/construction/metadata", request), "metadata")
        fees = metadata.get("suggested_fee")
        need(isinstance(fees, list) and len(fees) == 1 and fees[0].get("currency") == ICP_CURRENCY
             and digits(fees[0].get("value")), f"suggested_fee: {json.dumps(fees)[:160]}")
        return ok(f"suggested fee {fees[0]['value']} e8s")

    def broadcast():
        return refusal(rosetta(base, "/construction/submit", {"signed_transaction": "00"}))

    def verification():
        txhash = sample["tx"]
        body = body_of(rosetta(base, "/search/transactions", {"transaction_identifier": {"hash": txhash}, "limit": 2}),
                       "search by hash")
        txs = body.get("transactions") or []
        need(len(txs) == 1, f"{len(txs)} results for {txhash}")
        tx = txs[0]
        need(tx.get("transaction", {}).get("transaction_identifier", {}).get("hash", "").lower() == txhash.lower(),
             "another transaction")
        operations = tx["transaction"].get("operations") or []
        need(operations and all(op.get("status") == "COMPLETED" for op in operations), "operations not COMPLETED")
        need(isinstance(tx.get("block_identifier", {}).get("index"), int), "no block index")
        return ok(f"{txhash} in block {tx['block_identifier']['index']}, {len(operations)} completed operations")

    checks = {"balance": balance, "history": history, "fee": fee, "broadcast": broadcast, "verification": verification}
    return {"capabilities": {name: attempt(check) for name, check in checks.items()}, "sample": sample}


# ══ The IC's HTTP interface (core/src/api/icp_replica.rs) ═══════════════════

def principal_bytes(text):
    packed = text.replace("-", "").upper()
    packed += "=" * (-len(packed) % 8)
    return base64.b32decode(packed)[4:]


def principal_text(raw):
    packed = base64.b32encode(zlib.crc32(raw).to_bytes(4, "big") + raw).decode().lower().rstrip("=")
    return "-".join(packed[i:i + 5] for i in range(0, len(packed), 5))


ANONYMOUS = b"\x04"


def ic_envelope(content):
    content = {**content, "sender": ANONYMOUS, "ingress_expiry": time.time_ns() + 180 * 10**9}
    return cbor_encode(Tag(55799, {"content": content}))


def ic_post(base, canister, route, body):
    return _answer(base, lambda: common.request(
        "POST", f"{base}/api/v2/canister/{canister}/{route}", data=body,
        headers={"Content-Type": "application/cbor", "Accept": "application/cbor"}))


def probe_icp_replica(row):
    base = row["endpoint"].rstrip("/")
    ledger = principal_text(bytes.fromhex(ICP_LEDGER_HEX))
    notes = []

    def health():
        reply = _answer(base, lambda: common.request("GET", f"{base}/api/v2/status", headers={"Accept": "application/cbor"}))
        need(reply.status == 200, f"status: {reply.brief()}")
        status = cbor_whole(reply.body)
        need(isinstance(status, CborMap) and not status.duplicates, "status: not a CBOR map with distinct fields")
        need(status.get("replica_health_status") == "healthy", f"replica_health_status {status.get('replica_health_status')!r}")
        return f"healthy, ic_api_version {status.get('ic_api_version')}"

    try:
        state = ok(health())
    except Exception as error:  # noqa: BLE001
        state = fail(f"/api/v2/status: {error}")

    def broadcast():
        reply = ic_post(base, ledger, "call", b"\x00")
        return refusal(reply)

    def verification():
        need(state["ok"], state["detail"])
        envelope = ic_envelope({"request_type": "read_state",
                                "paths": [[b"time"], [b"request_status", bytes(32)]]})
        reply = ic_post(base, ICP_GOVERNANCE, "read_state", envelope)
        need(reply.status == 200, f"read_state: {reply.brief()}")
        answer = cbor_whole(reply.body)
        certificate = cbor_whole(need(answer.get("certificate"), "read_state: no certificate"))
        need(isinstance(certificate.get("tree"), list) and isinstance(certificate.get("signature"), bytes),
             "certificate: no tree or signature")
        return ok(f"read_state answered a certificate ({len(answer['certificate'])} bytes, "
                  f"{'with' if 'delegation' in certificate else 'without'} delegation); "
                  "its BLS signature is not checked here")

    def staking():
        need(state["ok"], state["detail"])
        envelope = ic_envelope({"request_type": "query", "canister_id": principal_bytes(ICP_GOVERNANCE),
                                "method_name": "list_known_neurons", "arg": b"DIDL\x00\x00"})
        reply = ic_post(base, ICP_GOVERNANCE, "query", envelope)
        need(reply.status == 200, f"query: {reply.brief()}")
        answer = cbor_whole(reply.body)
        need(answer.get("status") == "replied", f"query status {answer.get('status')!r}: {answer.get('reject_message')}")
        arg = (answer.get("reply") or {}).get("arg")
        need(isinstance(arg, bytes) and arg.startswith(b"DIDL"), "query: reply is not Candid")
        return ok(f"governance list_known_neurons replied with {len(arg)} bytes of Candid")

    notes.append("Unchecked here, core's `spectra endpoints probe` and staking flows cover them: certificate "
                 "BLS verification, and the owner-signed list_neurons query (this probe's query is anonymous).")
    return {"capabilities": {"broadcast": attempt(broadcast), "verification": attempt(verification),
                             "staking": attempt(staking)},
            "sample": {"ledger": ledger, "governance": ICP_GOVERNANCE}, "status": state, "notes": notes}


# ══ VeChainThor REST (core/src/api/thor_rest.rs) ═══════════════════════════

TRANSFER_TOPIC = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
SEL_BALANCE_OF = "70a08231"
SEL_DECIMALS = "313ce567"


def thor_quantity(text):
    return isinstance(text, str) and re.fullmatch(r"0x[0-9a-fA-F]{1,32}", text) is not None


def derive_vechain(base):
    best = body_of(GET(f"{base}/blocks/best"), "best block")["number"]
    logs = body_of(POST(f"{base}/logs/transfer", {
        "range": {"unit": "block", "from": best - 360, "to": best},
        "options": {"offset": 0, "limit": 50}, "order": "desc"}), "recent transfers")
    senders = []
    for log in logs:
        if int(log.get("amount", "0x0"), 16) > 0 and log["sender"] not in senders:
            senders.append(log["sender"])
    energy = common.tokens("vechain")[0]["contract"] if common.tokens("vechain") else None
    for sender in senders[:15]:
        held = body_of(GET(f"{base}/accounts/{sender}"), "account")
        if int(held.get("balance", "0x0"), 16) == 0 or int(held.get("energy", "0x0"), 16) == 0:
            continue
        if energy:
            topic = "0x" + "0" * 24 + sender[2:].lower()
            events = body_of(POST(f"{base}/logs/event", {
                "range": {"unit": "block", "from": 0, "to": best}, "options": {"offset": 0, "limit": 1},
                "criteriaSet": [{"address": energy, "topic0": TRANSFER_TOPIC, "topic1": topic},
                                {"address": energy, "topic0": TRANSFER_TOPIC, "topic2": topic}],
                "order": "desc"}), "token events")
            if not events:
                continue
        log = next(entry for entry in logs if entry["sender"] == sender)
        return {"address": sender, "tx": log["meta"]["txID"], "block": log["meta"]["blockNumber"],
                "how": "the sender of a VET transfer in the last hour that has also moved the catalog token"}
    raise Bad("no recent VET sender with token transfers")


def probe_thor(row):
    base = row["endpoint"].rstrip("/")
    try:
        sample = shared_sample(("thor", row["chain_id"]), lambda: from_any(
            row, common.endpoints("thor-rest", row["chain_id"]), derive_vechain))
    except Bad as error:
        return no_sample(row, error)
    address, txid = sample["address"], sample["tx"]
    tokens = common.tokens(row["chain_id"])
    genesis = GET(f"{base}/blocks/0")
    identity = (ok("genesis matches") if isinstance(genesis.json, dict) and genesis.json.get("number") == 0
                and str(genesis.json.get("id", "")).lower() == VECHAIN_GENESIS
                else fail(f"blocks/0: {genesis.brief(160)}"))

    def needs_identity():
        if not identity["ok"]:
            raise Bad(f"network check: {identity['detail']}")

    def account():
        body = body_of(GET(f"{base}/accounts/{address}"), "account")
        need(thor_quantity(body.get("balance")) and thor_quantity(body.get("energy")),
             f"account: balance or energy not a hex quantity: {json.dumps(body)[:160]}")
        return body

    def call(clauses, caller=None, revision="best"):
        body = {"clauses": clauses}
        if caller:
            body["caller"] = caller
        results = body_of(POST(f"{base}/accounts/*?revision={revision}", body), "call")
        need(isinstance(results, list) and len(results) == len(clauses), f"call: {json.dumps(results)[:160]}")
        for result in results:
            need(isinstance(result.get("data"), str) and isinstance(result.get("gasUsed"), int)
                 and isinstance(result.get("reverted"), bool), f"call result: {json.dumps(result)[:160]}")
        return results

    def balance():
        body = account()
        return ok(f"{int(body['balance'], 16)} wei VET, {int(body['energy'], 16)} VTHO")

    def best_block():
        block = body_of(GET(f"{base}/blocks/best"), "best block")
        need(isinstance(block.get("number"), int) and str(block.get("id", ""))[2:10] == f"{block['number']:08x}",
             "best block: id does not carry its number")
        need(block.get("baseFeePerGas") is None or thor_quantity(block["baseFeePerGas"]), "best block: bad baseFeePerGas")
        return block

    def log_page(path, criteria, to):
        logs = body_of(POST(f"{base}{path}", {"range": {"unit": "block", "from": 0, "to": to},
                                               "options": {"offset": 0, "limit": 50},
                                               "criteriaSet": criteria, "order": "desc"}), path)
        need(isinstance(logs, list) and len(logs) <= 50, f"{path}: not a list of at most 50")
        for log in logs:
            meta = log.get("meta", {})
            need(re.fullmatch(r"0x[0-9a-f]{64}", str(meta.get("txID", ""))) and isinstance(meta.get("blockNumber"), int)
                 and isinstance(meta.get("blockTimestamp"), int), f"{path}: a log without txID, block or time")
        return logs

    def history():
        to = best_block()["number"]
        logs = log_page("/logs/transfer", [{"sender": address}, {"recipient": address}], to)
        need(logs, "no VET transfers for an address that just sent one")
        for log in logs:
            need(thor_quantity(log.get("amount")) and log.get("sender") and log.get("recipient"), "a transfer without amount or parties")
        return ok(f"{len(logs)} VET transfers on the first page")

    def token_history():
        to = best_block()["number"]
        topic = "0x" + "0" * 24 + address[2:].lower()
        criteria = [c for t in tokens for c in (
            {"address": t["contract"], "topic0": TRANSFER_TOPIC, "topic1": topic},
            {"address": t["contract"], "topic0": TRANSFER_TOPIC, "topic2": topic})]
        logs = log_page("/logs/event", criteria, to)
        need(logs, "no token transfers for an address chosen for having them")
        for log in logs:
            need(len(log.get("topics", [])) == 3 and isinstance(log.get("data"), str), "a Transfer event without topics or data")
        return ok(f"{len(logs)} token Transfer events on the first page")

    def token_balance():
        answers = []
        for token in tokens:
            data = "0x" + SEL_BALANCE_OF + "0" * 24 + address[2:].lower()
            results = call([{"to": token["contract"], "value": "0x0", "data": data},
                            {"to": token["contract"], "value": "0x0", "data": "0x" + SEL_DECIMALS}])
            need(not any(r["reverted"] for r in results), f"{token['token_id']}: reverted {results}")
            decimals = int(results[1]["data"], 16)
            need(decimals == token["decimals"], f"{token['token_id']}: {decimals} decimals, catalog says {token['decimals']}")
            answers.append(f"{token['token_id']} {int(results[0]['data'], 16)} at {decimals}dp")
        return ok("; ".join(answers))

    def fee():
        needs_identity()
        block = best_block()
        tip = body_of(GET(f"{base}/fees/priority"), "fees/priority").get("maxPriorityFeePerGas")
        need(thor_quantity(tip), f"fees/priority: {tip!r}")
        account()
        result = call([{"to": address, "value": "0x1", "data": "0x"}], caller=address, revision="next")[0]
        need(not result["reverted"] and not result.get("vmError"), f"simulation failed: {result}")
        return ok(f"block {block['number']} base fee {block.get('baseFeePerGas')}, tip {tip}; "
                  f"simulated 1-wei self-transfer used {result['gasUsed']} gas")

    def broadcast():
        reply = POST(f"{base}/transactions", {"raw": "0x00"})
        return refusal(reply)

    def verification():
        needs_identity()
        receipt = body_of(GET(f"{base}/transactions/{txid}/receipt"), "receipt")
        need(isinstance(receipt, dict), f"no receipt for {txid}")
        need(str(receipt.get("meta", {}).get("txID", "")).lower() == txid.lower(), "receipt of another transaction")
        need(isinstance(receipt.get("reverted"), bool) and isinstance(receipt["meta"].get("blockNumber"), int),
             "receipt without reverted or block number")
        return ok(f"{txid} in block {receipt['meta']['blockNumber']}, reverted {receipt['reverted']}")

    checks = {"balance": balance, "history": history, "token-history": token_history, "token-balance": token_balance,
              "fee": fee, "broadcast": broadcast, "verification": verification}
    return {"capabilities": {name: attempt(check) for name, check in checks.items()}, "sample": sample,
            "identity": identity}


# ══ Lotus JSON-RPC and Filfox (core/src/api/lotus_json_rpc.rs, filfox.rs) ═══

MAX_FEE = "70000000000000000"  # send::filecoin::MAX_FEE


def derive_filecoin(base):
    head = rpc_value(base, "Filecoin.ChainHead")["Height"]
    for back in range(5, 40):
        tipset = rpc_value(base, "Filecoin.ChainGetTipSetByHeight", [head - back, []])
        for block in tipset.get("Cids") or []:
            messages = rpc_value(base, "Filecoin.ChainGetBlockMessages", [block])
            for signed in messages.get("SecpkMessages") or []:
                message = signed["Message"]
                if message.get("Method") == 0 and message.get("Value", "0") != "0" and message["From"].startswith("f1"):
                    cid = (signed.get("CID") or {}).get("/")
                    need(cid, "a signed message without its CID")
                    return {"address": message["From"], "tx": cid, "height": head - back,
                            "how": "the sender of a FIL transfer in a recent tipset"}
    raise Bad("no secp256k1 FIL transfer in 35 tipsets")


def filecoin_sample(row):
    lotus = common.endpoints("lotus-json-rpc", row["chain_id"])
    first = row if row["api"] == "lotus-json-rpc" else lotus[0]
    return shared_sample(("filecoin", row["chain_id"]), lambda: from_any(first, lotus, derive_filecoin))


def probe_lotus(row):
    base = row["endpoint"].rstrip("/")
    try:
        sample = filecoin_sample(row)
    except Bad as error:
        return no_sample(row, error)
    address, cid = sample["address"], sample["tx"]

    def identity():
        name = rpc_value(base, "Filecoin.StateNetworkName")
        need(name == FILECOIN_NETWORK, f"network {name!r}, not {FILECOIN_NETWORK}")
        genesis = rpc_value(base, "Filecoin.ChainGetGenesis")
        need(((genesis.get("Cids") or [{}])[0]).get("/") == FILECOIN_GENESIS, "another genesis block")

    def attofil(value, what):
        need(digits(value), f"{what}: {value!r} is not a whole number of attoFIL")
        return int(value)

    def balance():
        return ok(f"{attofil(rpc_value(base, 'Filecoin.WalletBalance', [address]), 'WalletBalance')} attoFIL")

    def fee():
        identity()
        attofil(rpc_value(base, "Filecoin.WalletBalance", [address]), "WalletBalance")
        actor = rpc_value(base, "Filecoin.StateGetActor", [address, []])
        need(isinstance(actor, dict) and isinstance(actor.get("Nonce"), int), f"StateGetActor: {json.dumps(actor)[:160]}")
        need(actor.get("DelegatedAddress") is None or isinstance(actor["DelegatedAddress"], str), "bad DelegatedAddress")
        nonce = rpc_value(base, "Filecoin.MpoolGetNonce", [address])
        need(isinstance(nonce, int), f"MpoolGetNonce: {nonce!r}")
        message = {"Version": 0, "To": address, "From": address, "Nonce": actor["Nonce"], "Value": "1",
                   "GasLimit": 0, "GasFeeCap": "0", "GasPremium": "0", "Method": 0, "Params": None}
        estimate = rpc_value(base, "Filecoin.GasEstimateMessageGas", [message, {"MaxFee": MAX_FEE}, []])
        for field in ("To", "From", "Value", "Method"):
            need(estimate.get(field) == message[field], f"estimate for another message: {field} {estimate.get(field)!r}")
        need(isinstance(estimate.get("GasLimit"), int) and estimate["GasLimit"] > 0, "estimate: no GasLimit")
        attofil(estimate.get("GasFeeCap"), "GasFeeCap")
        attofil(estimate.get("GasPremium"), "GasPremium")
        return ok(f"GasLimit {estimate['GasLimit']}, fee cap {estimate['GasFeeCap']}, premium {estimate['GasPremium']}; "
                  f"nonce {actor['Nonce']} (mpool {nonce})")

    def broadcast():
        return rpc_refusal(RPC(base, "Filecoin.MpoolPush", []))

    def verification():
        identity()
        found = rpc_value(base, "Filecoin.StateSearchMsg", [[], {"/": cid}, -1, False])
        need(found is not None, f"StateSearchMsg answered null for {cid}, mined at {sample['height']} (reads as pending)")
        need((found.get("Message") or {}).get("/") == cid, "another message")
        need(isinstance((found.get("Receipt") or {}).get("ExitCode"), int) and isinstance(found.get("Height"), int),
             "no exit code or height")
        return ok(f"{cid} at height {found['Height']}, exit code {found['Receipt']['ExitCode']}")

    checks = {"balance": balance, "fee": fee, "broadcast": broadcast, "verification": verification}
    return {"capabilities": {name: attempt(check) for name, check in checks.items()}, "sample": sample}


def probe_filfox(row):
    base = row["endpoint"].rstrip("/")
    try:
        sample = filecoin_sample(row)
    except Bad as error:
        return no_sample(row, error)
    address = sample["address"]

    def history():
        body = body_of(GET(f"{base}/address/{address}/transfers?pageSize=100&page=0&type=transfer"), "transfers")
        need(isinstance(body.get("totalCount"), int), "no totalCount")
        transfers = need(body.get("transfers") if isinstance(body.get("transfers"), list) else None, "no transfers list")
        need(transfers, "no transfers for an address that just sent FIL")
        for transfer in transfers:
            need(transfer.get("message") and re.fullmatch(r"-?\d+", str(transfer.get("value"))),
                 f"a transfer without message or value: {json.dumps(transfer)[:160]}")
        return ok(f"{len(transfers)} of {body['totalCount']} transfers on the first page")

    return {"capabilities": {"history": attempt(history)}, "sample": sample}


# ══ Tezos RPC and TzKT (core/src/api/tezos_rpc.rs, tzkt.rs) ════════════════

EDSIG_ZERO = b58check(bytes([9, 245, 205, 134, 18]) + bytes(64))


def derive_tezos(base):
    for depth in range(3, 15):
        operations = body_of(GET(f"{base}/chains/main/blocks/head~{depth}/operations/3"), "manager operations")
        for operation in operations:
            for content in operation.get("contents", []):
                result = content.get("metadata", {}).get("operation_result", {})
                if (content.get("kind") == "transaction" and str(content.get("source", "")).startswith("tz1")
                        and int(content.get("amount", "0")) > 0 and result.get("status") == "applied"):
                    return {"address": content["source"], "tx": operation["hash"],
                            "how": f"the source of an applied tez transfer at head~{depth}"}
    raise Bad("no applied tz1 transfer in head~3..head~14")


def tezos_sample(row):
    nodes = common.endpoints("tezos-rpc", row["chain_id"])
    first = row if row["api"] == "tezos-rpc" else nodes[0]
    return shared_sample(("tezos", row["chain_id"]), lambda: from_any(first, nodes, derive_tezos))


def probe_tezos(row):
    base = row["endpoint"].rstrip("/")
    try:
        sample = tezos_sample(row)
    except Bad as error:
        return no_sample(row, error)
    address = sample["address"]

    def identity():
        chain_id = body_of(GET(f"{base}/chains/main/chain_id"), "chain_id")
        need(chain_id == TEZOS_CHAIN_ID, f"chain id {chain_id!r}, not {TEZOS_CHAIN_ID}")

    def header(block):
        head = body_of(GET(f"{base}/chains/main/blocks/{block}/header"), f"{block} header")
        need(str(head.get("hash", "")).startswith("B") and isinstance(head.get("level"), int), f"{block} header: no hash or level")
        return head

    def balance():
        value = body_of(GET(f"{base}/chains/main/blocks/head/context/contracts/{address}/balance"), "balance")
        need(digits(value), f"balance: {value!r}")
        return ok(f"{value} mutez")

    def fee():
        identity()
        protocols = body_of(GET(f"{base}/chains/main/blocks/head/protocols"), "protocols")
        need(protocols.get("protocol") and protocols.get("next_protocol"), "protocols: missing")
        branch = header("head~2")["hash"]
        account = body_of(GET(f"{base}/chains/main/blocks/head/context/contracts/{address}"), "account")
        need(digits(account.get("balance")) and digits(account.get("counter")) and isinstance(account.get("revealed"), bool),
             f"account: balance, counter or revealed missing: {json.dumps(account)[:160]}")
        need(account["revealed"], "the sample account is not revealed")
        content = {"kind": "transaction", "source": address, "fee": "0", "counter": str(int(account["counter"]) + 1),
                   "gas_limit": "1040000", "storage_limit": "60000", "amount": "1", "destination": address}
        reply = POST(f"{base}/chains/main/blocks/head/helpers/scripts/simulate_operation",
                     {"operation": {"branch": branch, "contents": [content], "signature": EDSIG_ZERO},
                      "chain_id": TEZOS_CHAIN_ID})
        answer = body_of(reply, "simulate_operation")
        results = answer.get("contents") or []
        need(len(results) == 1 and results[0].get("kind") == "transaction", "simulation answered for other operations")
        outcome = results[0].get("metadata", {}).get("operation_result", {})
        need(outcome.get("status") == "applied", f"simulation {outcome.get('status')}: {json.dumps(outcome.get('errors'))[:160]}")
        need(digits(outcome.get("consumed_milligas")), "simulation: no consumed_milligas")
        return ok(f"protocol {protocols['protocol'][:12]}…, branch head~2; simulated self-transfer used "
                  f"{outcome['consumed_milligas']} milligas")

    def broadcast():
        return refusal(POST(f"{base}/injection/operation?chain=main", "00"))

    def verification():
        identity()
        return ok(f"head level {header('head')['level']}")

    checks = {"balance": balance, "fee": fee, "broadcast": broadcast, "verification": verification}
    return {"capabilities": {name: attempt(check) for name, check in checks.items()}, "sample": sample}


def probe_tzkt(row):
    base = row["endpoint"].rstrip("/")
    try:
        sample = tezos_sample(row)
    except Bad as error:
        return no_sample(row, error)
    address, txhash = sample["address"], sample["tx"]

    def identity():
        head = body_of(GET(f"{base}/v1/head"), "head")
        need(head.get("chainId") == TEZOS_CHAIN_ID, f"indexes {head.get('chainId')!r}, not {TEZOS_CHAIN_ID}")

    def history():
        identity()
        operations = body_of(GET(f"{base}/v1/accounts/{address}/operations?type=transaction&limit=50"), "operations")
        need(isinstance(operations, list) and operations, "no transactions for an address that just sent one")
        for operation in operations:
            need(isinstance(operation.get("hash"), str) and isinstance(operation.get("id"), int)
                 and isinstance(operation.get("amount"), int) and isinstance(operation.get("status"), str),
                 f"an operation without hash, id, amount or status: {json.dumps(operation)[:160]}")
        return ok(f"{len(operations)} transactions on the first page")

    def verification():
        identity()
        operations = body_of(GET(f"{base}/v1/operations/{txhash}"), "operation")
        need(isinstance(operations, list) and operations, f"no operation for {txhash}")
        need(all(operation.get("hash") == txhash for operation in operations), "another operation")
        need(isinstance(operations[0].get("level"), int) and operations[0]["level"] > 0, "no level")
        return ok(f"{txhash} at level {operations[0]['level']}, "
                  f"{'applied' if all(o.get('status') == 'applied' for o in operations) else 'not applied'}")

    return {"capabilities": {"history": attempt(history), "verification": attempt(verification)}, "sample": sample}


# ══ Starknet JSON-RPC (core/src/api/starknet_json_rpc.rs) ══════════════════

QUERY_VERSION = "0x100000000000000000000000000000003"
STARK_GENERATOR_X = "0x1ef15c18599971b7beced415a40f0c7deacfd9b0d1819e03d723d8bc943cfca"


def felt(value):
    return isinstance(value, str) and re.fullmatch(r"0x[0-9a-fA-F]{1,64}", value) is not None


def starknet_call(base, contract, entry_point, calldata):
    answer = rpc_value(base, "starknet_call", [{"contract_address": contract,
                                                 "entry_point_selector": starknet_selector(entry_point),
                                                 "calldata": calldata}, "latest"], f"call {entry_point}")
    need(isinstance(answer, list) and all(felt(x) for x in answer), f"call {entry_point}: {answer!r}")
    return [int(x, 16) for x in answer]


def derive_starknet(base):
    head = rpc_value(base, "starknet_blockNumber")
    seen = set()
    for number in range(head, head - 40, -1):
        block = rpc_value(base, "starknet_getBlockWithTxHashes", [{"block_number": number}])
        for txhash in block.get("transactions", []):
            tx = rpc_value(base, "starknet_getTransactionByHash", [txhash])
            sender = tx.get("sender_address")
            if tx.get("type") != "INVOKE" or not sender or sender in seen:
                continue
            seen.add(sender)
            reply = RPC(base, "starknet_getClassHashAt", ["latest", sender])
            if int(str(common.rpc_result(reply) or "0x0"), 16) == int(STARKNET_ACCOUNT_CLASS, 16):
                return {"address": sender, "tx": txhash, "block": number,
                        "how": "the sender of a recent INVOKE whose account is the class core signs for"}
            if len(seen) >= 80:
                break
    raise Bad(f"no sender of the registry's account class among {len(seen)} recent senders")


def probe_starknet(row):
    base = row["endpoint"].rstrip("/")
    try:
        sample = shared_sample(("starknet", row["chain_id"]), lambda: from_any(
            row, common.endpoints("starknet-json-rpc", row["chain_id"]), derive_starknet))
    except Bad as error:
        return no_sample(row, error)
    address, txhash = sample["address"], sample["tx"]
    tokens = common.tokens(row["chain_id"])

    def identity():
        chain_id = rpc_value(base, "starknet_chainId")
        need(felt(chain_id) and int(chain_id, 16) == int.from_bytes(STARKNET_CHAIN_ID.encode(), "big"),
             f"chain id {chain_id!r}, not {STARKNET_CHAIN_ID}")

    def u256(words, what):
        need(len(words) == 2 and words[1] == 0 and words[0] < 1 << 128, f"{what}: not a u256 under 2^128: {words}")
        return words[0]

    def balance():
        return ok(f"{u256(starknet_call(base, STARKNET_FEE_TOKEN, 'balanceOf', [address]), 'balanceOf')} fri")

    def token_balance():
        answers = []
        for token in tokens:
            held = u256(starknet_call(base, token["contract"], "balanceOf", [address]), f"{token['token_id']} balanceOf")
            decimals = starknet_call(base, token["contract"], "decimals", [])
            need(decimals == [token["decimals"]], f"{token['token_id']}: decimals {decimals}, catalog says {token['decimals']}")
            answers.append(f"{token['token_id']} {held}")
        return ok("; ".join(answers))

    def fee():
        identity()
        class_hash = rpc_value(base, "starknet_getClassHashAt", ["latest", address])
        need(felt(class_hash) and int(class_hash, 16) == int(STARKNET_ACCOUNT_CLASS, 16), f"class {class_hash}")
        owner, guardian = (starknet_call(base, address, "get_owner", []), starknet_call(base, address, "get_guardian", []))
        need(len(owner) == 1 and len(guardian) == 1, "get_owner/get_guardian: not one felt each")
        nonce = rpc_value(base, "starknet_getNonce", ["latest", address])
        need(felt(nonce), f"getNonce: {nonce!r}")
        held = u256(starknet_call(base, STARKNET_FEE_TOKEN, "balanceOf", [address]), "balanceOf")
        zero = {"max_amount": "0x0", "max_price_per_unit": "0x0"}
        invoke = {"type": "INVOKE", "sender_address": address,
                  "calldata": ["0x1", STARKNET_FEE_TOKEN, starknet_selector("transfer"), "0x3", "0x1", hex(min(held, 1)), "0x0"],
                  "signature": [STARK_GENERATOR_X, "0x1"], "nonce": nonce,
                  "resource_bounds": {"l1_gas": zero, "l2_gas": zero, "l1_data_gas": zero},
                  "tip": "0x0", "paymaster_data": [], "nonce_data_availability_mode": "L1",
                  "fee_data_availability_mode": "L1", "account_deployment_data": [], "version": QUERY_VERSION}
        estimates = rpc_value(base, "starknet_estimateFee",
                              {"request": [invoke], "simulation_flags": [], "block_id": "latest"})
        need(isinstance(estimates, list) and len(estimates) == 1, f"estimateFee: {json.dumps(estimates)[:160]}")
        estimate = estimates[0]
        need(estimate.get("unit") == "FRI", f"estimate unit {estimate.get('unit')!r}")
        for field in ("l1_gas_consumed", "l1_gas_price", "l2_gas_consumed", "l2_gas_price",
                      "l1_data_gas_consumed", "l1_data_gas_price"):
            need(felt(estimate.get(field)), f"estimate: {field} {estimate.get(field)!r}")
        return ok(f"nonce {nonce}, guardian {'none' if guardian == [0] else 'set'}; L2 gas {estimate['l2_gas_consumed']} "
                  f"at {estimate['l2_gas_price']}, L1 data gas {estimate['l1_data_gas_consumed']}")

    def broadcast():
        return rpc_refusal(RPC(base, "starknet_addInvokeTransaction", {"invoke_transaction": {}}))

    def verification():
        identity()
        status = rpc_value(base, "starknet_getTransactionStatus", [txhash])
        finality = status.get("finality_status")
        need(finality in ("ACCEPTED_ON_L2", "ACCEPTED_ON_L1", "REJECTED", "RECEIVED", "CANDIDATE", "PRE_CONFIRMED"),
             f"finality {finality!r}")
        if finality.startswith("ACCEPTED"):
            need(status.get("execution_status") in ("SUCCEEDED", "REVERTED"), f"execution {status.get('execution_status')!r}")
        return ok(f"{txhash}: {finality}/{status.get('execution_status')}")

    checks = {"balance": balance, "token-balance": token_balance, "fee": fee, "broadcast": broadcast,
              "verification": verification}
    return {"capabilities": {name: attempt(check) for name, check in checks.items()}, "sample": sample,
            "notes": ["The fee estimate carries core's fixed invalid signature at the query version, which no "
                      "sequencer includes."]}


# ══ Hedera mirror node and address book (core/src/api/hedera_mirror.rs, hedera_hapi.rs) ══

def hedera_id(text):
    return isinstance(text, str) and re.fullmatch(r"0\.0\.\d+", text) is not None


def derive_hedera(base):
    body = body_of(GET(f"{base}/api/v1/transactions?transactiontype=CRYPTOTRANSFER&result=success&limit=50&order=desc"),
                   "recent transfers")
    for tx in body.get("transactions", []):
        payer = tx["transaction_id"].split("-")[0]
        if int(payer.split(".")[2]) > 1000 and not tx.get("scheduled") and tx.get("nonce", 0) == 0:
            return {"address": payer, "tx": tx["transaction_id"],
                    "how": "the payer of a recent successful CRYPTOTRANSFER"}
    raise Bad("no recent transfer paid by a user account")


def hedera_identity(origin):
    block = body_of(GET(f"{origin}/api/v1/blocks/0"), "blocks/0")
    need(str(block.get("hash", "")).lower() == HEDERA_GENESIS, f"block 0 is {block.get('hash')}")


def probe_hedera_mirror(row):
    base = row["endpoint"].rstrip("/")
    try:
        sample = shared_sample(("hedera", row["chain_id"]), lambda: from_any(
            row, common.endpoints("hedera-mirror", row["chain_id"]), derive_hedera))
    except Bad as error:
        return no_sample(row, error)
    account, txid = sample["address"], sample["tx"]

    def next_link(body):
        link = (body.get("links") or {}).get("next")
        need(link is None or (isinstance(link, str) and link.startswith("/api/v1/")), f"next link {link!r}")
        return link

    def balance():
        hedera_identity(base)
        body = body_of(GET(f"{base}/api/v1/accounts/{account}?transactions=false"), "account")
        need(body.get("account") == account, "another account")
        need(isinstance((body.get("balance") or {}).get("balance"), int), "no balance")
        key = body.get("key") or {}
        return ok(f"{body['balance']['balance']} tinybars; key {key.get('_type')}")

    def history():
        hedera_identity(base)
        body = body_of(GET(f"{base}/api/v1/transactions?account.id={account}&transactiontype=CRYPTOTRANSFER"
                           "&limit=25&order=desc"), "transactions")
        txs = need(body.get("transactions") if isinstance(body.get("transactions"), list) else None, "no transactions list")
        need(txs, "no transfers for an account that just paid one")
        for tx in txs:
            need(isinstance(tx.get("transaction_id"), str), "a transaction without id")
            for transfer in tx.get("transfers") or []:
                need(hedera_id(transfer.get("account")) and isinstance(transfer.get("amount"), int),
                     f"a transfer without account or amount: {transfer}")
        return ok(f"{len(txs)} transfers, next link {'present' if next_link(body) else 'absent'}")

    def fee():
        hedera_identity(base)
        rates = body_of(GET(f"{base}/api/v1/network/exchangerate"), "exchange rate")
        for field in ("current_rate", "next_rate"):
            rate = rates.get(field) or {}
            need(all(isinstance(rate.get(k), int) and rate[k] > 0 for k in ("cent_equivalent", "hbar_equivalent")),
                 f"{field}: {rate}")
        blocks = body_of(GET(f"{base}/api/v1/blocks?limit=1&order=desc"), "latest block").get("blocks") or []
        need(blocks and re.fullmatch(r"\d+\.\d{1,9}", str((blocks[0].get("timestamp") or {}).get("to"))),
             "latest block: no timestamp")
        current = rates["current_rate"]
        return ok(f"{current['cent_equivalent']} cents per {current['hbar_equivalent']} hbar; "
                  f"network time {blocks[0]['timestamp']['to']}")

    def verification():
        hedera_identity(base)
        body = body_of(GET(f"{base}/api/v1/transactions/{txid}"), "transaction")
        found = [tx for tx in body.get("transactions") or [] if tx.get("transaction_id") == txid
                 and tx.get("scheduled") is False and (tx.get("nonce") or 0) == 0]
        need(found and isinstance(found[0].get("result"), str), f"no result for {txid}")
        return ok(f"{txid}: {found[0]['result']}")

    checks = {"balance": balance, "history": history, "fee": fee, "verification": verification}
    return {"capabilities": {name: attempt(check) for name, check in checks.items()}, "sample": sample}


def probe_hedera_hapi(row):
    book = row["endpoint"].rstrip("/")

    def broadcast():
        origin = need(book[:-len("/api/v1/network/nodes")] if book.endswith("/api/v1/network/nodes") else None,
                      "not a mirror node's /api/v1/network/nodes")
        hedera_identity(origin)
        url, nodes, skipped, pages = book, [], [], 0
        while url and pages < 8:
            body = body_of(GET(url), "address book")
            pages += 1
            for node in need(body.get("nodes") if isinstance(body.get("nodes"), list) else None, "no nodes list"):
                need(hedera_id(node.get("node_account_id")), f"a node without an account: {json.dumps(node)[:120]}")
                cert = str(node.get("node_cert_hash") or "")
                raw = bytes.fromhex(cert[2:]) if cert.startswith("0x") and re.fullmatch(r"[0-9a-fA-F]*", cert[2:]) else b""
                tls = [(e.get("domain_name") or e.get("ip_address_v4"), e.get("port"))
                       for e in node.get("service_endpoints") or []
                       if e.get("port") not in (50211, 0, None) and (e.get("domain_name") or e.get("ip_address_v4"))]
                if len(raw) == 48 and tls:
                    nodes.append(node["node_account_id"])
                else:
                    skipped.append(f"{node['node_account_id']} (cert {len(raw)} bytes, {len(tls)} TLS endpoints)")
            link = (body.get("links") or {}).get("next")
            url = f"{origin}{link}" if link else None
        need(not url, "more than eight address-book pages")
        need(nodes, f"the book names no node core can reach over pinned TLS; skipped {skipped[:4]}")
        return unchecked(f"address book names {len(nodes)} nodes with a 48-byte certificate hash and a TLS "
                         f"endpoint ({len(skipped)} skipped); gRPC submission to them is core's check "
                         "(`spectra endpoints probe`)")

    return {"capabilities": {"broadcast": attempt(broadcast)}, "sample": {}}


PROBES = {
    "cosmos-rest": probe_cosmos,
    "substrate-json-rpc": probe_substrate,
    "koios": probe_koios,
    "algod": probe_algod,
    "algorand-indexer": probe_algorand_indexer,
    "icp-rosetta": probe_icp_rosetta,
    "icp-replica": probe_icp_replica,
    "thor-rest": probe_thor,
    "lotus-json-rpc": probe_lotus,
    "filfox": probe_filfox,
    "tezos-rpc": probe_tezos,
    "tzkt": probe_tzkt,
    "starknet-json-rpc": probe_starknet,
    "hedera-mirror": probe_hedera_mirror,
    "hedera-hapi": probe_hedera_hapi,
}
