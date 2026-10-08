#!/usr/bin/env python3
"""XRP Ledger issued currencies and Stellar credit assets, against loopback
nodes. Two assets with one code and different issuers stay two assets in the
token list, the balances, discovery and history. A trust line is opened,
removed and paid through, each signed byte for byte as the network's own SDK
signs it (core/tests/fixtures/issued-assets.json); what the network would
refuse is refused before anything is built."""
import base64
import copy
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
OTHER = 'rDsbeomae4FXwgQTJp9Rs64Qg9vDiTCdBv'
DESTINATION = 'rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe'
SOLO = '534F4C4F00000000000000000000000000000000'
OTHER_STELLAR = 'GBUXQE5RNV267EEVS6COJSHRKIE52GFVVA66TMM7UNAYLAOZP36PZ7YX'


def stellar_secret(seed: bytes) -> str:
    """A Stellar secret seed's strkey: version 18 << 3, the seed, CRC16-XMODEM."""
    payload = bytes([18 << 3]) + seed
    crc = 0
    for byte in payload:
        crc ^= byte << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) & 0xFFFF if crc & 0x8000 else (crc << 1) & 0xFFFF
    return base64.b32encode(payload + crc.to_bytes(2, 'little')).decode().rstrip('=')


def xrp_line(peer, balance, limit='1000', currency='USD'):
    return {'account': peer, 'balance': balance, 'currency': currency, 'limit': limit, 'limit_peer': '0'}


