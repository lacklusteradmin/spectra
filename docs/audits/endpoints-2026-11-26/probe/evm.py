"""EVM nodes (`evm-json-rpc`) and their keyless indexers (`blockscout`).

Every request is one core's adapters make (core/src/api/evm_json_rpc.rs,
core/src/api/blockscout.rs, core/src/send/evm.rs), checked against the fields
those adapters decode. Per-network facts (EIP-155 id, legacy pricing, tip
floor, rollup fee oracle) are read from core/src/registry/evm.rs so the probe
cannot drift from the registry.

A sample per network is derived from the network itself: the sender of a
recent ERC-20 transfer of a catalogued token, or, where the catalog lists no
token, the sender of a recent transaction. Every endpoint of that network is
asked about the same address and transaction.

Broadcast is checked with `eth_sendRawTransaction ["0x00"]`: one byte that
decodes to no transaction. Nothing is signed.
"""
import json
import os
import pathlib
import re
import sys
import threading
import time

import common

# ── Registry facts, parsed from core ──────────────────────────────────────

_EVM_RS = (common.ROOT / "core" / "src" / "registry" / "evm.rs").read_text()

# Where the enum and chains.toml spell a chain differently
# (core/src/registry/tests/mod.rs `expected_id`).
_ID_EXCEPTIONS = {
    "MegaEth": "megaeth",
    "ZeroG": "0g",
    "BnbChain": "bnb",
    "OpBnb": "opbnb",
    "ZkSyncEra": "zksync-era",
    "ZkSyncEraSepolia": "zksync-era-sepolia",
    "BnbChainTestnet": "bnb-testnet",
}


def _catalog_id(variant):
    if variant in _ID_EXCEPTIONS:
        return _ID_EXCEPTIONS[variant]
    out = []
    for i, ch in enumerate(variant):
        starts_word = ch.isupper() and i > 0 and (
            variant[i - 1].islower() or variant[i - 1].isdigit()
            or (i + 1 < len(variant) and variant[i + 1].islower()))
        if starts_word:
            out.append("-")
        out.append(ch.lower())
    return "".join(out)


def _fn_body(name):
    start = _EVM_RS.index(f"fn {name}(")
    end = _EVM_RS.index("\n    }\n", start)
    return _EVM_RS[start:end]


CHAIN_IDS = {_catalog_id(v): int(n) for v, n in re.findall(r"Chain::(\w+) => (\d+)", _fn_body("evm_chain_id"))}
_LEGACY = {_catalog_id(v) for v in re.findall(r"Self::(\w+)", _fn_body("evm_legacy_transactions"))}
_TIP_FLOOR = {_catalog_id(v): int(n.replace("_", ""))
              for v, n in re.findall(r"Self::(\w+) => ([\d_]+)", _fn_body("evm_min_priority_fee_wei"))}
_ROLLUP_BODY = _fn_body("evm_rollup_fee_model")
_ROLLUP_CONCRETE = {_catalog_id(v): m for v, m in re.findall(
    r"self == Self::(\w+) \{\s*return Some\(OpStackFeeModel::(\w+)\)", _ROLLUP_BODY)}
_ROLLUP_FAMILY = {}
for _arm, _model in re.findall(r"((?:\|?\s*Self::\w+\s*)+)=>\s*Some\(OpStackFeeModel::(\w+)\)", _ROLLUP_BODY):
    for _variant in re.findall(r"Self::(\w+)", _arm):
        _ROLLUP_FAMILY[_catalog_id(_variant)] = _model
assert CHAIN_IDS and _LEGACY and _TIP_FLOOR and _ROLLUP_FAMILY, "registry parse found nothing"


def _family(chain):
    """`mainnet_counterpart`: a test network's mainnet, or the chain itself."""
    return common.CHAINS.get(chain, {}).get("mainnet", chain)


def is_legacy(chain):
    return _family(chain) in _LEGACY


def tip_floor(chain):
    return _TIP_FLOOR.get(_family(chain), 0)


def rollup_model(chain):
    if chain in _ROLLUP_CONCRETE:
        return _ROLLUP_CONCRETE[chain]
    return _ROLLUP_FAMILY.get(_family(chain))


# ── ABI and wire helpers ──────────────────────────────────────────────────

SEL_BALANCE_OF = "70a08231"
SEL_DECIMALS = "313ce567"
SEL_SYMBOL = "95d89b41"
SEL_SUPPORTS_INTERFACE = "01ffc9a7"
SEL_TRANSFER = "a9059cbb"
SEL_L1_FEE_UPPER_BOUND = "f1c7a58b"   # getL1FeeUpperBound(uint256)
SEL_OPERATOR_FEE = "275aedd2"         # getOperatorFee(uint256)
SEL_L1_FEE = "49948e0e"               # getL1Fee(bytes)
NFT_INTERFACES = ("80ac58cd", "d9b67a26", "ffffffff")  # ERC-721, ERC-1155, ERC-165's invalid id
TRANSFER_TOPIC = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
GAS_ORACLE = "0x420000000000000000000000000000000000000F"
# An empty account above zkSync Era's reserved system range (below 2^16).
RECIPIENT = "0x5bec7a00000000000000000000000000000c0ffe"
UNKNOWN_HASH = "0x" + "de" * 32
NORMAL_TX_TYPES = {None, "0x0", "0x1", "0x2", "0x3", "0x4", "0x00", "0x01", "0x02"}
U128 = (1 << 128) - 1
U64 = (1 << 64) - 1


class Bad(Exception):
    """An answer core's adapter would refuse; the message says why."""


