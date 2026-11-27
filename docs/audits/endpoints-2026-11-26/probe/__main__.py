"""Probe every endpoint in core/data/endpoints.toml for each capability it
claims, read-only and without credentials, and print the evidence as JSON.

    python3 -B docs/audits/endpoints-2026-11-26/probe [--api A] [--chain C] [--id I] [--out FILE]

Each module beside this one maps API names to a function taking one catalog
row and answering `{"capabilities": {capability: result}, "sample": {...}}`,
a result being `common.ok`, `common.fail` or `common.unchecked`. Network
identity is core's check (`spectra endpoints probe`), recorded beside this.
"""
import argparse
import concurrent.futures
import datetime
import importlib
import json
import pathlib
import sys
import threading
import traceback
import urllib.parse

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import common  # noqa: E402

PROBES = {}
for path in sorted(HERE.glob("*.py")):
    if path.stem in {"__main__", "common"}:
        continue
    module = importlib.import_module(path.stem)
    for api, function in module.PROBES.items():
        assert api not in PROBES, f"{api} is probed twice"
        PROBES[api] = function

_host_slots = {}
_host_lock = threading.Lock()


def _slot(url):
    """At most two endpoints of one host at a time, so a shared operator's
    rate limit is not what the record measures."""
    host = urllib.parse.urlsplit(url).hostname or url
    with _host_lock:
        return _host_slots.setdefault(host, threading.Semaphore(2))


def run(row):
    record = {
        "id": row["id"],
        "chain": row["chain_id"],
        "api": row["api"],
        "endpoint": row["endpoint"],
        "claimed": row["capabilities"],
    }
    function = PROBES.get(row["api"])
    if function is None:
        record["capabilities"] = {c: common.unchecked(f"no probe for {row['api']}") for c in row["capabilities"]}
    else:
        with _slot(row["endpoint"]):
            try:
                answer = function(row)
            except Exception as error:  # noqa: BLE001 — a probe bug is recorded, not fatal
                answer = {"capabilities": {}, "error": f"{type(error).__name__}: {error}",
                          "traceback": traceback.format_exc(limit=3)}
        record.update({key: value for key, value in answer.items() if key != "capabilities"})
        found = answer.get("capabilities", {})
        record["capabilities"] = {
            capability: found.get(capability, common.fail("not probed")) for capability in row["capabilities"]
        }
        extra = sorted(set(found) - set(row["capabilities"]))
        if extra:
            record["unclaimed"] = {capability: found[capability] for capability in extra}
    results = [result["ok"] for result in record["capabilities"].values()]
    record["passed"] = all(result is True for result in results)
    return record


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--api")
    parser.add_argument("--chain")
    parser.add_argument("--id")
    parser.add_argument("--out")
    parser.add_argument("--workers", type=int, default=12)
    args = parser.parse_args()
    rows = [row for row in common.endpoints(args.api, args.chain) if args.id in (None, row["id"])]
    started = datetime.datetime.now(datetime.timezone.utc)
    with concurrent.futures.ThreadPoolExecutor(args.workers) as pool:
        records = list(pool.map(run, rows))
    document = {
        "checked_at_utc": started.replace(microsecond=0).isoformat(),
        "user_agent": common.USER_AGENT,
        "credentials": "none",
        "total": len(records),
        "passed": sum(record["passed"] for record in records),
        "endpoints": records,
    }
    text = json.dumps(document, indent=1, ensure_ascii=False) + "\n"
    if args.out:
        pathlib.Path(args.out).write_text(text)
    else:
        sys.stdout.write(text)
    for record in records:
        if not record["passed"]:
            failing = {c: r["detail"] for c, r in record["capabilities"].items() if r["ok"] is not True}
            print(f"FAIL {record['id']}: {json.dumps(failing)[:400]}", file=sys.stderr)


if __name__ == "__main__":
    main()
