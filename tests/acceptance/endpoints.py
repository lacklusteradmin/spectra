#!/usr/bin/env python3
"""The CLI's own endpoint views and routing, offline: the listing's source
filter, the health verdict it aggregates from core's probes, a network known
only by an endpoint added here, a probe that asks an endpoint's network, and
`--endpoint` naming a network's endpoint for one process: reads go there
alone, and a broadcast to a node on another network submits nothing."""
from harness import LEGAL, Handler, JsonRpc, Spectra, scratch, serve


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


class Evm(JsonRpc):
    """An EVM node on the chain id in its state, journaling what it is asked
    and what is submitted to it."""

    def call(self, method, params):
        self.state["asked"].append(method)
        if method == "eth_sendRawTransaction":
            self.state["submitted"].append(params[0])
        values = {"eth_chainId": self.state["chain"], "eth_getTransactionCount": "0x7",
                  "eth_getBalance": hex(10**19), "eth_estimateGas": "0x5208", "eth_getCode": "0x",
                  "eth_feeHistory": {"baseFeePerGas": ["0x3b9aca00"], "reward": [["0x77359400"]]},
                  "eth_sendRawTransaction": "0x" + "ab" * 32}
        return values[method]


def evm(chain):
    return serve(Evm, chain=chain, asked=[], submitted=[])


if __name__ == '__main__':
    with scratch("endpoints") as directory:
        spectra = Spectra(directory)

        def listed(source):
            return spectra.run("endpoints", "list", "--source", source)["endpoints"]

        catalog = listed("built-in")
        assert catalog and all(row["isBuiltIn"] for row in catalog)
        url = "http://127.0.0.1:1/node"
        spectra.run("endpoints", "add", "--chain", "solana", "--api", "solana-json-rpc", "--capabilities", "broadcast",
                    url)
        custom = listed("custom")
        assert [(row["endpoint"], row["isBuiltIn"]) for row in custom] == [(url, False)], custom
        assert listed("built-in") == catalog
        print("The listing filters the catalog by source")

        # A network with no API configured is reported apart, and is not healthy:
        # the probe reports it and exits 1.
        health = spectra.fail("endpoints", "probe", "--chain", "dash-testnet", code=1)
        assert not health["ok"] and health["networksWithoutApis"] == ["dash-testnet"], health
        assert health["total"] == 0, health

        # One endpoint, answering as it should and then not. Blockbook on Dash
        # cannot name its network, and the probe says so.
        with serve(Node, fail=False) as node:
            spectra.run("endpoints", "add", "--chain", "dash-testnet", "--api", "blockbook", "--capabilities",
                        "balance,broadcast", node.url)
            health = spectra.run("endpoints", "probe", "--chain", "dash-testnet")
            assert health["ok"] and health["networksWithoutApis"] == [], health
            assert (health["total"], health["unreachable"], health["uncheckedApis"]) == (1, 0, 0), health
            assert "cannot name its network" in health["endpoints"][0]["detail"], health
            node.state["fail"] = True
            health = spectra.fail("endpoints", "probe", "--chain", "dash-testnet", code=1)
            assert not health["ok"] and health["unreachable"] == 1, health
            assert health["endpoints"][0]["detail"], health
            print("The health verdict counts networks without APIs and unreachable endpoints")

        # The catalog lists no endpoint for dash-testnet: its balance is read
        # from an endpoint added here.
        with serve(Node, fail=False) as node:
            spectra.run("endpoints", "add", "--chain", "dash-testnet", "--api", "blockbook", "--capabilities",
                        "balance,history,utxo,verification", node.url)
            spectra.run("wallet", "import", "--chain", "dash-testnet", "--name", "Custom DASH",
                        env={"SPECTRA_SEED": LEGAL, "SPECTRA_PASSWORD": "fixture-password"})
            balance = spectra.run("balance", "Custom DASH")
            assert (balance["amount"], balance["smallestUnit"]) == ("0.00123456", "123456"), balance
        print("A network with only a custom endpoint reads its balance from it")

        # The probe asks a node which network it is on: Ethereum's chain id on
        # Sepolia fails, Sepolia's passes.
        with evm("0x1") as node:
            spectra.run("endpoints", "add", "--chain", "ethereum-sepolia", "--api", "evm-json-rpc", "--capabilities",
                        "balance,fee,broadcast,verification", node.url)
            spectra.run("endpoints", "custom-only", "--chain", "ethereum-sepolia", "true")
            health = spectra.fail("endpoints", "probe", "--chain", "ethereum-sepolia", code=1)
            assert health["unreachable"] == 1 and "wrong network" in health["endpoints"][0]["detail"], health
            node.state["chain"] = hex(11155111)
            health = spectra.run("endpoints", "probe", "--chain", "ethereum-sepolia")
            assert health["ok"] and health["endpoints"][0]["detail"] == "network verified", health
            assert set(node.state["asked"]) == {"eth_chainId"}, node.state["asked"]
        print("A probe asks each endpoint its network")

    with scratch("endpoint-override") as directory, evm("0x1") as saved, evm("0x1") as named, \
            evm("0x89") as other:
        spectra = Spectra(directory, SPECTRA_PASSWORD="override-fixture-password", SPECTRA_SEED=LEGAL)
        spectra.run("wallet", "import", "--chain", "ethereum", "--name", "Named")
        spectra.run("endpoints", "add", "--chain", "ethereum", "--api", "evm-json-rpc", "--capabilities",
                    "balance,fee,broadcast,verification", saved.url)
        spectra.run("endpoints", "custom-only", "--chain", "ethereum", "true")

        def build(endpoint):
            return spectra.process("send", "build", "Named", "--to", "0x" + "22" * 20, "--amount", "0.01",
                                   "--endpoint", endpoint)

        # The endpoint the command names is the network's only one for that
        # process; the saved one is not asked.
        built = build(named.url)
        assert built.returncode == 0, (built.stdout, built.stderr)
        assert "eth_getTransactionCount" in named.state["asked"] and not saved.state["asked"], saved.state
        print("--endpoint reads through the named endpoint alone")

        # Signed against the named node, the send is refused at a node on another
        # network: it is asked its chain id and given nothing.
        artifact = spectra.run("send", "build", "Named", "--to", "0x" + "22" * 20, "--amount", "0.01",
                               "--endpoint", named.url)["artifact"]
        signed = spectra.run("send", "sign", artifact["id"], "--review-digest", artifact["review_digest"],
                             "--endpoint", named.url)["artifact"]
        failed = spectra.fail("send", "broadcast-signed", signed["id"], "--endpoint", other.url, "--yes")
        assert "wrong network" in str(failed), failed
        assert not other.state["submitted"] and other.state["asked"] == ["eth_chainId"], other.state
        assert not named.state["submitted"] and not saved.state["submitted"]
        print("A broadcast to a node on another network submits nothing")
