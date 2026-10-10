#!/usr/bin/env python3
"""The CLI's own endpoint views, offline: the listing's source filter and the
health verdict it aggregates from core's probes."""
import http.server
import json
import subprocess
import sys
import threading
import tempfile

with tempfile.TemporaryDirectory(prefix="spectra-endpoints-") as directory:
    def run(*args):
        result = subprocess.run([sys.argv[1], "--data-dir", directory, "--json", *args],
                                capture_output=True, text=True, timeout=30)
        assert result.returncode == 0, (args, result.stdout, result.stderr)
        return json.loads(result.stdout)

    def listed(source):
        return run("endpoints", "--catalog", "--source", source)["endpoints"]

    catalog = listed("built-in")
    assert catalog and all(row["isBuiltIn"] for row in catalog)
    url = "http://127.0.0.1:1/node"
    run("endpoints", "--chain", "solana", "--api", "solana-json-rpc", "--capabilities", "broadcast", "--add", url)
    custom = listed("custom")
    assert [(row["endpoint"], row["isBuiltIn"]) for row in custom] == [(url, False)], custom
    assert listed("built-in") == catalog
    print("The listing filters the catalog by source")

    # A network with no API configured is reported apart, and is not healthy.
    health = run("endpoints", "--chain", "dash-testnet")
    assert not health["ok"] and health["networksWithoutApis"] == ["dash-testnet"], health
    assert health["total"] == 0, health

    # One endpoint, answering as it should and then not.
    state = {"fail": False}
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_): pass
        def do_GET(self):
            response = {"error": "Internal server error"} if state["fail"] else {"blockbook": {"bestHeight": 123}}
            payload = json.dumps(response).encode()
            self.send_response(200); self.send_header("Content-Length", str(len(payload)))
            self.end_headers(); self.wfile.write(payload)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
    try:
        node = f"http://127.0.0.1:{server.server_port}"
        run("endpoints", "--chain", "dash-testnet", "--api", "blockbook", "--capabilities", "balance,fee,broadcast", "--add", node)
        health = run("endpoints", "--chain", "dash-testnet")
        assert health["ok"] and health["networksWithoutApis"] == [], health
        assert (health["total"], health["unreachable"], health["uncheckedApis"]) == (1, 0, 0), health
        state["fail"] = True
        health = run("endpoints", "--chain", "dash-testnet")
        assert not health["ok"] and health["unreachable"] == 1, health
        assert health["endpoints"][0]["detail"], health
    finally:
        server.shutdown(); server.server_close(); worker.join()
    print("The health verdict counts networks without APIs and unreachable endpoints")