def _hex_int(value, what, limit=U128):
    """A JSON-RPC quantity as `parse_hex_u128`/`parse_hex_u64` read it."""
    if not isinstance(value, str):
        raise Bad(f"{what}: expected a hex string, got {json.dumps(value)[:60]}")
    digits = value[2:] if value.startswith("0x") else value
    if not digits or not re.fullmatch(r"[0-9a-fA-F]+", digits):
        raise Bad(f"{what}: not a hex quantity: {value[:60]!r}")
    number = int(digits, 16)
    if number > limit:
        raise Bad(f"{what}: exceeds {'u64' if limit == U64 else 'u128'}")
    return number


def _abi_u128(value, what):
    """One uint256 ABI word with nothing above u128 (`abi_u128`)."""
    if not isinstance(value, str):
        raise Bad(f"{what}: expected a hex string, got {json.dumps(value)[:60]}")
    word = value[2:] if value.startswith("0x") else value
    if len(word) != 64 or not re.fullmatch(r"[0-9a-fA-F]+", word):
        raise Bad(f"{what}: expected one 32-byte word, got {value[:80]!r}")
    if int(word[:32], 16):
        raise Bad(f"{what}: exceeds u128")
    return int(word[32:], 16)


def _abi_string(value):
    """`decode_abi_string_or_bytes32`, or None."""
    if not isinstance(value, str):
        return None
    try:
        raw = bytes.fromhex(value[2:] if value.startswith("0x") else value)
    except ValueError:
        return None
    if not raw:
        return None
    if len(raw) >= 64 and int.from_bytes(raw[24:32], "big") == 32:
        length = int.from_bytes(raw[56:64], "big")
        if len(raw) >= 64 + length:
            try:
                text = raw[64:64 + length].decode().rstrip("\0")
                if text:
                    return text
            except UnicodeDecodeError:
                pass
    head = raw[:32].split(b"\0", 1)[0]
    try:
        return head.decode() or None
    except UnicodeDecodeError:
        return None


def _word(address):
    return address.lower().removeprefix("0x").rjust(64, "0")


def _topic_address(topic):
    return "0x" + topic[-40:].lower() if isinstance(topic, str) and len(topic) == 66 else None


def _rpc(url, method, params=None):
    """`json_rpc::call`'s decode: `(result, None)`, or `(None, problem)`."""
    reply = common.rpc(url, method, params)
    if reply.error or reply.status != 200:
        return None, _http_problem(reply)
    body = reply.json
    if not isinstance(body, dict):
        return None, f"not a JSON-RPC object: {reply.brief(160)}"
    if body.get("error") is not None:
        return None, f"rpc error: {json.dumps(body['error'])[:220]}"
    if "result" not in body:
        return None, f"missing result: {reply.brief(160)}"
    return body["result"], None


def _batch(url, calls):
    """`call_batch_each`: one JSON-RPC batch whose answers are matched by id.
    Returns `([(result, problem), …], None)` or `(None, problem)` when the
    batch as a whole is refused."""
    body = [{"jsonrpc": "2.0", "id": i + 1, "method": m, "params": p} for i, (m, p) in enumerate(calls)]
    reply = common.post(url, body)
    if reply.error or reply.status != 200:
        return None, _http_problem(reply)
    answers = reply.json
    if not isinstance(answers, list):
        return None, f"batch: expected an array, got {reply.brief(200)}"
    if len(answers) != len(calls):
        return None, f"batch: expected {len(calls)} responses, got {len(answers)}: {reply.brief(160)}"
    ordered = sorted(answers, key=lambda a: a.get("id") if isinstance(a, dict) and isinstance(a.get("id"), int) else 0)
    out = []
    for answer in ordered:
        if not isinstance(answer, dict):
            out.append((None, "not an object"))
        elif answer.get("error") is not None:
            out.append((None, f"rpc error: {json.dumps(answer['error'])[:160]}"))
        elif "result" not in answer:
            out.append((None, "missing result"))
        else:
            out.append((answer["result"], None))
    return out, None


def _http_problem(reply):
    if reply.error:
        return reply.brief(200)
    blocked = _cloudflare(reply)
    return f"{blocked}: {reply.brief(120)}" if blocked else reply.brief(200)


def _cloudflare(reply):
    text = reply.text[:4000].lower()
    if any(mark in text for mark in ("just a moment", "cf-chl", "challenge-platform", "attention required")):
        return f"Cloudflare challenge (HTTP {reply.status})"
    return None


def _eth_call(to, data, block="latest"):
    return ("eth_call", [{"to": to, "data": "0x" + data}, block])


# ── The host limit, for requests made outside a row's own slot ────────────

def _foreign_slot(url):
    """The driver's per-host semaphore for `url`, so deriving a sample from
    another endpoint keeps to two requests per host."""
    main = sys.modules.get("__main__")
    slot = getattr(main, "_slot", None)
    return slot(url) if slot else None


class _Held:
    def __init__(self, url, own):
        self.slot = None if url == own else _foreign_slot(url)
        self.acquired = False

    def __enter__(self):
        if self.slot is not None:
            self.acquired = self.slot.acquire(timeout=90)
        return self

    def __exit__(self, *_):
        if self.acquired:
            self.slot.release()


# ── Samples ───────────────────────────────────────────────────────────────

# `EVM_PROBE_SAMPLES=<file>` keeps derived samples across runs, so a run of
# one API asks about the same address as an earlier run of the other.
_SAMPLE_FILE = os.environ.get("EVM_PROBE_SAMPLES")
_samples = {}
if _SAMPLE_FILE and pathlib.Path(_SAMPLE_FILE).exists():
    _samples = {chain: sample for chain, sample in json.loads(pathlib.Path(_SAMPLE_FILE).read_text()).items()
                if sample.get("address")}
_sample_locks = {}
_locks_guard = threading.Lock()