def xrp_payment(hash_, sender, receiver, peer, low, before, after, currency='USD'):
    """A validated payment whose metadata moves `peer`'s currency on the
    holder's trust line, the line stored from its low side."""
    holder = XRPL['account']
    balance = lambda value: {'currency': currency, 'issuer': 'rrrrrrrrrrrrrrrrrrrrBZbvji', 'value': value}
    sides = {'LowLimit': {'issuer': holder if low else peer}, 'HighLimit': {'issuer': peer if low else holder}}
    nodes = [{'ModifiedNode': {
        'LedgerEntryType': 'RippleState', 'FinalFields': {'Balance': balance(after), **sides},
        'PreviousFields': {'Balance': balance(before)}}}]
    # The sender's root always pays the fee.
    nodes.append({'ModifiedNode': {'LedgerEntryType': 'AccountRoot', 'FinalFields': {
        'Account': sender, 'Balance': '49999988'}, 'PreviousFields': {'Balance': '50000000'}}})
    return {'tx': {'TransactionType': 'Payment', 'hash': hash_, 'Account': sender, 'Destination': receiver,
                   'Fee': '12', 'date': 800_000_000, 'ledger_index': 900},
            'meta': {'TransactionResult': 'tesSUCCESS', 'AffectedNodes': nodes}}


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
        state = self.state
        if path in ('', '/'):
            return self.reply({'network_passphrase': STELLAR['network_passphrase']})
        if path == '/ledgers':
            return self.reply({'_embedded': {'records': [{'sequence': 1000, 'base_reserve_in_stroops': 5_000_000}]}})
        if path == '/fee_stats':
            return self.reply({'fee_charged': {'mode': '100'}})
        if path.endswith('/payments'):
            return self.reply({'_embedded': {'records': state['stellar_payments']}})
        if path.startswith('/accounts/'):
            account = state['stellar'].get(path.split('/')[2])
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
        elif method == 'account_tx':
            result = {'transactions': state['account_tx']}
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
        self.journal = pathlib.Path(self.directory.name) / 'network.jsonl'
        Node.submitted.clear()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        assert not self.journal.exists() or not self.journal.read_text().strip(), self.journal.read_text()
        self.directory.cleanup()

    def run_cli(self, *args, success=True, env=None):
        p = subprocess.run([binary, '--data-dir', self.directory.name, '--json', *args], capture_output=True,
                           text=True, timeout=60,
                           env={**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(self.journal), **(env or {})})
        assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
        return json.loads(p.stdout) if success else (p.returncode, p.stdout + p.stderr)

    def refuses(self, words, *args):
        code, output = self.run_cli(*args, success=False)
        assert code == 3 and words in output, (words, code, output)

    def endpoint(self, chain, api, path):
        self.run_cli('endpoints', '--chain', chain, '--api', api, '--capabilities',
                     'balance,history,fee,broadcast,verification,token-balance,token-discovery,token-history',
                     '--add', self.base + path)
        self.run_cli('endpoints', '--chain', chain, '--custom-only', 'true')

    def holdings(self):
        with sqlite3.connect(pathlib.Path(self.directory.name) / 'spectra.sqlite') as db:
            wallet = json.loads(db.execute('SELECT payload FROM wallets').fetchone()[0])
        return {h.get('contractAddress'): h['amount'] for h in wallet['holdings']}

    def history_kinds(self):
        with sqlite3.connect(pathlib.Path(self.directory.name) / 'spectra.sqlite') as db:
            return [json.loads(row[0])['kind'] for row in db.execute('SELECT payload FROM history_records')]

    def sign_and_broadcast(self, artifact, path):
        signed = self.run_cli('send', 'sign', artifact['id'], '--review-digest', artifact['review_digest'],
                              '--endpoint', self.base + path)['artifact']
        self.run_cli('send', 'broadcast-signed', signed['id'], '--endpoint', self.base + path, '--yes')
        return signed

    def test_xrpl_issued_currencies_by_issuer(self):
        holder = XRPL['account']
        Node.state = {
            'roots': {
                holder: {'Balance': '50000000', 'Sequence': 9, 'OwnerCount': 1, 'Flags': 0},
                ISSUER: {'Balance': '100000000', 'Sequence': 1, 'OwnerCount': 9, 'Flags': 0},
                OTHER: {'Balance': '100000000', 'Sequence': 1, 'OwnerCount': 9, 'Flags': 0},
                DESTINATION: {'Balance': '30000000', 'Sequence': 1, 'OwnerCount': 1, 'Flags': 0},
            },
            'lines': {holder: [xrp_line(OTHER, '7.5')], DESTINATION: [xrp_line(ISSUER, '10')]},
            'account_tx': [],
        }
        imported = self.run_cli('wallet', 'import', '--chain', 'xrp', '--name', 'XRP', '--no-password',
                                '--private-key-env', 'XRP_KEY', env={'XRP_KEY': XRPL['key']})
        assert imported['wallet']['address'] == holder, imported
        self.endpoint('xrp', 'xrpl-json-rpc', '/xrpl')

        # One code, two issuers: two tokens. The hex spelling of USD is USD.
        usd, other_usd = f'USD.{ISSUER}', f'USD.{OTHER}'
        self.run_cli('token', 'add', '--chain', 'xrp', '--symbol', 'USD', '--name', 'GateHub USD', '--contract', usd)
        self.run_cli('token', 'add', '--chain', 'xrp', '--symbol', 'USD', '--name', 'Other USD', '--contract', other_usd)
        self.refuses('', 'token', 'add', '--chain', 'xrp', '--symbol', 'USD', '--name', 'Again',
                     '--contract', f'0000000000000000000000005553440000000000.{ISSUER}')
        self.refuses('', 'token', 'add', '--chain', 'xrp', '--symbol', 'XRP', '--name', 'Native',
                     '--contract', f'XRP.{ISSUER}')

        # Opening a trust line is TrustSet as ripple-binary-codec encodes it.
        lines = self.run_cli('wallet', 'trust-lines', 'XRP')['trust_lines']
        assert [(l['asset'], l['balance'], l['removalBlocked']) for l in lines['lines']] == [
            (other_usd, '7.5', 'The trust line still holds the token')], lines
        assert lines['reservePerLine'] == '0.2', lines
        trust = self.run_cli('wallet', 'trust', 'XRP', '--asset', usd)['artifact']
        assert trust['operation'] == {'kind': 'trust_asset', 'asset': usd, 'reserve': '0.2',
                                      'network_fee': '0.000012'}, trust['operation']
        signed = self.sign_and_broadcast(trust, '/xrpl')
        assert json.loads(signed['signed_payload'])['tx_blob_hex'] == XRPL['trust_sets'][0]['signed_hex']
        Node.state['lines'][holder].append(xrp_line(ISSUER, '150', '9999999999999999e80'))
        self.refuses('already trusts', 'wallet', 'trust', 'XRP', '--asset', usd)

        # Both balances, each under its own issuer.
        self.run_cli('refresh', '--wallet', 'XRP')
        held = self.holdings()
        assert (held.get(usd), held.get(other_usd)) == ('150', '7.5'), held
        discovered = self.run_cli('token', 'discover', '--wallet', 'XRP')
        assert {(t['contract'], t['balance']) for t in discovered['holdings']} >= {
            (usd, '150'), (other_usd, '7.5')}, discovered

        # A payment with the issuer's 0.2% rate: SendMax is the cost, rounded up.
        Node.state['roots'][ISSUER]['TransferRate'] = 1_002_000_000
        Node.state['roots'][holder]['Sequence'] = 7
        self.refuses('recipient has no trust line', 'send', 'build', '--from', 'XRP', '--to', DESTINATION,
                     '--contract', other_usd, '--amount', '1')
        self.refuses('16 significant digits', 'send', 'build', '--from', 'XRP', '--to', DESTINATION,
                     '--contract', usd, '--amount', '12345678901.234567')
        payment = self.run_cli('send', 'build', '--from', 'XRP', '--to', DESTINATION,
                               '--contract', usd, '--amount', '123.456')['artifact']
        assert payment['review']['transfer_terms'] == {'debited': '123.702912', 'received': '123.456',
                                                       'fee': '0.246912', 'hook_program': None, 'carried_native': None}, payment['review']
        # A rate raised before signing is a different payment.
        Node.state['roots'][ISSUER]['TransferRate'] = 1_005_000_000
        code, output = self.run_cli('send', 'sign', payment['id'], '--review-digest', payment['review_digest'],
                                    '--endpoint', self.base + '/xrpl', success=False)
        assert 'fee changed' in output, output
        Node.state['roots'][ISSUER]['TransferRate'] = 1_002_000_000
        signed = self.sign_and_broadcast(payment, '/xrpl')
        assert json.loads(signed['signed_payload'])['tx_blob_hex'] == XRPL['payments'][0]['signed_hex']

        # Removing an empty line is a TrustSet of zero.
        Node.state['lines'][holder].append(xrp_line(ISSUER, '0', '1000', SOLO))
        Node.state['roots'][holder]['Sequence'] = 10
        self.refuses('still holds', 'wallet', 'untrust', 'XRP', '--asset', other_usd)
        removal = self.run_cli('wallet', 'untrust', 'XRP', '--asset', f'{SOLO}.{ISSUER}')['artifact']
        signed = self.sign_and_broadcast(removal, '/xrpl')
        assert json.loads(signed['signed_payload'])['tx_blob_hex'] == XRPL['trust_sets'][1]['signed_hex']
        assert sorted(self.history_kinds()) == ['removeTrustLine', 'send', 'trustAsset'], self.history_kinds()

        # History names each issued currency by its issuer.
        Node.state['account_tx'] = [
            xrp_payment('A1' * 32, OTHER, holder, OTHER, True, '7.5', '12.5'),
            xrp_payment('B2' * 32, holder, DESTINATION, ISSUER, False, '-150', '-26.297088'),
        ]
        self.run_cli('history', 'XRP', '--save')
        rows = self.run_cli('txs', '--page', '--wallet', 'XRP')['page']['records']
        provider = {(row['deploymentId'], row['kind'], row['amount'], row['assetDisplayName'])
                    for row in rows if row.get('transactionHistorySource') == 'rust'}
        assert provider == {(f'xrp:trust line token:{other_usd}', 'receive', '5', 'Other USD'),
                            (f'xrp:trust line token:{usd}', 'send', '123.702912', 'GateHub USD')}, rows

    def test_stellar_credit_assets_by_issuer(self):
        source, issuer = STELLAR['source'], STELLAR['issuer']
        usdc, other_usdc = f'USDC:{issuer}', f'USDC:{OTHER_STELLAR}'
        long_asset = f'LONGASSET12:{issuer}'

        def line(code, issuer_, balance, limit='922337203685.4775807'):
            return {'balance': balance, 'limit': limit, 'buying_liabilities': '0.0000000',
                    'selling_liabilities': '0.0000000', 'is_authorized': True,
                    'asset_type': 'credit_alphanum4' if len(code) <= 4 else 'credit_alphanum12',
                    'asset_code': code, 'asset_issuer': issuer_}

        def account(sequence, lines, native='100.0000000'):
            return {'balances': [*lines, {'balance': native, 'asset_type': 'native',
                                          'buying_liabilities': '0.0000000', 'selling_liabilities': '0.0000000'}],
                    'sequence': str(sequence), 'subentry_count': len(lines), 'num_sponsoring': 0, 'num_sponsored': 0}

        Node.state = {'stellar_payments': [], 'stellar': {
            source: account(123456789014, [line('USDC', OTHER_STELLAR, '3.0000000')]),
            issuer: account(1, []),
            OTHER_STELLAR: account(1, []),
            STELLAR['destination']: account(1, [line('USDC', issuer, '0.0000000'),
                                                line('LONGASSET12', issuer, '0.0000000')]),
        }}
        self.run_cli('wallet', 'import', '--chain', 'stellar-testnet', '--name', 'XLM', '--no-password',
                     '--private-key-env', 'XLM_KEY',
                     env={'XLM_KEY': stellar_secret(bytes.fromhex(STELLAR['seed']))})
        self.endpoint('stellar-testnet', 'horizon', '/horizon')
        self.run_cli('token', 'add', '--chain', 'stellar-testnet', '--symbol', 'USDC', '--name', 'USDC',
                     '--contract', usdc)
        self.run_cli('token', 'add', '--chain', 'stellar-testnet', '--symbol', 'USDC', '--name', 'Other USDC',
                     '--contract', other_usdc)
        self.refuses('', 'token', 'add', '--chain', 'stellar-testnet', '--symbol', 'USDC', '--name', 'Again',
                     '--contract', f'USDC:{issuer.lower()}')
        tokens = [t for t in self.run_cli('token', 'list')['tokens']
                  if t['chain_id'] == 'stellar-testnet' and t['symbol'] == 'USDC']
        assert sorted((t['contract'], t['decimals']) for t in tokens) == sorted([(usdc, 7), (other_usdc, 7)]), tokens

        # ChangeTrust and the asset payment, as the Stellar SDK signs them.
        trust = self.run_cli('wallet', 'trust', 'XLM', '--asset', usdc)['artifact']
        assert trust['operation']['reserve'] == '0.5', trust['operation']
        signed = self.sign_and_broadcast(trust, '/horizon')
        assert json.loads(signed['signed_payload'])['signed_xdr_b64'] == STELLAR['trustlines'][0]['envelope_b64']
        Node.state['stellar'][source] = account(123456789012, [line('USDC', OTHER_STELLAR, '3.0000000'),
                                                               line('USDC', issuer, '100.0000000')])
        self.run_cli('refresh', '--wallet', 'XLM')
        held = self.holdings()
        assert (held.get(usdc), held.get(other_usdc)) == ('100', '3'), held
        self.refuses('recipient has no trustline', 'send', 'build', '--from', 'XLM', '--to', STELLAR['destination'],
                     '--contract', other_usdc, '--amount', '1')
        payment = self.run_cli('send', 'build', '--from', 'XLM', '--to', STELLAR['destination'],
                               '--contract', usdc, '--amount', '12.3456789')['artifact']
        assert payment['review']['transfer_terms'] is None, payment['review']
        signed = self.sign_and_broadcast(payment, '/horizon')
        assert json.loads(signed['signed_payload'])['signed_xdr_b64'] == STELLAR['payments'][0]['envelope_b64']

        # An empty trustline comes off with a limit of zero.
        Node.state['stellar'][source] = account(123456789015, [line('LONGASSET12', issuer, '0.0000000'),
                                                               line('USDC', issuer, '87.6543211')])
        self.refuses('still holds', 'wallet', 'untrust', 'XLM', '--asset', usdc)
        removal = self.run_cli('wallet', 'untrust', 'XLM', '--asset', long_asset)['artifact']
        signed = self.sign_and_broadcast(removal, '/horizon')
        assert json.loads(signed['signed_payload'])['signed_xdr_b64'] == STELLAR['trustlines'][1]['envelope_b64']
        assert sorted(self.history_kinds()) == ['removeTrustLine', 'send', 'trustAsset'], self.history_kinds()

        # A credit asset's history row is its own, by issuer.
        Node.state['stellar_payments'] = [
            {'type': 'payment', 'paging_token': '1', 'created_at': '2026-10-07T01:00:00Z',
             'transaction_hash': 'dd' * 32, 'asset_type': 'credit_alphanum4', 'asset_code': 'USDC',
             'asset_issuer': OTHER_STELLAR, 'from': OTHER_STELLAR, 'to': source, 'amount': '2.5000000'}]
        self.run_cli('history', 'XLM', '--save')
        rows = self.run_cli('txs', '--page', '--wallet', 'XLM')['page']['records']
        received = {(row['deploymentId'], row['amount'], row['assetDisplayName'])
                    for row in rows if row['kind'] == 'receive'}
        assert received == {(f'stellar-testnet:stellar asset:{other_usdc}', '2.5', 'Other USDC')}, rows


if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('Run without -O or PYTHONOPTIMIZE: assertions must remain enabled.')
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]], verbosity=2)
