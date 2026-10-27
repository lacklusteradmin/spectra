#!/usr/bin/env python3
"""A NEAR staged send's durable retries against a loopback node, across
processes: a gas price past the reviewed budget submits nothing; a reply
naming another transaction leaves it uncertain, and the retry submits the
same bytes; once the network reports it final, that outcome is recorded
before any stale nonce, expiry or fee is considered. Quotes, NEP-145
registration and the build and signing refusals are tested in core."""
from harness import ABANDON, Spectra, checked, scratch, serve, suite

staking = suite('staking')
UNIT = 10**24


class Node(staking.Node):
    """The staking suite's NEAR node, on mainnet, with the sender's account as
    a send reads it."""
    def near(self, method, params):
        s = self.state
        if method == 'status':
            return {'chain_id': 'mainnet'}
        if method == 'query' and params['request_type'] == 'view_account':
            return {'amount': str(s['balance']), 'locked': '0', 'storage_usage': 1000}
        return super().near(method, params)


with serve(Node, chain='near', owner='', nonce=7, balance=100*UNIT, gas_price=100000000, authority=True,
           foreign=True, final=False, succeeded=True, submitted=[], errors=[], hash=None) as node, \
        scratch('near-send') as directory:
    s, endpoint = node.state, node.url
    run = checked(Spectra(directory, SPECTRA_SEED=ABANDON), s['errors'])
    s['owner'] = run('wallet', 'import', '--chain', 'near', '--name', 'Near', '--no-password')['wallet']['address']
    run('endpoints', '--chain', 'near', '--api', 'near-json-rpc',
        '--capabilities', 'balance,fee,verification,token-balance,broadcast', '--add', endpoint)
    prepared = run('send', 'build', 'Near', '--to', '22'*32, '--amount', '1', '--endpoint', endpoint)['artifact']
    signed = run('send', 'sign', prepared['id'], '--review-digest', prepared['review_digest'],
                 '--endpoint', endpoint)['artifact']
    s['hash'] = signed['transaction_hash']
    assert signed == run('send', 'inspect', signed['id'])['artifact']
    broadcast = ('send', 'broadcast-signed', signed['id'], '--endpoint', endpoint, '--yes')
    # The signed bytes cap no gas price: past the reviewed budget, nothing is submitted.
    s['gas_price'] *= 2; run(*broadcast, success=False); s['gas_price'] //= 2
    assert not s['submitted']
    uncertain = run(*broadcast)['artifact']
    assert uncertain['attempts'][-1]['outcome'] == 'Uncertain', uncertain
    s['gas_price'] *= 2; run(*broadcast, success=False); s['gas_price'] //= 2
    assert len(s['submitted']) == 1
    s['foreign'] = False
    accepted = run(*broadcast)['artifact']
    assert accepted['attempts'][-1]['outcome'] == 'Accepted', accepted
    assert s['submitted'][0] == s['submitted'][1]
    # The final status is recovered before stale nonce, expiry and fee checks.
    s['final'] = True; s['expired'] = True; s['nonce'] += 1; s['gas_price'] *= 2; s['balance'] = 0
    refusal = run(*broadcast, success=False)
    assert 'already confirmed' in refusal['error'], refusal
    assert len(s['submitted']) == 2
    record = run('txs', '--record', signed['id'])['record']
    assert record['status'] == 'confirmed' and record['transactionHash'] == s['hash'], record
    print('near native: durable retries and final status recovery passed', flush=True)