def sample_for(chain, own_url=None):
    """One active address, a mined transaction of it and a catalogued token,
    shared by every endpoint of `chain`."""
    with _locks_guard:
        lock = _sample_locks.setdefault(chain, threading.Lock())
    with lock:
        if chain not in _samples:
            sample = _derive(chain, own_url)
            with _locks_guard:
                _samples[chain] = sample
                if _SAMPLE_FILE:
                    pathlib.Path(_SAMPLE_FILE).write_text(json.dumps(_samples, indent=1, sort_keys=True))
        return _samples[chain]


def _derivation_urls(chain):
    urls = [row["endpoint"] for row in common.endpoints("evm-json-rpc", chain)]
    # A shared, rate-limited operator last.
    return sorted(urls, key=lambda url: "1rpc.io" in url)


def _derive(chain, own_url):
    expected = CHAIN_IDS.get(chain)
    problems = []
    usable = []
    for url in _derivation_urls(chain):
        with _Held(url, own_url):
            reported, problem = _rpc(url, "eth_chainId")
        if problem:
            problems.append(f"{url}: eth_chainId {problem}")
            continue
        try:
            if _hex_int(reported, "eth_chainId", U64) != expected:
                problems.append(f"{url}: eth_chainId {reported}, registry {expected}")
                continue
        except Bad as error:
            problems.append(f"{url}: {error}")
            continue
        usable.append(url)
    contracts = {row["contract"].lower(): row for row in common.tokens(chain)}
    finders = ((_from_direct_transfers, _from_transfer_logs, _from_any_transfer_logs, _from_recent_blocks)
               if contracts else (_from_any_transfer_logs, _from_recent_blocks))
    fallback = None
    by_url = {url: _Finder(url) for url in usable}
    for find in finders:
        for url in usable:
            with _Held(url, own_url):
                try:
                    found = find(by_url[url], contracts)
                except Bad as error:
                    problems.append(f"{url}: {find.__name__}: {error}")
                    continue
            if found:
                found["derived_from"] = url
                if found["token_transfer"]:
                    return found
                fallback = fallback or found
            break
    if fallback:
        if not fallback.get("token"):
            seen = [c for finder in by_url.values() for c, ok in finder.checked.items() if ok]
            if seen:
                fallback["token"] = {"id": "(not catalogued)", "contract": seen[0], "decimals": None}
        return fallback
    problems = problems or [f"no funded sender found on {', '.join(usable) or 'no usable endpoint'}"]
    return {"problem": "no sample: " + "; ".join(problems)}


class _Finder:
    """One endpoint a sample is derived from, and the funding a sample needs:
    enough for a plain transfer at several times today's gas price, so
    `eth_estimateGas` at the fee core would set is not refused for funds."""

    def __init__(self, url):
        self.url = url
        self._floor = None
        self.checked = {}

    def head(self):
        head, problem = _rpc(self.url, "eth_blockNumber")
        if problem:
            raise Bad(f"eth_blockNumber {problem}")
        return _hex_int(head, "eth_blockNumber", U64)

    def floor(self):
        if self._floor is None:
            price, problem = _rpc(self.url, "eth_gasPrice")
            try:
                price = 0 if problem else _hex_int(price, "eth_gasPrice")
            except Bad:
                price = 0
            self._floor = 200_000 * max(price, 1)
        return self._floor

    def funded(self, address):
        """The latest nonce of a sender with history and funds, or None."""
        if not address or address == "0x" + "0" * 40:
            return None
        nonce, p1 = _rpc(self.url, "eth_getTransactionCount", [address, "latest"])
        balance, p2 = _rpc(self.url, "eth_getBalance", [address, "latest"])
        try:
            if p1 or p2:
                return None
            nonce, balance = _hex_int(nonce, "nonce", U64), _hex_int(balance, "balance")
        except Bad:
            return None
        return nonce if nonce >= 1 and balance >= self.floor() else None

    def erc20(self, contract):
        """True when `contract` answers `decimals()` as an ERC-20 would."""
        method, params = _eth_call(contract, SEL_DECIMALS)
        result, problem = _rpc(self.url, method, params)
        try:
            return not problem and _abi_u128(result, "decimals") <= 38
        except Bad:
            return False

    def logs(self, addresses, spans):
        """The newest Transfer logs, widening the block range until ten
        appear or the node refuses a wider one: `[]` when none, Bad when it
        refuses the first query."""
        head = self.head()
        best = []
        for span in spans:
            query = {"fromBlock": hex(max(0, head - span)), "toBlock": hex(head), "topics": [TRANSFER_TOPIC]}
            if addresses:
                query["address"] = addresses
            found, problem = _rpc(self.url, "eth_getLogs", [query])
            if problem:
                if span == spans[0]:
                    raise Bad(f"eth_getLogs {problem}")
                break
            found = [log for log in found or [] if isinstance(log, dict) and len(log.get("topics") or []) >= 3]
            if len(found) >= len(best):
                best = found
            if len(best) >= 10:
                break
        return sorted(best, key=lambda log: (int(log.get("blockNumber", "0x0"), 16),
                                             int(log.get("logIndex", "0x0"), 16)), reverse=True)


def _token(row):
    return {"id": row["token_id"], "contract": row["contract"].lower(), "decimals": row.get("decimals")}


