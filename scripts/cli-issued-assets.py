#!/usr/bin/env python3
"""XRP Ledger issued currencies and Stellar credit assets, against loopback
nodes: on each network a trust line is opened, paid through and removed,
each staged transaction signed, broadcast once and recorded in history by
what it did. The rules themselves — trust lines, issuer fees, balances by
issuer, the signed bytes — are tested in core."""
import http.server
import json
import os
import pathlib
import sqlite3
import subprocess
import sys
import tempfile
import threading
import unittest

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
fixtures = json.loads((pathlib.Path(__file__).resolve().parents[1] / 'core/tests/fixtures/issued-assets.json').read_text())
XRPL, STELLAR = fixtures['xrpl'], fixtures['stellar']
ISSUER = XRPL['issuer']
DESTINATION = 'rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe'
SOLO = '534F4C4F00000000000000000000000000000000'


def xrp_line(peer, balance, currency='USD'):
    return {'account': peer, 'balance': balance, 'currency': currency, 'limit': '1000', 'limit_peer': '0'}


class Node(http.server.BaseHTTPRequestHandler):
    """An XRPL JSON-RPC node under /xrpl and a Horizon under /horizon."""
    state = {}
    submitted = []

    def log_message(self, *_):
        pass

    def reply(self, payload, status=200):
        data = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        path = self.path.removeprefix('/horizon').split('?')[0]
        if path in ('', '/'):
            return self.reply({'network_passphrase': STELLAR['network_passphrase']})
        if path == '/ledgers':
            return self.reply({'_embedded': {'records': [{'sequence': 1000, 'base_reserve_in_stroops': 5_000_000}]}})
        if path == '/fee_stats':
            return self.reply({'fee_charged': {'mode': '100'}})
        if path.startswith('/accounts/'):
            account = self.state['stellar'].get(path.split('/')[2])
            return self.reply(account) if account else self.reply({'status': 404}, 404)
        raise AssertionError(self.path)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        if self.path.startswith('/horizon/transactions'):
            self.submitted.append(body['tx'])
            return self.reply({'hash': 'cc' * 32})
        method, params, state = body['method'], (body.get('params') or [{}])[0], self.state
        if method == 'server_info':
            result = {'info': {'network_id': 0}}
        elif method == 'server_state':
            result = {'state': {'validated_ledger': {'seq': 1000, 'reserve_base': 1_000_000, 'reserve_inc': 200_000}}}
        elif method == 'fee':
            result = {'drops': {'open_ledger_fee': '12'}}
        elif method == 'account_info':
            root = state['roots'].get(params['account'])
            result = {'account_data': root} if root else {'error': 'actNotFound', 'status': 'error'}
        elif method == 'account_lines':
            account, peer = params['account'], params.get('peer')
            if account not in state['roots']:
                result = {'error': 'actNotFound', 'status': 'error'}
            else:
                lines = [line for line in state['lines'].get(account, []) if peer in (None, line['account'])]
                result = {'account': account, 'lines': lines}
        elif method == 'submit':
            self.submitted.append(params['tx_blob'])
            result = {'engine_result': 'tesSUCCESS', 'accepted': True, 'tx_json': {'hash': 'AB' * 32}}
        else:
            raise AssertionError(body)
        self.reply({'result': result})


