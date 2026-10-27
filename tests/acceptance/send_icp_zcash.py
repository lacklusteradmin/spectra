#!/usr/bin/env python3
"""ICP and transparent Zcash sends staged across processes, against loopback
providers; no real funds or keys.

Each network's transfer is built, signed and broadcast in its own process:
nothing is submitted before `broadcast-signed --yes`, the signed send is read
back as signed, and a retry submits the same bytes. What each stage checks
is core's, tested with `cargo test`.
"""
import json

from harness import ABANDON, Handler, Spectra, scratch, serve

NETWORK = {'blockchain': 'Internet Computer', 'network': '00000000000000020101'}


class Node(Handler):
    def do_GET(self):
        state = self.state
        if self.path == '/api/v2/block-index/0':
            return self.reply({'blockHash': state['genesis']})
        if self.path == '/api/v2':
            return self.reply({'backend': {'blocks': state['height'],
                                           'consensus': {'chaintip': state['branch'], 'nextblock': state['branch']}}})
        if self.path.startswith('/api/v2/utxo/'):
            return self.reply([{'txid': '11' * 32, 'vout': 2, 'value': '1000000', 'confirmations': 100}])
        raise AssertionError(self.path)

    def do_POST(self):
        state = self.state
        raw = self.raw_body()
        if self.path == '/api/v2/sendtx/':
            state['submitted'].append(raw.decode())
            return self.reply({'result': state['expected']})
        body = json.loads(raw)
        if self.path == '/network/list':
            return self.reply({'network_identifiers': [NETWORK]})
        if self.path == '/construction/preprocess':
            return self.reply({'options': {'request_types': ['TRANSACTION']}})
        if self.path == '/construction/metadata':
            return self.reply({'suggested_fee': [{'value': '10000', 'currency': {'symbol': 'ICP', 'decimals': 8}}]})
        if self.path == '/construction/submit':
            assert body['network_identifier'] == NETWORK
            assert body['signed_transaction']
            state['submitted'].append(body['signed_transaction'])
            return self.reply({'transaction_identifier': {'hash': state['expected']}})
        raise AssertionError(self.path)


# NU6.3 (Ironwood) is the network's upgrade since block 3,428,143.
with serve(Node, submitted=[], expected='', branch='37a5165b', height=3500000,
           genesis='00040fe8ec8471911baa1db1266ea15dd06b4a8a5c453883c000b031973dce08') as node, \
        scratch('icp-zec') as directory:
    state, url = node.state, node.url
    spectra = Spectra(directory, SPECTRA_PASSWORD='fixture-password', SPECTRA_SEED=ABANDON)
    for chain, name in [('internet-computer', 'IcpStages'), ('zcash', 'ZecStages')]:
        spectra.run('wallet', 'import', '--chain', chain, '--name', name)
        icp = chain == 'internet-computer'
        spectra.run('endpoints', '--chain', chain, '--api', 'icp-rosetta' if icp else 'blockbook',
                    '--capabilities', 'balance,history,fee,broadcast,verification' + ('' if icp else ',utxo'),
                    '--add', url)
        # Sending back to the derived account is a protocol fixture, no real network.
        destination = spectra.run('send', 'identity', name)['address']
        before = len(state['submitted'])
        prepared = spectra.run('send', 'build', name, '--to', destination, '--amount', '0.001',
                               '--endpoint', url)['artifact']
        assert prepared['stage'] == 'Prepared' and len(state['submitted']) == before
        signed = spectra.run('send', 'sign', prepared['id'], '--review-digest', prepared['review_digest'],
                             '--endpoint', url)['artifact']
        assert signed['stage'] == 'Signed' and len(state['submitted']) == before
        assert spectra.run('send', 'inspect', prepared['id'])['artifact'] == signed
        state['expected'] = signed['transaction_hash']
        assert len(state['expected']) == 64
        sent = spectra.run('send', 'broadcast-signed', prepared['id'], '--endpoint', url, '--yes')['artifact']
        assert sent['attempts'][-1]['outcome'] == 'Accepted', sent
        spectra.run('send', 'broadcast-signed', prepared['id'], '--endpoint', url, '--yes')
        assert len(state['submitted']) == before + 2 and state['submitted'][-1] == state['submitted'][-2]
        print(f'PASS {chain}: build, sign and broadcast in separate processes, and an identical retry')