def _from_direct_transfers(finder, contracts):
    """The sender of a `transfer` call to a catalogued token in one of the
    last three blocks: an address with native history and a token transfer."""
    head = finder.head()
    for number in range(head, max(-1, head - 3), -1):
        block, problem = _rpc(finder.url, "eth_getBlockByNumber", [hex(number), True])
        if problem:
            raise Bad(f"eth_getBlockByNumber {problem}")
        tried = 0
        for tx in (block or {}).get("transactions") or []:
            if not isinstance(tx, dict) or tx.get("type") not in NORMAL_TX_TYPES:
                continue
            to, data = (tx.get("to") or "").lower(), tx.get("input") or ""
            if to not in contracts or not data.startswith("0x" + SEL_TRANSFER):
                continue
            tried += 1
            if tried > 4:
                break
            sender = (tx.get("from") or "").lower()
            nonce = finder.funded(sender)
            if nonce is not None and nonce >= 2:
                return {
                    "address": sender, "tx": tx["hash"], "block": number, "nonce": nonce,
                    "how": "sender of a transfer call to a catalogued token",
                    "token": _token(contracts[to]), "token_transfer": True,
                }
    return None


def _from_logs(finder, logs, token_of):
    """The sender of a transaction behind one of `logs` that moved an ERC-20
    to or from itself; else the first funded sender. `token_of` names the
    token read for a log, or None for a log that is no ERC-20 transfer
    (an ERC-721 one, whose sender still has native history)."""
    fallback = None
    seen = set()
    for log in logs:
        tx_hash = log.get("transactionHash")
        if tx_hash in seen:
            continue
        seen.add(tx_hash)
        if len(seen) > 25:
            break
        token = token_of(log) if len(log["topics"]) == 3 else None
        tx, problem = _rpc(finder.url, "eth_getTransactionByHash", [tx_hash])
        if problem or not isinstance(tx, dict) or tx.get("type") not in NORMAL_TX_TYPES:
            continue
        sender = (tx.get("from") or "").lower()
        moved = token is not None and sender in {_topic_address(log["topics"][1]), _topic_address(log["topics"][2])}
        nonce = finder.funded(sender)
        if nonce is None:
            continue
        candidate = {
            "address": sender, "tx": tx_hash, "block": int(log["blockNumber"], 16), "nonce": nonce,
            "token": token, "token_transfer": moved,
        }
        if moved and nonce >= 2:
            return candidate
        if fallback is None or (moved and not fallback["token_transfer"]):
            fallback = candidate
    return fallback


def _from_transfer_logs(finder, contracts):
    """A sender behind a recent Transfer log of a catalogued token."""
    logs = finder.logs(list(contracts)[:40], (5, 50, 500, 1000, 5000))
    found = _from_logs(finder, logs, lambda log: _token(contracts[log["address"].lower()]))
    if found:
        found["how"] = "sender of a recent catalogued-token transfer"
    return found


def _from_any_transfer_logs(finder, contracts):
    """A sender behind a recent ERC-20 Transfer log of any contract. Where
    the catalog lists tokens, the token read stays a catalogued one."""
    logs = finder.logs(None, (2, 20, 200, 1000, 5000, 20000))
    checked = finder.checked

    def token_of(log):
        contract = log["address"].lower()
        if contracts:
            return _token(next(iter(contracts.values())))
        if len(log.get("data") or "") != 66:
            return None
        if contract not in checked:
            checked[contract] = finder.erc20(contract)
        return {"id": "(not catalogued)", "contract": contract, "decimals": None} if checked[contract] else None

    found = _from_logs(finder, logs, token_of)
    if found:
        if contracts:
            found["token"] = _token(next(iter(contracts.values())))
            found["token_transfer"] = False
        found["how"] = ("sender behind a recent Transfer log" if not found["token_transfer"]
                        else "sender of a recent transfer of an uncatalogued ERC-20")
    return found


def _from_recent_blocks(finder, contracts):
    """The sender of an ordinary transaction in the last 300 blocks; where the
    catalog lists no token, preferably one calling `transfer` on an ERC-20 in
    the last 40 (a node that refuses an address-less `eth_getLogs` still
    shows those)."""
    head = finder.head()
    fallback = None
    for number in range(head, max(-1, head - 300), -1):
        if fallback and (contracts or number < head - 40):
            break
        block, problem = _rpc(finder.url, "eth_getBlockByNumber", [hex(number), True])
        if problem:
            raise Bad(f"eth_getBlockByNumber {problem}")
        for tx in ((block or {}).get("transactions") or [])[:30]:
            if not isinstance(tx, dict) or tx.get("type") not in NORMAL_TX_TYPES:
                continue
            to, data = (tx.get("to") or "").lower(), tx.get("input") or ""
            transfer = not contracts and to and data.startswith("0x" + SEL_TRANSFER)
            if not transfer and fallback:
                continue
            if transfer and to not in finder.checked:
                finder.checked[to] = finder.erc20(to)
            transfer = transfer and finder.checked[to]
            sender = (tx.get("from") or "").lower()
            nonce = finder.funded(sender)
            if nonce is None:
                continue
            token = next(iter(contracts.values()), None)
            found = {
                "address": sender, "tx": tx["hash"], "block": number, "nonce": nonce,
                "how": "sender of a recent transaction",
                "token": token and _token(token), "token_transfer": False,
            }
            if transfer:
                found.update(how="sender of a transfer call to an uncatalogued ERC-20", token_transfer=True,
                             token={"id": "(not catalogued)", "contract": to, "decimals": None})
                return found
            fallback = fallback or found
    return fallback


# ── evm-json-rpc ──────────────────────────────────────────────────────────

