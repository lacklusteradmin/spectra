#!/usr/bin/env python3
"""A staged EVM send across processes, against a loopback node.

Build, sign and broadcast each run in their own process on one store: the
signed send is read back as signed, nothing reaches the node until
`broadcast-signed --yes`, and a retry sends the same bytes. Two processes
racing to sign sends at one nonce: one wins, the other is refused. What each
stage checks is core's, tested with `cargo test`.
"""
import concurrent.futures

from harness import ABANDON, JsonRpc, Spectra, scratch, serve


class Node(JsonRpc):
    def call(self, method, params):
        if method == 'eth_sendRawTransaction':
            self.state['submitted'].append(params[0])
        values = {'eth_chainId': '0x1', 'eth_getTransactionCount': '0x7', 'eth_getBalance': hex(10**37),
                  'eth_estimateGas': '0x5208', 'eth_getCode': '0x',
                  'eth_feeHistory': {'baseFeePerGas': ['0x3b9aca00'], 'reward': [['0x77359400']]},
                  'eth_sendRawTransaction': self.state['expected']}
        assert method in values, method
        return values[method]


with serve(Node, expected='', submitted=[]) as node, scratch('stages') as directory:
    state, url = node.state, node.url
    spectra = Spectra(directory, SPECTRA_PASSWORD='stage-fixture-password', SPECTRA_SEED=ABANDON)
    spectra.run('wallet', 'import', '--chain', 'ethereum', '--name', 'Stages')
    spectra.run('endpoints', '--chain', 'ethereum', '--api', 'evm-json-rpc',
                '--capabilities', 'balance,fee,broadcast,verification,token-balance', '--add', url)

    def build():
        return spectra.run('send', 'build', 'Stages', '--to', '0x' + '22' * 20, '--amount', '0.01',
                           '--endpoint', url)['artifact']

    def sign(artifact):
        return spectra.process('send', 'sign', artifact['id'], '--review-digest', artifact['review_digest'],
                               '--endpoint', url)

    built = build()
    assert built['stage'] == 'Prepared' and not state['submitted'], built
    signed = spectra.run('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                         '--endpoint', url)['artifact']
    assert signed['stage'] == 'Signed' and signed['signed_payload'] and not state['submitted'], signed
    assert spectra.run('send', 'inspect', signed['id'])['artifact'] == signed

    state['expected'] = signed['transaction_hash']
    sent = spectra.run('send', 'broadcast-signed', signed['id'], '--endpoint', url, '--yes')['artifact']
    assert [attempt['outcome'] for attempt in sent['attempts']] == ['Accepted'], sent
    retried = spectra.run('send', 'broadcast-signed', signed['id'], '--endpoint', url, '--yes')['artifact']
    assert [attempt['outcome'] for attempt in retried['attempts']] == ['Accepted', 'Accepted'], retried
    assert state['submitted'] == [signed['signed_payload']] * 2, state['submitted']
    assert spectra.run('send', 'inspect', signed['id'])['artifact'] == retried

    # Both contenders are built at the nonce after the signed send's.
    contenders = [build(), build()]
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as workers:
        raced = list(workers.map(sign, contenders))
    assert sum(r.returncode == 0 for r in raced) == 1, [(r.stdout, r.stderr) for r in raced]
    assert any('reserved' in r.stdout for r in raced if r.returncode), [(r.stdout, r.stderr) for r in raced]
    assert len(state['submitted']) == 2, state['submitted']
print('transparent send stage acceptance passed')
