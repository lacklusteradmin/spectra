#!/usr/bin/env python3
"""A TON jetton staged send against loopback TON Center v2 and v3 nodes,
across processes: built, signed, broadcast once as signed, and resolved from
its trace by the locally derived message hash — not final while the trace is
incomplete, failed when its root transaction aborted, and resolved again by
a fresh process after a lost status write. Token reads, payloads and the
build and signing refusals of every token network are tested in core."""
import base64
import urllib.parse

from harness import Handler, Spectra, fixture, scratch, serve

# The 0x01 * 32 key's TON account, as a W5 wallet.
OWNER = '0:9d1e1843624c4d175a695a8c2de8a5a61f03b93336e8caa4e164bf6cbbab205e'
CONTRACT = '0:' + '44' * 32
DESTINATION = '0:' + '22' * 32
JETTON_WALLET = '0:' + '33' * 32


class Node(Handler):
    def do_GET(self):
        live = self.state
        url = urllib.parse.urlsplit(self.path)
        path, query = url.path, urllib.parse.parse_qs(url.query)
        if path == '/getAddressBalance':
            self.reply({'ok': True, 'result': '10000000000'})
        elif path == '/getMasterchainInfo':
            self.reply({'ok': True, 'result': {'init': {'workchain': -1, 'seqno': 0,
                'root_hash': 'F6OpKZKqvqeFp6CQmFomXNMfMj2EnaUSOXN+Mh+wVWk=',
                'file_hash': 'XplPz01CXAps5qeSWUtxcyBfdAo5zVb1N979KLSKD24='}}})
        elif path == '/getAddressInformation':
            self.reply({'ok': True, 'result': {'state': 'active'}})
        elif path == '/v3/masterchainInfo':
            self.reply({'first': {'workchain': -1, 'shard': '8000000000000000', 'seqno': 1, 'global_id': -239,
                'root_hash': '8GYhhrigd8CwZGrRT59iulLDcgiTYuvOAzFJxugc0Ts=',
                'file_hash': 'V+XzykEwun4yePZhAEPZk77RbMfMOgS/S4GiJkSKY6s='}})
        elif path == '/v3/jetton/masters':
            self.reply({'jetton_masters': [{'jetton_content': {'decimals': '6'}}]})
        elif path == '/v3/jetton/wallets':
            self.reply({'jetton_wallets': [{'address': JETTON_WALLET, 'owner': query['owner_address'][0],
                                            'jetton': CONTRACT, 'balance': '223456789'}],
                        'metadata': {CONTRACT: {'token_info': [{'type': 'jetton_masters',
                                                                'extra': {'decimals': '6'}}]}}})
        elif path == '/v3/traces':
            assert query['msg_hash'] == [base64.b64decode(live['external_hash']).hex()], query
            live['traces'] += 1
            self.reply(live['trace_response'])
        else:
            raise AssertionError(self.path)

    def do_POST(self):
        live = self.state
        body = self.body()
        path = urllib.parse.urlsplit(self.path).path
        if path == '/runGetMethod':
            return self.reply({'ok': True, 'result': {'exit_code': 0, 'stack': [['num', hex(7)]]}})
        assert path == '/sendBocReturnHash', self.path
        live['submitted'].append(body['boc'])
        self.reply({'ok': True, 'result': {'hash': live['external_hash']}})


def trace_for(external_hash):
    """The fixture's jetton transfer trace, as this send's: its sender, master,
    jetton wallets, amount and external message."""
    vectors = fixture('ton-status-v3.json')
    details = vectors['traces'][0]['actions'][0]['details']
    replacements = {details['sender']: OWNER, details['receiver']: DESTINATION, details['asset']: CONTRACT,
                    details['sender_jetton_wallet']: JETTON_WALLET,
                    details['receiver_jetton_wallet']: '0:' + '55' * 32,
                    vectors['traces'][0]['external_hash']: external_hash}

    def replace(value):
        if isinstance(value, str): return replacements.get(value, value)
        if isinstance(value, list): return [replace(row) for row in value]
        if isinstance(value, dict): return {key: replace(row) for key, row in value.items()}
        return value
    vectors = replace(vectors)
    trace = vectors['traces'][0]
    trace['actions'][0]['details']['amount'] = '123456789'
    trace['actions'][0]['details']['query_id'] = '7'
    return vectors


with serve(Node, external_hash=None, trace_response=None, submitted=[], traces=0) as node, \
        scratch('jetton-send') as directory:
    live, endpoint = node.state, node.url
    spectra = Spectra(directory, SPECTRA_PRIVATE_KEY='01' * 32, SPECTRA_PASSWORD='token-vector-password')
    run = spectra.run

    def poll():
        return run('txs', '--poll-chain', 'ton')['changes']

    run('wallet', 'import', '--chain', 'ton', '--name', 'Token', '--private-key-env', 'SPECTRA_PRIVATE_KEY')
    run('endpoints', '--chain', 'ton', '--api', 'toncenter-v2',
        '--capabilities', 'balance,fee,verification,token-balance,broadcast', '--add', endpoint)
    run('endpoints', '--chain', 'ton', '--api', 'toncenter-v3',
        '--capabilities', 'verification,token-balance,token-discovery', '--add', endpoint + '/v3')
    prepared = run('send', 'build', 'Token', '--to', DESTINATION, '--endpoint', endpoint,
                   '--contract', CONTRACT, '--decimals', '6', '--amount', '123.456789')['artifact']
    signed = run('send', 'sign', prepared['id'], '--review-digest', prepared['review_digest'],
                 '--endpoint', endpoint)['artifact']
    assert run('send', 'inspect', signed['id'])['artifact'] == signed
    # The durable hash is the locally derived external-message hash; the
    # trace names its transactions by other hashes.
    live['external_hash'] = signed['transaction_hash']
    assert live['external_hash'] and not live['submitted'], signed
    live['trace_response'] = trace_for(live['external_hash'])
    trace = live['trace_response']['traces'][0]
    trace['is_incomplete'] = True
    run('send', 'broadcast-signed', signed['id'], '--endpoint', endpoint, '--yes')
    assert live['submitted'] == [signed['signed_payload']], live['submitted']
    assert poll() == [], 'an incomplete trace is not final'
    trace['is_incomplete'] = False
    root = trace['transactions'][trace['trace']['tx_hash']]
    root['description']['aborted'] = True
    changes = poll()
    assert len(changes) == 1 and changes[0]['newStatus'] == 'failed', changes
    # Lose the final status write: a fresh process resolves the same
    # external message from its trace again.
    spectra.lose_status(signed['id'])
    root['description']['aborted'] = False
    changes = poll()
    assert len(changes) == 1 and changes[0]['newStatus'] == 'confirmed', changes
    assert live['traces'] == 3 and len(live['submitted']) == 1, live
    print('TON jetton staged send: broadcast once, resolved from its trace across processes')