def probe_rpc(row):
    url, chain = row["endpoint"], row["chain_id"]
    notes = []
    expected = CHAIN_IDS.get(chain)
    reported, problem = _rpc(url, "eth_chainId")
    identity = {"eth_chainId": reported, "registry_chain_id": expected}
    try:
        identity["eth_chainId_decimal"] = _hex_int(reported, "eth_chainId", U64) if not problem else None
    except Bad as error:
        problem = str(error)
    identity["matches"] = identity.get("eth_chainId_decimal") == expected if not problem else None
    if problem:
        identity["problem"] = problem
    sample = sample_for(chain, url)
    record = {"sample": {**identity, **{k: v for k, v in sample.items() if k != "nonce"}}}
    if identity["matches"] is False:
        notes.append(f"CHAIN ID MISMATCH: eth_chainId {reported} ({identity['eth_chainId_decimal']}), registry {expected}")
        wrong = common.fail(f"wrong network: eth_chainId {identity['eth_chainId_decimal']}, registry {expected}")
        record["capabilities"] = {c: wrong for c in row["capabilities"]}
        record["notes"] = notes
        return record

    caps = {}
    address = sample.get("address")
    if not address:
        own = f"eth_chainId {identity['problem']}; " if identity.get("problem") else ""
        no_sample = common.fail(f"{own}{sample.get('problem', 'no sample')}")
    caps["balance"] = _probe_balance(url, address) if address else no_sample
    caps["fee"] = _probe_fee(url, chain, address, notes) if address else no_sample
    # Read on every node of a network whose catalog lists tokens, so a node
    # that serves it without claiming it shows; elsewhere only where claimed.
    if common.tokens(chain) or "token-balance" in row["capabilities"]:
        if not address:
            caps["token-balance"] = no_sample
        elif sample.get("token"):
            caps["token-balance"] = _probe_token_balance(url, chain, sample, notes)
        else:
            caps["token-balance"] = common.unchecked("no ERC-20 found to read: the catalog lists none on this "
                                                     "network and no recent log or transfer call named one")
    caps["verification"] = _probe_verification(url, sample) if address else no_sample
    caps["broadcast"] = _probe_broadcast(url)
    # Last, so a node's per-second limit tripped by it costs no other check.
    if caps.get("token-balance", {}).get("ok"):
        time.sleep(1)
        wide = _wide_batch(url, chain, address)
        if wide:
            caps["token-balance"] = common.ok(f"{caps['token-balance']['detail']}; PARTIAL: {wide}")
            notes.append(f"token-balance: {wide}")
    record["capabilities"] = caps
    if notes:
        record["notes"] = notes
    return record


def _probe_balance(url, address):
    result, problem = _rpc(url, "eth_getBalance", [address, "latest"])
    if problem:
        return common.fail(f"eth_getBalance {problem}")
    try:
        return common.ok(f"eth_getBalance {_hex_int(result, 'eth_getBalance')} wei")
    except Bad as error:
        return common.fail(str(error))


def _probe_fee(url, chain, address, notes):
    """`fetch_fee_estimate`, then what `prepare_transfer` reads to build a
    plain transfer: the pending nonce, `eth_estimateGas` of the transaction
    it will sign and, on an OP Stack network, the gas price oracle."""
    steps = []
    try:
        if is_legacy(chain):
            price, problem = _rpc(url, "eth_gasPrice")
            if problem:
                raise Bad(f"eth_gasPrice {problem}")
            price = _hex_int(price, "eth_gasPrice")
            if price == 0:
                raise Bad("eth_gasPrice: zero gas price (core refuses it)")
            max_fee = tip = price
            steps.append(f"eth_gasPrice {price}")
        else:
            history, problem = _rpc(url, "eth_feeHistory", ["0x4", "latest", [25, 75]])
            if problem:
                raise Bad(f"eth_feeHistory {problem}")
            if not isinstance(history, dict):
                raise Bad(f"eth_feeHistory: not an object: {json.dumps(history)[:120]}")
            base_fees = history.get("baseFeePerGas")
            if not isinstance(base_fees, list) or not base_fees:
                raise Bad("eth_feeHistory: missing or empty baseFeePerGas")
            base = _hex_int(base_fees[-1], "baseFeePerGas")
            rewards = history.get("reward")
            if not isinstance(rewards, list) or not rewards or not isinstance(rewards[-1], list):
                raise Bad(f"eth_feeHistory: missing reward ({json.dumps(rewards)[:80]})")
            tip = _hex_int(rewards[-1][0], "reward sample") if rewards[-1] else 0
            tip = max(tip, tip_floor(chain))
            max_fee = 2 * base + tip
            if max_fee == 0:
                raise Bad("eth_feeHistory prices a transfer at zero (core refuses: invalid EIP-1559 fees)")
            steps.append(f"eth_feeHistory base {base}, tip {tip}, max {max_fee}")
        nonce, problem = _rpc(url, "eth_getTransactionCount", [address, "pending"])
        if problem:
            raise Bad(f"eth_getTransactionCount pending {problem}")
        nonce = _hex_int(nonce, "eth_getTransactionCount", U64)
        steps.append(f"pending nonce {nonce}")
        request = {"from": address, "to": RECIPIENT, "value": "0x0", "data": "0x", "nonce": hex(nonce)}
        if is_legacy(chain):
            request["gasPrice"] = hex(max_fee)
        else:
            request.update({"maxFeePerGas": hex(max_fee), "maxPriorityFeePerGas": hex(tip), "accessList": []})
        gas, problem = _rpc(url, "eth_estimateGas", [request])
        if problem:
            raise Bad(f"eth_estimateGas {problem}")
        gas = _hex_int(gas, "eth_estimateGas", U64)
        if gas == 0:
            raise Bad("eth_estimateGas: zero")
        steps.append(f"eth_estimateGas {gas}")
        model = rollup_model(chain)
        if model:
            steps.append(_rollup_fee(url, model, gas))
    except Bad as error:
        done = f" (after {'; '.join(steps)})" if steps else ""
        return common.fail(f"{error}{done}")
    return common.ok("; ".join(steps))


