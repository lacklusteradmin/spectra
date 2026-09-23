#!/usr/bin/env python3
"""Typed endpoint persistence, source filters and API selection, offline."""
import json
import subprocess
import sys
import tempfile

with tempfile.TemporaryDirectory(prefix="spectra-endpoints-") as directory:
    def run(*args, success=True):
        result = subprocess.run([sys.argv[1], "--data-dir", directory, "--json", *args], capture_output=True, text=True, timeout=30)
        assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
        return json.loads(result.stdout)

    catalog = run("endpoints", "--catalog", "--source", "built-in")
    types = {(row["chainId"], row["api"]): row["supportedCapabilities"] for row in catalog["endpoints"] if row["supportedCapabilities"]}
    for index, (chain, api) in enumerate(sorted(types)):
        url = f"https://custom-{index}.example/api"
        run("endpoints", "--chain", chain, "--api", api, "--capabilities", ",".join(types[(chain, api)]), "--add", url)
    custom = run("endpoints", "--catalog", "--source", "custom")
    assert len(custom["endpoints"]) == len(types)
    assert all(not row["isBuiltIn"] for row in custom["endpoints"])
    assert {(row["chainId"], row["api"]) for row in custom["endpoints"]} == set(types)
    built_in = run("endpoints", "--catalog", "--source", "built-in")
    assert built_in["endpoints"] == catalog["endpoints"]
    run("endpoints", "--chain", "solana", "--api", "esplora", "--capabilities", "balance", "--add", "https://wrong.example", success=False)
    run("endpoints", "--chain", "solana", "--api", "solana-json-rpc", "--capabilities", "broadcast", "--add", "file:///tmp/node", success=False)
    run("endpoints", "--chain", "solana", "--api", "solana-json-rpc", "--capabilities", "broadcast", "--add", "https://node.example")
    configured = run("send", "configured-endpoints", "solana")
    assert "https://node.example" in json.dumps(configured)
    assert "https://node.example" not in json.dumps(run("send", "configured-endpoints", "solana-devnet"))
    run("endpoints", "--chain", "solana", "--api", "solana-json-rpc", "--capabilities", "broadcast", "--add", "https://node.example/", success=False)
    for cap in ["made-up", "history"]:
        run("endpoints", "--chain", "ethereum", "--api", "evm-json-rpc", "--capabilities", cap, "--add", "https://invalid-capability.example", success=False)
    for url, caps in [("https://balance-only.example", "balance"), ("https://broadcast-only.example", "broadcast")]:
        run("endpoints", "--chain", "ethereum", "--api", "evm-json-rpc", "--capabilities", caps, "--add", url)
    saved = run("endpoints", "--catalog", "--chain", "ethereum", "--source", "custom")["endpoints"]
    assert next(row for row in saved if row["endpoint"] == "https://balance-only.example")["capabilities"] == ["balance"]
    assert next(row for row in saved if row["endpoint"] == "https://broadcast-only.example")["capabilities"] == ["broadcast"]
    destinations = run("send", "configured-endpoints", "ethereum")["endpoints"]
    assert "https://balance-only.example" not in destinations
    assert "https://broadcast-only.example" in destinations
    print(f"{len(types)} catalog network/API pairs persist; source filters and network isolation passed")

# Missing built-in providers stay empty, while supported custom APIs still work.
with tempfile.TemporaryDirectory(prefix="spectra-empty-endpoints-") as directory:
    for chain, api in [("zcash", "blockbook"), ("bitcoin-gold", "blockbook"),
                       ("dash", "blockbook"), ("dogecoin-testnet", "blockcypher"),
                       ("monero-stagenet", "monero-daemon-rpc")]:
        assert run("send", "configured-endpoints", chain)["endpoints"] == [], chain
        health = run("endpoints", "--chain", chain)
        assert not health["ok"] and health["networksWithoutApis"] == [chain], health
        url = "https://custom.example/" + chain
        run("endpoints", "--chain", chain, "--api", api, "--capabilities", "fee,broadcast", "--add", url)
        assert run("send", "configured-endpoints", chain)["endpoints"] == [url], chain
    catalog = run("endpoints", "--catalog", "--source", "built-in")
    assert any(row["chainId"] == "ethereum-sepolia" for row in catalog["configured"])
    for chain in ["base", "arbitrum", "optimism", "avalanche", "mantle", "blast"]:
        assert any(row["chainId"] == chain and row["api"] == "blockscout" for row in catalog["endpoints"])
    assert not any(row["chainId"] == "berachain" and "history" in row["capabilities"] for row in catalog["endpoints"])
    print("Missing providers remain empty; supported custom APIs and concrete network catalogs passed")

# A testnet without public providers can exercise the real health command offline.
import http.server
import threading
with tempfile.TemporaryDirectory(prefix="spectra-health-") as directory:
    state = {"fail": False}
    seen = []
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_): pass
        def do_POST(self):
            call = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            method = call["method"]
            seen.append(method)
            values = {"eth_chainId": "0x66eee", "eth_blockNumber": "0x123", "eth_getBalance": "0x0"}
            response = {"jsonrpc": "2.0", "id": call["id"], "result": values[method]}
            if state["fail"] and method == "eth_blockNumber":
                response = {"jsonrpc": "2.0", "id": call["id"], "error": {"code": -32046, "message": "Cannot fulfill request"}}
            payload = json.dumps(response).encode()
            self.send_response(200); self.send_header("Content-Length", str(len(payload)))
            self.end_headers(); self.wfile.write(payload)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
    try:
        url = f"http://127.0.0.1:{server.server_port}"
        run("endpoints", "--chain", "arbitrum-sepolia", "--api", "evm-json-rpc", "--capabilities", "balance,fee,broadcast", "--add", url)
        health = run("endpoints", "--chain", "arbitrum-sepolia")
        assert health["ok"] and health["uncheckedApis"] == 0, health
        assert seen == ["eth_chainId", "eth_blockNumber", "eth_getBalance"], seen
        state["fail"] = True
        health = run("endpoints", "--chain", "arbitrum-sepolia")
        assert not health["ok"] and health["unreachable"] == 1, health
        assert "eth_blockNumber" in health["endpoints"][0]["detail"], health
    finally:
        server.shutdown(); server.server_close(); worker.join()
    print("CLI health rejects a node whose identity responds but chain reads fail")
