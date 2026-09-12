#!/usr/bin/env python3
"""Offline proof of network/deployment identity through the actual CLI."""
import json
import pathlib
import sqlite3
import subprocess
import sys
import tempfile

with tempfile.TemporaryDirectory(prefix="spectra-identity-") as directory:
    def run(*args, succeeds=True):
        result = subprocess.run([sys.argv[1], "--data-dir", directory, "--json", *args], text=True, capture_output=True)
        assert (result.returncode == 0) == succeeds, (args, result.stdout, result.stderr)
        return json.loads(result.stdout) if succeeds else None

    networks = {n["id"]: n for n in run("chains", "--testnets")["chains"]}
    assert not networks["ethereum"]["isTestnet"] and networks["ethereum-sepolia"]["isTestnet"]
    assert networks["ethereum"]["family"] == networks["ethereum-sepolia"]["family"]
    assert all("symbol" not in n for n in networks.values())
    assert networks["arbitrum"]["nativeSymbol"] == "ETH"
    def catalog(network):
        return run("token", "catalog", "--chain", network)["tokens"]
    eth = next(t for t in catalog("ethereum") if t["id"] == "ethereum:native")
    btc = next(t for t in catalog("bitcoin") if t["id"] == "bitcoin:native")
    mnt_native = next(t for t in catalog("mantle") if t["id"] == "mantle:native")
    mnt_token = next(t for t in catalog("ethereum") if t["symbol"] == "MNT")
    assert eth["kind"] == btc["kind"] == mnt_native["kind"] == "Native"
    assert mnt_token["token_id"] == mnt_native["token_id"] and mnt_token["id"] != mnt_native["id"]
    test_eth = catalog("ethereum-sepolia")[0]
    assert test_eth["coingecko_id"] == "" and test_eth["token_id"] != eth["token_id"]
    run("token", "catalog", "--chain", "ETH", succeeds=False)
    arb = next(t for t in catalog("arbitrum") if t["symbol"] == "ARB")
    fee = run("send", "affordability", "--chain", "arbitrum", "--deployment", arb["id"], "--symbol", "ARB", "--amount", "1", "--fee", "0.5", "--balance", "2", "--gas-balance", "0.1")
    assert fee["verdict"] == "feeExceedsGasBalance" and fee["gasSymbol"] == "ETH"
    assembly = run("send", "assemble", "--chain", "ethereum", "--symbol", "ETH", "--contract", "0x1111111111111111111111111111111111111111", "--decimals", "6", "--from", "0x2222222222222222222222222222222222222222", "--to", "0x3333333333333333333333333333333333333333", "--amount", "1")
    assert assembly["isNative"] is False and assembly["valueWei"] == "0"
    run("wallet", "watch", "--chain", "ethereum", "--name", "Identity", "--address", "0x1111111111111111111111111111111111111111")
    # Seed deterministic balances, then let separate CLI processes read/group/route them.
    with sqlite3.connect(pathlib.Path(directory) / "spectra.sqlite") as db:
        wallet = json.loads(db.execute("SELECT payload FROM wallets").fetchone()[0])
        assert wallet["networkId"] == "ethereum" and "networkMode" not in wallet
        def holding(network, amount, contract=None):
            return dict(name="Ether", symbol="ETH", coinGeckoId="ethereum", chainName=network, tokenStandard="ERC-20" if contract else "Native", contractAddress=contract, amount=amount, priceUsd=100)
        wallet["holdings"] = [holding("Ethereum", 1), holding("Base", 2), holding("Ethereum Sepolia", 3), holding("Ethereum", 4, "0x1111111111111111111111111111111111111111")]
        db.execute("UPDATE wallets SET payload=?", (json.dumps(wallet),))
    groups = run("portfolio", "--stored", "--pin-token", "ethereum")["groups"]
    assert all(g["isPinned"] == (g["id"] == "ethereum") for g in groups)
    options = run("portfolio", "--pin-options")["options"]
    assert any(o["token_id"] == "bitcoin" for o in options)
    assert len([o for o in options if o["symbol"] == "ETH"]) >= 2
    run("portfolio", "--stored", "--pin-token", "ETH", succeeds=False)
    eth_group = next(g for g in groups if g["id"] == eth["token_id"])
    assert sum(h["coin"]["amount"] for h in eth_group["holdings"]) == 3
    test_group = next(g for g in groups if g["id"] == test_eth["token_id"])
    assert all(h["valueUsd"] is None for h in test_group["holdings"])
    assert any(g["id"].startswith("custom:ethereum:erc-20:") for g in groups)
    run("send", "preview", "--wallet", "Identity", "--holding", "Ethereum|ETH", "--amount", "1", succeeds=False)
    run("token", "add", "--chain", "ethereum", "--symbol", "ETH", "--name", "Lookalike", "--contract", "invalid", "--decimals", "18", succeeds=False)
    run("network", "set", "ethereum-sepolia")
    run("network", "set", "ethereum")
    with sqlite3.connect(pathlib.Path(directory) / "spectra.sqlite") as db:
        wallet = json.loads(db.execute("SELECT payload FROM wallets").fetchone()[0])
        assert wallet["networkId"] == "ethereum"
    assert len(next(n for n in run("network", "list")["families"] if n["family"] == "ethereum")["choices"]) >= 3
print("network/token identity CLI checks passed")