def _rollup_fee(url, model, gas):
    """`fetch_rollup_fee` for a plain transfer of about 60 unsigned bytes."""
    size = 60
    if model == "Bedrock":
        payload = "02" + "11" * (size - 1)
        data = SEL_L1_FEE + f"{32:064x}{size:064x}" + payload.ljust(((size + 31) // 32) * 64, "0")
        calls = [("getL1Fee(bytes)", data)]
    else:
        calls = [("getL1FeeUpperBound", SEL_L1_FEE_UPPER_BOUND + f"{size:064x}")]
        if model == "FjordWithOperator":
            calls.append(("getOperatorFee", SEL_OPERATOR_FEE + f"{gas:064x}"))
    answers = []
    for name, data in calls:
        method, params = _eth_call(GAS_ORACLE, data)
        result, problem = _rpc(url, method, params)
        if problem:
            raise Bad(f"gas oracle {name} {problem}")
        answers.append(f"{name} {_abi_u128(result, 'gas oracle ' + name)}")
    return f"{model} oracle: " + ", ".join(answers)


def _probe_token_balance(url, chain, sample, notes):
    """`fetch_erc20_balance_words` (balanceOf and decimals in one batch),
    `fetch_erc20_metadata` (decimals, symbol and three ERC-165 checks in one
    batch) and the lone `fetch_erc20_balance_of` call."""
    token = sample.get("token") or {}
    contract, address = token.get("contract"), sample["address"]
    if not contract:
        return common.fail("no catalogued token to read")
    balance_of = SEL_BALANCE_OF + _word(address)
    try:
        answers, problem = _batch(url, [_eth_call(contract, balance_of), _eth_call(contract, SEL_DECIMALS)])
        if problem:
            raise Bad(f"balance batch: {problem}")
        for result, problem in answers:
            if problem:
                raise Bad(f"balance batch: {problem}")
        raw = _abi_u128(answers[0][0], "balanceOf")
        decimals = _abi_u128(answers[1][0], "decimals")
        if decimals > 38:
            raise Bad(f"decimals {decimals} exceeds 38")
        calls = [_eth_call(contract, SEL_DECIMALS), _eth_call(contract, SEL_SYMBOL)] + [
            _eth_call(contract, SEL_SUPPORTS_INTERFACE + iid + "0" * 56) for iid in NFT_INTERFACES]
        answers, problem = _batch(url, calls)
        if problem:
            raise Bad(f"metadata batch: {problem}")
        if answers[0][1] or answers[1][1]:
            raise Bad(f"metadata batch: {answers[0][1] or answers[1][1]}")
        symbol = _abi_string(answers[1][0]) or ""
        claims = [isinstance(r, str) and r[2:].lower() == "0" * 63 + "1" for r, _ in answers[2:]]
        if not claims[2] and (claims[0] or claims[1]):
            raise Bad("the contract reports an NFT standard (core refuses it as not fungible)")
        method, params = _eth_call(contract, balance_of)
        single, problem = _rpc(url, method, params)
        if problem:
            raise Bad(f"eth_call balanceOf {problem}")
        if _abi_u128(single, "balanceOf") != raw:
            notes.append(f"balanceOf differed between the batch ({raw}) and a single call moments later "
                         f"({_abi_u128(single, 'balanceOf')})")
    except Bad as error:
        return common.fail(f"{token.get('id')} {contract}: {error}")
    detail = f"{symbol or token.get('id')} {contract}: balanceOf {raw}, decimals {decimals}"
    if token.get("decimals") is not None and token["decimals"] != decimals:
        notes.append(f"catalog decimals {token['decimals']} for {token['id']}, contract says {decimals}")
    return common.ok(detail)


def _wide_batch(url, chain, address):
    """The batch core sends for a wallet tracking many tokens: five
    contracts, balanceOf and decimals each (`MAX_BATCH_CALLS` is 10). A
    problem, or None."""
    contracts = [row["contract"].lower() for row in common.tokens(chain)][:5]
    if len(contracts) < 2:
        return None
    calls = []
    for contract in contracts:
        calls += [_eth_call(contract, SEL_BALANCE_OF + _word(address)), _eth_call(contract, SEL_DECIMALS)]
    answers, problem = _batch(url, calls)
    if problem:
        return f"a batch of {len(calls)} calls ({len(contracts)} tokens) is refused: {problem}"
    failed = [f"{contracts[i // 2]}: {p}" for i, (_, p) in enumerate(answers) if p]
    if failed:
        return f"{len(failed)} of {len(calls)} calls in a {len(contracts)}-token batch failed, e.g. {failed[0][:160]}"
    return None


def _probe_verification(url, sample):
    """`fetch_receipt` for a mined transaction and for an unknown hash, and
    `fetch_tx_nonce`."""
    tx = sample["tx"]
    try:
        receipt, problem = _rpc(url, "eth_getTransactionReceipt", [tx])
        if problem:
            raise Bad(f"eth_getTransactionReceipt {problem}")
        if receipt is None:
            time.sleep(4)
            receipt, problem = _rpc(url, "eth_getTransactionReceipt", [tx])
            if problem:
                raise Bad(f"eth_getTransactionReceipt {problem}")
        if receipt is None:
            head, _ = _rpc(url, "eth_blockNumber")
            raise Bad(f"no receipt for {tx}, mined at block {sample.get('block')}; node head {head}")
        if not isinstance(receipt, dict):
            raise Bad(f"receipt is not an object: {json.dumps(receipt)[:100]}")
        block = receipt.get("blockNumber")
        block = _hex_int(block, "receipt blockNumber", U64) if isinstance(block, str) and block not in ("", "0x") else None
        if block is None:
            raise Bad("receipt for a mined transaction has no blockNumber")
        status = receipt.get("status")
        if status not in ("0x1", "0x0", "0x01", "0x00"):
            raise Bad(f"receipt status {status!r}")
        gas_used = receipt.get("gasUsed")
        if gas_used is not None:
            _hex_int(gas_used, "gasUsed")
        price = receipt.get("effectiveGasPrice")
        if price is not None:
            _hex_int(price, "effectiveGasPrice")
        extras = []
        if price is None:
            extras.append("no effectiveGasPrice")
        if "l1Fee" in receipt:
            extras.append(f"l1Fee {receipt['l1Fee']}")
        unknown, problem = _rpc(url, "eth_getTransactionReceipt", [UNKNOWN_HASH])
        if problem:
            raise Bad(f"eth_getTransactionReceipt of an unknown hash {problem}")
        if unknown is not None:
            raise Bad(f"an unknown hash answered {json.dumps(unknown)[:100]}, not null")
        by_hash, problem = _rpc(url, "eth_getTransactionByHash", [tx])
        if problem:
            raise Bad(f"eth_getTransactionByHash {problem}")
        if not isinstance(by_hash, dict):
            raise Bad(f"eth_getTransactionByHash answered {json.dumps(by_hash)[:80]}")
        _hex_int(by_hash.get("nonce"), "transaction nonce", U64)
    except Bad as error:
        return common.fail(str(error))
    tail = f" ({', '.join(extras)})" if extras else ""
    return common.ok(f"receipt at block {block}, status {status}{tail}; unknown hash null; nonce read")


_UNSERVED = re.compile(
    r"method[^\"]{0,40}(not found|not exist|does not exist|not available|not supported|unsupported|"
    r"not allowed|disabled|is not whitelisted|forbidden)|unsupported method|not whitelisted|whitelist|"
    r"api[ -]?key|unauthori[sz]ed|forbidden|authenticat|access denied|permission|rate.?limit|too many requests|"
    r"exceeded|quota|capacity|usage limit|limit reached|upgrade|plan\b", re.I)
_UNDECODABLE = re.compile(
    r"rlp|decod|typed ?transaction|transaction type|tx type|too short|invalid sender|invalid transaction|"
    r"reading transaction object failed|invalid tx|"
    r"malformed|unmarshal|deserial|invalid raw|eof|invalid signature|empty typed|invalid (tx|rlp|input|params?)|"
    r"parse|cannot unmarshal|bytes|insufficient|invalid argument", re.I)


def _probe_broadcast(url):
    """`eth_sendRawTransaction ["0x00"]`: a node that serves the method
    refuses the payload as no transaction."""
    reply = common.rpc(url, "eth_sendRawTransaction", ["0x00"])
    if reply.error:
        return common.fail(reply.brief(200))
    if reply.status in (401, 403, 404, 405) or _cloudflare(reply):
        return common.fail(_http_problem(reply))
    body = reply.json
    error = body.get("error") if isinstance(body, dict) else None
    if not error:
        if isinstance(body, dict) and "result" in body:
            return common.fail(f"accepted an undecodable payload: {reply.brief(160)}")
        return common.fail(_http_problem(reply))
    message = json.dumps(error)[:240]
    code = error.get("code") if isinstance(error, dict) else None
    if code == -32601 or _UNSERVED.search(message):
        return common.fail(f"HTTP {reply.status} refused the method: {message}")
    if _UNDECODABLE.search(message):
        return common.ok(f"refused as no transaction (HTTP {reply.status}): {message}")
    return common.fail(f"unrecognised refusal (HTTP {reply.status}): {message}")


# ── blockscout ────────────────────────────────────────────────────────────

def explorer_query_url(base, params):
    """`explorer_query_url` for an open source."""
    base = base.rstrip("/")
    suffix = "" if base.endswith("/etherscan") or base.endswith("/api") else "/api"
    return f"{base}{suffix}?{params}"


def is_routescan(base):
    return base.rstrip("/").endswith("/etherscan")


_UINT = re.compile(r"\+?[0-9]+")


def _uint(value, what, limit):
    if not isinstance(value, str) or not _UINT.fullmatch(value) or int(value) > limit:
        raise Bad(f"{what}: {json.dumps(value)[:40]}")
    return int(value)


def _explorer_rows(url):
    """The `{status, message, result}` envelope and `etherscan_result_rows`."""
    reply = common.get(url)
    if reply.error or reply.status != 200:
        raise Bad(_http_problem(reply))
    body = reply.json
    if not isinstance(body, dict):
        raise Bad(f"not JSON: {_cloudflare(reply) or reply.brief(160)}")
    status, message = body.get("status"), body.get("message")
    if not isinstance(status, str):
        raise Bad(f"status is {json.dumps(status)}, not a string (core cannot decode it)")
    result = body.get("result")
    if isinstance(result, list):
        return result, message
    if status == "1":
        raise Bad(f"non-list result: {message}")
    raise Bad(f"explorer refused: {message!r}, result {json.dumps(result)[:120]}")


def _decode_txlist(rows):
    """`fetch_history`'s decode of each row; any bad row fails the page."""
    out = []
    for row in rows:
        if not isinstance(row, dict):
            raise Bad("row is not an object")
        for field in ("hash", "blockNumber", "timeStamp", "from", "to", "value", "gasPrice", "gasUsed"):
            if not isinstance(row.get(field), str):
                raise Bad(f"history row {row.get('hash')}: {field} is {json.dumps(row.get(field))[:40]}")
        for field in ("isError", "txreceipt_status"):
            if row.get(field) is not None and not isinstance(row[field], str):
                raise Bad(f"history row {row['hash']}: {field} is {json.dumps(row[field])}")
        is_error, receipt = row.get("isError"), row.get("txreceipt_status")
        if not (is_error in ("0", "1") or receipt in ("0", "1")):
            raise Bad(f"history row {row['hash']}: missing execution status")
        _uint(row["gasPrice"], "history gas price", U128)
        _uint(row["gasUsed"], "history gas used", U128)
        _uint(row["blockNumber"], "history block number", U64)
        if _uint(row["timeStamp"], "history time", U64) == 0:
            raise Bad(f"history row {row['hash']}: no time")
        out.append(row)
    return out


def _decode_tokentx(rows):
    """`fetch_token_transfers`'s decode; NFT rows are skipped, any bad
    fungible row fails the page."""
    out = []
    for row in rows:
        if not isinstance(row, dict):
            raise Bad("row is not an object")
        for field in ("blockNumber", "timeStamp", "hash", "from", "to", "contractAddress", "value"):
            if not isinstance(row.get(field), str):
                raise Bad(f"token row {row.get('hash')}: {field} is {json.dumps(row.get(field))[:40]}")
        if row.get("tokenID") is not None and not isinstance(row["tokenID"], str):
            raise Bad(f"token row {row['hash']}: tokenID is {json.dumps(row['tokenID'])}")
        if row.get("tokenID"):
            continue
        if not row["value"] or not row["value"].isdigit() or not row["value"].isascii():
            raise Bad(f"token transfer {row['hash']}: malformed value {row['value']!r}")
        if _uint(row["timeStamp"], "token time", U64) == 0:
            raise Bad(f"token row {row['hash']}: no time")
        _uint(row["blockNumber"], "token block number", U64)
        out.append(row)
    return out


def _decode_tokenlist(rows):
    """`parse_token_list`: fungible rows whose contract and balance read."""
    held = []
    for row in rows:
        if not isinstance(row, dict):
            continue
        field = lambda *names: next((row[n] for n in names if isinstance(row.get(n), str)), None)  # noqa: E731
        kind = field("type")
        if kind is not None and kind != "ERC-20":
            continue
        contract, balance = field("contractAddress", "TokenAddress"), field("balance", "TokenQuantity")
        if contract is None or balance is None or not _UINT.fullmatch(balance) or int(balance) > U128:
            continue
        held.append({"contract": contract.lower(), "balance": int(balance)})
    return held


def probe_indexer(row):
    base, chain = row["endpoint"], row["chain_id"]
    sample = sample_for(chain)
    record = {"sample": {k: v for k, v in sample.items() if k != "nonce"}}
    address = sample.get("address")
    if not address:
        failed = common.fail(sample.get("problem", "no sample"))
        record["capabilities"] = {c: failed for c in row["capabilities"]}
        return record
    caps, notes = {}, []

    url = explorer_query_url(base, f"module=account&action=txlist&address={address}&sort=desc&page=1&offset=50")
    try:
        rows = _decode_txlist(_explorer_rows(url)[0])
        if not rows:
            raise Bad(f"no transactions for {address}, which sent {sample['tx']} at block {sample['block']}")
        listed = any(r["hash"].lower() == sample["tx"].lower() for r in rows)
        newest = max(int(r["blockNumber"]) for r in rows)
        caps["history"] = common.ok(
            f"{len(rows)} rows, newest block {newest}; sample tx {'listed' if listed else 'not on this page'}")
    except Bad as error:
        caps["history"] = common.fail(str(error))

    url = explorer_query_url(base, f"module=account&action=tokentx&address={address}&page=1&offset=50&sort=desc")
    try:
        raw = _explorer_rows(url)[0]
        rows = _decode_tokentx(raw)
        if not rows and sample.get("token_transfer"):
            raise Bad(f"no token transfers for {address}, whose transfer {sample['tx']} "
                      f"of {sample['token']['id']} is at block {sample['block']}")
        listed = any(r["hash"].lower() == sample["tx"].lower() for r in rows)
        caps["token-history"] = common.ok(
            f"{len(rows)} fungible rows ({len(raw) - len(rows)} NFT)"
            + (f"; sample transfer {'listed' if listed else 'not on this page'}" if sample.get("token_transfer") else ""))
    except Bad as error:
        caps["token-history"] = common.fail(str(error))

    if is_routescan(base):
        url = explorer_query_url(base, f"module=account&action=addresstokenbalance&address={address}&page=1&offset=1000")
    else:
        url = explorer_query_url(base, f"module=account&action=tokenlist&address={address}")
    try:
        raw = _explorer_rows(url)[0]
        held = _decode_tokenlist(raw)
        detail = f"{len(held)} ERC-20 holdings of {len(raw)} rows"
        token = sample.get("token")
        if token:
            balance = _sample_token_balance(chain, address, token["contract"])
            listed = any(h["contract"] == token["contract"] for h in held)
            detail += f"; {token['id']} {'listed' if listed else 'not listed'} (node balanceOf {balance})"
            if balance and not listed:
                if not held:
                    raise Bad(f"empty list for {address}, which holds {balance} of {token['id']} {token['contract']}")
                notes.append(f"token-discovery omits {token['id']} {token['contract']}, of which the node "
                             f"reports {address} holds {balance}")
        caps["token-discovery"] = common.ok(detail)
    except Bad as error:
        caps["token-discovery"] = common.fail(str(error))

    record["capabilities"] = caps
    if notes:
        record["notes"] = notes
    return record


_token_balances = {}


def _sample_token_balance(chain, address, contract):
    """The sample's balance of its token, from the network's nodes, once."""
    key = (chain, contract)
    with _locks_guard:
        lock = _sample_locks.setdefault(("balance", chain), threading.Lock())
    with lock:
        if key not in _token_balances:
            value = None
            for url in _derivation_urls(chain):
                method, params = _eth_call(contract, SEL_BALANCE_OF + _word(address))
                with _Held(url, None):
                    result, problem = _rpc(url, method, params)
                if not problem:
                    try:
                        value = _abi_u128(result, "balanceOf")
                        break
                    except Bad:
                        continue
            _token_balances[key] = value
        return _token_balances[key]


PROBES = {"evm-json-rpc": probe_rpc, "blockscout": probe_indexer}