class IssuedAssetTests(unittest.TestCase):
    def setUp(self):
        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.base = f'http://127.0.0.1:{self.server.server_port}'
        self.directory = tempfile.TemporaryDirectory(prefix='spectra-issued-')
        Node.submitted.clear()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.directory.cleanup()

    def run_cli(self, *args, env=None):
        p = subprocess.run([binary, '--data-dir', self.directory.name, '--json', *args], capture_output=True,
                           text=True, timeout=60, env={**os.environ, **(env or {})})
        assert p.returncode == 0, (args, p.stdout, p.stderr)
        return json.loads(p.stdout)

    def endpoint(self, chain, api, path):
        self.run_cli('endpoints', '--chain', chain, '--api', api, '--capabilities',
                     'balance,history,fee,broadcast,verification,token-balance', '--add', self.base + path)
        self.run_cli('endpoints', '--chain', chain, '--custom-only', 'true')

    def history_kinds(self):
        with sqlite3.connect(pathlib.Path(self.directory.name) / 'spectra.sqlite') as db:
            return sorted(json.loads(row[0])['kind'] for row in db.execute('SELECT payload FROM history_records'))

    def sign_and_broadcast(self, artifact, path, field):
        """Signs the reviewed transaction and broadcasts it: the node receives
        exactly the signed bytes, once."""
        before = len(Node.submitted)
        signed = self.run_cli('send', 'sign', artifact['id'], '--review-digest', artifact['review_digest'],
                              '--endpoint', self.base + path)['artifact']
        assert len(Node.submitted) == before, 'signing broadcasts nothing'
        self.run_cli('send', 'broadcast-signed', signed['id'], '--endpoint', self.base + path, '--yes')
        assert Node.submitted[before:] == [json.loads(signed['signed_payload'])[field]], Node.submitted

    def test_xrpl_trust_pay_and_remove(self):
        holder = XRPL['account']
        Node.state = {
            'roots': {
                holder: {'Balance': '50000000', 'Sequence': 9, 'OwnerCount': 1, 'Flags': 0},
                ISSUER: {'Balance': '100000000', 'Sequence': 1, 'OwnerCount': 9, 'Flags': 0},
                DESTINATION: {'Balance': '30000000', 'Sequence': 1, 'OwnerCount': 1, 'Flags': 0},
            },
            'lines': {holder: [], DESTINATION: [xrp_line(ISSUER, '10')]},
        }
        self.run_cli('wallet', 'import', '--chain', 'xrp', '--name', 'XRP', '--no-password',
                     '--private-key-env', 'XRP_KEY', env={'XRP_KEY': XRPL['key']})
        self.endpoint('xrp', 'xrpl-json-rpc', '/xrpl')
        usd = f'USD.{ISSUER}'

        trust = self.run_cli('wallet', 'trust', 'XRP', '--asset', usd)['artifact']
        self.sign_and_broadcast(trust, '/xrpl', 'tx_blob_hex')
        Node.state['lines'][holder] = [xrp_line(ISSUER, '150'), xrp_line(ISSUER, '0', SOLO)]
        Node.state['roots'][holder]['Sequence'] = 10
        payment = self.run_cli('send', 'build', '--from', 'XRP', '--to', DESTINATION,
                               '--contract', usd, '--amount', '123.456')['artifact']
        self.sign_and_broadcast(payment, '/xrpl', 'tx_blob_hex')
        Node.state['roots'][holder]['Sequence'] = 11
        removal = self.run_cli('wallet', 'untrust', 'XRP', '--asset', f'{SOLO}.{ISSUER}')['artifact']
        self.sign_and_broadcast(removal, '/xrpl', 'tx_blob_hex')
        assert self.history_kinds() == ['removeTrustLine', 'send', 'trustAsset'], self.history_kinds()

    def test_stellar_trust_pay_and_remove(self):
        source, issuer = STELLAR['source'], STELLAR['issuer']
        usdc = f'USDC:{issuer}'

        def line(code, balance):
            return {'balance': balance, 'limit': '922337203685.4775807', 'buying_liabilities': '0.0000000',
                    'selling_liabilities': '0.0000000', 'is_authorized': True,
                    'asset_type': 'credit_alphanum4' if len(code) <= 4 else 'credit_alphanum12',
                    'asset_code': code, 'asset_issuer': issuer}

        def account(sequence, lines):
            return {'balances': [*lines, {'balance': '100.0000000', 'asset_type': 'native',
                                          'buying_liabilities': '0.0000000', 'selling_liabilities': '0.0000000'}],
                    'sequence': str(sequence), 'subentry_count': len(lines), 'num_sponsoring': 0, 'num_sponsored': 0}

        Node.state = {'stellar': {
            source: account(123456789014, []),
            issuer: account(1, []),
            STELLAR['destination']: account(1, [line('USDC', '0.0000000')]),
        }}
        self.run_cli('wallet', 'import', '--chain', 'stellar-testnet', '--name', 'XLM', '--no-password',
                     '--private-key-env', 'XLM_KEY', env={'XLM_KEY': STELLAR['seed']})
        self.endpoint('stellar-testnet', 'horizon', '/horizon')

        trust = self.run_cli('wallet', 'trust', 'XLM', '--asset', usdc)['artifact']
        self.sign_and_broadcast(trust, '/horizon', 'signed_xdr_b64')
        Node.state['stellar'][source] = account(123456789015, [line('USDC', '100.0000000'),
                                                               line('LONGASSET12', '0.0000000')])
        payment = self.run_cli('send', 'build', '--from', 'XLM', '--to', STELLAR['destination'],
                               '--contract', usdc, '--amount', '12.3456789')['artifact']
        self.sign_and_broadcast(payment, '/horizon', 'signed_xdr_b64')
        Node.state['stellar'][source]['sequence'] = '123456789016'
        removal = self.run_cli('wallet', 'untrust', 'XLM', '--asset', f'LONGASSET12:{issuer}')['artifact']
        self.sign_and_broadcast(removal, '/horizon', 'signed_xdr_b64')
        assert self.history_kinds() == ['removeTrustLine', 'send', 'trustAsset'], self.history_kinds()


if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('Run without -O or PYTHONOPTIMIZE: assertions must remain enabled.')
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]], verbosity=2)
