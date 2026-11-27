"""Account networks, part A: Solana, Sui, NEAR (node, Nearblocks, FastNEAR),
Aptos (node and indexer), the XRP Ledger, Stellar Horizon, Tron (node and
TronGrid) and TON Center v2 and v3.

Each check repeats the requests core's adapter (`core/src/api/<api>.rs`) makes
for a capability and checks the fields that adapter reads. Samples come from
the network itself, once per chain, so every endpoint of a network answers
about the same address: a recent transaction's signer that holds the catalog
token it moved, a stake authority, a recent transaction id. Testnets derive
their own.

Broadcast routes get a payload that decodes to no transaction (a zero byte in
the route's encoding). Nothing is signed; Sui's execution mutation gets bytes
that cannot deserialize.
"""
import base64
import hashlib
import json
import re
import threading
import time
import urllib.parse

import common
from common import fail, ok, unchecked


# ── Plumbing ───────────────────────────────────────────────────────────────

class Bad(Exception):
    """A check that found the endpoint wanting; its text is the detail."""


def need(condition, detail):
    if not condition:
        raise Bad(detail)


def _short(text, limit=300):
    text = str(text).replace("\n", " ")
    return text if len(text) <= limit else text[:limit] + "…"


_DIGITS = re.compile(r"^-?\d+$")


def digits(value):
    return isinstance(value, str) and bool(_DIGITS.match(value))


def is_int(value):
    return isinstance(value, int) and not isinstance(value, bool)


# Keyless TON Center answers one request a second per client, and keyless
# Nearblocks four a minute, rejected ones counted (2026-11-26: the fifth of
# five 11 s or 16 s apart was refused). Each operator's mainnet and testnet
# hosts share one budget. Pacing keeps the record about the endpoint, not
# about the probe.
_SHARED_PACE = {
    "api-testnet.nearblocks.io": "api.nearblocks.io",
    "testnet.toncenter.com": "toncenter.com",
    "api.devnet.solana.com": "api.mainnet-beta.solana.com",
    "rpc.testnet.near.org": "rpc.mainnet.near.org",
}
_PACED = {
    "toncenter.com": 1.25,
    "api.nearblocks.io": 20.0,
    "rpc.mainnet.near.org": 0.6,
    "api.trongrid.io": 0.4,
    "nile.trongrid.io": 0.4,
    "api.mainnet-beta.solana.com": 0.35,
    "solana-rpc.publicnode.com": 0.1,
    # Solana's public nodes limit each method on its own; getTransaction is
    # the one history repeats.
    "api.mainnet-beta.solana.com#getTransaction": 1.0,
}
_pace_lock = threading.Lock()
_pace_next = {}


def _pace(url, method=None):
    host = urllib.parse.urlsplit(url).hostname or ""
    host = _SHARED_PACE.get(host, host)
    if method and f"{host}#{method}" in _PACED:
        host = f"{host}#{method}"
    gap = _PACED.get(host)
    if not gap:
        return
    with _pace_lock:
        now = time.monotonic()
        slot = max(now, _pace_next.get(host, 0.0))
        _pace_next[host] = slot + gap
    if slot > now:
        time.sleep(slot - now)


def _request(method, url, **options):
    body = options.get("json_body")
    _pace(url, body.get("method") if isinstance(body, dict) and isinstance(body.get("method"), str) else None)
    return common.request(method, url, **options)


def _get(url, **options):
    return _request("GET", url, **options)


def _post(url, body=None, **options):
    return _request("POST", url, json_body=body, **options)


def _refusal(reply):
    """Why a reply is no answer at all, or None."""
    if reply.error:
        return reply.brief()
    text = reply.text.lower()
    if reply.status in (401, 403):
        return f"credentials demanded or refused ({reply.brief()})"
    if reply.status == 429:
        return ("rate limited, key demanded" if "key" in text else "rate limited") + f" ({reply.brief()})"
    return None


def body_of(reply, what):
    """The JSON of a 2xx reply, or Bad naming what came back."""
    refused = _refusal(reply)
    if refused:
        raise Bad(f"{what}: {refused}")
    if not 200 <= (reply.status or 0) < 300:
        raise Bad(f"{what}: {reply.brief()}")
    if reply.json is None:
        raise Bad(f"{what}: not JSON ({reply.brief()})")
    return reply.json


def jget(url, what=None):
    return body_of(_get(url), what or urllib.parse.urlsplit(url).path)


def jpost(url, body, what=None):
    return body_of(_post(url, body), what or urllib.parse.urlsplit(url).path)


def jsonrpc(url, method, params, *, ident=1):
    """A JSON-RPC 2.0 call's result; Bad on an error object."""
    reply = _post(url, {"jsonrpc": "2.0", "id": ident, "method": method, "params": params})
    refused = _refusal(reply)
    if refused:
        raise Bad(f"{method}: {refused}")
    body = reply.json
    if isinstance(body, dict) and body.get("error") is not None:
        raise Bad(f"{method}: error {_short(json.dumps(body['error']), 240)}")
    if not 200 <= (reply.status or 0) < 300 or not isinstance(body, dict) or "result" not in body:
        raise Bad(f"{method}: {reply.brief()}")
    return body["result"]


def graphql(url, document, variables, *, operation=True):
    """The `data` of a GraphQL answer; Bad on `errors`."""
    request_body = {"query": document, "variables": variables}
    name = document.split()[1].split("(")[0].split("{")[0] if document.startswith(("query ", "mutation ")) else ""
    if operation and name:
        request_body["operationName"] = name
    reply = _post(url, request_body)
    refused = _refusal(reply)
    what = name or "query"
    if refused:
        raise Bad(f"{what}: {refused}")
    body = reply.json
    if isinstance(body, dict) and body.get("errors"):
        messages = [error.get("message", "") for error in body["errors"] if isinstance(error, dict)]
        raise Bad(f"{what}: errors {_short('; '.join(messages) or body['errors'], 240)}")
    if not 200 <= (reply.status or 0) < 300 or not isinstance(body, dict) or not isinstance(body.get("data"), dict):
        raise Bad(f"{what}: {reply.brief()}")
    return body["data"]


def run(row, checks, sample, extra=()):
    """Each claimed capability's check, plus `extra` unclaimed ones."""
    results = {}
    for capability in list(row["capabilities"]) + [c for c in extra if c not in row["capabilities"]]:
        check = checks.get(capability)
        if check is None:
            results[capability] = fail("probe has no check for this capability")
            continue
        try:
            answer = check()
        except Bad as error:
            answer = fail(_short(error, 400))
        except Exception as error:  # noqa: BLE001 — a malformed answer the check did not foresee
            answer = fail(_short(f"unexpected answer shape ({type(error).__name__}: {error})", 400))
        results[capability] = ok(_short(answer, 400)) if isinstance(answer, str) else answer
    return {"capabilities": results, "sample": sample}


_samples = {}
_sample_guard = threading.Lock()
_sample_locks = {}


def chain_sample(chain, derive):
    """`derive(chain)` once per chain; a failure is cached as `{"error": …}`."""
    with _sample_guard:
        lock = _sample_locks.setdefault(chain, threading.Lock())
    with lock:
        if chain not in _samples:
            try:
                _samples[chain] = derive(chain)
            except Bad as error:
                _samples[chain] = {"error": _short(error, 600)}
            except Exception as error:  # noqa: BLE001
                _samples[chain] = {"error": _short(f"{type(error).__name__}: {error}", 600)}
        return _samples[chain]


def each_endpoint(api, chain, derive_at, avoid=()):
    """`derive_at(url)` on each catalog endpoint of `api` on `chain` until one
    yields a sample; hosts in `avoid` last, so that sampling does not spend
    the budget of an endpoint the record is about."""
    errors = []
    rows = sorted(common.endpoints(api, chain), key=lambda row: urllib.parse.urlsplit(row["endpoint"]).hostname in avoid)
    for row in rows:
        try:
            found = derive_at(row["endpoint"])
            found["derived_via"] = row["id"]
            return found
        except Bad as error:
            errors.append(f"{row['id']}: {_short(error, 200)}")
        except Exception as error:  # noqa: BLE001
            errors.append(f"{row['id']}: {type(error).__name__}: {_short(error, 200)}")
    raise Bad("no endpoint yielded a sample: " + "; ".join(errors))


def sample_for(chain, derive, *keys):
    """The chain's sample, or Bad when it lacks one of `keys`."""
    found = chain_sample(chain, derive)
    if "error" in found:
        raise Bad(f"no sample: {found['error']}")
    for key in keys:
        if not found.get(key):
            raise Bad(f"sample has no {key}" + (f" ({found['why_' + key]})" if found.get("why_" + key) else ""))
    return found


def preferred_token(chain):
    rows = common.tokens(chain)
    for wanted in ("tether", "usd-coin", "usd-coin-testnet"):
        for row in rows:
            if row["token_id"] == wanted:
                return row
    return rows[0] if rows else None


def judge_broadcast(reply, what, decoded=("deserial", "decode", "invalid", "malformed", "parse", "unexpected end")):
    """A route that refuses the payload as undecodable passes; one that is
    not served, demands credentials or refuses it for another reason fails."""
    refused = _refusal(reply)
    if refused:
        raise Bad(f"{what}: {refused}")
    if reply.status in (404, 405, 410, 501):
        raise Bad(f"{what}: route not served ({reply.brief()})")
    text = reply.text.lower()
    for word in ("method not found", "method_not_found", "not supported", "unsupported", "disabled",
                 "not allowed", "forbidden", "api key", "apikey", "deprecated"):
        if word in text:
            raise Bad(f"{what}: refused for a reason other than decoding ({reply.brief()})")
    need(any(word in text for word in decoded), f"{what}: answer does not read as a decoding refusal ({reply.brief()})")
    return f"refused as undecodable: {reply.brief(220)}"


# ── Encodings ──────────────────────────────────────────────────────────────

_B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def b58decode(text):
    number = 0
    for char in text:
        number = number * 58 + _B58.index(char)
    body = number.to_bytes((number.bit_length() + 7) // 8, "big") if number else b""
    return b"\0" * (len(text) - len(text.lstrip("1"))) + body


def b58encode(data):
    number = int.from_bytes(data, "big")
    out = ""
    while number:
        number, rest = divmod(number, 58)
        out = _B58[rest] + out
    return "1" * (len(data) - len(data.lstrip(b"\0"))) + out


def tron_base58(hex41):
    raw = bytes.fromhex(hex41)
    check = hashlib.sha256(hashlib.sha256(raw).digest()).digest()[:4]
    return b58encode(raw + check)


def tron_hex(address):
    return b58decode(address)[:-4].hex()


def _crc16(data):
    crc = 0
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) if crc & 0x8000 else crc << 1
            crc &= 0xFFFF
    return crc


def ton_friendly(raw, testnet):
    """A raw `wc:hex` address in its non-bounceable user-friendly form."""
    workchain, account = raw.split(":")
    tag = 0x51 | (0x80 if testnet else 0)
    data = bytes([tag, int(workchain) & 0xFF]) + bytes.fromhex(account)
    return base64.urlsafe_b64encode(data + _crc16(data).to_bytes(2, "big")).decode()


def ton_raw(raw):
    workchain, account = raw.split(":")
    return f"{int(workchain)}:{account.lower()}"


# ── Solana (solana_json_rpc.rs) ────────────────────────────────────────────

STAKE_PROGRAM = "Stake11111111111111111111111111111111111111"
TOKEN_PROGRAMS = ["TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"]


def _solana_sample(chain):
    token = preferred_token(chain)

    def at(url):
        need(token, "the catalog lists no token")
        mint = token["contract"]
        rows = jsonrpc(url, "getSignaturesForAddress", [mint, {"limit": 25}])
        need(isinstance(rows, list) and rows, f"getSignaturesForAddress({mint}) listed nothing")
        found, v1, tried = None, None, 0
        for row in rows:
            if row.get("err") is not None or tried >= (16 if found else 20) or (found and v1):
                continue
            tried += 1
            tx = jsonrpc(url, "getTransaction", [row["signature"], {"encoding": "json", "maxSupportedTransactionVersion": 1}])
            if not tx:
                continue
            if tx.get("version") == 1:
                v1 = v1 or row["signature"]
                continue
            message = tx["transaction"]["message"]
            signers = message["accountKeys"][:message["header"]["numRequiredSignatures"]]
            for balance in tx["meta"].get("postTokenBalances") or []:
                if not found and balance.get("owner") in signers and balance.get("mint") == mint and int(balance["uiTokenAmount"]["amount"]) > 0:
                    found = {"address": balance["owner"], "token": mint, "token_id": token["token_id"],
                             "token_holder": balance["owner"], "tx": row["signature"]}
        need(found, f"none of {tried} recent {token['token_id']} transactions was signed by an account left holding it")
        if v1:
            found["v1_tx"] = v1
        if not common.is_testnet(chain):
            found.update(_solana_staker(url))
        return found

    return each_endpoint("solana-json-rpc", chain, at)


