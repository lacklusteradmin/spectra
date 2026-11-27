"""What every API module of the endpoint probe shares: HTTP with Spectra's
user agent and no credentials, the catalog files, and the shape of a result.

A probe only reads. A broadcast route is checked by submitting a payload that
decodes to no transaction, which a service refuses before it could relay
anything; nothing is signed.
"""
import json
import pathlib
import time
import tomllib
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[4]
DATA = ROOT / "core" / "data"
USER_AGENT = "spectra-core/0.1.0"
TIMEOUT = 25


class Reply:
    """One HTTP exchange. `error` is set when no HTTP answer arrived."""

    def __init__(self, url, status=None, body=b"", ms=0, error=None):
        self.url = url
        self.status = status
        self.body = body
        self.ms = ms
        self.error = error
        try:
            self.json = json.loads(body) if body else None
        except ValueError:
            self.json = None

    @property
    def text(self):
        return self.body.decode("utf-8", "replace")

    def brief(self, limit=240):
        """The answer in one line, for a result's detail."""
        if self.error:
            return f"no answer: {self.error}"[:limit]
        return f"HTTP {self.status}: {self.text.strip()}"[:limit].replace("\n", " ")


def request(method, url, *, json_body=None, data=None, headers=None, timeout=TIMEOUT, retries=1):
    """Send one request, retrying once after a transport error, a 429 or a
    5xx, as core's diagnostics profile does."""
    head = {"User-Agent": USER_AGENT, "Accept": "application/json"}
    if json_body is not None:
        data = json.dumps(json_body).encode()
        head["Content-Type"] = "application/json"
    head.update(headers or {})
    for attempt in range(retries + 1):
        started = time.monotonic()
        req = urllib.request.Request(url, data=data, headers=head, method=method)
        try:
            with urllib.request.urlopen(req, timeout=timeout) as response:
                reply = Reply(url, response.status, response.read(), _ms(started))
        except urllib.error.HTTPError as error:
            try:
                body = error.read()
            except Exception as cut:  # noqa: BLE001 — a body cut short is still a status
                body = f"(body unreadable: {type(cut).__name__})".encode()
            reply = Reply(url, error.code, body, _ms(started))
        except Exception as error:  # noqa: BLE001 — every transport failure is evidence
            reply = Reply(url, error=f"{type(error).__name__}: {error}", ms=_ms(started))
        retryable = reply.error is not None or reply.status == 429 or (reply.status or 0) >= 500
        if not retryable or attempt == retries:
            return reply
        time.sleep(1.5 if reply.status == 429 else 0.6)
    return reply


def _ms(started):
    return int((time.monotonic() - started) * 1000)


def get(url, **options):
    return request("GET", url, **options)


def post(url, json_body=None, **options):
    return request("POST", url, json_body=json_body, **options)


def rpc(url, method, params=None, **options):
    """A JSON-RPC 2.0 call. The reply's `json` is the whole envelope."""
    body = {"jsonrpc": "2.0", "id": 1, "method": method, "params": [] if params is None else params}
    return post(url, body, **options)


def rpc_result(reply):
    """The `result` of a JSON-RPC reply, or None on an error object or no answer."""
    if not isinstance(reply.json, dict) or reply.json.get("error") is not None:
        return None
    return reply.json.get("result")


# ── Results ────────────────────────────────────────────────────────────────

def ok(detail, **evidence):
    return {"ok": True, "detail": detail, **evidence}


def fail(detail, **evidence):
    return {"ok": False, "detail": detail, **evidence}


def unchecked(detail):
    """A capability this probe cannot exercise over HTTP; core's own probe or
    another record covers it. Not a pass."""
    return {"ok": None, "detail": detail}


def from_reply(reply, accept, describe):
    """`ok` when `accept(reply)` is true, with `describe(reply)` as the detail;
    otherwise `fail` with the reply in brief."""
    try:
        accepted = accept(reply)
    except Exception as error:  # noqa: BLE001
        return fail(f"{type(error).__name__}: {error}; {reply.brief()}")
    if accepted:
        try:
            return ok(describe(reply))
        except Exception as error:  # noqa: BLE001
            return ok(f"answered ({type(error).__name__} describing it)")
    return fail(reply.brief())


# ── Catalog ────────────────────────────────────────────────────────────────

def _load(name):
    with open(DATA / name, "rb") as file:
        return tomllib.load(file)


ENDPOINTS = _load("endpoints.toml")["endpoints"]
_TOKENS = _load("tokens.toml")
_TESTNET_TOKENS = _load("testnet-tokens.toml")
CHAINS = {row["id"]: row for row in _load("chains.toml")["chains"]}


def deployments(chain):
    """The catalog's deployments on `chain`: tokens carry `standard` and
    `contract`; the network's own coin carries neither."""
    rows = _TOKENS.get("deployments", []) + _TESTNET_TOKENS.get("deployments", [])
    return [row for row in rows if row["chain_id"] == chain]


def tokens(chain):
    """The catalog's token deployments on `chain`, coins excluded."""
    return [row for row in deployments(chain) if row.get("contract")]


def is_testnet(chain):
    return "mainnet" in CHAINS.get(chain, {})


def endpoints(api=None, chain=None):
    return [
        row for row in ENDPOINTS
        if (api is None or row["api"] == api) and (chain is None or row["chain_id"] == chain)
    ]
