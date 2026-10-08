#!/usr/bin/env python3
"""XRP destination tags and Stellar memos against loopback nodes. A payment
to an account that asks for one (XRP's lsfRequireDestTag, Stellar's SEP-29
config.memo_required) is refused without one, before anything is read for
the transaction and again before signing; a malformed or foreign one is
refused before any request; the one given is reviewed, bound into the review
digest, and signed exactly as each network's SDK signs it — payments and
account closings alike."""
import base64
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
vectors = json.loads((pathlib.Path(__file__).resolve().parents[1] / 'core/tests/fixtures/payment-memos.json').read_text())
XRPL, STELLAR = vectors['xrpl'], vectors['stellar']
XRP_SENDER = XRPL['payments'][0]['transaction']['Account']
XRP_DESTINATION = XRPL['payments'][0]['transaction']['Destination']
REQUIRE_DEST_TAG = 0x00020000


def stellar_secret(seed: bytes) -> str:
    """A Stellar secret seed's strkey: version 18 << 3, the seed, CRC16-XMODEM."""
    payload = bytes([18 << 3]) + seed
    crc = 0
    for byte in payload:
        crc ^= byte << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) & 0xFFFF if crc & 0x8000 else (crc << 1) & 0xFFFF
    return base64.b32encode(payload + crc.to_bytes(2, 'little')).decode().rstrip('=')


class Node(http.server.BaseHTTPRequestHandler):
    """An XRPL JSON-RPC node under /xrpl and a Horizon under /horizon."""
    state = {}
    requests = []

    def log_message(self, *_):
        pass

    def reply(self, payload, status=200):
        data = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        self.requests.append(self.path)
        path = self.path.removeprefix('/horizon')
        if path in ('/', ''):
            return self.reply({'network_passphrase': STELLAR['network_passphrase']})
        if path.startswith('/ledgers'):
            return self.reply({'_embedded': {'records': [
                {'sequence': 1_000, 'base_reserve_in_stroops': 5_000_000}]}})
        if path.startswith('/fee_stats'):
            return self.reply({'fee_charged': {'mode': '100'}})
        if path.startswith('/accounts/'):
            account = self.state['stellar'].get(path.split('/')[2])
            return self.reply(account) if account else self.reply({'status': 404}, 404)
        raise AssertionError(self.path)

    def do_POST(self):
        call = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        method = call['method']
        params = (call.get('params') or [{}])[0]
        self.requests.append(method)
        accounts = self.state['xrp']
        if method == 'server_info':
            result = {'info': {'network_id': 0}}
        elif method == 'fee':
            result = {'drops': {'open_ledger_fee': '12'}}
        elif method == 'server_state':
            result = {'state': {'validated_ledger': {
                'seq': 93_000_300, 'reserve_base': 1_000_000, 'reserve_inc': 200_000}}}
        elif method == 'account_info':
            root = accounts.get(params['account'])
            result = {'account_data': root} if root else {'error': 'actNotFound', 'status': 'error'}
        elif method == 'account_objects':
            result = {'account_objects': []}
        else:
            raise AssertionError(call)
        self.reply({'result': result})


