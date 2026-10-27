#!/usr/bin/env python3
"""The CLI's own endpoint views, offline: the listing's source filter and the
health verdict it aggregates from core's probes."""
from harness import Handler, Spectra, scratch, serve


class Node(Handler):
    def do_GET(self):
        self.reply({"error": "Internal server error"} if self.state["fail"] else {"blockbook": {"bestHeight": 123}})


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
