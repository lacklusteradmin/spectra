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
    types = {(row["chainId"], row["api"]) for row in catalog["endpoints"] if row["api"]}
    for index, (chain, api) in enumerate(sorted(types)):
        url = f"https://custom-{index}.example/api"
        run("endpoints", "--chain", chain, "--api", api, "--add", url)
    custom = run("endpoints", "--catalog", "--source", "custom")
    assert len(custom["endpoints"]) == len(types)
    assert all(not row["isBuiltIn"] for row in custom["endpoints"])
    assert {(row["chainId"], row["api"]) for row in custom["endpoints"]} == types
    built_in = run("endpoints", "--catalog", "--source", "built-in")
    assert built_in["endpoints"] == catalog["endpoints"]
    run("endpoints", "--chain", "solana", "--api", "esplora", "--add", "https://wrong.example", success=False)
    run("endpoints", "--chain", "solana", "--api", "solana-json-rpc", "--add", "file:///tmp/node", success=False)
    run("endpoints", "--chain", "solana", "--api", "solana-json-rpc", "--add", "https://node.example")
    configured = run("send", "configured-endpoints", "solana")
    assert "https://node.example" in json.dumps(configured)
    assert "https://node.example" not in json.dumps(run("send", "configured-endpoints", "solana-devnet"))
    run("endpoints", "--chain", "solana", "--api", "solana-json-rpc", "--add", "https://node.example/", success=False)
    print(f"{len(types)} catalog network/API pairs persist; source filters and network isolation passed")
