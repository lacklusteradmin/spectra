"""The UTXO family's indexers (Esplora, Blockbook, BlockCypher, WhatsOnChain,
BCH REST v2, Chronik, UniSat's Open API, Insight, the Kaspa REST server),
Monero daemons, lightwalletd and Litecoin peers.

Each probe makes the requests Spectra's adapter for the API
(`core/src/api/<api>.rs`) makes for a claimed capability, and reads the
answer as the adapter's serde (or prost) types decode it: a required field
that is missing or null, or a value of another JSON type than the adapter
declares (a number where it reads a string), fails as it would in core.
Reads retry as core's `ChainRead` profile does (three attempts), a broadcast
as `ChainWrite` does (two).

Sample: per network, an address paid by a non-coinbase transaction at least
`DEPTH` blocks below the tip, with a short history and an unspent output,
and that transaction as the confirmed one `verification` asks about. The
first endpoint of the network whose API can derive one derives it from the
network itself; every endpoint of the network is asked about the same
address and transaction.

Broadcast: the hex `00`, which decodes to no transaction, in the route's own
body format (Kaspa's route takes a structured transaction, so it is sent a
`transaction` that is a string). It passes when the service refuses it as
undecodable or invalid, which shows the route is served; it fails when the
route is missing, not allowed, or asks for credentials. Nothing is signed.

lightwalletd (gRPC) and Litecoin peers (P2P) are not HTTP: their
capabilities are `unchecked`, and core's own probe (`spectra endpoints
probe`) covers them. This records only whether the TLS host answers, or the
TCP port accepts a connection.
"""
import hashlib
import http.client
import json
import re
import socket
import ssl
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

import common

DEPTH = 6  # blocks below the tip the sample transaction is taken from
WALK = 60  # blocks walked back at most to find one with a usable output
MAX_HISTORY = 200  # transactions a sample address may have at most
READ = 2  # retries: core's ChainRead profile makes three attempts
WRITE = 1  # ChainWrite makes two
SETUP = 4  # deriving the sample is not a capability; it retries more
# Keyless rate limits (three requests a second; dcrdata answers 429 sooner):
# requests to these hosts start at least this many seconds apart.
PACE = {"api.whatsonchain.com": 0.4, "api.blockcypher.com": 0.4, "dcrdata.decred.org": 0.6}
# Hosts asked over one kept-alive connection per thread, as core's pooled
# client asks every host: dcrdata drops many new TLS handshakes.
KEEP_ALIVE = {"dcrdata.decred.org"}


# ── HTTP ───────────────────────────────────────────────────────────────────

_pace_guard = threading.Lock()
_pace_locks = {}
_pace_last = {}


def _pace(url):
    host = urllib.parse.urlsplit(url).hostname
    gap = PACE.get(host)
    if gap is None:
        return
    with _pace_guard:
        lock = _pace_locks.setdefault(host, threading.Lock())
    with lock:
        wait = _pace_last.get(host, 0.0) + gap - time.monotonic()
        if wait > 0:
            time.sleep(wait)
        _pace_last[host] = time.monotonic()


_kept = threading.local()


def _kept_alive(method, url, body, head, timeout):
    """One exchange over this thread's open connection to the host, through
    the environment's proxy as urllib would go: `(status, headers, body)`."""
    parts = urllib.parse.urlsplit(url)
    port = parts.port or (443 if parts.scheme == "https" else 80)
    pool = getattr(_kept, "pool", None)
    if pool is None:
        pool = _kept.pool = {}
    key = (parts.scheme, parts.hostname, port)
    connection = pool.get(key)
    if connection is None:
        kind = http.client.HTTPSConnection if parts.scheme == "https" else http.client.HTTPConnection
        proxy = urllib.request.getproxies().get(parts.scheme)
        if proxy and not urllib.request.proxy_bypass(parts.hostname):
            via = urllib.parse.urlsplit(proxy)
            connection = kind(via.hostname, via.port, timeout=timeout)
            connection.set_tunnel(parts.hostname, port)
        else:
            connection = kind(parts.hostname, port, timeout=timeout)
        pool[key] = connection
    try:
        connection.request(method, parts.path + (f"?{parts.query}" if parts.query else ""), body=body, headers=head)
        response = connection.getresponse()
        data = response.read()
    except Exception:
        connection.close()
        pool.pop(key, None)
        raise
    if response.will_close:
        connection.close()
        pool.pop(key, None)
    return response.status, {k.lower(): v for k, v in response.getheaders()}, data


def _request(method, url, *, body=None, json_body=None, content_type=None,
             accept="application/json", retries=READ, timeout=common.TIMEOUT):
    """One exchange as `common.request` makes it, retrying a transport
    error, a 429 or a 5xx, but keeping the answer's headers
    (`reply.headers`, names lower-cased) and pacing rate-limited hosts."""
    head = {"User-Agent": common.USER_AGENT, "Accept": accept}
    if json_body is not None:
        body = json.dumps(json_body).encode()
        content_type = "application/json"
    if content_type:
        head["Content-Type"] = content_type
    reply, failures = None, []
    for attempt in range(retries + 1):
        _pace(url)
        started = time.monotonic()
        req = urllib.request.Request(url, data=body, headers=head, method=method)
        headers = {}
        try:
            if urllib.parse.urlsplit(url).hostname in KEEP_ALIVE:
                status, headers, data = _kept_alive(method, url, body, head, timeout)
                reply = common.Reply(url, status, data, _ms(started))
                reply.headers = headers
                retryable = status == 429 or status >= 500
                if not retryable or attempt == retries:
                    return reply
                time.sleep(1.5 if status == 429 else 0.6)
                continue
            with urllib.request.urlopen(req, timeout=timeout) as response:
                headers = {k.lower(): v for k, v in response.headers.items()}
                reply = common.Reply(url, response.status, response.read(), _ms(started))
        except urllib.error.HTTPError as error:
            headers = {k.lower(): v for k, v in (error.headers or {}).items()}
            reply = common.Reply(url, error.code, error.read(), _ms(started))
        except Exception as error:  # noqa: BLE001 — every transport failure is evidence
            reply = common.Reply(url, error=f"{type(error).__name__}: {error}", ms=_ms(started))
        reply.headers = headers
        if reply.error:
            failures.append(reply.error.split(":")[0] if failures else reply.error)
        retryable = reply.error is not None or reply.status == 429 or (reply.status or 0) >= 500
        if not retryable or attempt == retries:
            if reply.error and len(failures) > 1:  # every attempt failed: say how each did
                reply.error = f"{failures[0]}; then " + ", ".join(failures[1:])
            return reply
        time.sleep(1.5 if reply.status == 429 else 0.6)
    return reply


def _ms(started):
    return int((time.monotonic() - started) * 1000)


def _get(url, retries=READ, accept="application/json", timeout=common.TIMEOUT):
    return _request("GET", url, retries=retries, accept=accept, timeout=timeout)


def _probe_get(url, accept="application/json"):
    """A candidate sample address's activity: a busy address's history can
    take an indexer long to count, and is passed over rather than waited for."""
    return _get(url, retries=1, accept=accept, timeout=12)


def _post(url, json_body, retries=WRITE):
    return _request("POST", url, json_body=json_body, retries=retries)


def _post_raw(url, body, content_type, retries=WRITE, accept="application/json"):
    return _request("POST", url, body=body, content_type=content_type, retries=retries, accept=accept)


# ── Results ────────────────────────────────────────────────────────────────

class Refused(Exception):
    """No answer, or one the adapter would not accept."""


class NoSample(Exception):
    """The network has no sample to ask about."""


def _capability(check, *args):
    try:
        result = check(*args)
    except Refused as error:
        return common.fail(str(error))
    except NoSample as error:
        return common.unchecked(str(error))
    except Exception as error:  # noqa: BLE001 — a probe bug is recorded, not fatal
        return common.fail(f"probe error: {type(error).__name__}: {error}")
    return result if isinstance(result, dict) else common.ok(result)


def _answered(reply):
    if reply.error or not 200 <= (reply.status or 0) < 300:
        raise Refused(reply.brief())
    return reply


def _json(reply):
    _answered(reply)
    if reply.json is None and reply.body.strip() != b"null":
        raise Refused(f"not JSON: {reply.brief()}")
    return reply.json


def _text(reply):
    return _answered(reply).text


def _int_text(reply):
    text = _text(reply).strip()
    if not text.isdigit():
        raise Refused(f"not a number: {reply.brief()}")
    return int(text)


def _is_int(value, low, high):
    return isinstance(value, int) and not isinstance(value, bool) and low <= value < high


_KINDS = {
    "u64": (lambda v: _is_int(v, 0, 2**64), "an unsigned integer"),
    "u32": (lambda v: _is_int(v, 0, 2**32), "a u32"),
    "i64": (lambda v: _is_int(v, -2**63, 2**63), "an integer"),
    "i32": (lambda v: _is_int(v, -2**31, 2**31), "an i32"),
    "f64": (lambda v: isinstance(v, (int, float)) and not isinstance(v, bool), "a number"),
    "str": (lambda v: isinstance(v, str), "a string"),
    "bool": (lambda v: isinstance(v, bool), "a boolean"),
    "any": (lambda v: True, "anything"),
    "f64map": (lambda v: isinstance(v, dict) and all(
        isinstance(x, (int, float)) and not isinstance(x, bool) for x in v.values()), "an object of numbers"),
}


def _describe(value):
    if value is None:
        return "null"
    if isinstance(value, bool):
        return str(value).lower()
    if isinstance(value, (int, float)):
        return f"the number {value}"
    if isinstance(value, str):
        return f"the string {json.dumps(value[:40])}"
    if isinstance(value, list):
        return f"an array of {len(value)}"
    return "an object"


def _shape(value, spec, where="answer"):
    """Check `value` as serde would decode it into `spec`: a kind name
    (`_KINDS`), `[item]` for an array, a tuple for a fixed-length array, or
    a dict of fields. A field's key is its name, prefixed `?` for an
    `Option` (missing or null passes) or `~` for `#[serde(default)]`
    (missing passes, null does not); `a|b` names an alias."""
    if isinstance(spec, str):
        accept, name = _KINDS[spec]
        if not accept(value):
            raise Refused(f"`{where}` is {_describe(value)}; the adapter reads {name}")
        return value
    if isinstance(spec, tuple):
        if not isinstance(value, list) or len(value) != len(spec):
            raise Refused(f"`{where}` is {_describe(value)}; the adapter reads a {len(spec)}-element array")
        for index, (item, sub) in enumerate(zip(value, spec)):
            _shape(item, sub, f"{where}[{index}]")
        return value
    if isinstance(spec, list):
        if not isinstance(value, list):
            raise Refused(f"`{where}` is {_describe(value)}; the adapter reads an array")
        for index, item in enumerate(value):
            _shape(item, spec[0], f"{where}[{index}]")
        return value
    if not isinstance(value, dict):
        raise Refused(f"`{where}` is {_describe(value)}; the adapter reads an object")
    for key, sub in spec.items():
        mode = key[0] if key[0] in "?~" else ""
        names = key.lstrip("?~").split("|")
        present = [name for name in names if name in value]
        if not present:
            if mode:
                continue
            raise Refused(f"`{where}.{names[0]}` is missing")
        name = present[0]
        if value[name] is None:
            if mode == "?":
                continue
            raise Refused(f"`{where}.{name}` is null")
        _shape(value[name], sub, f"{where}.{name}")
    return value