def _solana_staker(url):
    """A stake authority delegating to a mid-sized validator that holds few
    stake accounts, so the staking read stays small."""
    votes = jsonrpc(url, "getVoteAccounts", [{"commitment": "confirmed"}])["current"]
    ranked = sorted(votes, key=lambda vote: vote["activatedStake"])
    for vote in ranked[len(ranked) // 3:len(ranked) // 3 + 3]:
        # The delegation's voter sits at byte 124 of a stake account, the
        # staker at 12.
        rows = jsonrpc(url, "getProgramAccounts", [STAKE_PROGRAM, {
            "encoding": "base64", "commitment": "confirmed", "dataSlice": {"offset": 12, "length": 32},
            "filters": [{"dataSize": 200}, {"memcmp": {"offset": 124, "bytes": vote["votePubkey"]}}]}])
        for row in rows[:6]:
            staker = b58encode(base64.b64decode(row["account"]["data"][0]))
            owned = jsonrpc(url, "getProgramAccounts", [STAKE_PROGRAM, {
                "encoding": "base64", "commitment": "confirmed", "dataSlice": {"offset": 0, "length": 0},
                "filters": [{"dataSize": 200}, {"memcmp": {"offset": 12, "bytes": staker}}]}])
            if 1 <= len(owned) <= 10:
                return {"staker": staker, "staker_vote_account": vote["votePubkey"]}
    return {"why_staker": "no delegator of three mid-sized validators holds between one and ten stake accounts"}


def _solana_parse_stake(account):
    need(account.get("owner") == STAKE_PROGRAM, "stake account: owner is not the Stake program")
    parsed = account["data"]["parsed"]
    need(parsed.get("type") in ("initialized", "delegated"), f"stake account type {parsed.get('type')!r}")
    meta = parsed["info"]["meta"]
    unit = lambda v: digits(v) or is_int(v)  # noqa: E731
    need(unit(account["lamports"]) and unit(meta["rentExemptReserve"]), "stake account: lamports or rent reserve")
    need(isinstance(meta["authorized"]["staker"], str) and isinstance(meta["authorized"]["withdrawer"], str), "stake account: authorities")
    need(unit(meta["lockup"]["epoch"]) and is_int(meta["lockup"]["unixTimestamp"]), "stake account: lockup")
    if parsed["type"] == "delegated":
        delegation = parsed["info"]["stake"]["delegation"]
        need(isinstance(delegation["voter"], str), "stake delegation: voter")
        need(all(unit(delegation[k]) for k in ("stake", "activationEpoch", "deactivationEpoch")), "stake delegation: amounts or epochs")
    return parsed["type"]


def probe_solana(row):
    url, chain = row["endpoint"], row["chain_id"]
    get_sample = lambda *keys: sample_for(chain, _solana_sample, *keys)  # noqa: E731

    def balance():
        s = get_sample("address")
        value = jsonrpc(url, "getBalance", [s["address"], {"commitment": "confirmed"}]).get("value")
        need(is_int(value), f"getBalance value {value!r}")
        return f"getBalance: {value} lamports"

    def core_transaction(signature):
        """getTransaction exactly as core sends it; None when the node refuses
        it for its version."""
        reply = _post(url, {"jsonrpc": "2.0", "id": 1, "method": "getTransaction",
                            "params": [signature, {"encoding": "json", "commitment": "confirmed", "maxSupportedTransactionVersion": 1}]})
        error = reply.json.get("error") if isinstance(reply.json, dict) else None
        if isinstance(error, dict) and error.get("code") == -32015:
            return None
        if error is not None or not isinstance(reply.json, dict) or "result" not in reply.json:
            raise Bad(f"getTransaction: {reply.brief()}")
        return reply.json["result"]

    def version_refusals(signatures):
        """Core's getTransaction asks for versions up to 1; a node refuses a
        newer transaction, and core then refuses the whole page."""
        refused = [sig for sig in signatures if core_transaction(sig) is None]
        if refused:
            raise Bad(f"core's request refused: getTransaction(maxSupportedTransactionVersion 1) answers -32015 "
                      f"for {refused[0]}{f' and {len(refused) - 1} more' if len(refused) > 1 else ''}; "
                      f"a history page holding one fails whole")

    def transactions(address):
        rows = jsonrpc(url, "getSignaturesForAddress", [address, {"limit": 5, "commitment": "confirmed"}])
        need(isinstance(rows, list) and rows, "getSignaturesForAddress listed nothing for an active address")
        need(all(isinstance(r.get("signature"), str) and r["signature"] for r in rows), "getSignaturesForAddress: a row without a signature")
        out, refused = [], []
        for listed in rows:
            tx = core_transaction(listed["signature"])
            if tx is None:
                refused.append(listed["signature"])
                continue
            need(tx is not None, f"getTransaction null for listed {listed['signature']}")
            out.append((listed["signature"], tx))
        return out, refused

    def history():
        s = get_sample("address")
        txs, refused = transactions(s["address"])
        for signature, tx in txs:
            meta = tx["meta"]
            need(isinstance(tx["transaction"]["message"]["accountKeys"], list), f"{signature}: accountKeys")
            need(isinstance(meta["preBalances"], list) and isinstance(meta["postBalances"], list), f"{signature}: pre/post balances")
            need(is_int(meta["fee"]) and is_int(tx["slot"]), f"{signature}: fee or slot")
            need(is_int(tx.get("blockTime")), f"{signature}: blockTime {tx.get('blockTime')!r}")
        version_refusals(refused + ([s["v1_tx"]] if s.get("v1_tx") else []))
        return f"getSignaturesForAddress 5 rows; getTransaction answered each with balances, fee and blockTime"

    def token_history():
        s = get_sample("token_holder")
        txs, refused = transactions(s["token_holder"])
        token_tx = core_transaction(s["tx"])
        need(token_tx is not None, f"getTransaction refused the sample token transfer {s['tx']}")
        rows = 0
        for _, tx in txs + [(s["tx"], token_tx)]:
            for pointer in ("preTokenBalances", "postTokenBalances"):
                for entry in tx["meta"].get(pointer) or []:
                    if entry.get("owner") != s["token_holder"]:
                        continue
                    need(isinstance(entry.get("mint"), str) and is_int(entry.get("accountIndex")), "token balance row: mint or accountIndex")
                    need(digits(entry["uiTokenAmount"]["amount"]) and is_int(entry["uiTokenAmount"]["decimals"]), "token balance row: uiTokenAmount")
                    rows += 1
        need(rows, "no pre/postTokenBalances row owned by the holder")
        version_refusals(refused + ([s["v1_tx"]] if s.get("v1_tx") else []))
        return f"{rows} token balance rows owned by the holder, with mint, accountIndex and uiTokenAmount"

    def token_accounts(params):
        answer = jsonrpc(url, "getTokenAccountsByOwner", params)
        need(isinstance(answer.get("value"), list), "getTokenAccountsByOwner: no account list")
        held = {}
        for account in answer["value"]:
            info = account["account"]["data"]["parsed"]["info"]
            amount = info["tokenAmount"]
            need(isinstance(info["mint"], str) and digits(amount["amount"]) and is_int(amount["decimals"]), "token account: mint, amount or decimals")
            need(isinstance(account.get("pubkey"), str) and is_int(account["account"].get("lamports")), "token account: pubkey or lamports")
            held[info["mint"]] = held.get(info["mint"], 0) + int(amount["amount"])
        return held

    def token_discovery():
        s = get_sample("token_holder")
        held = {}
        for program in TOKEN_PROGRAMS:
            held.update(token_accounts([s["token_holder"], {"programId": program}, {"encoding": "jsonParsed", "commitment": "confirmed"}]))
        need(held.get(s["token"], 0) > 0, f"listing lacks the holder's {s['token_id']}")
        return f"both token programs listed: {len(held)} mints, {s['token_id']} among them"

    def token_balance():
        s = get_sample("token_holder")
        held = token_accounts([s["token_holder"], {"mint": s["token"]}, {"encoding": "jsonParsed", "commitment": "confirmed"}])
        need(set(held) <= {s["token"]}, "answered for another mint")
        mint = jsonrpc(url, "getAccountInfo", [s["token"], {"encoding": "jsonParsed", "commitment": "confirmed"}])["value"]
        need(mint and mint.get("owner") in TOKEN_PROGRAMS, "mint account: not owned by a token program")
        parsed = mint["data"]["parsed"]
        need(parsed.get("type") == "mint" and is_int(parsed["info"].get("decimals")) and parsed["info"].get("isInitialized") is True, "mint account: type, decimals or isInitialized")
        return f"{s['token_id']} balance {held.get(s['token'], 0)}; mint decimals {parsed['info']['decimals']}"

    def fee():
        s = get_sample("address")
        blockhash = jsonrpc(url, "getLatestBlockhash", [{"commitment": "confirmed"}])["value"]["blockhash"]
        need(isinstance(blockhash, str) and len(b58decode(blockhash)) == 32, f"getLatestBlockhash: {blockhash!r}")
        # A legacy message: one signer (the sample), no instructions. Not a transaction.
        message = bytes([1, 0, 0, 1]) + b58decode(s["address"]).rjust(32, b"\0") + b58decode(blockhash) + bytes([0])
        value = jsonrpc(url, "getFeeForMessage", [base64.b64encode(message).decode(), {"commitment": "confirmed"}])["value"]
        need(is_int(value), f"getFeeForMessage value {value!r}")
        return f"getLatestBlockhash and getFeeForMessage ({value} lamports for one signature)"

    def broadcast():
        reply = _post(url, {"jsonrpc": "2.0", "id": 1, "method": "sendTransaction",
                            "params": ["AA==", {"encoding": "base64", "preflightCommitment": "confirmed"}]})
        error = (reply.json or {}).get("error") if isinstance(reply.json, dict) else None
        need(not (isinstance(error, dict) and error.get("code") == -32601), f"sendTransaction: method not found ({reply.brief()})")
        return judge_broadcast(reply, "sendTransaction", decoded=("deserial", "decode", "invalid", "failed to", "io error", "parse"))

    def verification():
        s = get_sample("tx")
        result = jsonrpc(url, "getSignatureStatuses", [[s["tx"]], {"searchTransactionHistory": True}])
        rows = result.get("value")
        need(isinstance(rows, list) and len(rows) == 1, "getSignatureStatuses: not one row")
        need(rows[0] is not None, f"getSignatureStatuses: no status for {s['tx']}")
        need(rows[0].get("confirmationStatus") == "finalized", f"confirmationStatus {rows[0].get('confirmationStatus')!r}")
        need("err" in rows[0] and is_int(rows[0].get("slot")), "status row: err or slot")
        return f"getSignatureStatuses: finalized at slot {rows[0]['slot']}"

    def staking():
        s = get_sample("staker")
        epoch = jsonrpc(url, "getEpochInfo", [{"commitment": "confirmed"}]).get("epoch")
        need(is_int(epoch), "getEpochInfo: epoch")
        jsonrpc(url, "getLatestBlockhash", [{"commitment": "confirmed"}])["value"]["blockhash"]
        kinds = {}
        for offset in (12, 44):
            rows = jsonrpc(url, "getProgramAccounts", [STAKE_PROGRAM, {"encoding": "jsonParsed", "commitment": "confirmed",
                                                                       "filters": [{"dataSize": 200}, {"memcmp": {"offset": offset, "bytes": s["staker"]}}]}])
            need(isinstance(rows, list), f"getProgramAccounts(offset {offset}): not a list")
            for found in rows:
                need(isinstance(found.get("pubkey"), str), "stake account: pubkey")
                kinds[found["pubkey"]] = _solana_parse_stake(found["account"])
        need(kinds, f"getProgramAccounts found no stake account for {s['staker']}, a recent stake authority")
        simulated = _post(url, {"jsonrpc": "2.0", "id": 1, "method": "simulateTransaction",
                                "params": ["AA==", {"encoding": "base64", "sigVerify": False, "commitment": "confirmed"}]})
        judge_broadcast(simulated, "simulateTransaction")
        return f"epoch {epoch}; {len(kinds)} stake accounts ({', '.join(sorted(set(kinds.values())))}); simulateTransaction served"

    checks = {"balance": balance, "history": history, "token-history": token_history, "token-discovery": token_discovery,
              "token-balance": token_balance, "fee": fee, "broadcast": broadcast, "verification": verification, "staking": staking}
    return run(row, checks, chain_sample(chain, _solana_sample), extra=["broadcast"])


# ── Sui (sui_graphql.rs) ───────────────────────────────────────────────────

SUI_COIN = "0x2::sui::SUI"
STAKED_SUI = "0x3::staking_pool::StakedSui"
SUI_EPOCH = "query Epoch { epoch { epochId referenceGasPrice } }"
SUI_BALANCE = ("query Balance($owner: SuiAddress!, $coinType: String!) { "
               "address(address: $owner) { balance(coinType: $coinType) { totalBalance coinBalance } } }")
SUI_BALANCES = ("query Balances($owner: SuiAddress!, $after: String) { "
                "address(address: $owner) { balances(first: 50, after: $after) { "
                "pageInfo { hasNextPage endCursor } nodes { coinType { repr } totalBalance } } } }")
SUI_OBJECTS = ("query Objects($owner: SuiAddress!, $type: String!, $after: String) { "
               "address(address: $owner) { objects(first: 50, after: $after, filter: {type: $type}) { "
               "pageInfo { hasNextPage endCursor } "
               "nodes { address version digest owner { __typename ... on AddressOwner { address { address } } } "
               "contents { type { repr } json } } } } }")
SUI_OBJECT = ("query Object($id: SuiAddress!) { object(address: $id) { "
              "address version digest "
              "owner { __typename ... on AddressOwner { address { address } } "
              "... on Shared { initialSharedVersion } } "
              "asMoveObject { contents { type { repr } json } } } }")
SUI_COIN_METADATA = "query CoinMetadata($coinType: String!) { coinMetadata(coinType: $coinType) { decimals } }"
SUI_VALIDATORS = ("query Validators($after: String) { epoch { validatorSet { "
                  "activeValidators(first: 50, after: $after) { "
                  "pageInfo { hasNextPage endCursor } nodes { contents { json } } } } } }")
SUI_HISTORY = ("query History($owner: SuiAddress!, $before: String) { "
               "address(address: $owner) { transactions(last: 25, before: $before, relation: AFFECTED) { "
               "pageInfo { hasPreviousPage startCursor } "
               "nodes { digest sender { address } gasInput { gasSponsor { address } } "
               "effects { timestamp checkpoint { sequenceNumber } "
               "gasEffects { gasSummary { computationCost storageCost storageRebate } } "
               "balanceChanges(first: 50) { pageInfo { hasNextPage endCursor } "
               "nodes { owner { address } coinType { repr } amount } } } } } } }")
SUI_STATUS = ("query Status($digest: String!) { transactionEffects(digest: $digest) { "
              "digest status checkpoint { sequenceNumber } } }")
SUI_SIMULATE = ("query Simulate($transaction: JSON!) { "
                "simulateTransaction(transaction: $transaction) { effects { status executionError { message } "
                "gasEffects { gasSummary { computationCost storageCost storageRebate } } } } }")
SUI_EXECUTE = ("mutation Execute($transaction: Base64!, $signatures: [Base64!]!) { "
               "executeTransaction(transactionDataBcs: $transaction, signatures: $signatures) { "
               "effects { digest status executionError { message } } } }")
SUI_HOLDERS = ("query Holders($type: String!) { objects(first: 12, filter: {type: $type}) { "
               "nodes { owner { __typename ... on AddressOwner { address { address } } } } } }")


def _sui_long(address):
    return "0x" + address.lower().removeprefix("0x").rjust(64, "0")


def _sui_short_type(repr_):
    package, _, rest = repr_.partition("::")
    return "0x" + (package.lower().removeprefix("0x").lstrip("0") or "0") + "::" + rest


def _sui_sample(chain):
    token = preferred_token(chain)

    def holders(url, type_):
        data = graphql(url, SUI_HOLDERS, {"type": type_})
        return [n["owner"]["address"]["address"] for n in data["objects"]["nodes"] if n["owner"]["__typename"] == "AddressOwner"]

    def at(url):
        need(token, "the catalog lists no token")
        coin = token["contract"]
        candidates = holders(url, f"0x2::coin::Coin<{coin}>")
        need(candidates, f"no address-owned {token['token_id']} coin object listed")
        for holder in candidates[:4]:
            nodes = graphql(url, SUI_HISTORY, {"owner": holder, "before": None})["address"]["transactions"]["nodes"]
            moved = [n for n in nodes for c in n["effects"]["balanceChanges"]["nodes"]
                     if _sui_short_type(c["coinType"]["repr"]) == _sui_short_type(coin)]
            if moved:
                found = {"address": holder, "token": coin, "token_id": token["token_id"], "token_holder": holder,
                         "tx": nodes[-1]["digest"]}
                if not common.is_testnet(chain):
                    stakers = holders(url, STAKED_SUI)
                    found["staker"] = stakers[0] if stakers else None
                return found
        raise Bad(f"no listed {token['token_id']} holder moved it in its last 25 transactions")

    return each_endpoint("sui-graphql", chain, at)


def probe_sui(row):
    url, chain = row["endpoint"], row["chain_id"]
    get_sample = lambda *keys: sample_for(chain, _sui_sample, *keys)  # noqa: E731

    def read_balance(owner, coin):
        address = graphql(url, SUI_BALANCE, {"owner": owner, "coinType": coin})["address"]
        need(isinstance(address, dict), "Balance: address missing")
        held = address.get("balance")
        if held is None:
            return 0
        need(digits(held["totalBalance"]) and digits(held["coinBalance"]), "Balance: totalBalance or coinBalance not digits")
        return int(held["totalBalance"])

    def history_page(owner):
        connection = graphql(url, SUI_HISTORY, {"owner": owner, "before": None})["address"]["transactions"]
        need(isinstance(connection["pageInfo"]["hasPreviousPage"], bool), "History: pageInfo.hasPreviousPage")
        nodes = connection["nodes"]
        need(nodes, "History: no transactions for an active address")
        for node in nodes:
            effects = node["effects"]
            need(isinstance(node["digest"], str) and node["digest"], "History: digest")
            need(isinstance(effects["timestamp"], str), "History: effects.timestamp")
            need(is_int(effects["checkpoint"]["sequenceNumber"]), "History: checkpoint.sequenceNumber")
            gas = effects["gasEffects"]["gasSummary"]
            need(all(digits(gas[k]) or is_int(gas[k]) for k in ("computationCost", "storageCost", "storageRebate")), "History: gasSummary")
            need(isinstance(effects["balanceChanges"]["pageInfo"]["hasNextPage"], bool), "History: balanceChanges.pageInfo")
            for change in effects["balanceChanges"]["nodes"]:
                need(isinstance(change["coinType"]["repr"], str) and digits(change["amount"]), "History: balance change coinType or amount")
        return nodes

    def balance():
        s = get_sample("address")
        return f"SUI totalBalance {read_balance(s['address'], SUI_COIN)}"

    def history():
        s = get_sample("address")
        nodes = history_page(s["address"])
        return f"History: {len(nodes)} transactions with effects, gas and balance changes"

    def token_history():
        s = get_sample("token_holder")
        nodes = history_page(s["token_holder"])
        types = {_sui_short_type(c["coinType"]["repr"]) for n in nodes for c in n["effects"]["balanceChanges"]["nodes"]} - {SUI_COIN}
        need(types, "History: no token balance change on the first page")
        coin = sorted(types)[0]
        decimals = graphql(url, SUI_COIN_METADATA, {"coinType": coin})["coinMetadata"]["decimals"]
        need(is_int(decimals), f"CoinMetadata({coin}): decimals {decimals!r}")
        return f"{len(types)} token types moved on the first page; CoinMetadata answered decimals"

    def token_discovery():
        s = get_sample("token_holder")
        connection = graphql(url, SUI_BALANCES, {"owner": s["token_holder"], "after": None})["address"]["balances"]
        need(isinstance(connection["pageInfo"]["hasNextPage"], bool), "Balances: pageInfo")
        held = {}
        for node in connection["nodes"]:
            need(digits(node["totalBalance"]), "Balances: totalBalance")
            held[_sui_short_type(node["coinType"]["repr"])] = int(node["totalBalance"])
        need(held.get(_sui_short_type(s["token"]), 0) > 0, f"Balances lacks the holder's {s['token_id']}")
        return f"Balances: {len(held)} coin types, {s['token_id']} among them"

    def token_balance():
        s = get_sample("token_holder")
        held = read_balance(s["token_holder"], s["token"])
        decimals = graphql(url, SUI_COIN_METADATA, {"coinType": s["token"]})["coinMetadata"]["decimals"]
        need(is_int(decimals), f"CoinMetadata decimals {decimals!r}")
        return f"{s['token_id']} totalBalance {held}; decimals {decimals}"

    def simulate_garbage():
        reply = _post(url, {"query": SUI_SIMULATE, "operationName": "Simulate",
                            "variables": {"transaction": {"bcs": {"value": "AA=="}}}})
        return judge_broadcast(reply, "simulateTransaction",
                               decoded=("deserial", "decode", "invalid", "bcs", "parse", "unexpected end", "eof"))

    def fee():
        epoch = graphql(url, SUI_EPOCH, {})["epoch"]
        need(is_int(epoch["epochId"]) and digits(epoch["referenceGasPrice"]), "Epoch: epochId or referenceGasPrice")
        simulated = simulate_garbage()
        return f"epoch {epoch['epochId']} reference gas price {epoch['referenceGasPrice']}; simulateTransaction {simulated}"

    def broadcast():
        reply = _post(url, {"query": SUI_EXECUTE, "operationName": "Execute",
                            "variables": {"transaction": "AA==", "signatures": ["AA=="]}})
        return judge_broadcast(reply, "executeTransaction",
                               decoded=("deserial", "decode", "invalid", "bcs", "parse", "unexpected end", "eof", "signature"))

    def verification():
        s = get_sample("tx")
        effects = graphql(url, SUI_STATUS, {"digest": s["tx"]})["transactionEffects"]
        need(isinstance(effects, dict) and effects.get("digest") == s["tx"], f"Status: effects for {s['tx']} missing or for another digest")
        need(effects.get("status") in ("SUCCESS", "FAILURE"), f"Status: status {effects.get('status')!r}")
        need(is_int((effects.get("checkpoint") or {}).get("sequenceNumber")), "Status: checkpoint.sequenceNumber")
        return f"Status: {effects['status']} at checkpoint {effects['checkpoint']['sequenceNumber']}"

    def staking():
        s = get_sample("staker")
        epoch = graphql(url, SUI_EPOCH, {})["epoch"]
        need(is_int(epoch["epochId"]), "Epoch: epochId")
        validators = graphql(url, SUI_VALIDATORS, {"after": None})["epoch"]["validatorSet"]["activeValidators"]
        need(validators["nodes"], "Validators: none")
        for node in validators["nodes"]:
            fields = node["contents"]["json"]
            need(isinstance(fields["metadata"]["sui_address"], str), "validator: metadata.sui_address")
            need(digits(fields["commission_rate"]) and digits(fields["staking_pool"]["sui_balance"]), "validator: commission or pool balance")
            need(isinstance(fields["staking_pool"]["id"], str), "validator: staking_pool.id")
        system = graphql(url, SUI_OBJECT, {"id": "0x5"})["object"]
        need(system["owner"]["__typename"] == "Shared" and system["owner"]["initialSharedVersion"] == 1, "system state: owner")
        need(system["asMoveObject"]["contents"]["type"]["repr"].endswith("::sui_system::SuiSystemState"), "system state: type")
        objects = graphql(url, SUI_OBJECTS, {"owner": s["staker"], "type": STAKED_SUI, "after": None})["address"]["objects"]
        need(objects["nodes"], f"Objects: no StakedSui for {s['staker']}, a listed holder of one")
        for node in objects["nodes"]:
            need(is_int(node["version"]) and len(b58decode(node["digest"])) == 32, "StakedSui: version or digest")
            need(_sui_long(node["owner"]["address"]["address"]) == _sui_long(s["staker"]), "StakedSui: owner")
            fields = node["contents"]["json"]
            need(digits(fields["principal"]) and digits(fields["stake_activation_epoch"]) and isinstance(fields["pool_id"], str), "StakedSui: principal, epoch or pool")
        return (f"epoch {epoch['epochId']}; {len(validators['nodes'])} validators on the first page; "
                f"system state shared at 1; {len(objects['nodes'])} StakedSui")

    checks = {"balance": balance, "history": history, "token-history": token_history, "token-discovery": token_discovery,
              "token-balance": token_balance, "fee": fee, "broadcast": broadcast, "verification": verification, "staking": staking}
    return run(row, checks, chain_sample(chain, _sui_sample))


# ── NEAR (near_json_rpc.rs, nearblocks.rs, fastnear.rs) ───────────────────

NEAR_WHITELIST = "lockup-whitelist.near"
_NEAR_EMPTY_ROOT = "11111111111111111111111111111111"


def _near_rpc(url, method, params):
    result = jsonrpc(url, method, params, ident="1")
    if isinstance(result, dict) and result.get("error"):
        raise Bad(f"{method}: {_short(result['error'], 200)}")
    return result


def _near_view(url, contract, method, args):
    result = _near_rpc(url, "query", {"request_type": "call_function", "finality": "final", "account_id": contract,
                                      "method_name": method,
                                      "args_base64": base64.b64encode(json.dumps(args).encode()).decode()})
    raw = result.get("result")
    need(isinstance(raw, list) and all(is_int(b) and 0 <= b < 256 for b in raw), f"{method}: result bytes missing")
    return json.loads(bytes(raw))


def _near_sample(chain):
    tokens = {row["contract"]: row for row in common.tokens(chain)}

    def at(url):
        height = _near_rpc(url, "block", {"finality": "final"})["header"]["height"]
        found = {}
        for back in range(20, 32):
            try:
                block = _near_rpc(url, "block", {"block_id": height - back})
            except Bad:
                continue  # a skipped height
            for chunk_header in block["chunks"]:
                if chunk_header.get("tx_root") == _NEAR_EMPTY_ROOT:
                    continue
                for tx in _near_rpc(url, "chunk", {"chunk_id": chunk_header["chunk_hash"]})["transactions"]:
                    found.setdefault("tx", tx["hash"])
                    found.setdefault("tx_sender", tx["signer_id"])
                    found.setdefault("address", tx["signer_id"])
                    calls = [a.get("FunctionCall", {}).get("method_name") for a in tx["actions"] if isinstance(a, dict)]
                    if tx["receiver_id"] in tokens and "ft_transfer" in calls and "token_holder" not in found:
                        found.update(token_holder=tx["signer_id"], token=tx["receiver_id"],
                                     token_id=tokens[tx["receiver_id"]]["token_id"], token_tx=tx["hash"])
            if "tx" in found and ("token_holder" in found or back >= 25):
                break
        need("tx" in found, "no transaction in twelve recent blocks")
        if "token_holder" not in found:
            _near_token_holder_from_fastnear(chain, tokens, found)
        if not common.is_testnet(chain):
            found.update(_near_staker(url))
        return found

    return each_endpoint("near-json-rpc", chain, at, avoid=("rpc.mainnet.near.org", "rpc.testnet.near.org"))


def _near_token_holder_from_fastnear(chain, tokens, found):
    token = preferred_token(chain)
    # The catalog's FastNEAR row on mainnet; FastNEAR's testnet API, which the
    # catalog does not list, only as a sample source.
    rows = common.endpoints("fastnear", chain)
    base = rows[0]["endpoint"] if rows else {"near-testnet": "https://test.api.fastnear.com"}.get(chain)
    if not token or not base:
        found["why_token_holder"] = "no catalog token moved in the scanned blocks"
        return
    try:
        top = jget(f"{base.rstrip('/')}/v1/ft/{token['contract']}/top", "fastnear ft top")["accounts"]
        holder = next(a["account_id"] for a in top[1:] if int(a["balance"]) > 0)
        found.update(token_holder=holder, token=token["contract"], token_id=token["token_id"],
                     token_holder_via=f"FastNEAR {base}/v1/ft/{{token}}/top")
    except (Bad, StopIteration, KeyError, ValueError, TypeError) as error:
        found["why_token_holder"] = f"no catalog token moved in the scanned blocks; FastNEAR top holders: {_short(error, 120)}"


def _near_staker(url):
    validators = _near_rpc(url, "validators", [None])["current_validators"]
    ranked = sorted(validators, key=lambda v: int(v["stake"]), reverse=True)
    for validator in ranked[4:14]:
        try:
            accounts = _near_view(url, validator["account_id"], "get_accounts", {"from_index": 0, "limit": 25})
        except Bad:
            continue
        for account in accounts:
            if int(account.get("staked_balance", "0")) > 10 ** 24 and re.fullmatch(r"[a-z0-9._-]{2,64}", account["account_id"]):
                return {"staker": account["account_id"], "staking_pool": validator["account_id"]}
    return {"why_staker": "no delegator found in ten validators' first get_accounts page"}


def probe_near(row):
    url, chain = row["endpoint"], row["chain_id"]
    get_sample = lambda *keys: sample_for(chain, _near_sample, *keys)  # noqa: E731

    def balance():
        s = get_sample("address")
        account = _near_rpc(url, "query", {"request_type": "view_account", "finality": "final", "account_id": s["address"]})
        need(digits(account.get("amount")), f"view_account: amount {account.get('amount')!r}")
        need(digits(account.get("locked")) and is_int(account.get("storage_usage")), "view_account: locked or storage_usage")
        return f"view_account amount {account['amount']} yocto"

    def fee():
        price = _near_rpc(url, "gas_price", [None]).get("gas_price")
        need(digits(price) and int(price) > 0, f"gas_price {price!r}")
        config = _near_rpc(url, "EXPERIMENTAL_protocol_config", {"finality": "final"})
        runtime = config["runtime_config"]
        need(digits(runtime.get("min_gas_purchase_price")) or is_int(runtime.get("min_gas_purchase_price")),
             f"EXPERIMENTAL_protocol_config: runtime_config.min_gas_purchase_price {runtime.get('min_gas_purchase_price')!r}")
        costs = runtime["transaction_costs"]
        parts = [costs["action_receipt_creation_config"], costs["action_creation_config"]["transfer_cost"],
                 costs["action_creation_config"]["create_account_cost"], costs["action_creation_config"]["add_key_cost"]["full_access_cost"]]
        for part in parts:
            need(all(digits(part[k]) or is_int(part[k]) for k in ("send_sir", "send_not_sir", "execution")), "transaction_costs: send/execution fee")
        return f"gas_price {price}; protocol config carries transfer costs and min_gas_purchase_price"

    def broadcast():
        reply = _post(url, {"jsonrpc": "2.0", "id": "1", "method": "broadcast_tx_commit", "params": ["AA=="]})
        return judge_broadcast(reply, "broadcast_tx_commit",
                               decoded=("parse", "deserial", "decode", "invalid", "failed", "unexpected end"))

    def token_balance():
        s = get_sample("token_holder")
        raw = _near_view(url, s["token"], "ft_balance_of", {"account_id": s["token_holder"]})
        need(digits(raw), f"ft_balance_of: {raw!r}")
        meta = _near_view(url, s["token"], "ft_metadata", {})
        need(all(isinstance(meta.get(k), str) for k in ("spec", "name", "symbol")) and is_int(meta.get("decimals")), "ft_metadata: spec, name, symbol or decimals")
        return f"{s['token_id']} ft_balance_of {raw}; ft_metadata decimals {meta['decimals']}"

    def verification():
        s = get_sample("tx", "tx_sender")
        answer = _near_rpc(url, "tx", {"tx_hash": s["tx"], "sender_account_id": s["tx_sender"], "wait_until": "FINAL"})
        need(answer["transaction"]["hash"] == s["tx"] and answer["transaction"]["signer_id"] == s["tx_sender"], "tx: identity mismatch")
        need(answer.get("final_execution_status") == "FINAL", f"tx: final_execution_status {answer.get('final_execution_status')!r}")
        need(isinstance(answer.get("status"), dict) and ("SuccessValue" in answer["status"] or "Failure" in answer["status"]), "tx: status")
        return f"tx FINAL, {'SuccessValue' if 'SuccessValue' in answer['status'] else 'Failure'}"

    def staking():
        s = get_sample("staker", "staking_pool")
        validators = _near_rpc(url, "validators", [None])["current_validators"]
        need(validators and all(isinstance(v["account_id"], str) and digits(v["stake"]) and isinstance(v["is_slashed"], bool)
                                and is_int(v["num_produced_blocks"]) and is_int(v["num_expected_blocks"]) for v in validators),
             "validators: current_validators fields")
        pool, owner = s["staking_pool"], s["staker"]
        approved = _near_view(url, NEAR_WHITELIST, "is_whitelisted", {"staking_pool_account_id": pool})
        need(isinstance(approved, bool), "is_whitelisted: not a bool")
        need(isinstance(_near_view(url, pool, "get_owner_id", {}), str), "get_owner_id")
        account = _near_view(url, pool, "get_account", {"account_id": owner})
        need(account.get("account_id") == owner and digits(account.get("staked_balance")) and digits(account.get("unstaked_balance"))
             and isinstance(account.get("can_withdraw"), bool), "get_account fields")
        fraction = _near_view(url, pool, "get_reward_fee_fraction", {})
        need(is_int(fraction.get("numerator")) and is_int(fraction.get("denominator")) and fraction["denominator"] > 0, "get_reward_fee_fraction")
        need(isinstance(_near_view(url, pool, "is_staking_paused", {}), bool), "is_staking_paused")
        return f"{len(validators)} validators; pool {pool} (whitelisted {approved}) answered get_account {account['staked_balance']} staked"

    checks = {"balance": balance, "fee": fee, "broadcast": broadcast, "token-balance": token_balance,
              "verification": verification, "staking": staking}
    return run(row, checks, chain_sample(chain, _near_sample))


def _nearblocks_next(response, what):
    if "meta" not in response and response.get("data") == []:
        return None
    if "data" not in response and "message" in response:
        raise Bad(f"{what}: refused: {str(response['message'])[:160]}")
    meta = response.get("meta")
    need(isinstance(meta, dict), f"{what}: a nonempty page without `meta`; core refuses it (\"missing pagination metadata\")")
    nxt = meta.get("next_page")
    need(nxt is None or (isinstance(nxt, str) and nxt), f"{what}: invalid next_page {nxt!r}")
    return nxt


def _nearblocks_decimals(meta):
    value = meta.get("decimals") if isinstance(meta, dict) else None
    return is_int(value) or digits(value)


def probe_nearblocks(row):
    base, chain = row["endpoint"].rstrip("/"), row["chain_id"]
    get_sample = lambda *keys: sample_for(chain, _near_sample, *keys)  # noqa: E731

    def history():
        s = get_sample("address")
        page = jget(f"{base}/accounts/{s['address']}/receipts?limit=50", "receipts")
        receipts = page.get("data")
        need(isinstance(receipts, list) and receipts, "receipts: no data for an active account")
        need(len(receipts) <= 50 and all(isinstance(r.get("transaction_hash"), str) and r["transaction_hash"] for r in receipts), "receipts: transaction_hash")
        _nearblocks_next(page, "receipts")
        tx_hash = receipts[0]["transaction_hash"]
        root = jget(f"{base}/txns/{tx_hash}/receipts", "txns receipts").get("data")
        need(isinstance(root, dict), f"txns/{{hash}}/receipts: data is {type(root).__name__}, core reads one receipt tree")
        need(isinstance(root.get("block", {}).get("block_timestamp"), str), "receipt tree: block.block_timestamp")
        pending, seen = [root], 0
        while pending and seen < 200:
            receipt = pending.pop()
            seen += 1
            need(isinstance(receipt.get("receipt_id"), str) and isinstance(receipt.get("receipts"), list), "receipt: receipt_id or receipts")
            need(isinstance(receipt.get("outcome", {}).get("status"), bool), "receipt: outcome.status")
            need(isinstance(receipt.get("predecessor_account_id"), str) and isinstance(receipt.get("receiver_account_id"), str), "receipt: parties")
            need(isinstance(receipt.get("actions"), list), "receipt: actions")
            for action in receipt["actions"]:
                if action.get("action") in ("TRANSFER", "FUNCTION_CALL"):
                    need(digits((action.get("args") or {}).get("deposit")), "receipt action: args.deposit")
            pending.extend(receipt["receipts"])
        return f"receipts: {len(receipts)} rows; receipt tree of {tx_hash} complete ({seen} receipts)"

    def token_history():
        s = get_sample("token_holder")
        page = jget(f"{base}/accounts/{s['token_holder']}/ft-txns?limit=50", "ft-txns")
        rows = page.get("data")
        need(isinstance(rows, list) and rows, "ft-txns: no data for a token holder")
        need(all(isinstance(r.get("transaction_hash"), str) and r["transaction_hash"] for r in rows), "ft-txns: transaction_hash")
        _nearblocks_next(page, "ft-txns")
        tx_hash = rows[0]["transaction_hash"]
        full = jget(f"{base}/txns/{tx_hash}/fts", "txns fts")
        activities = full.get("data")
        need(isinstance(activities, list) and activities, "txns/{hash}/fts: no activities")
        need(full.get("meta", {}).get("next_page") is None, "txns/{hash}/fts: paginated")
        for activity in activities:
            need(isinstance(activity.get("affected_account_id"), str) and isinstance(activity.get("cause"), str), "fts: affected account or cause")
            need(digits(activity.get("delta_amount")) and isinstance(activity.get("contract_account_id"), str), "fts: delta_amount or contract")
            need(isinstance(activity.get("receipt_id"), str) and is_int(activity.get("event_index")) and is_int(activity.get("event_type")), "fts: receipt, event index or type")
            need(digits(activity.get("block_timestamp")) and _nearblocks_decimals(activity.get("meta")), "fts: block_timestamp or meta.decimals")
        return f"ft-txns: {len(rows)} rows; {tx_hash} has {len(activities)} complete activities"

    def token_discovery():
        s = get_sample("token_holder")
        page = jget(f"{base}/accounts/{s['token_holder']}/assets/fts?limit=250", "assets/fts")
        rows = page.get("data")
        need(isinstance(rows, list) and len(rows) <= 250, "assets/fts: no data list")
        for entry in rows:
            need(isinstance(entry.get("contract"), str) and digits(entry.get("amount")) and _nearblocks_decimals(entry.get("meta")), "assets/fts: contract, amount or meta.decimals")
        _nearblocks_next(page, "assets/fts")
        held = {entry["contract"] for entry in rows if int(entry["amount"]) > 0}
        need(s["token"] in held, f"assets/fts lacks the holder's {s['token_id']}")
        return f"assets/fts: {len(rows)} contracts, {s['token_id']} among them"

    checks = {"history": history, "token-history": token_history, "token-discovery": token_discovery}
    return run(row, checks, chain_sample(chain, _near_sample))


def probe_fastnear(row):
    base, chain = row["endpoint"].rstrip("/"), row["chain_id"]

    def staking():
        s = sample_for(chain, _near_sample, "staker", "staking_pool")
        answer = jget(f"{base}/v1/account/{s['staker']}/staking", "account staking")
        need(answer.get("account_id") == s["staker"], f"account_id {answer.get('account_id')!r}")
        pools = answer.get("pools")
        need(isinstance(pools, list) and all(re.fullmatch(r"[a-z0-9._-]{2,64}", p.get("pool_id", "")) for p in pools), "pools: pool_id")
        ids = {p["pool_id"] for p in pools}
        need(s["staking_pool"] in ids, f"pools lacks {s['staking_pool']}, where get_accounts lists the owner")
        return f"{len(ids)} pools, {s['staking_pool']} among them"

    return run(row, {"staking": staking}, chain_sample(chain, _near_sample))


# ── Aptos (aptos_rest.rs, aptos_indexer.rs) ────────────────────────────────

APT_COIN = "0x1::aptos_coin::AptosCoin"
FA_METADATA = "0x1::fungible_asset::Metadata"


def _aptos_pad(address):
    return "0x" + address.lower().removeprefix("0x").rjust(64, "0")


def _aptos_chain_id(chain):
    return 1 if chain == "aptos" else 2


def _aptos_indexer_query(url, query, variables, chain):
    data = graphql(url, query, variables, operation=False)
    chain_id = (data.get("ledger_infos") or [{}])[0].get("chain_id")
    need(chain_id == _aptos_chain_id(chain), f"ledger_infos chain_id {chain_id!r}")
    return data


def _aptos_sample(chain):
    token = preferred_token(chain)

    def rest_at(url):
        rows = jget(f"{url.rstrip('/')}/transactions?limit=50", "transactions")
        users = [t for t in rows if t.get("type") == "user_transaction" and t.get("success") is True]
        need(users, "no successful user transaction among the latest 50")
        return {"tx": users[-1]["hash"], "address": users[-1]["sender"]}

    found = each_endpoint("aptos-rest", chain, rest_at)

    def indexer_at(url):
        found_here = {}
        if token:
            asset = _aptos_pad(token["contract"]) if "::" not in token["contract"] else token["contract"]
            # Unordered: an ordered scan of this table times out and spends the
            # anonymous compute-unit budget.
            rows = _aptos_indexer_query(url, "query Holders($asset:String!){ledger_infos{chain_id} current_fungible_asset_balances(where:{asset_type:{_eq:$asset},amount:{_gt:\"0\"}},limit:10){owner_address amount is_primary}}",
                                        {"asset": asset}, chain)["current_fungible_asset_balances"]
            rows = [r for r in rows if r.get("is_primary")]
            need(rows, f"no primary-store holder of {token['token_id']}")
            found_here.update(token_holder=rows[0]["owner_address"], token=token["contract"], token_id=token["token_id"])
        if not common.is_testnet(chain):
            rows = _aptos_indexer_query(url, "query Delegators{ledger_infos{chain_id} current_delegator_balances(where:{shares:{_gt:\"0\"}},limit:5){delegator_address pool_address pool_type}}",
                                        {}, chain)["current_delegator_balances"]
            rows = [r for r in rows if r.get("pool_type") == "active_shares"]
            if rows:
                found_here.update(staker=rows[0]["delegator_address"], staking_pool=rows[0]["pool_address"])
        return found_here

    try:
        found.update({k: v for k, v in each_endpoint("aptos-indexer", chain, indexer_at).items() if k != "derived_via"})
    except Bad as error:
        found["why_token_holder"] = found["why_staker"] = f"indexer: {_short(error, 200)}"
    return found


def _aptos_view(base, function, type_arguments, arguments):
    values = jpost(f"{base}/view", {"function": function, "type_arguments": type_arguments, "arguments": arguments}, f"view {function}")
    need(isinstance(values, list), f"view {function}: not an array")
    return values


def probe_aptos_rest(row):
    base, chain = row["endpoint"].rstrip("/"), row["chain_id"]
    get_sample = lambda *keys: sample_for(chain, _aptos_sample, *keys)  # noqa: E731

    def balance():
        s = get_sample("address")
        values = _aptos_view(base, "0x1::coin::balance", [APT_COIN], [s["address"]])
        need(values and digits(values[0]), f"coin::balance: {values!r}")
        return f"coin::balance<AptosCoin> {values[0]} octas"

    def fee():
        estimate = jget(f"{base}/estimate_gas_price", "estimate_gas_price").get("gas_estimate")
        need(is_int(estimate) and estimate > 0, f"gas_estimate {estimate!r}")
        return f"gas_estimate {estimate}"

    def broadcast():
        reply = _request("POST", f"{base}/transactions", data=b"\x00",
                         headers={"Content-Type": "application/x.aptos.signed_transaction+bcs"})
        return judge_broadcast(reply, "POST /transactions")

    def token_balance():
        s = get_sample("token_holder")
        contract = s["token"]
        if "::" in contract:
            values = _aptos_view(base, "0x1::coin::balance", [contract], [s["token_holder"]])
            decimals = _aptos_view(base, "0x1::coin::decimals", [contract], [])
        else:
            values = _aptos_view(base, "0x1::primary_fungible_store::balance", [FA_METADATA], [s["token_holder"], contract])
            decimals = _aptos_view(base, "0x1::fungible_asset::decimals", [FA_METADATA], [contract])
        need(values and digits(values[0]), f"balance view: {values!r}")
        need(decimals and is_int(decimals[0]), f"decimals view: {decimals!r}")
        return f"{s['token_id']} balance {values[0]}; decimals {decimals[0]}"

    def verification():
        s = get_sample("tx")
        tx = jget(f"{base}/transactions/by_hash/{s['tx']}", "transactions/by_hash")
        need(str(tx.get("hash", "")).lower() == s["tx"].lower(), "by_hash: another hash")
        need(tx.get("type") == "user_transaction" and digits(tx.get("version")) and isinstance(tx.get("success"), bool), "by_hash: type, version or success")
        return f"user_transaction at version {tx['version']}, success {tx['success']}"

    def staking():
        s = get_sample("staker", "staking_pool")
        pool, owner = s["staking_pool"], s["staker"]
        need(_aptos_view(base, "0x1::delegation_pool::delegation_pool_exists", [], [pool]) == [True], "delegation_pool_exists: not [true]")
        stake = _aptos_view(base, "0x1::delegation_pool::get_stake", [], [pool, owner])
        need(len(stake) == 3 and all(digits(v) for v in stake), f"get_stake: {stake!r}")
        withdrawal = _aptos_view(base, "0x1::delegation_pool::get_pending_withdrawal", [], [pool, owner])
        need(len(withdrawal) == 2 and isinstance(withdrawal[0], bool) and digits(withdrawal[1]), f"get_pending_withdrawal: {withdrawal!r}")
        commission = _aptos_view(base, "0x1::delegation_pool::operator_commission_percentage", [], [pool])
        need(len(commission) == 1 and digits(commission[0]), f"operator_commission_percentage: {commission!r}")
        allowed = _aptos_view(base, "0x1::delegation_pool::delegator_allowlisted", [], [pool, owner])
        need(len(allowed) == 1 and isinstance(allowed[0], bool), f"delegator_allowlisted: {allowed!r}")
        resource = jget(f"{base}/accounts/{pool}/resource/0x1::stake::StakePool", "StakePool")
        need(digits(resource["data"]["locked_until_secs"]), "StakePool: locked_until_secs")
        validators = jget(f"{base}/accounts/0x1/resource/0x1::stake::ValidatorSet", "ValidatorSet")["data"]["active_validators"]
        need(isinstance(validators, list) and validators, "ValidatorSet: active_validators")
        return f"delegation views answered (active {stake[0]}); StakePool and {len(validators)} active validators"

    checks = {"balance": balance, "fee": fee, "broadcast": broadcast, "token-balance": token_balance,
              "verification": verification, "staking": staking}
    return run(row, checks, chain_sample(chain, _aptos_sample))


_APTOS_HISTORY = ("query History($owner:String!){ledger_infos{chain_id} account_transactions(where:{account_address:{_eq:$owner}},"
                  "order_by:{transaction_version:desc},limit:50){transaction_version fungible_asset_activities(where:{owner_address:{_eq:$owner}},"
                  "limit:1001){amount asset_type type is_gas_fee is_transaction_success metadata{decimals}}}}")
_APTOS_HOLDINGS = ("query Holdings($owner:String!,$after:String!){ledger_infos{chain_id} current_fungible_asset_balances("
                   "where:{owner_address:{_eq:$owner},storage_id:{_gt:$after}},order_by:{storage_id:asc},limit:1000)"
                   "{storage_id asset_type amount metadata{decimals}}}")


def _aptos_native(asset):
    return asset == APT_COIN or _aptos_pad(asset) == _aptos_pad("0xa")


def probe_aptos_indexer(row):
    url, chain = row["endpoint"], row["chain_id"]
    get_sample = lambda *keys: sample_for(chain, _aptos_sample, *keys)  # noqa: E731

    def history_rows(owner):
        rows = _aptos_indexer_query(url, _APTOS_HISTORY, {"owner": _aptos_pad(owner)}, chain)["account_transactions"]
        need(isinstance(rows, list) and rows, "account_transactions: none for an active account")
        versions = [int(r["transaction_version"]) for r in rows]
        need(versions == sorted(versions, reverse=True) and len(set(versions)) == len(versions), "account_transactions: versions not descending")
        tokens = 0
        for r in rows:
            for activity in r["fungible_asset_activities"]:
                need(isinstance(activity["is_gas_fee"], bool) and isinstance(activity["type"], str) and isinstance(activity["asset_type"], str), "activity: gas flag, type or asset")
                need(is_int(activity["amount"]) or digits(activity["amount"]) or activity["amount"] is None, "activity: amount")
                if not _aptos_native(activity["asset_type"]) and not activity["is_gas_fee"]:
                    need(is_int((activity.get("metadata") or {}).get("decimals")), f"activity: metadata.decimals for {activity['asset_type']}")
                    tokens += 1
        return rows, tokens

    def history():
        s = get_sample("address")
        rows, _ = history_rows(s["address"])
        detail = f"account_transactions: {len(rows)} versions with asset activities"
        if s.get("staker"):
            # Core lists an owner's delegation pools here, through History.
            pools = _aptos_indexer_query(url, "query Stakes($owner:String!,$after:String!){ledger_infos{chain_id} current_delegator_balances(where:{delegator_address:{_eq:$owner},shares:{_gt:0},pool_address:{_gt:$after}},distinct_on:[pool_address],order_by:{pool_address:asc},limit:1000){pool_address}}",
                                         {"owner": _aptos_pad(s["staker"]), "after": ""}, chain)["current_delegator_balances"]
            need(_aptos_pad(s["staking_pool"]) in {_aptos_pad(p["pool_address"]) for p in pools}, "delegator pools (core's staking discovery) lack the sample pool")
            detail += f"; staking discovery listed {len(pools)} pools"
        return detail

    def token_history():
        s = get_sample("token_holder")
        rows, tokens = history_rows(s["token_holder"])
        need(tokens, "no token activity on the first page")
        return f"account_transactions: {len(rows)} versions, {tokens} token activities with decimals"

    def token_discovery():
        s = get_sample("token_holder")
        rows = _aptos_indexer_query(url, _APTOS_HOLDINGS, {"owner": _aptos_pad(s["token_holder"]), "after": ""}, chain)["current_fungible_asset_balances"]
        need(isinstance(rows, list), "current_fungible_asset_balances: not a list")
        held = set()
        for r in rows:
            need(isinstance(r["storage_id"], str) and isinstance(r["asset_type"], str), "holding: storage_id or asset_type")
            need(is_int(r["amount"]) or digits(r["amount"]), "holding: amount")
            if not _aptos_native(r["asset_type"]):
                need(is_int((r.get("metadata") or {}).get("decimals")), f"holding: metadata.decimals for {r['asset_type']}")
            if int(r["amount"]) > 0:
                held.add(_aptos_pad(r["asset_type"]) if "::" not in r["asset_type"] else r["asset_type"])
        wanted = _aptos_pad(s["token"]) if "::" not in s["token"] else s["token"]
        need(wanted in held, f"holdings lack the holder's {s['token_id']}")
        return f"current_fungible_asset_balances: {len(rows)} stores, {s['token_id']} among them"

    checks = {"history": history, "token-history": token_history, "token-discovery": token_discovery}
    return run(row, checks, chain_sample(chain, _aptos_sample))


# ── XRP Ledger (xrpl_json_rpc.rs) ──────────────────────────────────────────

def _xrpl(url, method, params):
    reply = _post(url, {"method": method, "params": [params]})
    refused = _refusal(reply)
    if refused:
        raise Bad(f"{method}: {refused}")
    body = reply.json
    need(isinstance(body, dict) and isinstance(body.get("result"), dict), f"{method}: {reply.brief()}")
    result = body["result"]
    if result.get("status") == "error" or body.get("error"):
        raise Bad(f"{method}: {result.get('error')} {result.get('error_message', '')}".strip())
    return result


def _xrpl_sample(chain):
    token = preferred_token(chain)

    def at(url):
        if token:
            currency, issuer = token["contract"].split(".")
            lines = _xrpl(url, "account_lines", {"account": issuer, "ledger_index": "validated", "limit": 20})["lines"]
            holders = [line["account"] for line in lines if line["balance"].startswith("-")]
            need(holders, f"no holder among {token['token_id']} issuer's first lines")
            holder = holders[0]
            txs = _xrpl(url, "account_tx", {"account": holder, "limit": 5, "ledger_index_min": -1, "ledger_index_max": -1, "forward": False})["transactions"]
            need(txs, "holder has no transactions")
            entry = txs[0]
            tx = entry.get("tx") or entry.get("tx_json") or {}
            return {"address": holder, "token_holder": holder, "token": token["contract"], "token_id": token["token_id"],
                    "tx": tx.get("hash") or entry.get("hash")}
        # No catalog token: a recent payment's sender, and a trust line with
        # a balance from the validated ledger's state.
        index = _xrpl(url, "ledger", {"ledger_index": "validated"})["ledger_index"]
        active = None
        for back in range(0, 20):
            ledger = _xrpl(url, "ledger", {"ledger_index": int(index) - back, "transactions": True, "expand": True})["ledger"]
            for tx in ledger.get("transactions") or []:
                meta = tx.get("metaData") or tx.get("meta") or {}
                if tx.get("TransactionType") == "Payment" and meta.get("TransactionResult") == "tesSUCCESS":
                    active = {"address": tx["Account"], "tx": tx["hash"]}
                    break
            if active:
                break
        need(active, "no successful payment in twenty validated ledgers")
        marker = None
        for _ in range(10):
            params = {"ledger_index": "validated", "type": "state", "limit": 200}
            if marker:
                params["marker"] = marker
            page = _xrpl(url, "ledger_data", params)
            for entry in page.get("state") or []:
                if entry.get("LedgerEntryType") != "RippleState":
                    continue
                value = float(entry["Balance"]["value"])
                low, high = entry["LowLimit"]["issuer"], entry["HighLimit"]["issuer"]
                if value:
                    holder, issuer = (low, high) if value > 0 else (high, low)
                    currency = entry["Balance"]["currency"]
                    active.update(token_holder=holder, token=f"{currency}.{issuer}", token_id=f"{currency} (derived)")
                    return active
            marker = page.get("marker")
            if not marker:
                break
        active["why_token_holder"] = "no trust line with a balance in the validated ledger's first state pages"
        return active

    return each_endpoint("xrpl-json-rpc", chain, at)


_xrpl_first = {}


def _xrpl_first_ledger(chain):
    """The earliest ledger any of the network's endpoints keeps."""
    with _sample_guard:
        if chain in _xrpl_first:
            return _xrpl_first[chain]
    firsts = []
    for row in common.endpoints("xrpl-json-rpc", chain):
        try:
            ranges = _xrpl(row["endpoint"], "server_info", {})["info"]["complete_ledgers"]
            firsts.append(int(re.split(r"[-,]", ranges)[0]))
        except (Bad, KeyError, ValueError, TypeError):
            continue
    with _sample_guard:
        _xrpl_first[chain] = min(firsts) if firsts else 0
        return _xrpl_first[chain]


def probe_xrpl(row):
    url, chain = row["endpoint"], row["chain_id"]
    get_sample = lambda *keys: sample_for(chain, _xrpl_sample, *keys)  # noqa: E731

    def balance():
        s = get_sample("address")
        value = _xrpl(url, "account_info", {"account": s["address"], "ledger_index": "validated"})["account_data"]["Balance"]
        need(digits(value), f"account_data.Balance {value!r}")
        return f"account_info Balance {value} drops"

    def account_tx(address):
        result = _xrpl(url, "account_tx", {"account": address, "limit": 50, "ledger_index_min": -1, "ledger_index_max": -1, "forward": False})
        low, high = result.get("ledger_index_min"), result.get("ledger_index_max")
        need(is_int(low) and is_int(high), "account_tx: ledger_index_min or ledger_index_max")
        # Core asks from the earliest ledger the node keeps; a node that keeps
        # a recent window only answers an address's history in part.
        first = _xrpl_first_ledger(chain)
        need(low <= first + 1000,
             f"account_tx: the node keeps ledgers {low}-{high} only (the network's others from {first}), so older transactions are missing")
        txs = result.get("transactions")
        need(isinstance(txs, list) and txs, "account_tx: no transactions for an active account")
        shaped = [entry for entry in txs if isinstance(entry.get("tx"), dict)]
        need(shaped, f"account_tx entries carry no `tx` (keys: {sorted(txs[0])}); core reads `tx`")
        for entry in shaped:
            tx = entry["tx"]
            need(isinstance(tx.get("TransactionType"), str) and isinstance(tx.get("hash"), str), "account_tx: TransactionType or hash")
            need(isinstance(entry.get("meta"), dict) and isinstance(entry["meta"].get("AffectedNodes"), list), "account_tx: meta.AffectedNodes")
            if tx["TransactionType"] == "Payment" and entry["meta"].get("TransactionResult") == "tesSUCCESS":
                need(is_int(tx.get("date")), "account_tx: Payment without date")
        return shaped, result

    def history():
        s = get_sample("address")
        shaped, result = account_tx(s["address"])
        return (f"account_tx: {len(shaped)} entries with tx and meta, ledgers {result['ledger_index_min']}-{result['ledger_index_max']}"
                + ("; marker" if result.get("marker") else ""))

    def token_history():
        s = get_sample("token_holder")
        shaped, _ = account_tx(s["token_holder"])
        lines = 0
        for entry in shaped:
            for node in entry["meta"]["AffectedNodes"]:
                (kind, body), = node.items()
                if body.get("LedgerEntryType") != "RippleState":
                    continue
                fields = body.get("NewFields") if kind == "CreatedNode" else body.get("FinalFields")
                if not fields:
                    continue
                if s["token_holder"] in (fields.get("LowLimit", {}).get("issuer"), fields.get("HighLimit", {}).get("issuer")):
                    need(isinstance(fields.get("Balance", {}).get("currency"), str) and isinstance(fields["Balance"].get("value"), str), "RippleState: Balance")
                    lines += 1
        need(lines, "no trust-line change of the holder on the first page")
        return f"{lines} trust-line changes of the holder in account_tx metadata"

    def lines(params):
        result = _xrpl(url, "account_lines", params)
        need(isinstance(result.get("lines"), list), "account_lines: no lines")
        for line in result["lines"]:
            need(all(isinstance(line.get(k), str) for k in ("account", "currency", "balance", "limit", "limit_peer")), "account_lines: line fields")
        return result["lines"]

    def token_discovery():
        s = get_sample("token_holder")
        found = lines({"account": s["token_holder"], "ledger_index": "validated", "limit": 400})
        currency, issuer = s["token"].split(".")
        need(any(l["account"] == issuer and l["currency"] == currency for l in found), f"account_lines lacks {s['token_id']}")
        return f"account_lines: {len(found)} lines, {s['token_id']} among them"

    def token_balance():
        s = get_sample("token_holder")
        currency, issuer = s["token"].split(".")
        found = lines({"account": s["token_holder"], "ledger_index": "validated", "limit": 400, "peer": issuer})
        match = [l for l in found if l["currency"] == currency]
        need(match, f"account_lines with peer lacks {s['token_id']}")
        return f"{s['token_id']} line balance {match[0]['balance']}"

    def fee():
        value = _xrpl(url, "fee", {}).get("drops", {}).get("open_ledger_fee")
        need(digits(value), f"fee: drops.open_ledger_fee {value!r}")
        return f"open_ledger_fee {value} drops"

    def broadcast():
        reply = _post(url, {"method": "submit", "params": [{"tx_blob": "00"}]})
        return judge_broadcast(reply, "submit", decoded=("invalid", "malformed", "deserial", "parse", "bad", "txn"))

    def verification():
        s = get_sample("tx")
        reply = _post(url, {"method": "tx", "params": [{"transaction": s["tx"], "binary": False}]})
        result = (body_of(reply, "tx") or {}).get("result") or {}
        need(result.get("status") != "error", f"tx: {result.get('error')} {result.get('error_message', '')}")
        need(result.get("validated") is True, "tx: not validated")
        tx_hash = result.get("hash") or result.get("tx_json", {}).get("hash")
        ledger = result.get("ledger_index") or result.get("tx_json", {}).get("ledger_index")
        need(isinstance(tx_hash, str) and tx_hash.upper() == s["tx"].upper(), "tx: hash missing or another")
        need(is_int(ledger) and ledger > 0, "tx: ledger_index")
        need(isinstance(result.get("meta", {}).get("TransactionResult"), str), "tx: meta.TransactionResult")
        return f"tx validated in ledger {ledger}: {result['meta']['TransactionResult']}"

    checks = {"balance": balance, "history": history, "token-history": token_history, "token-discovery": token_discovery,
              "token-balance": token_balance, "fee": fee, "broadcast": broadcast, "verification": verification}
    return run(row, checks, chain_sample(chain, _xrpl_sample))


# ── Stellar Horizon (horizon.rs) ───────────────────────────────────────────

def _stellar_amount(text):
    return isinstance(text, str) and bool(re.fullmatch(r"-?\d+(\.\d{1,7})?", text))


def _horizon_sample(chain):
    token = preferred_token(chain)

    def at(url):
        base = url.rstrip("/")
        if token:
            code, issuer = token["contract"].split(":")
            records = jget(f"{base}/accounts?asset={code}:{issuer}&limit=20", "accounts by asset")["_embedded"]["records"]
            for record in records:
                held = [b for b in record["balances"] if b.get("asset_code") == code and b.get("asset_issuer") == issuer]
                if held and float(held[0]["balance"]) > 0:
                    payments = jget(f"{base}/accounts/{record['id']}/payments?limit=10&order=desc", "payments")["_embedded"]["records"]
                    if payments:
                        return {"address": record["id"], "token_holder": record["id"], "token": token["contract"],
                                "token_id": token["token_id"], "tx": payments[0]["transaction_hash"]}
            raise Bad(f"no listed {token['token_id']} holder with payments")
        records = jget(f"{base}/payments?order=desc&limit=200&include_failed=false", "payments")["_embedded"]["records"]
        need(records, "no recent payments")
        tried = 0
        for record in records:
            if record.get("type") != "payment" or not record.get("asset_type", "").startswith("credit_alphanum") \
                    or record["to"] == record.get("asset_issuer") or tried >= 6:
                continue
            tried += 1
            balances = jget(f"{base}/accounts/{record['to']}", "accounts")["balances"]
            if any(b.get("asset_code") == record["asset_code"] and b.get("asset_issuer") == record["asset_issuer"]
                   and float(b["balance"]) > 0 for b in balances):
                return {"address": record["to"], "token_holder": record["to"], "tx": record["transaction_hash"],
                        "token": f"{record['asset_code']}:{record['asset_issuer']}", "token_id": f"{record['asset_code']} (derived)"}
        first = next(r for r in records if r.get("type") in ("payment", "create_account"))
        return {"address": first.get("to") or first.get("account"), "tx": first["transaction_hash"],
                "why_token_holder": "no recipient of a credit-asset payment among the latest 200 still holds it"}

    return each_endpoint("horizon", chain, at)


def probe_horizon(row):
    base, chain = row["endpoint"].rstrip("/"), row["chain_id"]
    get_sample = lambda *keys: sample_for(chain, _horizon_sample, *keys)  # noqa: E731

    def account(address):
        found = jget(f"{base}/accounts/{address}", "accounts")
        need(isinstance(found.get("balances"), list) and isinstance(found.get("sequence"), str), "account: balances or sequence")
        need(is_int(found.get("subentry_count")), "account: subentry_count")
        for line in found["balances"]:
            need(_stellar_amount(line.get("balance")) and isinstance(line.get("asset_type"), str), "account: balance line")
        return found

    def balance():
        s = get_sample("address")
        native = [b for b in account(s["address"])["balances"] if b["asset_type"] == "native"]
        need(native, "account: no native balance")
        return f"native balance {native[0]['balance']}"

    def payments(address):
        page = jget(f"{base}/accounts/{address}/payments?limit=50&order=desc&include_failed=false", "payments")
        records = page["_embedded"]["records"]
        need(records, "payments: none for an active account")
        for r in records:
            need(isinstance(r.get("paging_token"), str) and isinstance(r.get("type"), str), "payment: paging_token or type")
            need(isinstance(r.get("transaction_hash"), str) and isinstance(r.get("created_at"), str), "payment: transaction_hash or created_at")
            if r["type"] == "payment":
                need(isinstance(r.get("from"), str) and isinstance(r.get("to"), str) and _stellar_amount(r.get("amount")), "payment: from, to or amount")
            if r["type"] == "create_account":
                need(isinstance(r.get("funder"), str) and isinstance(r.get("account"), str) and _stellar_amount(r.get("starting_balance")), "create_account fields")
        return records

    def history():
        s = get_sample("address")
        return f"payments: {len(payments(s['address']))} records"

    def token_history():
        s = get_sample("token_holder")
        records = payments(s["token_holder"])
        credit = [r for r in records if r["type"] == "payment" and r.get("asset_type", "").startswith("credit_alphanum")]
        need(credit and all(isinstance(r.get("asset_code"), str) and isinstance(r.get("asset_issuer"), str) for r in credit),
             "no credit-asset payment with asset_code and asset_issuer on the first page")
        return f"payments: {len(credit)} credit-asset payments of {len(records)}"

    def credit_lines(address):
        lines = [b for b in account(address)["balances"] if b["asset_type"] != "native"]
        for line in lines:
            if line["asset_type"].startswith("credit_alphanum"):
                need(isinstance(line.get("asset_code"), str) and isinstance(line.get("asset_issuer"), str), "trustline: code or issuer")
                need(_stellar_amount(line.get("limit")) and isinstance(line.get("is_authorized"), bool), "trustline: limit or is_authorized")
        return lines

    def token_discovery():
        s = get_sample("token_holder")
        code, issuer = s["token"].split(":")
        lines = credit_lines(s["token_holder"])
        need(any(l.get("asset_code") == code and l.get("asset_issuer") == issuer for l in lines), f"balances lack {s['token_id']}")
        return f"{len(lines)} trustlines, {s['token_id']} among them"

    def token_balance():
        s = get_sample("token_holder")
        code, issuer = s["token"].split(":")
        line = [l for l in credit_lines(s["token_holder"]) if l.get("asset_code") == code and l.get("asset_issuer") == issuer]
        need(line, f"balances lack {s['token_id']}")
        ledgers = jget(f"{base}/ledgers?order=desc&limit=1", "ledgers")
        need(is_int(ledgers["_embedded"]["records"][0].get("base_reserve_in_stroops")), "ledgers: base_reserve_in_stroops")
        account(issuer)
        return f"{s['token_id']} balance {line[0]['balance']}; base reserve and issuer read"

    def fee():
        stats = jget(f"{base}/fee_stats", "fee_stats")
        mode = (stats.get("fee_charged") or {}).get("mode")
        need(digits(mode), f"fee_stats: fee_charged.mode {mode!r}")
        return f"fee_charged.mode {mode} stroops"

    def broadcast():
        """`submit_envelope_b64`: `tx`, form-encoded."""
        reply = _request("POST", f"{base}/transactions", data=b"tx=AA%3D%3D",
                         headers={"Content-Type": "application/x-www-form-urlencoded"})
        return judge_broadcast(reply, "POST /transactions", decoded=("malformed", "invalid", "decode", "xdr"))

    def verification():
        s = get_sample("tx")
        tx = jget(f"{base}/transactions/{s['tx']}", "transactions")
        need(str(tx.get("hash", "")).lower() == s["tx"].lower(), "transaction: another hash")
        need(is_int(tx.get("ledger")) and tx["ledger"] > 0 and isinstance(tx.get("successful"), bool), "transaction: ledger or successful")
        return f"transaction in ledger {tx['ledger']}, successful {tx['successful']}"

    checks = {"balance": balance, "history": history, "token-history": token_history, "token-discovery": token_discovery,
              "token-balance": token_balance, "fee": fee, "broadcast": broadcast, "verification": verification}
    return run(row, checks, chain_sample(chain, _horizon_sample))


# ── Tron (tron_http.rs, trongrid_v1.rs) ────────────────────────────────────

def _tron_post(base, path, body):
    answer = jpost(f"{base}{path}", body, path)
    if isinstance(answer, dict) and answer.get("Error"):
        raise Bad(f"{path}: Error {_short(answer['Error'], 200)}")
    return answer


def _tron_sample(chain):
    tokens = {tron_hex(row["contract"]): row for row in common.tokens(chain) if row["standard"] == "TRC-20"}
    preferred = preferred_token(chain)

    def scan(blocks):
        """A transfer of the preferred token, else of another catalog token
        (any token on a network the catalog lists none for), else a TRX
        transfer."""
        candidate = fallback = None
        for block in blocks:
            for tx in block.get("transactions") or []:
                if (tx.get("ret") or [{}])[0].get("contractRet") != "SUCCESS":
                    continue
                contract = tx["raw_data"]["contract"][0]
                value = contract["parameter"]["value"]
                if contract["type"] == "TransferContract":
                    fallback = fallback or {"address": tron_base58(value["owner_address"]), "tx": tx["txID"]}
                if contract["type"] != "TriggerSmartContract" or not str(value.get("data", "")).startswith("a9059cbb"):
                    continue
                target = value["contract_address"]
                if tokens and target not in tokens:
                    continue
                token_id = tokens[target]["token_id"] if target in tokens else f"{tron_base58(target)} (derived)"
                found = {"address": tron_base58(value["owner_address"]), "token_holder": tron_base58(value["owner_address"]),
                         "token": tron_base58(target), "token_id": token_id, "tx": tx["txID"]}
                if not tokens or (preferred and token_id == preferred["token_id"]):
                    return found
                candidate = candidate or found
        return candidate or fallback

    def at(url):
        base = url.rstrip("/")
        head = _tron_post(base, "/wallet/getnowblock", {})["block_header"]["raw_data"]["number"]
        found = scan([_tron_post(base, "/wallet/getblockbynum", {"num": head - 40})])
        if not found or "token_holder" not in found:
            blocks = _tron_post(base, "/wallet/getblockbylimitnext", {"startNum": head - 100, "endNum": head - 40}).get("block") or []
            found = scan(blocks) or found
        need(found, "no successful transfer in sixty solidified blocks")
        if "token_holder" not in found:
            found["why_token_holder"] = "no successful TRC-20 transfer in sixty solidified blocks"
        trc10 = [row for row in common.tokens(chain) if row["standard"] == "TRC-10"]
        if trc10:
            found["trc10"] = trc10[0]["contract"]
        return found

    return each_endpoint("tron-http", chain, at)


def probe_tron_http(row):
    base, chain = row["endpoint"].rstrip("/"), row["chain_id"]
    get_sample = lambda *keys: sample_for(chain, _tron_sample, *keys)  # noqa: E731

    def account(address):
        found = _tron_post(base, "/wallet/getaccount", {"address": address, "visible": True})
        need(isinstance(found, dict), "getaccount: not an object")
        need(found.get("address") == address, f"getaccount: address {found.get('address')!r}")
        need("balance" not in found or is_int(found["balance"]), "getaccount: balance")
        for asset in found.get("assetV2") or []:
            need(isinstance(asset.get("key"), str) and is_int(asset.get("value")), "getaccount: assetV2 row")
        return found

    def balance():
        s = get_sample("address")
        return f"getaccount balance {account(s['address']).get('balance', 0)} sun"

    def fee():
        s = get_sample("address")
        parameters = _tron_post(base, "/wallet/getchainparameters", {}).get("chainParameter")
        need(isinstance(parameters, list), "getchainparameters: chainParameter")
        for name in ("getTransactionFee", "getEnergyFee", "getMultiSignFee"):
            rows = [p for p in parameters if p.get("key") == name]
            need(len(rows) == 1 and ("value" not in rows[0] or is_int(rows[0]["value"])), f"getchainparameters: {name}")
        block = _tron_post(base, "/wallet/getnowblock", {})
        need(is_int(block["block_header"]["raw_data"]["number"]) and len(bytes.fromhex(block["blockID"])) == 32, "getnowblock: number or blockID")
        resources = _tron_post(base, "/wallet/getaccountresource", {"address": s["address"], "visible": True})
        need(isinstance(resources, dict), "getaccountresource: not an object")
        return "getchainparameters fees, getnowblock reference, getaccountresource"

    def broadcast():
        reply = _post(f"{base}/wallet/broadcasthex", {"transaction": "00"})
        if "rejected by admission check" in reply.text and reply.status == 200:
            return f"route served; refused by the node's admission check before decoding: {reply.brief(160)}"
        return judge_broadcast(reply, "broadcasthex", decoded=("protocolbuffer", "invalid tag", "parse", "decode"))

    def constant(contract, owner, selector, parameter):
        answer = _tron_post(base, "/wallet/triggerconstantcontract",
                            {"owner_address": owner, "contract_address": contract, "function_selector": selector,
                             "parameter": parameter, "visible": True})
        result = (answer.get("constant_result") or [None])[0]
        need(isinstance(result, str) and re.fullmatch(r"[0-9a-fA-F]*", result) and len(result) >= 64, f"{selector}: constant_result {result!r}")
        return result

    def token_balance():
        s = get_sample("token_holder")
        holder_hex = tron_hex(s["token_holder"])[2:]
        raw = int(constant(s["token"], s["token_holder"], "balanceOf(address)", holder_hex.rjust(64, "0"))[:64], 16)
        decimals = int(constant(s["token"], s["token"], "decimals()", "")[:64], 16)
        constant(s["token"], s["token"], "symbol()", "")
        detail = f"{s['token_id']} balanceOf {raw}, decimals {decimals}, symbol"
        if s.get("trc10"):
            meta = _tron_post(base, "/wallet/getassetissuebyid", {"value": s["trc10"]})
            need(meta.get("id") == s["trc10"] and isinstance(meta.get("name"), str), "getassetissuebyid: id or name")
            need("precision" not in meta or is_int(meta["precision"]), "getassetissuebyid: precision")
            account(s["token_holder"])
            detail += f"; TRC-10 {s['trc10']} metadata and getaccount assetV2"
        return detail

    def verification():
        s = get_sample("tx")
        info = _tron_post(base, "/walletsolidity/gettransactioninfobyid", {"value": s["tx"]})
        need(isinstance(info, dict) and info, f"gettransactioninfobyid: empty for {s['tx']}, solidified 40 blocks ago")
        need(str(info.get("id", "")).lower() == s["tx"].lower() and is_int(info.get("blockNumber")), "gettransactioninfobyid: id or blockNumber")
        receipt = info.get("receipt", {}).get("result")
        need(receipt is None or isinstance(receipt, str), "gettransactioninfobyid: receipt.result")
        return f"solidity receipt in block {info['blockNumber']}" + (f", {receipt}" if receipt else "")

    checks = {"balance": balance, "fee": fee, "broadcast": broadcast, "token-balance": token_balance, "verification": verification}
    return run(row, checks, chain_sample(chain, _tron_sample))


def probe_trongrid(row):
    base, chain = row["endpoint"].rstrip("/"), row["chain_id"]
    get_sample = lambda *keys: sample_for(chain, _tron_sample, *keys)  # noqa: E731

    def listing(address, path):
        answer = jget(f"{base}/{address}{path}?limit=50&only_confirmed=true&order_by=block_timestamp,desc", path)
        data = answer.get("data")
        need(isinstance(data, list) and data, f"{path}: no data for an active account")
        need(len(data) < 50 or isinstance(answer.get("meta", {}).get("fingerprint"), str), f"{path}: full page without fingerprint")
        return data

    def history():
        s = get_sample("address")
        data = listing(s["address"], "/transactions")
        transfers = 0
        for tx in data:
            contract = tx["raw_data"]["contract"][0]
            need(isinstance(tx.get("txID"), str) and is_int(tx.get("block_timestamp")), "transaction: txID or block_timestamp")
            if contract["type"] == "TransferContract" and tx["ret"][0]["contractRet"] == "SUCCESS":
                value = contract["parameter"]["value"]
                need(isinstance(value.get("owner_address"), str) and isinstance(value.get("to_address"), str) and is_int(value.get("amount")), "TransferContract value")
                transfers += 1
        return f"/transactions: {len(data)} rows, {transfers} TRX transfers"

    def token_history():
        s = get_sample("token_holder")
        data = listing(s["token_holder"], "/transactions/trc20")
        transfers = [tx for tx in data if tx.get("type") == "Transfer"]
        need(transfers, "/transactions/trc20: no Transfer rows")
        for tx in transfers:
            need(isinstance(tx.get("transaction_id"), str) and digits(tx.get("value")), "trc20: transaction_id or value")
            need(is_int(tx.get("token_info", {}).get("decimals")) and isinstance(tx["token_info"].get("address"), str), "trc20: token_info")
            need(isinstance(tx.get("from"), str) and isinstance(tx.get("to"), str) and is_int(tx.get("block_timestamp")), "trc20: from, to or block_timestamp")
        return f"/transactions/trc20: {len(transfers)} Transfer rows with token_info"

    def token_discovery():
        s = get_sample("token_holder")
        answer = jget(f"{base}/{s['token_holder']}", "account")
        rows = (answer.get("data") or [{}])[0].get("trc20")
        need(isinstance(rows, list), "data[0].trc20 missing")
        held = {}
        for entry in rows:
            for contract, raw in entry.items():
                need(digits(raw), "trc20: balance not digits")
                held[contract] = int(raw)
        need(held.get(s["token"], 0) > 0 or s["token_id"].endswith("(derived)"), f"trc20 lacks the holder's {s['token_id']}")
        return f"data[0].trc20: {len(held)} tokens" + (f", {s['token_id']} among them" if s["token"] in held else "")

    checks = {"history": history, "token-history": token_history, "token-discovery": token_discovery}
    return run(row, checks, chain_sample(chain, _tron_sample))


# ── TON (toncenter_v2.rs, toncenter_v3.rs) ─────────────────────────────────

def _ton_sample(chain):
    token = preferred_token(chain)
    testnet = common.is_testnet(chain)

    def standard_wallets(base, addresses):
        """The active standard wallets among `addresses`: core sends from
        one, and reads its seqno get-method."""
        if not addresses:
            return {}
        states = jget(f"{base}/walletStates?" + "&".join(f"address={a}" for a in addresses), "walletStates")["wallets"]
        found = {ton_raw(w["address"]): w for w in states if w.get("is_wallet") and w.get("status") == "active"}
        if found or not common.endpoints("toncenter-v2", chain):
            return found
        # The testnet indexer recognises no wallet code: a contract whose
        # seqno get-method answers, as core reads it, stands in.
        v2 = common.endpoints("toncenter-v2", chain)[0]["endpoint"].rstrip("/")
        for address in addresses[:6]:
            answer = jpost(f"{v2}/runGetMethod", {"address": address, "method": "seqno", "stack": []}, "runGetMethod")
            result = answer.get("result") or {}
            if result.get("exit_code") == 0 and len(result.get("stack") or []) == 1:
                found[address] = {"wallet_type": "seqno get-method answers (v2)"}
                break
        return found

    def external(base, owner):
        txs = jget(f"{base}/transactions?account={owner}&limit=6&sort=desc", "transactions")["transactions"]
        return next((t["in_msg"]["hash"] for t in txs if t.get("in_msg") and t["in_msg"].get("source") is None), None)

    def at(url):
        base = url.rstrip("/")
        def transfers(master):
            return [t for t in jget(f"{base}/jetton/transfers?jetton_master={master}&limit=60&sort=desc", "jetton/transfers")["jetton_transfers"]
                    if not t.get("transaction_aborted")]

        if token:
            rows = transfers(token["contract"])
        else:
            # A jetton whose master content states its decimals, the only
            # place core reads them, that moved recently.
            masters = jget(f"{base}/jetton/masters?limit=50&offset=0", "jetton/masters").get("jetton_masters") or []
            rows = []
            for master in [m for m in masters if (m.get("jetton_content") or {}).get("decimals") is not None][:5]:
                rows = transfers(ton_raw(master["address"]))
                if rows:
                    break
        need(rows, "no recent jetton transfers")
        sources = list(dict.fromkeys(ton_raw(t["source"]) for t in rows))
        targets = list(dict.fromkeys(ton_raw(t["destination"]) for t in rows))
        candidates = list(dict.fromkeys(sources[:20] + targets[:20]))
        wallets = standard_wallets(base, candidates)
        found = {}
        for owner in [a for a in sources if a in wallets][:4]:
            message = external(base, owner)
            if message:
                found.update(address_raw=owner, tx=message, wallet_type=wallets[owner].get("wallet_type"))
                break
        holder = found.get("address_raw") or next((a for a in candidates if a in wallets), sources[0])
        master = next(ton_raw(t["jetton_master"]) for t in rows if holder in (ton_raw(t["source"]), ton_raw(t["destination"])))
        found.update(token_holder_raw=holder, token_holder=ton_friendly(holder, testnet), token=master,
                     token_id=token["token_id"] if token else f"{master} (derived)")
        if "tx" not in found:
            # A standard wallet that signed a recent external message.
            recent = jget(f"{base}/transactions?limit=60&sort=desc", "transactions")["transactions"]
            signers = list(dict.fromkeys(ton_raw(t["account"]) for t in recent if t.get("in_msg") and t["in_msg"].get("source") is None))[:20]
            senders = standard_wallets(base, signers)
            owner = next((a for a in signers if a in senders), None)
            need(owner, "no standard wallet signed an external message in the latest 60 transactions")
            found["wallet_type"] = senders[owner].get("wallet_type")
            found.update(address_raw=owner, tx=next(t["in_msg"]["hash"] for t in recent
                                                     if ton_raw(t["account"]) == owner and t.get("in_msg") and t["in_msg"].get("source") is None))
        found["address"] = ton_friendly(found["address_raw"], testnet)
        found["tx_kind"] = "external message hash (base64)"
        return found

    return each_endpoint("toncenter-v3", chain, at)


def probe_toncenter_v2(row):
    base, chain = row["endpoint"].rstrip("/"), row["chain_id"]
    get_sample = lambda *keys: sample_for(chain, _ton_sample, *keys)  # noqa: E731

    def balance():
        s = get_sample("address")
        answer = jget(f"{base}/getAddressBalance?address={s['address']}", "getAddressBalance")
        need(answer.get("ok") is True and digits(answer.get("result")), f"getAddressBalance: {_short(answer, 160)}")
        return f"getAddressBalance {answer['result']} nanotons"

    def history():
        s = get_sample("address")
        reply = _get(f"{base}/getTransactions?address={s['address']}&limit=50&archival=true")
        if not 200 <= (reply.status or 0) < 300 and not _refusal(reply):
            plain = _get(f"{base}/getTransactions?address={s['address']}&limit=50")
            raise Bad(f"getTransactions (archival=true, as core asks): {reply.brief(200)}; without archival: HTTP {plain.status}")
        answer = body_of(reply, "getTransactions")
        rows = answer.get("result")
        need(isinstance(rows, list) and rows, "getTransactions: none for an active wallet")
        for tx in rows:
            need(isinstance(tx["transaction_id"]["hash"], str) and digits(tx["transaction_id"]["lt"]), "transaction_id")
            need(is_int(tx["utime"]) and digits(tx["fee"]) and isinstance(tx["out_msgs"], list), "utime, fee or out_msgs")
            for msg in ([tx["in_msg"]] if tx.get("in_msg") else []) + tx["out_msgs"]:
                need(isinstance(msg["source"], str) and isinstance(msg["destination"], str) and digits(msg["value"]), "message source, destination or value")
        return f"getTransactions: {len(rows)} transactions with messages and fees"

    def broadcast():
        reply = _post(f"{base}/sendBocReturnHash", {"boc": "AA=="})
        return judge_broadcast(reply, "sendBocReturnHash", decoded=("bag_of_cells", "bag-of-cells", "boc", "deserial", "parse"))

    def verification():
        s = get_sample("address")
        info = jget(f"{base}/getMasterchainInfo", "getMasterchainInfo")
        init = info.get("result", {}).get("init", {})
        need(info.get("ok") is True and init.get("workchain") == -1 and init.get("seqno") == 0
             and isinstance(init.get("root_hash"), str) and isinstance(init.get("file_hash"), str), "getMasterchainInfo: result.init")
        state = jget(f"{base}/getAddressInformation?address={s['address']}", "getAddressInformation")
        need(state.get("ok") is True and state["result"].get("state") == "active", f"getAddressInformation: state {state.get('result', {}).get('state')!r}")
        seqno = jpost(f"{base}/runGetMethod", {"address": s["address"], "method": "seqno", "stack": []}, "runGetMethod")
        stack = seqno.get("result", {}).get("stack")
        need(seqno.get("ok") is True and seqno["result"].get("exit_code") == 0, f"runGetMethod seqno: {_short(seqno, 160)}")
        need(isinstance(stack, list) and len(stack) == 1 and stack[0][0] == "num" and str(stack[0][1]).startswith("0x"), f"runGetMethod seqno stack {stack!r}")
        return f"zero state, active wallet, seqno {int(stack[0][1], 16)}"

    checks = {
        "balance": balance, "history": history, "broadcast": broadcast, "verification": verification,
        "fee": lambda: unchecked("core quotes TON's fee from the registry (static_fee_units); toncenter_v2.rs makes no fee request"),
        "token-balance": lambda: unchecked("toncenter_v2.rs makes no jetton request; core reads jetton balances and decimals through toncenter-v3"),
    }
    return run(row, checks, chain_sample(chain, _ton_sample))


def probe_toncenter_v3(row):
    base, chain = row["endpoint"].rstrip("/"), row["chain_id"]
    get_sample = lambda *keys: sample_for(chain, _ton_sample, *keys)  # noqa: E731
    state = {}

    def first_block():
        if "first" not in state:
            first = jget(f"{base}/masterchainInfo", "masterchainInfo").get("first", {})
            need(first.get("workchain") == -1 and first.get("shard") == "8000000000000000" and first.get("seqno") == 1
                 and is_int(first.get("global_id")) and isinstance(first.get("root_hash"), str), "masterchainInfo: first block fields")
            state["first"] = first
        return state["first"]

    def wallet_decimals(response, master):
        infos = (response.get("metadata") or {}).get(master.upper(), {}) or (response.get("metadata") or {}).get(master, {})
        for info in infos.get("token_info") or []:
            if info.get("type") == "jetton_masters":
                value = (info.get("extra") or {}).get("decimals")
                return value if is_int(value) or digits(value) else None
        return None

    def token_balance():
        s = get_sample("token_holder")
        first_block()
        answer = jget(f"{base}/jetton/wallets?owner_address={s['token_holder']}&jetton_address={s['token']}&limit=2", "jetton/wallets")
        wallets = answer.get("jetton_wallets")
        need(isinstance(wallets, list) and len(wallets) == 1, f"jetton/wallets: {len(wallets or [])} wallets, core needs exactly one")
        wallet = wallets[0]
        need(all(isinstance(wallet.get(k), str) for k in ("address", "owner", "jetton")) and digits(wallet.get("balance")), "jetton wallet fields")
        masters = jget(f"{base}/jetton/masters?address={s['token']}&limit=1", "jetton/masters").get("jetton_masters") or []
        decimals = ((masters[0] if masters else {}).get("jetton_content") or {}).get("decimals")
        need(is_int(decimals) or digits(decimals), f"jetton/masters: jetton_content.decimals {decimals!r}; core's fetch_jetton_decimals reads only this")
        return f"{s['token_id']} wallet balance {wallet['balance']}; master decimals {decimals}"

    def token_discovery():
        s = get_sample("token_holder")
        first_block()
        answer = jget(f"{base}/jetton/wallets?owner_address={s['token_holder']}&limit=1000&offset=0", "jetton/wallets")
        wallets = answer.get("jetton_wallets")
        need(isinstance(wallets, list) and wallets, "jetton/wallets: none for a jetton holder")
        held = {ton_raw(w["jetton"]): w for w in wallets if digits(w.get("balance")) and int(w["balance"]) > 0}
        need(s["token"] in held, f"jetton/wallets lacks {s['token_id']}")
        missing = [m for m in held if wallet_decimals(answer, held[m]["jetton"]) is None]
        return f"{len(wallets)} jetton wallets, {s['token_id']} among them" + (f"; {len(missing)} without decimals in metadata" if missing else "; metadata decimals for each")

    def token_history():
        s = get_sample("token_holder_raw")
        answer = jget(f"{base}/jetton/transfers?owner_address={s['token_holder_raw']}&limit=50&offset=0&sort=desc", "jetton/transfers")
        rows = answer.get("jetton_transfers")
        need(isinstance(rows, list) and rows, "jetton/transfers: none for a jetton sender")
        for transfer in rows:
            need(isinstance(transfer.get("transaction_aborted"), bool), "transfer: transaction_aborted")
            need(all(isinstance(transfer.get(k), str) for k in ("source", "destination", "jetton_master", "transaction_hash")), "transfer: parties, master or hash")
            need(digits(transfer.get("amount")) and is_int(transfer.get("transaction_now")) and digits(transfer.get("transaction_lt")), "transfer: amount, now or lt")
        return f"jetton/transfers: {len(rows)} rows"

    def verification():
        s = get_sample("tx")
        first_block()
        message = base64.b64decode(s["tx"] + "=" * (-len(s["tx"]) % 4)).hex()
        answer = jget(f"{base}/traces?msg_hash={message}&include_actions=true&limit=2", "traces")
        traces = answer.get("traces")
        need(isinstance(traces, list) and len(traces) == 1, f"traces: {len(traces or [])} for one external message")
        trace = traces[0]
        transactions = trace.get("transactions")
        need(isinstance(transactions, dict), "trace: transactions object")
        root = transactions.get(trace["trace"]["tx_hash"])
        need(isinstance(root, dict), "trace: root transaction missing")
        need(isinstance(trace.get("external_hash"), str) and isinstance(root["in_msg"].get("hash"), str), "trace: external_hash or in_msg.hash")
        need(isinstance(root.get("account"), str) and root["in_msg"].get("source") is None, "root: account or external source")
        need(isinstance(root.get("emulated"), bool) and root.get("finality") is not None, "root: emulated or finality")
        need(is_int(root.get("mc_block_seqno")) and isinstance(trace.get("is_incomplete"), bool), "root: mc_block_seqno or trace.is_incomplete")
        need(trace.get("trace_info", {}).get("trace_state") in ("complete", "pending") and is_int(trace["trace_info"].get("pending_messages")), "trace_info")
        description = root["description"]
        need(isinstance(description.get("aborted"), bool) and isinstance(description.get("compute_ph"), dict), "root: description")
        need(isinstance(trace.get("actions"), list), "trace: actions")
        return f"trace of {len(transactions)} transactions, {trace['trace_info']['trace_state']}, finality {root['finality']}"

    checks = {"token-balance": token_balance, "token-discovery": token_discovery, "token-history": token_history,
              "verification": verification}
    return run(row, checks, chain_sample(chain, _ton_sample))


PROBES = {
    "solana-json-rpc": probe_solana,
    "sui-graphql": probe_sui,
    "near-json-rpc": probe_near,
    "nearblocks": probe_nearblocks,
    "fastnear": probe_fastnear,
    "aptos-rest": probe_aptos_rest,
    "aptos-indexer": probe_aptos_indexer,
    "xrpl-json-rpc": probe_xrpl,
    "horizon": probe_horizon,
    "tron-http": probe_tron_http,
    "trongrid-v1": probe_trongrid,
    "toncenter-v2": probe_toncenter_v2,
    "toncenter-v3": probe_toncenter_v3,
}
