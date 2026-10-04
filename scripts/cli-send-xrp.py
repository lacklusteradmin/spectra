#!/usr/bin/env python3
"""XRP build/sign uses canonical SDK bytes and refuses unsafe provider input."""
import http.server
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/spectra").resolve())
fixture = json.loads((pathlib.Path(__file__).resolve().parents[1] / "core/tests/fixtures/xrp-mnemonic-payment.json").read_text())
state = dict(fee="12", sequence=7, requests=[])


class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        state["requests"].append(request)
        if request["method"] == "fee":
            result = {"drops": {"open_ledger_fee": state["fee"]}}
        elif request["method"] == "account_info":
            result = {"account_data": {"Sequence": state["sequence"]}}
        else:
            raise AssertionError(request)
        body = json.dumps({"result": result}).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


node = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Node)
threading.Thread(target=node.serve_forever, daemon=True).start()
endpoint = f"http://127.0.0.1:{node.server_port}"
try:
    for chain in ["xrp", "xrp-testnet"]:
        with tempfile.TemporaryDirectory(prefix="spectra-xrp-") as directory:
            def run(*args, success=True):
                result = subprocess.run(
                    [binary, "--data-dir", directory, "--json", *args],
                    capture_output=True, text=True, timeout=45,
                    env={**os.environ, "SPECTRA_PASSWORD": "xrp-fixture-password", "SPECTRA_SEED": fixture["mnemonic"]},
                )
                assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
                return json.loads(result.stdout)

            run("wallet", "import", "--chain", chain, "--name", "XRP", "--path", fixture["derivation_path"])

            def build(amount="123.456789", success=True):
                return run("send", "build", "--from", "XRP", "--to", fixture["transaction"]["Destination"],
                           "--amount", amount, "--endpoint", endpoint, success=success)

            for amount in ["0", "100000000000.000001", "18446744073709.551615"]:
                before = len(state["requests"])
                assert "XRP amount and fee" in str(build(amount, success=False))
                assert len(state["requests"]) == before, "invalid amounts must not contact providers"
            for fee in ["0", "100000000000000001", str(2**64 - 1)]:
                state["fee"] = fee
                assert "XRP amount and fee" in str(build(success=False))
            state["fee"] = "12"
            state["sequence"] = 2**32
            assert "invalid Sequence" in str(build(success=False))
            assert run("send", "list")["artifacts"] == [], "invalid inputs must never persist a prepared send"
            state["sequence"] = 7
            prepared = build()["artifact"]
            signed = run("send", "sign", prepared["id"], "--review-digest", prepared["review_digest"],
                         "--endpoint", endpoint)["artifact"]
            assert json.loads(signed["signed_payload"])["tx_blob_hex"] == fixture["signed_hex"]
            assert run("send", "inspect", prepared["id"])["artifact"] == signed
    print("XRP canonical signing and protocol validation acceptance passed")
finally:
    node.shutdown()
    node.server_close()