_HEX64 = re.compile(r"^[0-9a-fA-F]{64}$")


def _hex64(value, where):
    if not isinstance(value, str) or not _HEX64.match(value):
        raise Refused(f"`{where}` is {_describe(value)}, not a 32-byte hex hash")
    return value


def _digits(value, where):
    if not isinstance(value, str) or not value.isdigit() or not value.isascii():
        raise Refused(f"`{where}` is {_describe(value)}; the adapter reads unsigned integer digits")
    return int(value)


def _need(sample, *keys):
    if "error" in sample:
        raise NoSample(f"no sample on this network: {sample['error']}"[:400])
    missing = [key for key in keys if sample.get(key) in (None, "")]
    if missing:
        raise NoSample(f"the sample has no {', '.join(missing)}")
    return [sample[key] for key in keys]


def _height_matches(height, sample, what):
    """The confirmed height a status reports, against the sample's."""
    expected = sample.get("block_height")
    if expected is not None and height != expected:
        raise Refused(f"{what} reports height {height}; the sample transaction is in block {expected}")


# ── Broadcast ──────────────────────────────────────────────────────────────

_MISSING_ROUTE = re.compile(
    r"not found|not allowed|disabled|unauthori[sz]|forbidden|api[ _-]?key|access token|"
    r"authenticat|login|cannot post|no route|unknown (method|route|endpoint)|method not", re.I)
_UNDECODABLE = re.compile(
    r"decod|deserializ|pars(e|ing)|invalid|malformed|bad-txns|(?<!\d)-22\b|reject|unexpected end|too short|"
    r"validat|not a valid", re.I)


def _refusal(reply, extra=""):
    """A broadcast of `00`: refused as undecodable passes; a missing,
    disallowed or credentialed route fails."""
    if reply.error:
        return common.fail(reply.brief())
    text = reply.text
    if reply.status in (401, 403, 404, 405) or _MISSING_ROUTE.search(text):
        return common.fail(reply.brief())
    if 200 <= reply.status < 300 and re.search(r"\b[0-9a-fA-F]{64}\b", text) and not _UNDECODABLE.search(text):
        return common.fail(f"answered like an acceptance: {reply.brief()}")
    if _UNDECODABLE.search(text + " " + extra):
        return common.ok(f"refused as undecodable: {reply.brief(200)}{extra}")
    return common.fail(f"unrecognized answer: {reply.brief()}")


# ── Addresses and scripts ──────────────────────────────────────────────────

_B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
_B32 = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"

# Base58 version bytes per network (`registry::utxo`): P2PKH, then P2SH.
_VERSIONS = {
    "bitcoin": (b"\x00", [b"\x05"]), "bitcoin-testnet": (b"\x6f", [b"\xc4"]),
    "bitcoin-testnet-4": (b"\x6f", [b"\xc4"]), "bitcoin-signet": (b"\x6f", [b"\xc4"]),
    "bitcoin-cash": (b"\x00", [b"\x05"]), "bitcoin-sv": (b"\x00", [b"\x05"]),
    "bitcoin-sv-testnet": (b"\x6f", [b"\xc4"]), "dogecoin": (b"\x1e", [b"\x16"]),
    "litecoin": (b"\x30", [b"\x32", b"\x05"]), "litecoin-testnet": (b"\x6f", [b"\x3a", b"\xc4"]),
    "dash": (b"\x4c", [b"\x10"]), "bitcoin-gold": (b"\x26", [b"\x17"]),
    "peercoin": (b"\x37", [b"\x75"]), "peercoin-testnet": (b"\x6f", [b"\xc4"]),
    "qtum": (b"\x3a", [b"\x32"]), "fractal-bitcoin": (b"\x00", [b"\x05"]),
    "digibyte": (b"\x1e", [b"\x3f", b"\x05"]), "zcash": (b"\x1c\xb8", [b"\x1c\xbd"]),
    "decred": (b"\x07\x3f", [b"\x07\x1a"]),
}
_RANK = {"p2wpkh": 0, "p2pkh": 1}