class PaymentMemoTests(unittest.TestCase):
    def setUp(self):
        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.base = f'http://127.0.0.1:{self.server.server_port}'
        self.directory = tempfile.TemporaryDirectory(prefix='spectra-memos-')
        self.journal = pathlib.Path(self.directory.name) / 'network.jsonl'
        Node.requests.clear()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        assert not self.journal.exists() or not self.journal.read_text().strip(), self.journal.read_text()
        self.directory.cleanup()

    def cli(self, *args, success=True, env=None):
        p = subprocess.run([binary, '--data-dir', self.directory.name, '--json', *args], capture_output=True,
                           text=True, timeout=60,
                           env={**os.environ, 'SPECTRA_LOOPBACK_ONLY': str(self.journal), **(env or {})})
        assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
        return json.loads(p.stdout) if success else p.stdout + p.stderr

    def endpoint(self, chain, api, path):
        self.cli('endpoints', '--chain', chain, '--api', api, '--capabilities',
                 'balance,fee,broadcast,verification', '--add', self.base + path)
        self.cli('endpoints', '--chain', chain, '--custom-only', 'true')

    def refuses(self, words, *args, reads=None):
        """Refused with `words`; with `reads` given, having made only those requests."""
        Node.requests.clear()
        output = self.cli(*args, success=False)
        assert words in output, (words, output)
        if reads is not None:
            assert Node.requests == reads, (args, Node.requests)
        assert self.cli('send', 'list')['artifacts'] == [], 'a refusal builds nothing'

    def sign(self, artifact):
        return self.cli('send', 'sign', artifact['id'], '--review-digest', artifact['review_digest'])['artifact']

    def tamper(self, artifact, value):
        """Change the stored artifact's memo, view and request alike."""
        with sqlite3.connect(pathlib.Path(self.directory.name) / 'spectra.sqlite') as db:
            data = json.loads(db.execute('SELECT payload FROM send_artifacts WHERE id=?',
                                         (artifact['id'],)).fetchone()[0])
            data['view']['memo']['value'] = value
            data['request']['memo']['value'] = value
            db.execute('UPDATE send_artifacts SET payload=? WHERE id=?', (json.dumps(data), artifact['id']))

    def test_xrp_destination_tags(self):
        Node.state = {'xrp': {
            XRP_SENDER: {'Balance': '25000000', 'Sequence': 7, 'OwnerCount': 0, 'Flags': 0},
            XRP_DESTINATION: {'Balance': '50000000', 'Sequence': 5, 'OwnerCount': 0, 'Flags': REQUIRE_DEST_TAG},
        }}
        self.cli('wallet', 'import', '--chain', 'xrp', '--name', 'XRP', '--no-password',
                 '--private-key-env', 'XRP_KEY', env={'XRP_KEY': XRPL['key']})
        self.endpoint('xrp', 'xrpl-json-rpc', '/xrpl')
        send = ('send', 'build', '--from', 'XRP', '--to', XRP_DESTINATION, '--amount', '1')

        # The destination asks for a tag: refused having asked only that.
        self.refuses('requires a destination tag', *send, reads=['server_info', 'account_info'])
        # A tag out of range, malformed, or a Stellar memo: refused before any request.
        for flag, value, words in [('--destination-tag', '4294967296', 'whole number from 0 to 4294967295'),
                                   ('--destination-tag', '-1', 'whole number'),
                                   ('--destination-tag', '12a', 'whole number'),
                                   ('--memo-text', 'deposit', 'no such destination tag or memo')]:
            self.refuses(words, *send, f'{flag}={value}', reads=[])

        # Tag 0 is a tag: built, reviewed and signed as ripple-binary-codec
        # and ripple-keypairs sign it.
        built = self.cli(*send, '--destination-tag', '0')['artifact']
        assert built['memo'] == {'kind': 'destinationTag', 'value': '0'}, built
        signed = self.sign(built)
        blob = json.loads(signed['signed_payload'])['tx_blob_hex']
        assert blob == XRPL['payments'][0]['signed_hex'], blob

        # A tag changed after the review is refused, and so is signing it.
        tagged = self.cli(*send, '--destination-tag', '77')['artifact']
        self.tamper(tagged, '78')
        assert 'altered' in self.cli('send', 'inspect', tagged['id'], success=False)
        assert 'altered' in self.cli('send', 'sign', tagged['id'], '--review-digest', tagged['review_digest'],
                                     success=False)

        # A destination that starts asking for a tag after the review is
        # read again before signing.
        Node.state['xrp'][XRP_DESTINATION]['Flags'] = 0
        untagged = self.cli(*send)['artifact']
        assert untagged['memo'] is None, untagged
        Node.state['xrp'][XRP_DESTINATION]['Flags'] = REQUIRE_DEST_TAG
        assert 'requires a destination tag' in self.cli(
            'send', 'sign', untagged['id'], '--review-digest', untagged['review_digest'], success=False)

    def test_xrp_closes_into_an_account_that_asks_for_a_tag(self):
        vector = XRPL['account_delete']
        transaction = vector['transaction']
        Node.state = {'xrp': {
            XRP_SENDER: {'Balance': '25000000', 'Sequence': transaction['Sequence'], 'OwnerCount': 0, 'Flags': 0},
            XRP_DESTINATION: {'Balance': '50000000', 'Sequence': 5, 'OwnerCount': 0, 'Flags': REQUIRE_DEST_TAG},
        }}
        self.cli('wallet', 'import', '--chain', 'xrp', '--name', 'XRP', '--no-password',
                 '--private-key-env', 'XRP_KEY', env={'XRP_KEY': XRPL['key']})
        self.endpoint('xrp', 'xrpl-json-rpc', '/xrpl')
        self.refuses('requires a destination tag', 'wallet', 'close', 'XRP', '--to', XRP_DESTINATION)
        built = self.cli('wallet', 'close', 'XRP', '--to', XRP_DESTINATION,
                         '--destination-tag', str(transaction['DestinationTag']))['artifact']
        assert built['memo'] == {'kind': 'destinationTag', 'value': '2468'}, built
        blob = json.loads(self.sign(built)['signed_payload'])['tx_blob_hex']
        assert blob == vector['signed_hex'], blob

    def stellar_wallet(self, sequence, memo_required):
        def account(sequence, data):
            return {'balances': [{'balance': '5.0000000', 'asset_type': 'native'}], 'sequence': sequence,
                    'subentry_count': 0, 'num_sponsoring': 0, 'num_sponsored': 0,
                    'flags': {'auth_immutable': False}, 'data': data}
        Node.state = {'stellar': {
            STELLAR['source']: account(str(int(sequence) - 1), {}),
            STELLAR['destination']: account('1', {'config.memo_required': 'MQ=='} if memo_required else {}),
        }}
        self.cli('wallet', 'import', '--chain', 'stellar-testnet', '--name', 'XLM', '--no-password',
                 '--private-key-env', 'XLM_KEY', env={'XLM_KEY': stellar_secret(bytes.fromhex(STELLAR['seed']))})
        self.endpoint('stellar-testnet', 'horizon', '/horizon')

    def test_stellar_memos(self):
        payments = {(p['kind'], p['memo']): p for p in STELLAR['payments'] if p['asset'] is None}
        first = payments[('memoText', 'a')]
        self.stellar_wallet(first['sequence'], memo_required=True)
        send = ('send', 'build', '--from', 'XLM', '--to', STELLAR['destination'], '--amount', '1.5')

        # SEP-29: the destination asks for a memo; refused having read only that.
        self.refuses('requires a memo', *send, reads=['/horizon/', f"/horizon/accounts/{STELLAR['destination']}"])
        for flag, value, words in [('--memo-text', 'x' * 29, '1 to 28 bytes'),
                                   ('--memo-text', 'é' * 15, '1 to 28 bytes'),
                                   ('--memo-id', '18446744073709551616', 'whole number'),
                                   ('--destination-tag', '1', 'no such destination tag or memo')]:
            self.refuses(words, *send, f'{flag}={value}', reads=[])

        # Text and ID memos signed as @stellar/stellar-base signs them.
        for (kind, memo), flag in [(('memoText', 'a'), '--memo-text'),
                                   (('memoId', '18446744073709551615'), '--memo-id')]:
            vector = payments[(kind, memo)]
            Node.state['stellar'][STELLAR['source']]['sequence'] = str(int(vector['sequence']) - 1)
            built = self.cli(*send, flag, memo)['artifact']
            assert built['memo'] == {'kind': kind, 'value': memo}, built
            envelope = json.loads(self.sign(built)['signed_payload'])['signed_xdr_b64']
            assert envelope == vector['envelope_b64'], (kind, envelope)

        # A memo changed after the review is refused.
        Node.state['stellar'][STELLAR['source']]['sequence'] = '123456789100'
        noted = self.cli(*send, '--memo-text', 'deposit')['artifact']
        self.tamper(noted, 'elsewhere')
        assert 'altered' in self.cli('send', 'sign', noted['id'], '--review-digest', noted['review_digest'],
                                     success=False)

        # Asked for after the review, the memo's absence is refused at signing.
        Node.state['stellar'][STELLAR['destination']]['data'] = {}
        bare = self.cli(*send)['artifact']
        Node.state['stellar'][STELLAR['destination']]['data'] = {'config.memo_required': 'MQ=='}
        assert 'requires a memo' in self.cli('send', 'sign', bare['id'], '--review-digest', bare['review_digest'],
                                            success=False)

    def test_stellar_merges_into_an_account_that_asks_for_a_memo(self):
        vector = STELLAR['account_merge']
        self.stellar_wallet(vector['sequence'], memo_required=True)
        self.refuses('requires a memo', 'wallet', 'close', 'XLM', '--to', STELLAR['destination'])
        built = self.cli('wallet', 'close', 'XLM', '--to', STELLAR['destination'],
                         '--memo-text', vector['memo'])['artifact']
        assert built['memo'] == {'kind': 'memoText', 'value': vector['memo']}, built
        envelope = json.loads(self.sign(built)['signed_payload'])['signed_xdr_b64']
        assert envelope == vector['envelope_b64'], envelope


if __name__ == '__main__':
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]], verbosity=2)
