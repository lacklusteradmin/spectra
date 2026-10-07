#!/usr/bin/env python3
"""Monero (height, timestamp) checkpoints for core/data/monero-checkpoints.json.

Core turns a date into a restore height by taking the last checkpoint at or
before it, so a wallet never starts scanning after it was created: a Polyseed's
birthday, or the moment a wallet is created. The table needs no more than one
point a month from November 2021, Polyseed's epoch, onward.

Every header is read from two independent public daemons and must agree on
its hash. Read-only JSON-RPC (`get_block_header_by_height`, `get_info`).

Usage: generate-monero-checkpoints.py > core/data/monero-checkpoints.json
"""
import datetime, json, sys, urllib.request

NODES = {
    'monero': ['http://node.monerodevs.org:18089', 'http://node.sethforprivacy.com:18089'],
    'monero-stagenet': ['http://node.monerodevs.org:38089', 'http://stagenet.xmr-tw.org:38081'],
}
# A height before November 2021 on each network, and the spacing: 21,600
# blocks is 30 days at Monero's two-minute target.
START = {'monero': 2_480_000, 'monero-stagenet': 950_000}
STEP = 21_600
POLYSEED_EPOCH = 1_635_768_000


def rpc(node, method, params=None):
    body = json.dumps({'jsonrpc': '2.0', 'id': '0', 'method': method, 'params': params or {}}).encode()
    request = urllib.request.Request(node + '/json_rpc', body, {'Content-Type': 'application/json'})
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)['result']


def header(nodes, height):
    headers = [rpc(node, 'get_block_header_by_height', {'height': height})['block_header'] for node in nodes]
    assert len({h['hash'] for h in headers}) == 1, (height, headers)
    return headers[0]


def main():
    table = {}
    for network, nodes in NODES.items():
        tip = min(rpc(node, 'get_info')['height'] for node in nodes)
        first = header(nodes, START[network])
        assert first['timestamp'] <= POLYSEED_EPOCH, (network, first)
        points = []
        height = START[network]
        # Stop a day short of the tip, so every point is long settled.
        while height < tip - 720:
            block = header(nodes, height)
            points.append([height, block['timestamp'], block['hash']])
            height += STEP
        table[network] = points
    print(json.dumps({
        'provenance': 'scripts/generate-monero-checkpoints.py, ' + datetime.date.today().isoformat()
                      + ', headers agreed by ' + '; '.join(f'{k}: {", ".join(v)}' for k, v in NODES.items()),
        **table,
    }, indent=1))


main()
