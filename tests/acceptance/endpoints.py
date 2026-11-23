#!/usr/bin/env python3
"""The CLI's own endpoint views, offline: the listing's source filter, the
health verdict it aggregates from core's probes, and a network known only by
an endpoint added here."""
from harness import LEGAL, Handler, Spectra, scratch, serve


class Node(Handler):
    """A Blockbook backend whose first address asked about holds 123456
    duffs and has a transaction; every other address is unused."""

    def do_GET(self):
        if self.state["fail"]:
            self.reply({"error": "Internal server error"})
        elif self.path.startswith("/api/v2/address/"):
            address = self.path.split("/")[4].split("?")[0]
            funded = self.state.setdefault("funded", address) == address
            self.reply({"balance": "123456" if funded else "0", "unconfirmedBalance": "0",
                        "txs": 1 if funded else 0, "unconfirmedTxs": 0})
        elif self.path.startswith("/api/v2/utxo/"):
            self.reply([])
        else:
            self.reply({"blockbook": {"bestHeight": 123}})


with scratch("endpoints") as directory:
    spectra = Spectra(directory)

    def listed(source):
        return spectra.run("endpoints", "--catalog", "--source", source)["endpoints"]

    catalog = listed("built-in")
    assert catalog and all(row["isBuiltIn"] for row in catalog)
    url = "http://127.0.0.1:1/node"
    spectra.run("endpoints", "--chain", "solana", "--api", "solana-json-rpc", "--capabilities", "broadcast",
                "--add", url)
    custom = listed("custom")
    assert [(row["endpoint"], row["isBuiltIn"]) for row in custom] == [(url, False)], custom
    assert listed("built-in") == catalog
    print("The listing filters the catalog by source")

    # A network with no API configured is reported apart, and is not healthy.
    health = spectra.run("endpoints", "--chain", "dash-testnet")
    assert not health["ok"] and health["networksWithoutApis"] == ["dash-testnet"], health
    assert health["total"] == 0, health

    # One endpoint, answering as it should and then not.
    with serve(Node, fail=False) as node:
        spectra.run("endpoints", "--chain", "dash-testnet", "--api", "blockbook", "--capabilities",
                    "balance,fee,broadcast", "--add", node.url)
        health = spectra.run("endpoints", "--chain", "dash-testnet")
        assert health["ok"] and health["networksWithoutApis"] == [], health
        assert (health["total"], health["unreachable"], health["uncheckedApis"]) == (1, 0, 0), health
        node.state["fail"] = True
        health = spectra.run("endpoints", "--chain", "dash-testnet")
        assert not health["ok"] and health["unreachable"] == 1, health
        assert health["endpoints"][0]["detail"], health
        print("The health verdict counts networks without APIs and unreachable endpoints")

    # The catalog lists no endpoint for dash-testnet: its balance is read
    # from an endpoint added here.
    with serve(Node, fail=False) as node:
        spectra.run("endpoints", "--chain", "dash-testnet", "--api", "blockbook", "--capabilities",
                    "balance,history,utxo,verification", "--add", node.url)
        spectra.run("wallet", "import", "--chain", "dash-testnet", "--name", "Custom DASH",
                    env={"SPECTRA_SEED": LEGAL, "SPECTRA_PASSWORD": "fixture-password"})
        balance = spectra.run("balance", "Custom DASH")
        assert (balance["amount"], balance["smallestUnit"]) == ("0.00123456", "123456"), balance
    print("A network with only a custom endpoint reads its balance from it")