def _b58decode(text):
    number = 0
    for char in text:
        number = number * 58 + _B58.index(char)
    raw = number.to_bytes((number.bit_length() + 7) // 8, "big")
    return b"\x00" * (len(text) - len(text.lstrip("1"))) + raw


def _convertbits(data, frm, to, pad=True):
    acc, bits, out = 0, 0, []
    for value in data:
        acc = (acc << frm) | value
        bits += frm
        while bits >= to:
            bits -= to
            out.append((acc >> bits) & ((1 << to) - 1))
    if pad and bits:
        out.append((acc << (to - bits)) & ((1 << to) - 1))
    return out


def _segwit(address):
    """`(version, program)` of a bech32 address, or None."""
    lower = address.lower()
    if "1" not in lower or ":" in lower:
        return None
    _, _, data = lower.rpartition("1")
    if len(data) < 8 or any(char not in _B32 for char in data):
        return None
    values = [_B32.index(char) for char in data[:-6]]
    return values[0], bytes(_convertbits(values[1:], 5, 8, pad=False))


def _script(chain, address):
    """The output script an address pays, as hex, or None when unknown."""
    witness = _segwit(address)
    if witness:
        version, program = witness
        return (bytes([0x50 + version if version else 0, len(program)]) + program).hex()
    try:
        raw = _b58decode(address)
    except ValueError:
        return None
    p2pkh, p2sh = _VERSIONS.get(chain, (None, []))
    if p2pkh and raw.startswith(p2pkh) and len(raw) == len(p2pkh) + 24:
        return "76a914" + raw[len(p2pkh):-4].hex() + "88ac"
    for version in p2sh:
        if raw.startswith(version) and len(raw) == len(version) + 24:
            return "a914" + raw[len(version):-4].hex() + "87"
    return None


def _kind(chain, address):
    if ":" in address:  # CashAddr: its first payload character says the type
        return "p2pkh" if address.split(":", 1)[1][:1] == "q" else "other"
    script = _script(chain, address) or ""
    if script.startswith("0014") and len(script) == 44:
        return "p2wpkh"
    if script.startswith("76a914"):
        return "p2pkh"
    return "other"


def _cashaddr(prefix, payload, version=0):
    def polymod(values):
        check = 1
        for value in values:
            top = check >> 35
            check = ((check & 0x07FFFFFFFF) << 5) ^ value
            for bit, generator in enumerate(
                    (0x98F2BC8E61, 0x79B76D99E2, 0xF33E5FB3C4, 0xAE2EABE2A8, 0x1E4F43E470)):
                if (top >> bit) & 1:
                    check ^= generator
        return check ^ 1
    data = _convertbits(bytes([version]) + payload, 8, 5)
    checksum = polymod([ord(char) & 0x1F for char in prefix] + [0] + data + [0] * 8)
    return prefix + ":" + "".join(_B32[d] for d in data + [(checksum >> 5 * (7 - i)) & 31 for i in range(8)])


def _sha256d(data):
    return hashlib.sha256(hashlib.sha256(data).digest()).digest()


def _cvarint(raw, pos):
    first = raw[pos]
    if first < 0xFD:
        return first, pos + 1
    size = {0xFD: 2, 0xFE: 4, 0xFF: 8}[first]
    return int.from_bytes(raw[pos + 1:pos + 1 + size], "little"), pos + 1 + size


def _parse_tx(raw, peercoin):
    """The txid and outputs `(value, script)` of a Bitcoin-format
    transaction, as `blockbook::decode_transaction` (or, on Peercoin,
    `decode_peercoin_transaction`, whose version-1 and -2 transactions carry
    a time after the version) reads it."""
    version = int.from_bytes(raw[:4], "little", signed=True)
    pos = 8 if peercoin and version < 3 else 4
    head = raw[:pos]
    witness = raw[pos] == 0 and raw[pos + 1] == 1
    if witness:
        pos += 2
    start = pos
    inputs, pos = _cvarint(raw, pos)
    for _ in range(inputs):
        pos += 36
        length, pos = _cvarint(raw, pos)
        pos += length + 4
    count, pos = _cvarint(raw, pos)
    outputs = []
    for _ in range(count):
        value = int.from_bytes(raw[pos:pos + 8], "little")
        length, pos = _cvarint(raw, pos + 8)
        outputs.append((value, raw[pos:pos + length]))
        pos += length
    end = pos
    if witness:
        for _ in range(inputs):
            items, pos = _cvarint(raw, pos)
            for _ in range(items):
                length, pos = _cvarint(raw, pos)
                pos += length
    if len(raw) != pos + 4:
        raise ValueError("transaction length does not match its contents")
    return _sha256d(head + raw[start:end] + raw[pos:])[::-1].hex(), outputs


# ── Protobuf (Chronik) ─────────────────────────────────────────────────────

def _pb_varint(data, pos):
    shift = result = 0
    while True:
        if pos >= len(data):
            raise ValueError("truncated varint")
        byte = data[pos]
        pos += 1
        result |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return result, pos
        shift += 7


def _pb(data):
    """One protobuf message's fields: `{tag: [(wire type, value)]}`."""
    fields, pos = {}, 0
    while pos < len(data):
        key, pos = _pb_varint(data, pos)
        wire = key & 7
        if wire == 0:
            value, pos = _pb_varint(data, pos)
        elif wire == 1:
            value, pos = int.from_bytes(data[pos:pos + 8], "little"), pos + 8
        elif wire == 2:
            length, pos = _pb_varint(data, pos)
            value, pos = data[pos:pos + length], pos + length
            if pos > len(data):
                raise ValueError("truncated field")
        elif wire == 5:
            value, pos = int.from_bytes(data[pos:pos + 4], "little"), pos + 4
        else:
            raise ValueError(f"wire type {wire}")
        fields.setdefault(key >> 3, []).append((wire, value))
    return fields


def _pb_get(fields, tag, wire, what, repeated=False, signed=False):
    """A field core's message declares; a value of another wire type fails
    as prost fails."""
    values = []
    for found, value in fields.get(tag, []):
        if found != wire:
            raise Refused(f"{what}: field {tag} has wire type {found}, core's message declares {wire}")
        if signed and value >= 2**63:
            value -= 2**64
        values.append(value)
    if repeated:
        return values
    if values:
        return values[-1]
    return b"" if wire == 2 else 0


def _pb_decode(body, what):
    try:
        return _pb(body)
    except (ValueError, IndexError) as error:
        raise Refused(f"{what} is not protobuf: {error}") from None


def _display(hash_bytes, what):
    if len(hash_bytes) != 32:
        raise Refused(f"{what} is {len(hash_bytes)} bytes, not a 32-byte hash")
    return hash_bytes[::-1].hex()


# ── Sample ─────────────────────────────────────────────────────────────────

_sample_guard = threading.Lock()
_sample_locks = {}
_samples = {}


def _sample(chain):
    """The network's sample, derived once and shared by its endpoints."""
    with _sample_guard:
        lock = _sample_locks.setdefault(chain, threading.Lock())
    with lock:
        if chain not in _samples:
            _samples[chain] = _derive(chain)
        return dict(_samples[chain])


def _derive(chain):
    rows = [row for row in common.endpoints(chain=chain) if row["api"] in _DERIVERS]
    rows.sort(key=lambda row: list(_DERIVERS).index(row["api"]))
    errors = []
    for row in rows:
        try:
            found = _DERIVERS[row["api"]](row["endpoint"].rstrip("/"), chain)
        except Exception as error:  # noqa: BLE001 — the next endpoint may derive it
            errors.append(f"{row['id']}: {error if isinstance(error, Refused) else repr(error)}"[:300])
            continue
        return {**found, "derived_via": row["id"]}
    return {"error": "; ".join(errors) or "no endpoint of this network can derive a sample"}


def _pick(chain, batches, activity, budget=25):
    """The first candidate address, most like a wallet's own first, with a
    short history and an unspent output; `batches` yields the candidates of
    one block at a time."""
    tried, seen = [], set()
    for batch in batches:
        for candidate in sorted(batch, key=lambda c: _RANK.get(_kind(chain, c["address"]), 9)):
            if candidate["address"] in seen:
                continue
            seen.add(candidate["address"])
            if len(tried) >= budget:
                raise Refused("no short-history address with an unspent output: " + "; ".join(tried))
            try:
                count, outputs = activity(candidate["address"])
            except (Refused, KeyError, TypeError, ValueError) as error:
                tried.append(f"{candidate['address']}: {error}"[:160])
                continue
            tried.append(f"{candidate['address']}: {count} txs, {outputs} outputs")
            if 1 <= count <= MAX_HISTORY and (outputs is None or outputs > 0):
                return {**candidate, "address_tx_count": count, "address_outputs": outputs}
    raise Refused("no candidate address with a short history and an unspent output"
                  + (": " + "; ".join(tried) if tried else ""))


def _derive_esplora(base, chain):
    tip = _int_text(_get(f"{base}/blocks/tip/height", retries=SETUP))

    def blocks():
        """`(height, hash)` of blocks below the tip holding more than a coinbase."""
        height = tip - DEPTH
        while height > tip - DEPTH - WALK:
            listing = _get(f"{base}/blocks/{height}", retries=SETUP)
            if listing.status == 200 and isinstance(listing.json, list) and listing.json:
                for block in listing.json:
                    if block.get("tx_count", 0) > 1:
                        yield block["height"], block["id"]
                height = min(block["height"] for block in listing.json) - 1
            else:
                yield height, _text(_get(f"{base}/block-height/{height}", retries=SETUP)).strip()
                height -= 1

    def batches():
        for height, block in blocks():
            try:
                txs = _json(_get(f"{base}/block/{block}/txs/0", retries=SETUP))
            except Refused:
                continue
            yield [{"address": out["scriptpubkey_address"], "txid": tx["txid"], "block_height": height}
                   for tx in txs[1:] for out in tx.get("vout", []) if out.get("scriptpubkey_address")]

    def activity(address):
        stats = _json(_probe_get(f"{base}/address/{address}"))
        count = stats["chain_stats"]["tx_count"] + stats["mempool_stats"]["tx_count"]
        utxos = _probe_get(f"{base}/address/{address}/utxo")
        if utxos.status == 200 and isinstance(utxos.json, list):
            return count, len(utxos.json)
        # Fractal's mempool instance lists no outputs: a balance says there is one.
        held = stats["chain_stats"]["funded_txo_sum"] - stats["chain_stats"]["spent_txo_sum"]
        return count, 1 if held > 0 else 0

    # Inscription services pay most of Fractal's and the testnets' outputs.
    return _pick(chain, batches(), activity, budget=40)


def _bb_address(chain, address):
    """`BlockbookClient::normalize_address`: a whole CashAddr, prefix
    included (the samples are CashAddr already; a legacy address core
    converts the same way)."""
    prefix = {"bitcoin-cash": "bitcoincash", "bitcoin-cash-testnet": "bchtest"}.get(chain)
    if prefix and ":" not in address:
        return f"{prefix}:{address}"
    return address


def _derive_blockbook(base, chain):
    if chain in _PEERCOIN:
        _peercoin_identity(base, chain)
    tip = _json(_get(f"{base}/api/v2", retries=SETUP))["blockbook"]["bestHeight"]

    def batches():
        for height in range(tip - DEPTH, tip - DEPTH - WALK, -1):
            try:
                block = _json(_get(f"{base}/api/v2/block/{height}?page=1", retries=SETUP))
            except Refused:
                continue
            batch = []
            for tx in block.get("txs") or []:
                if not any(vin.get("addresses") for vin in tx.get("vin") or []):
                    continue  # a coinbase
                outputs = tx.get("vout") or []
                if outputs and not outputs[0].get("addresses") and outputs[0].get("value") in ("0", None):
                    continue  # a coinstake, whose outputs mature slowly
                batch += [{"address": out["addresses"][0], "txid": tx["txid"], "block_height": height}
                          for out in outputs if out.get("isAddress") and len(out.get("addresses") or []) == 1]
            yield batch

    def activity(address):
        # As the indexer names it, prefix and all: the capabilities ask as
        # core does.
        info = _json(_probe_get(f"{base}/api/v2/address/{address}?details=basic"))
        utxos = _json(_probe_get(f"{base}/api/v2/utxo/{address}"))
        return info["txs"] + info.get("unconfirmedTxs", 0), len(utxos)

    return _pick(chain, batches(), activity)


def _derive_insight(base, chain):
    status = _json(_get(f"{base}/status", retries=SETUP))
    tip = (status.get("info") or status)["blocks"]

    def batches():
        for height in range(tip - DEPTH, tip - DEPTH - 20, -1):
            try:
                block_hash = _json(_get(f"{base}/block-index/{height}", retries=SETUP))["blockHash"]
                block = _json(_get(f"{base}/block/{block_hash}", retries=SETUP))
            except Refused:
                continue
            block = block[0] if isinstance(block, list) else block
            batch = []
            for txid in block.get("tx", [])[1:15]:
                try:
                    tx = _json(_get(f"{base}/tx/{txid}", retries=SETUP))
                except Refused:
                    continue
                if not tx.get("vin") or not all(vin.get("addr") for vin in tx["vin"]):
                    continue  # a coinbase, stakebase or vote
                batch += [{"address": out["scriptPubKey"]["addresses"][0], "txid": txid, "block_height": height}
                          for out in tx.get("vout", [])
                          if len((out.get("scriptPubKey") or {}).get("addresses") or []) == 1]
            yield batch

    def activity(address):
        # dcrdata's TLS handshakes fail often from here: retry as the walk
        # does. Its output list can hang; a balance says there is an output.
        info = _json(_get(f"{base}/addr/{address}?noTxList=1", retries=SETUP))
        held = info.get("balanceSat") or round((info.get("balance") or 0) * 1e8)
        return info["txApperances"] + info.get("unconfirmedTxApperances", 0), 1 if held > 0 else 0

    return _pick(chain, batches(), activity, budget=30)


def _derive_kaspa(base, chain):
    block_hash = _json(_get(f"{base}/info/network", retries=SETUP))["sink"]

    def block_at(block_hash):
        """A block, waiting for the server's database, which can trail its
        node by seconds, to hold one just named."""
        for _ in range(6):
            reply = _get(f"{base}/blocks/{block_hash}?includeTransactions=true", retries=SETUP)
            if reply.status != 404:
                return _json(reply)
            time.sleep(3)
        return _json(reply)

    def batches():
        nonlocal block_hash
        for _ in range(WALK):
            block = block_at(block_hash)
            yield [{"address": out["verboseData"]["scriptPublicKeyAddress"],
                    "txid": tx["verboseData"]["transactionId"]}
                   for tx in block.get("transactions") or [] if tx.get("inputs")
                   for out in tx.get("outputs") or []
                   if (out.get("verboseData") or {}).get("scriptPublicKeyAddress")]
            block_hash = block["verboseData"]["selectedParentHash"]

    def activity(address):
        count = _json(_probe_get(f"{base}/addresses/{address}/transactions-count"))["total"]
        utxos = _json(_probe_get(f"{base}/addresses/{address}/utxos"))
        return count, len(utxos)

    found = _pick(chain, batches(), activity, budget=40)
    for _ in range(5):  # a transaction is accepted by a later chain block
        tx = _json(_get(f"{base}/transactions/{found['txid']}?inputs=false&outputs=false", retries=SETUP))
        if tx.get("accepting_block_blue_score"):
            return {**found, "accepting_blue_score": tx["accepting_block_blue_score"]}
        time.sleep(2)
    raise Refused(f"sample transaction {found['txid']} was not accepted")


def _derive_whatsonchain(base, chain):
    tip = _json(_get(f"{base}/chain/info", retries=SETUP))["blocks"]

    def batches():
        for height in range(tip - DEPTH, tip - DEPTH - 20, -1):
            try:
                block = _json(_get(f"{base}/block/height/{height}", retries=SETUP))
                txids = (block.get("tx") or [])[1:21]
                if not txids:
                    continue
                txs = _json(_request("POST", f"{base}/txs", json_body={"txids": txids}, retries=SETUP))
            except Refused:
                continue
            yield [{"address": out["scriptPubKey"]["addresses"][0], "txid": tx["txid"], "block_height": height}
                   for tx in txs if isinstance(tx, dict) for out in tx.get("vout") or []
                   if (out.get("scriptPubKey") or {}).get("type") == "pubkeyhash"
                   and len(out["scriptPubKey"].get("addresses") or []) == 1]

    def activity(address):
        index = _json(_probe_get(f"{base}/address/{address}/confirmed/history?limit=50&order=desc"))
        utxos = _json(_probe_get(f"{base}/address/{address}/unspent"))
        count = len(index["result"]) if not index.get("nextPageToken") else MAX_HISTORY + 1
        return count, len(utxos) if isinstance(utxos, list) else 0

    return _pick(chain, batches(), activity)


def _derive_chronik(base, chain):
    tip = _pb_get(_pb_decode(_answered(_get(f"{base}/blockchain-info", retries=SETUP, accept="*/*")).body,
                             "blockchain info"), 2, 0, "BlockchainInfo")
    hashes = {}

    def batches():
        for height in range(tip - DEPTH, tip - DEPTH - WALK, -1):
            page = _pb_decode(_answered(_get(f"{base}/block-txs/{height}?page=0&page_size=50",
                                             retries=SETUP, accept="*/*")).body, "block transactions")
            batch = []
            for raw in _pb_get(page, 1, 2, "TxHistoryPage", repeated=True):
                tx = _pb(raw)
                if _pb_get(tx, 12, 0, "Tx"):
                    continue  # a coinbase
                txid = _display(_pb_get(tx, 1, 2, "Tx"), "txid")
                for out in _pb_get(tx, 4, 2, "Tx", repeated=True):
                    script = _pb_get(_pb(out), 2, 2, "TxOutput")
                    if len(script) == 25 and script[:3] == b"\x76\xa9\x14" and script[23:] == b"\x88\xac":
                        address = _cashaddr("ecash", script[3:23])
                        hashes[address] = script[3:23].hex()
                        batch.append({"address": address, "txid": txid, "block_height": height})
            yield batch

    def activity(address):
        path = f"{base}/script/p2pkh/{hashes[address]}"
        history = _pb(_answered(_probe_get(f"{path}/history?page=0&page_size=1", accept="*/*")).body)
        utxos = _pb(_answered(_probe_get(f"{path}/utxos", accept="*/*")).body)
        return _pb_get(history, 3, 0, "TxHistoryPage"), len(_pb_get(utxos, 2, 2, "ScriptUtxos", repeated=True))

    found = _pick(chain, batches(), activity)
    return {**found, "script_hash": hashes[found["address"]]}


def _derive_monero(base, chain):
    header = _monero_rpc(base, "get_last_block_header", retries=SETUP)
    tip = header["block_header"]["height"]
    for height in range(tip - DEPTH, tip - DEPTH - 30, -1):
        block = _monero_rpc(base, "get_block", {"height": height}, retries=SETUP)
        if block.get("tx_hashes"):
            return {"txid": block["tx_hashes"][0], "block_height": height}
    block = _monero_rpc(base, "get_block", {"height": tip - DEPTH}, retries=SETUP)
    return {"txid": block["miner_tx_hash"], "block_height": tip - DEPTH, "coinbase": True}


# Derivers in the order a network's endpoints are asked to derive its
# sample. BlockCypher's keyless hourly limit leaves it out; BCH REST, UniSat,
# lightwalletd and Litecoin peers share a network with an API that derives.
_DERIVERS = {
    "esplora": _derive_esplora,
    "blockbook": _derive_blockbook,
    "insight": _derive_insight,
    "kaspa-rest": _derive_kaspa,
    "whatsonchain": _derive_whatsonchain,
    "chronik": _derive_chronik,
    "monero-daemon-rpc": _derive_monero,
}


def _fee_target(chain):
    """The confirmation target core asks for: Litecoin's MWEB quote asks for
    3 blocks; the Bitcoin-format sends ask for 6."""
    return 3 if chain in ("litecoin", "litecoin-testnet") else 6


def _decimals(chain):
    coins = [row for row in common.deployments(chain) if not row.get("contract")]
    return coins[0]["decimals"] if coins else 8


def _result(capabilities, sample, notes):
    answer = {"capabilities": capabilities, "sample": sample}
    if notes:
        answer["notes"] = notes
    return answer


def _probe(row, checks, sample, notes, last=()):
    """Run each claimed capability's check, those named in `last` last."""
    claimed = [capability for capability in row["capabilities"] if capability in checks]
    claimed.sort(key=lambda capability: capability in last)
    results = {capability: _capability(checks[capability]) for capability in claimed}
    return _result({capability: results[capability] for capability in row["capabilities"] if capability in results},
                   sample, notes)


# ── Esplora ────────────────────────────────────────────────────────────────

_ESPLORA_STATS = {"funded_txo_sum": "u64", "spent_txo_sum": "u64", "tx_count": "u64"}
_ESPLORA_STATUS = {"confirmed": "bool", "?block_height": "u64", "?block_time": "u64"}
_ESPLORA_TX = {
    "txid": "str", "status": _ESPLORA_STATUS,
    "vout": [{"?scriptpubkey_address": "str", "value": "u64"}],
    "vin": [{"?prevout": {"?scriptpubkey_address": "str", "value": "u64"}}],
    "?fee": "u64",
}
_ESPLORA_UTXO = {"txid": "str", "vout": "u32", "status": {"confirmed": "bool", "?block_height": "u64"}, "value": "u64"}
_MEMPOOL_FEES = {"fastestFee": "f64", "halfHourFee": "f64", "hourFee": "f64", "economyFee": "f64"}


def _esplora(row):
    base, chain = row["endpoint"].rstrip("/"), row["chain_id"]
    sample, notes = _sample(chain), []
    tip = _get(f"{base}/blocks/tip/height")
    notes.append(f"tip height {tip.text.strip()}" if tip.status == 200 else f"tip: {tip.brief(120)}")

    def balance():
        (address,) = _need(sample, "address")
        stats = _shape(_json(_get(f"{base}/address/{address}")),
                       {"chain_stats": _ESPLORA_STATS, "mempool_stats": _ESPLORA_STATS})
        chain_stats = stats["chain_stats"]
        if chain_stats["tx_count"] + stats["mempool_stats"]["tx_count"] == 0:
            raise Refused(f"counts no transaction for {address}, which has history")
        return (f"{chain_stats['funded_txo_sum'] - chain_stats['spent_txo_sum']} confirmed, "
                f"{chain_stats['tx_count']} txs")

    def history():
        address, txid = _need(sample, "address", "txid")
        txs = _shape(_json(_get(f"{base}/address/{address}/txs")), [_ESPLORA_TX])
        if not txs:
            raise Refused(f"lists no transaction for {address}, which has history")
        _confirmed_times((tx["txid"], tx["status"]["confirmed"], tx["status"].get("block_time")) for tx in txs)
        detail = f"{len(txs)} txs, sample tx listed: {any(tx['txid'] == txid for tx in txs)}"
        confirmed = [tx for tx in txs if tx["status"]["confirmed"]]
        if confirmed:  # core asks for the page after any confirmed row
            more = _shape(_json(_get(f"{base}/address/{address}/txs/chain/{confirmed[-1]['txid']}")), [_ESPLORA_TX])
            detail += f"; {len(confirmed)} confirmed a page, next page {len(more)} txs"
        return detail

    def utxo():
        (address,) = _need(sample, "address")
        outputs = _shape(_json(_get(f"{base}/address/{address}/utxo")), [_ESPLORA_UTXO])
        for index, output in enumerate(outputs):
            _hex64(output["txid"], f"answer[{index}].txid")
        return _outputs_detail(outputs, lambda o: o["status"]["confirmed"], lambda o: o["value"])

    def fee():
        target = _fee_target(chain)
        estimates = _get(f"{base}/fee-estimates")
        source = "/fee-estimates"
        try:
            rates = _shape(_json(estimates), "f64map")
        except Refused as error:
            recommended = _get(f"{base}/v1/fees/recommended")
            try:
                fees = _shape(_json(recommended), _MEMPOOL_FEES)
            except Refused as fallback:
                raise Refused(f"/fee-estimates: {error}; /v1/fees/recommended: {fallback}") from None
            rates = {"1": fees["fastestFee"], "3": fees["halfHourFee"], "6": fees["hourFee"],
                     "144": fees["economyFee"]}
            source = "/v1/fees/recommended (no /fee-estimates)"
        targets = sorted((int(key), value) for key, value in rates.items() if key.isdigit())
        rate = rates.get(str(target))
        if rate is None:
            above = [value for key, value in targets if key >= target]
            rate = above[0] if above else None
        if rate is None or not rate > 0:
            raise Refused(f"{source} gives no positive rate for {target} blocks (core would fall back to 10): {rates}"[:300])
        return f"{rate} sat/vB for {target} blocks from {source}"

    def verification():
        (txid,) = _need(sample, "txid")
        status = _shape(_json(_get(f"{base}/tx/{txid}/status")), _ESPLORA_STATUS)
        if not status["confirmed"]:
            raise Refused(f"reports {txid} unconfirmed")
        _height_matches(status.get("block_height"), sample, "status")
        return f"confirmed at {status.get('block_height')}"

    def broadcast():
        return _refusal(_post_raw(f"{base}/tx", b"00", "text/plain"))

    checks = {"balance": balance, "history": history, "utxo": utxo, "fee": fee,
              "verification": verification, "broadcast": broadcast}
    return _probe(row, checks, sample, notes)


def _confirmed_times(rows):
    """`api::time::history_time`: a confirmed transaction without a time
    fails the page."""
    for txid, confirmed, when in rows:
        if confirmed and not (when or 0) > 0:
            raise Refused(f"confirmed transaction {txid} has no time; core refuses the page")


def _outputs_detail(outputs, confirmed, value):
    if not outputs:
        return "lists no unspent output for the sample address"
    settled = [o for o in outputs if confirmed(o)]
    return f"{len(outputs)} outputs ({len(settled)} confirmed), {sum(value(o) for o in outputs)} in all"


# ── Blockbook ──────────────────────────────────────────────────────────────

_PEERCOIN = {"peercoin": ("Peercoin", "livenet"), "peercoin-testnet": ("Peercoin Testnet", "testnet")}
_VERIFIES_OUTPUTS = {"peercoin", "peercoin-testnet", "qtum"}  # `Chain::verifies_spent_outputs`
_ZCASH_GENESIS = {"zcash": "00040fe8ec8471911baa1db1266ea15dd06b4a8a5c453883c000b031973dce08"}
_BB_IO = {"?addresses": ["str"], "?value": "str"}
_BB_TX = {
    "txid": "str", "?hex": "str", "?confirmations": "u64", "?blockTime": "u64", "?blockHeight": "u64",
    "?fees": "str", "~vin": [_BB_IO], "~vout": [_BB_IO],
}
_BB_UTXO = {"txid": "str", "vout": "u32", "value": "str", "~confirmations": "u32", "?height": "u64"}


def _peercoin_identity(base, chain):
    """`verify_peercoin_endpoint`, which every Peercoin request runs first."""
    status = _shape(_json(_get(f"{base}/api/v2")),
                    {"?blockbook": {"coin": "str", "decimals": "u32"}, "backend": {"?chain": "str", "blocks": "u32"}})
    coin, network = _PEERCOIN[chain]
    identity = status.get("blockbook") or {}
    found = (identity.get("coin"), identity.get("decimals"), status["backend"].get("chain"))
    if found != (coin, _decimals(chain), network):
        raise Refused(f"Blockbook names {found}; core requires {(coin, _decimals(chain), network)}")
    return found


def _blockbook(row):
    base, chain = row["endpoint"].rstrip("/"), row["chain_id"]
    sample, notes = _sample(chain), []
    status = _get(f"{base}/api/v2")
    if status.status == 200 and isinstance(status.json, dict):
        indexer, backend = status.json.get("blockbook") or {}, status.json.get("backend") or {}
        notes.append(f"tip height {indexer.get('bestHeight')}, coin {indexer.get('coin')!r}, "
                     f"backend chain {backend.get('chain')!r}, in sync {indexer.get('inSync')}")
    else:
        notes.append(f"status: {status.brief(120)}")
    identity_error = None
    if chain in _PEERCOIN:
        try:
            notes.append(f"Peercoin identity verified: {_peercoin_identity(base, chain)}")
        except Refused as error:
            identity_error = f"Peercoin identity check, which every request runs first: {error}"

    def guarded(check):
        def run():
            if identity_error:
                raise Refused(identity_error)
            return check()
        return run

    def address():
        (found,) = _need(sample, "address")
        return _bb_address(chain, found)

    def asked(check):
        """Say how core spelled a CashAddr it asked about."""
        def run():
            try:
                return check()
            except Refused as error:
                original = sample.get("address") or ""
                if original != _bb_address(chain, original):
                    raise Refused(f"{error} (asked for {_bb_address(chain, original)!r})") from None
                raise
        return run

    def balance():
        info = _shape(_json(_get(f"{base}/api/v2/address/{address()}?details=basic")),
                      {"balance": "str", "?unconfirmedBalance": "str"})
        confirmed = _digits(info["balance"], "answer.balance")
        pending = info.get("unconfirmedBalance")
        if pending is not None and not re.fullmatch(r"[+-]?\d+", pending):
            raise Refused(f"`answer.unconfirmedBalance` is {_describe(pending)}; core parses an i64")
        return f"{confirmed} confirmed, {pending or 0} pending"

    def history():
        page = _shape(_json(_get(f"{base}/api/v2/address/{address()}?details=txs&page=1&pageSize=50")),
                      {"?totalPages": "u32", "~transactions": [_BB_TX]})
        txs = page.get("transactions") or []
        if page.get("totalPages") is None and len(txs) >= 50:
            raise Refused("a full history page omits totalPages")
        if not txs:
            raise Refused("lists no transaction for the sample address, which has history")
        _confirmed_times((tx["txid"], (tx.get("blockHeight") or 0) > 0, tx.get("blockTime")) for tx in txs)
        return (f"{len(txs)} txs of {page.get('totalPages')} pages, sample tx listed: "
                f"{any(tx['txid'] == sample.get('txid') for tx in txs)}")

    def utxo():
        outputs = _shape(_json(_get(f"{base}/api/v2/utxo/{address()}")), [_BB_UTXO])
        for index, output in enumerate(outputs):
            _digits(output["value"], f"answer[{index}].value")
            _hex64(output["txid"], f"answer[{index}].txid")
        detail = _outputs_detail(outputs, lambda o: o.get("confirmations", 0) > 0, lambda o: int(o["value"]))
        if chain in _VERIFIES_OUTPUTS:
            detail += "; " + _verified_outputs(base, chain, outputs)
        if chain in _ZCASH_GENESIS:
            detail += "; " + _zcash_context(base, chain)
        return detail

    def fee():
        target = _fee_target(chain)
        estimate = _shape(_json(_get(f"{base}/api/v2/estimatefee/{target}")), {"result": "str"})
        try:
            per_kb = float(estimate["result"])
        except ValueError:
            raise Refused(f"`answer.result` is {_describe(estimate['result'])}; core parses a number") from None
        if not per_kb > 0 or per_kb == float("inf"):
            raise Refused(f"no fee estimate: result {estimate['result']!r} for {target} blocks")
        return f"{per_kb * 10 ** _decimals(chain) / 1000:g} sat/vB for {target} blocks"

    def verification():
        (txid,) = _need(sample, "txid")
        tx = _shape(_json(_get(f"{base}/api/v2/tx/{txid}")), _BB_TX)
        if tx["txid"].lower() != txid.lower():
            raise Refused(f"answered transaction {tx['txid']} for {txid}")
        height = tx.get("blockHeight")
        if not (height or 0) > 0:
            raise Refused(f"reports {txid} unconfirmed (blockHeight {height})")
        _height_matches(height, sample, "status")
        return f"confirmed at {height}, {tx.get('confirmations')} confirmations"

    def broadcast():
        reply = _post_raw(f"{base}/api/v2/sendtx/", b"00", "text/plain")
        return _refusal(reply)

    checks = {"balance": asked(balance), "history": asked(history), "utxo": asked(utxo), "fee": fee,
              "verification": verification, "broadcast": broadcast}
    return _probe(row, {name: guarded(check) for name, check in checks.items()}, sample, notes)


def _verified_outputs(base, chain, outputs):
    """`fetch_verified_outputs` on Peercoin and Qtum: each output read from
    its transaction's raw bytes, hash-checked against its txid."""
    checked = 0
    for txid in sorted({output["txid"] for output in outputs})[:3]:
        tx = _shape(_json(_get(f"{base}/api/v2/tx/{txid}")), _BB_TX)
        if not tx.get("hex"):
            raise Refused(f"transaction {txid} has no raw bytes (`hex`); core reads every output from them")
        if tx.get("confirmations") is None:
            raise Refused(f"transaction {txid} has no confirmation count")
        if tx["txid"] != txid:
            raise Refused(f"answered transaction {tx['txid']} for {txid}")
        try:
            computed, tx_outputs = _parse_tx(bytes.fromhex(tx["hex"]), chain in _PEERCOIN)
        except (ValueError, IndexError, KeyError) as error:
            raise Refused(f"transaction {txid}'s raw bytes do not decode: {error}") from None
        if computed != txid:
            raise Refused(f"transaction {txid}'s raw bytes hash to {computed}")
        for output in outputs:
            if output["txid"] == txid:
                if output["vout"] >= len(tx_outputs) or tx_outputs[output["vout"]][0] != int(output["value"]):
                    raise Refused(f"output {txid}:{output['vout']} does not match its transaction")
        checked += 1
    return f"{checked} transactions' raw bytes hash to their ids and carry the listed outputs"


def _zcash_context(base, chain):
    """`zcash_context`, which a Zcash send reads from the endpoints serving
    verification and outputs: the genesis block and the consensus branches."""
    genesis = _shape(_json(_get(f"{base}/api/v2/block-index/0")), {"blockHash": "str"})
    if genesis["blockHash"] != _ZCASH_GENESIS[chain]:
        raise Refused(f"block 0 is {genesis['blockHash']}, not Zcash's genesis block")
    status = _shape(_json(_get(f"{base}/api/v2")),
                    {"backend": {"blocks": "u32", "?consensus": {"chaintip": "str", "nextblock": "str"}}})
    consensus = status["backend"].get("consensus")
    if not consensus:
        raise Refused("reports no Zcash consensus branch; core refuses to build a send")
    for name in ("chaintip", "nextblock"):
        if not re.fullmatch(r"[0-9a-fA-F]{8}", consensus[name]):
            raise Refused(f"consensus {name} {consensus[name]!r} is not an 8-digit branch id")
    return f"Zcash genesis and consensus branches {consensus['chaintip']}/{consensus['nextblock']} answered"


# ── BlockCypher ────────────────────────────────────────────────────────────

_BC_TXREF = {
    "tx_hash": "str", "~tx_output_n": "i32", "~tx_input_n": "i32", "value": "i64",
    "~block_height": "i64", "~spent": "bool", "?confirmed": "str",
}
_BC_ADDRESS = {"?hasMore": "bool", "~txrefs": [_BC_TXREF], "~unconfirmed_txrefs": [_BC_TXREF]}
_ISO = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$")


def _blockcypher(row):
    base, chain = row["endpoint"].rstrip("/"), row["chain_id"]
    sample, notes = _sample(chain), []

    def balance():
        (address,) = _need(sample, "address")
        info = _shape(_json(_get(f"{base}/addrs/{address}/balance")),
                      {"balance": "u64", "~unconfirmed_balance": "i64"})
        return f"{info['balance']} confirmed, {info.get('unconfirmed_balance', 0)} pending"

    def history():
        (address,) = _need(sample, "address")
        page = _shape(_json(_get(f"{base}/addrs/{address}?limit=50")), _BC_ADDRESS)
        refs = page.get("txrefs", []) + page.get("unconfirmed_txrefs", [])
        if not refs:
            raise Refused("lists no transaction for the sample address, which has history")
        for ref in page.get("txrefs", []):
            if ref.get("block_height", 0) > 0 and not _ISO.match(ref.get("confirmed") or ""):
                raise Refused(f"confirmed ref {ref['tx_hash']} has no RFC 3339 time ({ref.get('confirmed')!r})")
        detail = f"{len(refs)} refs, more: {page.get('hasMore')}"
        if page.get("hasMore", len(page.get("txrefs", [])) >= 50):
            heights = [ref["block_height"] for ref in page["txrefs"] if ref.get("block_height", 0) > 0]
            if not heights:
                raise Refused("a full history page has no block height to continue from")
            low = min(heights)
            cohort = _shape(_json(_get(f"{base}/addrs/{address}?limit=2000&before={low + 1}&after={low - 1}")),
                            _BC_ADDRESS)
            detail += f"; block {low} cohort {len(cohort.get('txrefs', []))} refs"
        return detail + f", sample tx listed: {any(ref['tx_hash'] == sample.get('txid') for ref in refs)}"

    def utxo():
        (address,) = _need(sample, "address")
        page = _shape(_json(_get(f"{base}/addrs/{address}?unspentOnly=true")), _BC_ADDRESS)
        outputs = [ref for ref in page.get("txrefs", []) + page.get("unconfirmed_txrefs", [])
                   if ref.get("tx_output_n", 0) >= 0 and not ref.get("spent") and ref["value"] >= 0]
        for ref in outputs:
            _hex64(ref["tx_hash"], "tx_hash")
        return _outputs_detail(outputs, lambda r: r.get("block_height", 0) > 0, lambda r: r["value"])

    def fee():
        info = _shape(_json(_get(base)), {"high_fee_per_kb": "f64", "medium_fee_per_kb": "f64",
                                          "low_fee_per_kb": "f64", "?height": "u64"})
        notes.append(f"tip height {info.get('height')}")
        target = _fee_target(chain)
        per_kb = info["high_fee_per_kb"] if target <= 2 else info["medium_fee_per_kb"] if target <= 6 \
            else info["low_fee_per_kb"]
        if not per_kb > 0:
            raise Refused(f"no fee estimate for {target} blocks: {per_kb}")
        return f"{per_kb / 1000:g} sat/vB for {target} blocks"

    def verification():
        (txid,) = _need(sample, "txid")
        tx = _shape(_json(_get(f"{base}/txs/{txid}")),
                    {"hash": "str", "?block_height": "i64", "?confirmations": "u64", "?confirmed": "str"})
        if not (tx.get("block_height") or 0) > 0:
            raise Refused(f"reports {txid} unconfirmed (block_height {tx.get('block_height')})")
        _height_matches(tx["block_height"], sample, "status")
        return f"confirmed at {tx['block_height']}, {tx.get('confirmations')} confirmations"

    def broadcast():
        return _refusal(_post(f"{base}/txs/push", {"tx": "00"}))

    checks = {"balance": balance, "history": history, "utxo": utxo, "fee": fee,
              "verification": verification, "broadcast": broadcast}
    return _probe(row, checks, sample, notes)


# ── WhatsOnChain ───────────────────────────────────────────────────────────

_WOC_INDEX = {"result": [{"tx_hash": "str", "~height": "i64"}], "?nextPageToken": "str", "~error": "str"}
_WOC_TX = {
    "~txid": "str", "?time": "u64", "?blocktime": "u64", "?blockheight": "i64",
    "~vin": [{"~txid": "str", "~vout": "u32"}],
    "~vout": [{"~value": "f64", "~n": "u32", "?scriptPubKey": {"?addresses": ["str"]}}],
}


def _whatsonchain(row):
    base, chain = row["endpoint"].rstrip("/"), row["chain_id"]
    sample, notes = _sample(chain), []
    info = _get(f"{base}/chain/info")
    notes.append(f"tip height {(info.json or {}).get('blocks')}" if info.status == 200 else f"tip: {info.brief(120)}")

    def balance():
        (address,) = _need(sample, "address")
        found = _shape(_json(_get(f"{base}/address/{address}/balance")), {"~confirmed": "i64", "~unconfirmed": "i64"})
        if "confirmed" not in found:
            raise Refused(f"answer names no confirmed balance; core would read 0: {found}"[:200])
        return f"{found['confirmed']} confirmed, {found.get('unconfirmed', 0)} pending"

    def utxo():
        (address,) = _need(sample, "address")
        outputs = _shape(_json(_get(f"{base}/address/{address}/unspent")),
                         [{"tx_hash": "str", "tx_pos": "u32", "value": "u64", "~height": "i64"}])
        for output in outputs:
            _hex64(output["tx_hash"], "tx_hash")
        return _outputs_detail(outputs, lambda o: o.get("height", 0) > 0, lambda o: o["value"])

    def history():
        (address,) = _need(sample, "address")
        pending = _shape(_json(_get(f"{base}/address/{address}/unconfirmed/history")), _WOC_INDEX)
        if pending.get("error"):
            raise Refused(f"unconfirmed history: {pending['error']}")
        reply = _get(f"{base}/address/{address}/confirmed/history?limit=50&order=desc")
        index = _shape(_json(reply), _WOC_INDEX)
        if index.get("error"):
            raise Refused(f"confirmed history: {index['error']}")
        token = index.get("nextPageToken") or reply.headers.get("next-page")
        if len(index["result"]) == 50 and not token:
            raise Refused("a full history page omits its continuation")
        items = pending["result"] + index["result"]
        if not items:
            raise Refused("lists no transaction for the sample address, which has history")
        first = items[0]["tx_hash"]
        tx = _shape(_json(_get(f"{base}/tx/hash/{first}")), _WOC_TX)
        parents = [vin for vin in tx.get("vin", []) if vin.get("txid")]
        if parents:
            parent = _shape(_json(_get(f"{base}/tx/hash/{parents[0]['txid']}")), _WOC_TX)
            if not any(out.get("n") == parents[0].get("vout", 0) for out in parent.get("vout", [])):
                raise Refused(f"previous output {parents[0]['txid']}:{parents[0].get('vout')} is missing")
        _confirmed_times([(first, (tx.get("blockheight") or items[0].get("height", 0)) > 0,
                           tx.get("blocktime") or tx.get("time"))])
        return (f"{len(pending['result'])} pending + {len(index['result'])} confirmed, details and previous "
                f"output read; sample tx listed: {any(i['tx_hash'] == sample.get('txid') for i in items)}")

    def verification():
        (txid,) = _need(sample, "txid")
        tx = _shape(_json(_get(f"{base}/tx/hash/{txid}")), _WOC_TX)
        height = tx.get("blockheight")
        if height is None or height < 0:
            raise Refused(f"reports {txid} unconfirmed (blockheight {height})")
        _height_matches(height, sample, "status")
        return f"confirmed at {height}"

    def broadcast():
        return _refusal(_post(f"{base}/tx/raw", {"txhex": "00"}))

    checks = {"balance": balance, "history": history, "utxo": utxo,
              "verification": verification, "broadcast": broadcast}
    return _probe(row, checks, sample, notes)


# ── BCH REST v2 ────────────────────────────────────────────────────────────

_BCH_TX = {
    "txid": "str", "?blockheight": "i64", "?blocktime": "u64", "?confirmations": "u64", "?fees": "f64",
    "~vin": [{"?cashAddress": "str", "?valueSat|value": "f64"}],
    "~vout": [{"value": "f64", "?scriptPubKey": {"~addresses": ["str"]}}],
}


def _bch_rest(row):
    base, chain = row["endpoint"].rstrip("/"), row["chain_id"]
    sample, notes = _sample(chain), []
    count = _get(f"{base}/blockchain/getBlockCount")
    notes.append(f"tip height {count.text.strip()}" if count.status == 200 else f"tip: {count.brief(120)}")

    def balance():
        (address,) = _need(sample, "address")
        info = _shape(_json(_get(f"{base}/address/details/{address}")),
                      {"balanceSat": "u64", "unconfirmedBalanceSat": "i64", "txAppearances": "u64",
                       "unconfirmedTxAppearances": "u64"})
        return f"{info['balanceSat']} confirmed, {info['txAppearances']} txs"

    def utxo():
        (address,) = _need(sample, "address")
        found = _shape(_json(_get(f"{base}/address/utxo/{address}")),
                       {"utxos": [{"txid": "str", "vout": "u32", "satoshis": "u64", "~height": "u64",
                                   "~confirmations": "u64"}]})
        for output in found["utxos"]:
            _hex64(output["txid"], "txid")
        return _outputs_detail(found["utxos"], lambda o: o.get("confirmations", 0) > 0 and o.get("height", 0) > 0,
                               lambda o: o["satoshis"])

    def history():
        (address,) = _need(sample, "address")
        page = _shape(_json(_get(f"{base}/address/transactions/{address}?page=0")),
                      {"?pagesTotal": "u32", "txs": [(_BCH_TX, "any")], "legacyAddress": "str", "cashAddress": "str"})
        if page.get("pagesTotal") is None:
            raise Refused("history omits pagesTotal")
        if not page["txs"]:
            raise Refused("lists no transaction for the sample address, which has history")
        ours = {address, page["cashAddress"], page["legacyAddress"]}
        for tx, _ in page["txs"]:
            for vin in tx.get("vin", []):
                if vin.get("cashAddress") in ours and vin.get("valueSat", vin.get("value")) is None:
                    raise Refused(f"input of {tx['txid']} has no value")
        _confirmed_times((tx["txid"], (tx.get("blockheight") or 0) > 0, tx.get("blocktime")) for tx, _ in page["txs"])
        return (f"{len(page['txs'])} txs of {page['pagesTotal']} pages, sample tx listed: "
                f"{any(tx['txid'] == sample.get('txid') for tx, _ in page['txs'])}")

    def verification():
        (txid,) = _need(sample, "txid")
        tx = _shape(_json(_get(f"{base}/transaction/details/{txid}")), _BCH_TX)
        if not (tx.get("blockheight") or 0) > 0:
            raise Refused(f"reports {txid} unconfirmed (blockheight {tx.get('blockheight')})")
        _height_matches(tx["blockheight"], sample, "status")
        return f"confirmed at {tx['blockheight']}"

    def broadcast():
        return _refusal(_post(f"{base}/rawtransactions/sendRawTransaction", {"hexes": ["00"]}))

    checks = {"balance": balance, "history": history, "utxo": utxo,
              "verification": verification, "broadcast": broadcast}
    return _probe(row, checks, sample, notes)


# ── Chronik ────────────────────────────────────────────────────────────────

def _chronik(row):
    """Chronik answers in protobuf. Checked: the status, the content type,
    and the fields core's prost messages declare and read (their wire types,
    hash lengths, the script an answer is for, and the txid); fields core
    does not declare are not."""
    base, chain = row["endpoint"].rstrip("/"), row["chain_id"]
    sample, notes = _sample(chain), []

    def fetch(path, what):
        reply = _answered(_get(f"{base}{path}", accept="*/*"))
        kind = reply.headers.get("content-type", "")
        if "protobuf" not in kind:
            raise Refused(f"{what} answered {kind or 'no content type'}, not protobuf: {reply.brief(120)}")
        return _pb_decode(reply.body, what)

    try:
        info = fetch("/blockchain-info", "blockchain info")
        notes.append(f"tip height {_pb_get(info, 2, 0, 'BlockchainInfo', signed=True)}")
    except Refused as error:
        notes.append(f"tip: {error}")

    def script():
        (hash_hex,) = _need(sample, "script_hash")
        return f"/script/p2pkh/{hash_hex}", bytes.fromhex("76a914" + hash_hex + "88ac")

    def utxos():
        path, expected = script()
        answer = fetch(f"{path}/utxos", "ScriptUtxos")
        if _pb_get(answer, 1, 2, "ScriptUtxos") != expected:
            raise Refused("answered for another script")
        rows = []
        for raw in _pb_get(answer, 2, 2, "ScriptUtxos", repeated=True):
            utxo = _pb(raw)
            sats = _pb_get(utxo, 5, 0, "ScriptUtxo", signed=True)
            if sats < 0:
                raise Refused("negative output amount")
            outpoint = _pb_get(utxo, 1, 2, "ScriptUtxo")
            if not outpoint:
                raise Refused("an output has no outpoint")
            point = _pb(outpoint)
            rows.append({"txid": _display(_pb_get(point, 1, 2, "OutPoint"), "outpoint txid"),
                         "vout": _pb_get(point, 2, 0, "OutPoint"), "sats": sats,
                         "height": _pb_get(utxo, 2, 0, "ScriptUtxo", signed=True),
                         "coinbase": _pb_get(utxo, 3, 0, "ScriptUtxo"), "token": 11 in utxo})
        return rows

    def balance():
        rows = utxos()
        confirmed = sum(r["sats"] for r in rows if r["height"] >= 0)
        return f"{confirmed} confirmed over {len(rows)} outputs (protobuf ScriptUtxos)"

    def utxo():
        rows = utxos()
        if any(r["coinbase"] for r in rows):
            fetch("/blockchain-info", "blockchain info")
        return (_outputs_detail(rows, lambda r: r["height"] >= 0, lambda r: r["sats"])
                + f", {sum(r['token'] for r in rows)} carrying a token (protobuf ScriptUtxos)")

    def history():
        path, expected = script()
        page = fetch(f"{path}/history?page=0&page_size=50", "TxHistoryPage")
        txs = _pb_get(page, 1, 2, "TxHistoryPage", repeated=True)
        if not txs:
            raise Refused("lists no transaction for the sample address, which has history")
        listed = []
        for raw in txs:
            tx = _pb(raw)
            txid = _display(_pb_get(tx, 1, 2, "Tx"), "txid")
            listed.append(txid)
            for raw_input in _pb_get(tx, 3, 2, "Tx", repeated=True):
                _pb_get(_pb(raw_input), 4, 0, "TxInput", signed=True)
            for raw_output in _pb_get(tx, 4, 2, "Tx", repeated=True):
                _pb_get(_pb(raw_output), 1, 0, "TxOutput", signed=True)
            block = _pb_get(tx, 8, 2, "Tx")
            if block and not _pb_get(_pb(block), 3, 0, "BlockMetadata", signed=True) > 0:
                raise Refused(f"confirmed transaction {txid} has no time; core refuses the page")
        return (f"{len(txs)} txs of {_pb_get(page, 3, 0, 'TxHistoryPage')} in "
                f"{_pb_get(page, 2, 0, 'TxHistoryPage')} pages (protobuf); sample tx listed: "
                f"{sample.get('txid') in listed}")

    def verification():
        (txid,) = _need(sample, "txid")
        tx = fetch(f"/tx/{txid}", "Tx")
        found = _display(_pb_get(tx, 1, 2, "Tx"), "txid")
        if found != txid.lower():
            raise Refused(f"answered transaction {found} for {txid}")
        block = _pb_get(tx, 8, 2, "Tx")
        if not block:
            raise Refused(f"reports {txid} unconfirmed")
        height = _pb_get(_pb(block), 1, 0, "BlockMetadata", signed=True)
        _height_matches(height, sample, "status")
        return f"confirmed at {height} (protobuf Tx)"

    def broadcast():
        body = b"\x0a\x01\x00"  # BroadcastTxRequest { raw_tx: [0x00], skip_token_checks: false }
        reply = _post_raw(f"{base}/broadcast-tx", body, "application/x-protobuf", accept="*/*")
        message = ""
        try:
            message = _pb_get(_pb(reply.body), 2, 2, "ErrorMessage").decode("utf-8", "replace")
        except (Refused, ValueError, IndexError):
            pass
        result = _refusal(reply, f" (ErrorMessage.msg {message!r})" if message else "")
        if result["ok"] and reply.status != 400:
            return common.fail(f"refused with HTTP {reply.status}; core reads a refusal as HTTP 400: {reply.brief()}")
        return result

    checks = {"balance": balance, "history": history, "utxo": utxo,
              "verification": verification, "broadcast": broadcast}
    return _probe(row, checks, sample, notes)


# ── UniSat Open API ────────────────────────────────────────────────────────

def _unisat(row):
    base, chain = row["endpoint"].rstrip("/"), row["chain_id"]
    sample, notes = _sample(chain), []

    def get(path, spec):
        answer = _shape(_json(_get(f"{base}/v1/indexer{path}")), {"code": "i64", "~msg": "str", "?data": "any"})
        if answer["code"] != 0:
            raise Refused(f"UniSat refused: code {answer['code']}, {answer.get('msg')!r}")
        if answer.get("data") is None:
            raise Refused("answer has no data")
        return _shape(answer["data"], spec, "data")

    try:
        notes.append(f"tip height {get('/blockchain/info', {'blocks': 'u64'})['blocks']}")
    except Refused as error:
        notes.append(f"tip: {error}")

    def balance():
        (address,) = _need(sample, "address")
        found = get(f"/address/{address}/balance", {"satoshi": "u64", "~pendingSatoshi": "i64"})
        return f"{found['satoshi']} confirmed, {found.get('pendingSatoshi', 0)} pending"

    def utxo():
        (address,) = _need(sample, "address")
        page = get(f"/address/{address}/available-utxo-data?cursor=0&size=64",
                   {"~total": "u64", "~utxo": [{"txid": "str", "vout": "u32", "satoshi": "u64", "scriptPk": "str",
                                                 "~height": "u64", "~confirmations": "u64", "~isSpent": "bool",
                                                 "~inscriptions": ["any"]}]})
        expected = _script(chain, address)
        for output in page.get("utxo", []):
            _hex64(output["txid"], "txid")
            if expected and output["scriptPk"].lower() != expected:
                raise Refused(f"output {output['txid']}:{output['vout']} pays {output['scriptPk']}, not the address")
        outputs = page.get("utxo", [])
        return (_outputs_detail(outputs, lambda o: o.get("confirmations", 0) > 0, lambda o: o["satoshi"])
                + f" of total {page.get('total')}")

    def history():
        (address,) = _need(sample, "address")
        page = get(f"/address/{address}/history?cursor=0&size=20",
                   {"total": "u64", "~detail": [{"txid": "str", "nIn": "u64", "nOut": "u64", "~inSatoshi": "u64",
                                                  "~outSatoshi": "u64", "~height": "u64", "~timestamp": "u64",
                                                  "~confirmations": "u64"}]})
        rows = page.get("detail", [])
        if not rows:
            raise Refused("lists no transaction for the sample address, which has history")
        first = rows[0]
        io = {"~address": "str", "satoshi": "u64"}
        ins = get(f"/tx/{first['txid']}/ins?cursor=0&size={min(64, first['nIn'])}", [io])
        outs = get(f"/tx/{first['txid']}/outs?cursor=0&size={min(64, first['nOut'])}", [io])
        if not outs or (first["nIn"] and not ins):
            raise Refused(f"transaction {first['txid']} is shorter than it says")
        _confirmed_times((r["txid"], r.get("confirmations", 0) > 0, r.get("timestamp")) for r in rows)
        return (f"{len(rows)} txs of {page['total']}, inputs and outputs read; sample tx listed: "
                f"{any(r['txid'] == sample.get('txid') for r in rows)}")

    def verification():
        (txid,) = _need(sample, "txid")
        tx = get(f"/tx/{txid}", {"txid": "str", "~height": "u64", "~timestamp": "u64", "~confirmations": "u64"})
        if tx["txid"].lower() != txid.lower():
            raise Refused(f"answered transaction {tx['txid']} for {txid}")
        if not tx.get("confirmations", 0) > 0:
            raise Refused(f"reports {txid} unconfirmed")
        _height_matches(tx.get("height"), sample, "status")
        return f"confirmed at {tx.get('height')}, {tx.get('confirmations')} confirmations"

    def broadcast():
        reply = _post(f"{base}/v1/indexer/local_pushtx", {"txHex": "00"})
        if isinstance(reply.json, dict) and reply.json.get("code") == 0:
            return common.fail(f"answered code 0, an acceptance: {reply.brief()}")
        return _refusal(reply)

    checks = {"balance": balance, "history": history, "utxo": utxo,
              "verification": verification, "broadcast": broadcast}
    return _probe(row, checks, sample, notes)


# ── Insight (dcrdata) ──────────────────────────────────────────────────────

_INSIGHT_TX = {
    "txid": "str", "~blockheight": "i64", "~time": "u64", "~fees": "f64",
    "~vin": [{"?addr": "str", "~valueSat": "u64", "~value": "f64"}],
    "~vout": [{"value": "f64", "?scriptPubKey": {"~addresses": ["str"]}}],
}


def _insight(row):
    base, chain = row["endpoint"].rstrip("/"), row["chain_id"]
    sample, notes = _sample(chain), []
    status = _get(f"{base}/status")
    if status.status == 200 and isinstance(status.json, dict):
        notes.append(f"tip height {(status.json.get('info') or status.json).get('blocks')}")
    else:
        notes.append(f"status: {status.brief(120)}")

    def balance():
        (address,) = _need(sample, "address")
        info = _shape(_json(_get(f"{base}/addr/{address}?noTxList=1")), {"?balanceSat": "u64", "~balance": "f64"})
        return f"balanceSat {info.get('balanceSat')}, balance {info.get('balance')}"

    def utxo():
        (address,) = _need(sample, "address")
        outputs = _shape(_json(_get(f"{base}/addr/{address}/utxo")),
                         [{"txid": "str", "vout": "u32", "~satoshis": "u64", "~amount": "f64", "~confirmations": "u32"}])
        for output in outputs:
            _hex64(output["txid"], "txid")
        return _outputs_detail(outputs, lambda o: o.get("confirmations", 0) > 0,
                               lambda o: o.get("satoshis") or round(o.get("amount", 0) * 1e8))

    def history():
        (address,) = _need(sample, "address")
        page = _shape(_json(_get(f"{base}/txs?address={address}&pageNum=0")), {"?pagesTotal": "u32", "~txs": [_INSIGHT_TX]})
        txs = page.get("txs", [])
        if page.get("pagesTotal") is None and len(txs) >= 10:
            raise Refused("a full history page omits pagesTotal")
        if not txs:
            raise Refused("lists no transaction for the sample address, which has history")
        _confirmed_times((tx["txid"], tx.get("blockheight", 0) > 0, tx.get("time")) for tx in txs)
        return f"{len(txs)} txs of {page.get('pagesTotal')} pages"

    def verification():
        (txid,) = _need(sample, "txid")
        tx = _shape(_json(_get(f"{base}/tx/{txid}")), _INSIGHT_TX)
        if not tx.get("blockheight", 0) > 0:
            raise Refused(f"reports {txid} unconfirmed")
        _height_matches(tx["blockheight"], sample, "status")
        return f"confirmed at {tx['blockheight']}"

    def fee():
        raise NoSample("Spectra's Insight adapter makes no fee request; a Decred send pays the registry's "
                       "static fee, so this claim has no request behind it")

    def broadcast():
        return _refusal(_post(f"{base}/tx/send", {"rawtx": "00"}))

    checks = {"balance": balance, "history": history, "utxo": utxo, "fee": fee,
              "verification": verification, "broadcast": broadcast}
    # dcrdata's output list can hang, and the handshakes after it fail for a
    # while: ask it last, so the other answers are not lost with it.
    return _probe(row, checks, sample, notes, last=("utxo",))


# ── Kaspa REST ─────────────────────────────────────────────────────────────

_KASPA_TX = {
    "~transaction_id": "str", "~block_time": "u64", "?accepting_block_blue_score": "u64",
    "~inputs": [{"?previous_outpoint_address": "str", "?previous_outpoint_amount": "u64"}],
    "~outputs": [{"~amount": "u64", "?script_public_key_address": "str"}],
}
_KASPA_NETWORK = {"kaspa": "kaspa-mainnet", "kaspa-testnet": "kaspa-testnet-10"}


def _kaspa(row):
    base, chain = row["endpoint"].rstrip("/"), row["chain_id"]
    sample, notes = _sample(chain), []
    info = _get(f"{base}/info/network")
    if info.status == 200 and isinstance(info.json, dict):
        notes.append(f"network {info.json.get('networkName')!r}, virtual DAA score {info.json.get('virtualDaaScore')}")
    else:
        notes.append(f"network: {info.brief(120)}")

    def balance():
        (address,) = _need(sample, "address")
        found = _shape(_json(_get(f"{base}/addresses/{address}/balance")), {"~balance": "u64"})
        if "balance" not in found:
            raise Refused(f"answer names no balance; core would read 0: {found}"[:200])
        return f"{found['balance']} sompi"

    def utxo():
        (address,) = _need(sample, "address")
        outputs = _shape(_json(_get(f"{base}/addresses/{address}/utxos")),
                         [{"address": "str", "outpoint": {"transactionId": "str", "index": "u32"},
                           "utxoEntry": {"amount": "str", "scriptPublicKey": {"~version": "u32", "scriptPublicKey": "str"},
                                         "?isCoinbase": "bool"}}])
        for index, output in enumerate(outputs):
            _digits(output["utxoEntry"]["amount"], f"answer[{index}].utxoEntry.amount")
            _hex64(output["outpoint"]["transactionId"], f"answer[{index}].outpoint.transactionId")
        own = [o for o in outputs if o["address"] == address]
        return _outputs_detail(own, lambda o: True, lambda o: int(o["utxoEntry"]["amount"]))

    def history():
        (address,) = _need(sample, "address")
        reply = _get(f"{base}/addresses/{address}/full-transactions-page?limit=50&resolve_previous_outpoints=light")
        txs = _shape(_json(reply), [_KASPA_TX])
        following = reply.headers.get("x-next-page-before")
        if len(txs) >= 50 and not following:
            raise Refused("a full history page omits its x-next-page-before header")
        if not txs:
            raise Refused("lists no transaction for the sample address, which has history")
        for tx in txs:  # every field has a default: check the ones core reads are there
            if not tx.get("transaction_id"):
                raise Refused("a transaction has no transaction_id; core would read an empty id")
            if not tx.get("block_time"):
                raise Refused(f"transaction {tx['transaction_id']} has no block_time; core refuses the page")
        if not any(address in (out.get("script_public_key_address"),) for tx in txs for out in tx.get("outputs", [])) \
                and not any(address == inp.get("previous_outpoint_address") for tx in txs for inp in tx.get("inputs", [])):
            raise Refused("no input or output names the address; core would net every transaction to zero")
        return (f"{len(txs)} txs, next page header {'present' if following else 'absent'}, sample tx listed: "
                f"{any(tx['transaction_id'] == sample.get('txid') for tx in txs)}")

    def verification():
        (txid,) = _need(sample, "txid")
        tx = _shape(_json(_get(f"{base}/transactions/{txid}")), _KASPA_TX)
        if tx.get("transaction_id") != txid:
            raise Refused(f"answered transaction {tx.get('transaction_id')!r} for {txid}")
        if tx.get("accepting_block_blue_score") is None:
            raise Refused(f"reports {txid} not accepted")
        return f"accepted at blue score {tx['accepting_block_blue_score']}"

    def fee():
        raise NoSample("Spectra's Kaspa REST adapter makes no fee request; a Kaspa send pays the registry's "
                       "static fee, so this claim has no request behind it")

    def broadcast():
        # The route takes a structured transaction; a string where the
        # object goes decodes to none.
        reply = _post(f"{base}/transactions", {"transaction": "00"})
        if reply.status == 422:
            return common.ok(f"refused as undecodable (request validation): {reply.brief(200)}")
        return _refusal(reply)

    checks = {"balance": balance, "history": history, "utxo": utxo, "fee": fee,
              "verification": verification, "broadcast": broadcast}
    answer = _probe(row, checks, sample, notes)
    if info.status == 200 and isinstance(info.json, dict) and info.json.get("networkName") != _KASPA_NETWORK.get(chain):
        answer.setdefault("notes", []).append(
            f"network name {info.json.get('networkName')!r}, expected {_KASPA_NETWORK.get(chain)!r}")
    return answer


# ── Monero daemon RPC ──────────────────────────────────────────────────────

_MONERO_NETTYPE = {"monero": "mainnet", "monero-stagenet": "stagenet"}


def _monero_rpc(base, method, params=None, retries=READ):
    body = {"jsonrpc": "2.0", "id": "0", "method": method}
    if params is not None:
        body["params"] = params
    answer = _json(_request("POST", f"{base}/json_rpc", json_body=body, retries=retries))
    if not isinstance(answer, dict) or answer.get("error") or not isinstance(answer.get("result"), dict):
        raise Refused(f"{method}: {json.dumps(answer)[:200]}")
    return answer["result"]


def _monero(row):
    """Every operation first runs `monero_daemon_rpc::daemon`: `/get_info`
    must name the network and say it is synchronized, `hard_fork_info` must
    be version 16, and a batched JSON-RPC call must answer."""
    base, chain = row["endpoint"].rstrip("/"), row["chain_id"]
    sample, notes = _sample(chain), []
    prerequisite = None
    try:
        info = _json(_request("POST", f"{base}/get_info", json_body={}))
        sample["get_info"] = {key: info.get(key) for key in
                              ("nettype", "synchronized", "height", "target_height", "restricted", "untrusted")}
        if info.get("nettype") != _MONERO_NETTYPE[chain] or info.get("synchronized") is not True:
            raise Refused(f"get_info names nettype {info.get('nettype')!r}, synchronized "
                          f"{info.get('synchronized')!r}; core requires {_MONERO_NETTYPE[chain]!r} and true")
        fork = _monero_rpc(base, "hard_fork_info")
        sample["get_info"]["hard_fork_version"] = fork.get("version")
        if fork.get("version") != 16:
            raise Refused(f"hard_fork_info version {fork.get('version')}; core requires 16")
        batch = _json(_request("POST", f"{base}/json_rpc", json_body=[
            {"jsonrpc": "2.0", "method": "on_get_block_hash", "params": [0], "id": 0},
            {"jsonrpc": "2.0", "method": "on_get_block_hash", "params": [1], "id": 1}]))
        if not isinstance(batch, list) and not (isinstance(batch, dict) and (batch.get("error") or {}).get("code") == -32700):
            raise Refused(f"batched on_get_block_hash answered {json.dumps(batch)[:160]}")
    except Refused as error:
        prerequisite = f"daemon check every operation runs first: {error}"

    def guarded(check):
        def run():
            if prerequisite:
                raise Refused(prerequisite)
            return check()
        return run

    def fee():
        result = _shape(_monero_rpc(base, "get_fee_estimate", {"grace_blocks": 10}),
                        {"status": "str", "?fees": ["u64"], "fee": "u64", "quantization_mask": "u64"})
        if result["status"] != "OK":
            raise Refused(f"get_fee_estimate status {result['status']!r}")
        fees = result.get("fees")
        rate = fees[1] if fees and len(fees) > 1 else result["fee"] * 5 if not fees else None
        if not rate or not result["quantization_mask"]:
            raise Refused(f"no normal-priority rate: fees {fees}, fee {result['fee']}, mask {result['quantization_mask']}")
        return f"{rate} piconero/byte at normal priority, quantization mask {result['quantization_mask']}"

    def verification():
        (txid,) = _need(sample, "txid")
        answer = _json(_request("POST", f"{base}/get_transactions",
                                json_body={"txs_hashes": [txid], "decode_as_json": False, "prune": True}))
        if not isinstance(answer, dict) or answer.get("status") != "OK" or answer.get("untrusted") is True:
            raise Refused(f"did not supply a trusted result: {json.dumps(answer)[:200]}")
        if txid in (answer.get("missed_tx") or []):
            raise Refused(f"does not know {txid} (missed_tx)")
        rows = answer.get("txs")
        if not isinstance(rows, list) or len(rows) != 1:
            raise Refused(f"expected one transaction, got {_describe(rows)}")
        tx = _shape(rows[0], {"tx_hash": "str", "in_pool": "bool", "?block_height": "u64"}, "txs[0]")
        if tx["tx_hash"].lower() != txid.lower():
            raise Refused(f"answered transaction {tx['tx_hash']} for {txid}")
        if tx["in_pool"]:
            raise Refused(f"reports {txid} in the pool")
        if tx.get("block_height") is None:
            raise Refused("missing block height")
        _height_matches(tx["block_height"], sample, "status")
        return f"confirmed at {tx['block_height']}"

    def broadcast():
        reply = _request("POST", f"{base}/send_raw_transaction",
                         json_body={"tx_as_hex": "00", "do_sanity_checks": False}, retries=WRITE)
        answer = reply.json
        if reply.error or reply.status != 200 or not isinstance(answer, dict):
            return _refusal(reply)
        if answer.get("status") == "OK":
            return common.fail(f"answered status OK: {reply.brief()}")
        if _MISSING_ROUTE.search(reply.text) or answer.get("status") in ("BUSY",):
            return common.fail(reply.brief())
        fields = ("double_spend", "fee_too_low", "invalid_input", "invalid_output", "low_mixin", "not_relayed",
                  "overspend", "too_big", "too_few_outputs", "reason")
        missing = [name for name in fields if name not in answer]
        detail = f"refused: status {answer.get('status')!r}, reason {answer.get('reason')!r}"
        if missing:
            detail += f"; the refusal lacks {', '.join(missing)}, which monero-daemon-rpc's SendRawResponse requires"
        return common.ok(detail)

    checks = {"fee": fee, "verification": verification, "broadcast": broadcast}
    return _probe(row, {name: guarded(check) for name, check in checks.items()}, sample, notes)


# ── lightwalletd and Litecoin peers ────────────────────────────────────────

def _lightwalletd(row):
    parts = urllib.parse.urlsplit(row["endpoint"])
    host, port = parts.hostname, parts.port or 443
    try:
        context = ssl.create_default_context()
        context.set_alpn_protocols(["h2"])
        with socket.create_connection((host, port), timeout=10) as raw:
            with context.wrap_socket(raw, server_hostname=host) as tls:
                seen = f"TLS host answered ({tls.version()}, ALPN {tls.selected_alpn_protocol()!r})"
    except Exception as error:  # noqa: BLE001 — the failure is the evidence
        seen = f"TLS host did not answer: {type(error).__name__}: {error}"
    reason = f"gRPC, not probed over HTTP; core's own probe (`spectra endpoints probe`) covers it. {seen}"
    return {"capabilities": {c: common.unchecked(reason) for c in row["capabilities"]}, "sample": {}, "notes": [seen]}


def _litecoin_p2p(row):
    """A peer speaks only after the client's `version`, so a connection it
    accepts stays open and silent; one a relay could not complete closes."""
    parts = urllib.parse.urlsplit(row["endpoint"])
    try:
        with socket.create_connection((parts.hostname, parts.port), timeout=10) as connection:
            peer = connection.getpeername()[0]
            connection.settimeout(5)
            try:
                held = connection.recv(1) != b""
                state = "then sent data unprompted" if held else "then closed it at once"
            except socket.timeout:
                state = "and held it open, silent, for 5 s as a peer awaiting `version` does"
            seen = f"TCP port accepted a connection (address {peer}) {state}"
            if peer.startswith(("198.18.", "198.19.")):
                seen += "; the address is a local proxy's fake-IP range, so the accept itself is the proxy's"
    except Exception as error:  # noqa: BLE001 — the failure is the evidence
        seen = f"TCP port refused: {type(error).__name__}: {error}"
    reason = (f"Litecoin peer-to-peer, not probed over HTTP; core's own probe (`spectra endpoints probe`) "
              f"covers it. {seen}")
    return {"capabilities": {c: common.unchecked(reason) for c in row["capabilities"]}, "sample": {}, "notes": [seen]}


PROBES = {
    "esplora": _esplora,
    "blockbook": _blockbook,
    "blockcypher": _blockcypher,
    "whatsonchain": _whatsonchain,
    "bch-rest-v2": _bch_rest,
    "chronik": _chronik,
    "unisat-open-api": _unisat,
    "insight": _insight,
    "kaspa-rest": _kaspa,
    "monero-daemon-rpc": _monero,
    "lightwalletd": _lightwalletd,
    "litecoin-p2p": _litecoin_p2p,
}
