#!/usr/bin/env python3
"""ERC-721 and ERC-1155 tokens against a loopback EVM node and Blockscout.

A wallet's NFTs are listed from the explorer's inventory, page after page.
Each transfer is built only after the contract says which standard it is and
a live read says the wallet holds what it sends; it signs byte for byte as
ethers.js signs the same transaction (core/tests/fixtures/nft-transfer-
vectors.json), and ownership is read again before signing. History carries
each token as its own asset, a quantity rather than an amount, and nothing
reads a collection as a fungible balance."""
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
import urllib.parse

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/spectra').resolve())
VECTORS = json.loads((pathlib.Path(__file__).resolve().parents[1]
                      / 'core/tests/fixtures/nft-transfer-vectors.json').read_text())['signed']
OWNER, RECIPIENT = VECTORS['signer'], VECTORS['recipient']
APES, ITEMS = VECTORS['erc721']['to'], VECTORS['erc1155']['to']
TOKEN = '0x' + '77' * 20       # an ERC-20 that does not implement ERC-165
EVERYTHING = '0x' + '88' * 20  # claims every interface, ERC-165's "invalid" one too
STRANGER = '0x' + '99' * 20
ZERO = '0x' + '00' * 20
SIGNED = {VECTORS['erc721']['raw']: VECTORS['erc721']['hash'], VECTORS['erc1155']['raw']: VECTORS['erc1155']['hash']}


def word(value):
    return '0x' + format(value, '064x')


def abi_string(text):
    return '0x' + format(32, '064x') + format(len(text), '064x') + text.encode().hex().ljust(64, '0')


def inventory_item(standard, contract, token_id, value, name=None, symbol=None, metadata=None):
    return {'id': token_id, 'token_type': standard, 'value': value, 'metadata': metadata,
            'token': {'address_hash': contract.upper().replace('0X', '0x'), 'name': name, 'symbol': symbol,
                      'type': standard, 'decimals': None}}


def transfer_row(contract, token_id, sender, receiver, hash_, value=None, name=None, symbol=None):
    row = {'blockNumber': '100', 'timeStamp': '1780000000', 'hash': hash_, 'from': sender, 'to': receiver,
           'contractAddress': contract, 'tokenID': token_id, 'tokenName': name, 'tokenSymbol': symbol,
           'tokenDecimal': ''}
    if value is not None:
        row['tokenValue'] = value
    return row


class Node(http.server.BaseHTTPRequestHandler):
    """An Ethereum node under /rpc and a Blockscout under /scout."""
    state = {}
    submitted = []

    def log_message(self, *_):
        pass

    def reply(self, payload):
        data = json.dumps(payload).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        url = urllib.parse.urlsplit(self.path)
        query = dict(urllib.parse.parse_qsl(url.query))
        path = url.path.removeprefix('/scout')
        if path == f'/api/v2/addresses/{OWNER}/nft':
            assert query['type'] == 'ERC-721,ERC-1155', query
            # The second page is asked for with the first one's cursor.
            if query.get('token_id') == '7':
                assert query['token_contract_address_hash'] == ITEMS and query['items_count'] == '50', query
                return self.reply({'items': [
                    inventory_item('ERC-404', STRANGER, '1', '1'),
                    inventory_item('ERC-721', APES, '5678', None, 'Apes', 'APE')], 'next_page_params': None})
            return self.reply({'items': [
                inventory_item('ERC-721', APES, '1234', '1', 'Apes', 'APE', {'name': 'Ape #1234'}),
                inventory_item('ERC-1155', ITEMS, '7', '5')],
                'next_page_params': {'token_type': 'ERC-1155', 'token_contract_address_hash': ITEMS,
                                     'token_id': '7', 'items_count': 50}})
        assert path == '/api', self.path
        if query['action'] == 'tokenlist':
            # Blockscout lists a collection beside the fungible tokens.
            return self.reply({'status': '1', 'message': 'OK', 'result': [
                {'balance': '2', 'contractAddress': APES, 'decimals': '', 'name': 'Apes', 'symbol': 'APE',
                 'type': 'ERC-721'}]})
        rows = self.state['lists'].get(query['action'], [])
        page, size = int(query['page']), int(query['offset'])
        chunk = rows[(page - 1) * size:page * size]
        return self.reply({'status': '1', 'message': 'OK', 'result': chunk} if chunk else
                          {'status': '0', 'message': 'No transactions found', 'result': []})

    def call(self, to, data):
        """An `eth_call`'s answer, or None for a revert."""
        selector, argument = data[2:10], data[10:]
        number = lambda index: int(argument[64 * index:64 * (index + 1)], 16)
        if selector == '01ffc9a7':
            interface = argument[:8]
            if to == TOKEN:
                return None
            claims = {APES: {'80ac58cd'}, ITEMS: {'d9b67a26'},
                      EVERYTHING: {'80ac58cd', 'd9b67a26', 'ffffffff'}}.get(to, set())
            return word(interface in claims)
        if selector == '6352211e' and to == APES:
            return word(int(OWNER, 16)) if number(0) in (1234, 5678) else None
        if selector == '00fdd58e' and to == ITEMS:
            assert argument[24:64] == OWNER[2:], argument
            return word(self.state['items'] if number(1) == 7 else 0)
        # An ERC-721 balanceOf counts tokens.
        if selector == '70a08231' and to == APES:
            return word(2)
        if selector == '06fdde03' and to == APES:
            return abi_string('Apes')
        if selector == '95d89b41' and to in (APES, TOKEN):
            return abi_string('APE' if to == APES else 'TKN')
        # A collection may answer decimals() too; it is still no fungible token.
        if selector == '313ce567' and to in (APES, TOKEN):
            return word(0 if to == APES else 18)
        return None

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))

        def answer(call):
            method, params = call['method'], call.get('params', [])
            if method == 'eth_call':
                result = self.call(params[0]['to'].lower(), params[0]['data'])
                if result is None:
                    return {'jsonrpc': '2.0', 'id': call['id'], 'error': {'code': 3, 'message': 'execution reverted'}}
                return {'jsonrpc': '2.0', 'id': call['id'], 'result': result}
            if method == 'eth_sendRawTransaction':
                raw = params[0]
                # Only the transactions ethers signs for the same fields.
                assert raw in SIGNED, raw
                self.submitted.append(raw)
                return {'jsonrpc': '2.0', 'id': call['id'], 'result': SIGNED[raw]}
            values = {'eth_chainId': '0x1', 'eth_getBalance': hex(10 * 10**18), 'eth_getCode': '0x',
                      'eth_getTransactionCount': '0x3', 'eth_estimateGas': '0x15f90', 'eth_blockNumber': '0x20',
                      'eth_feeHistory': {'baseFeePerGas': ['0x3b9aca00'], 'reward': [['0x77359400']]}}
            assert method in values, method
            return {'jsonrpc': '2.0', 'id': call['id'], 'result': values[method]}

        self.reply(list(map(answer, body)) if isinstance(body, list) else answer(body))


class NftTests(unittest.TestCase):
    def setUp(self):
        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.base = f'http://127.0.0.1:{self.server.server_port}'
        self.directory = tempfile.TemporaryDirectory(prefix='spectra-nfts-')
        self.journal = pathlib.Path(self.directory.name) / 'network.jsonl'
        Node.submitted.clear()
        Node.state = {'items': 5, 'lists': {}}

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

    def send_nft(self, contract, token_id, quantity, to=RECIPIENT, **kwargs):
        return self.run_cli('wallet', 'send-nft', 'Collector', '--contract', contract, '--token-id', token_id,
                            '--quantity', quantity, '--to', to, **kwargs)

    def test_nfts_are_tokens_not_balances(self):
        imported = self.run_cli('wallet', 'import', '--chain', 'ethereum', '--name', 'Collector', '--no-password',
                                '--private-key-env', 'KEY', env={'KEY': '01' * 32})
        assert imported['wallet']['address'].lower() == OWNER, imported
        self.run_cli('endpoints', '--chain', 'ethereum', '--api', 'evm-json-rpc', '--capabilities',
                     'balance,fee,broadcast,verification,token-balance', '--add', self.base + '/rpc')
        self.run_cli('endpoints', '--chain', 'ethereum', '--api', 'blockscout', '--capabilities',
                     'history,token-discovery,token-history', '--add', self.base + '/scout')
        self.run_cli('endpoints', '--chain', 'ethereum', '--custom-only', 'true')
        actions = [offer['action'] for offer in self.run_cli('wallet', 'actions', 'Collector')['actions']['actions']]
        assert 'nfts' in actions, actions

        # The inventory, both pages: an ERC-404 row is no NFT here.
        nfts = self.run_cli('wallet', 'nfts', 'Collector')['nfts']
        assert nfts['complete'], nfts
        assert [(n['standard'], n['contract'], n['tokenId'], n['quantity'], n['collection'], n['symbol'], n['name'])
                for n in nfts['nfts']] == [
            ('ERC-721', APES, '1234', '1', 'Apes', 'APE', 'Ape #1234'),
            ('ERC-1155', ITEMS, '7', '5', '', 'NFT', None),
            ('ERC-721', APES, '5678', '1', 'Apes', 'APE', None)], nfts

        # What the contract or the network would refuse is refused first.
        self.refuses('one at a time', 'wallet', 'send-nft', 'Collector', '--contract', APES, '--token-id', '1234',
                     '--quantity', '2', '--to', RECIPIENT)
        for contract, token_id, quantity, to, words in [
            (APES, '99', '1', RECIPIENT, 'does not own token 99'),
            (ITEMS, '7', '6', RECIPIENT, 'holds 5 of token 7, fewer than 6'),
            (ITEMS, '7', '0', RECIPIENT, 'at least one'),
            (APES, '1234', '1', OWNER, 'own address'),
            (APES, '1234', '1', ZERO, 'zero address'),
            (TOKEN, '1', '1', RECIPIENT, 'is not an ERC-721 or ERC-1155 collection'),
            (EVERYTHING, '1', '1', RECIPIENT, 'is not an ERC-721 or ERC-1155 collection'),
            (APES, '1.5', '1', RECIPIENT, 'Invalid token id'),
            (ITEMS, '7', '1e2', RECIPIENT, 'Invalid quantity'),
        ]:
            self.refuses(words, 'wallet', 'send-nft', 'Collector', '--contract', contract, '--token-id', token_id,
                         '--quantity', quantity, '--to', to)

        # One ERC-721 token: safeTransferFrom, signed as ethers signs it.
        built = self.send_nft(APES, '1234', '1')['artifact']
        assert built['operation'] == {'kind': 'transfer_nft', 'contract': APES, 'standard': 'ERC-721',
                                      'token_id': '1234', 'quantity': '1', 'collection': 'Apes',
                                      'network_fee': '0.000432'}, built['operation']
        assert (built['recipient'], built['amount'], built['symbol'], built['asset']) == (RECIPIENT, '1', 'APE', APES)
        signed = self.run_cli('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                              '--endpoint', self.base + '/rpc')['artifact']
        assert signed['signed_payload'] == VECTORS['erc721']['raw'], signed['signed_payload']
        self.run_cli('send', 'broadcast-signed', signed['id'], '--endpoint', self.base + '/rpc', '--yes')

        # Three of an ERC-1155 id. Holding fewer by the time it is signed is a
        # refusal; holding them again, it signs as ethers signs it.
        built = self.send_nft(ITEMS, '7', '3')['artifact']
        assert built['operation']['standard'] == 'ERC-1155' and built['symbol'] == 'NFT', built
        Node.state['items'] = 2
        code, output = self.run_cli('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                                    '--endpoint', self.base + '/rpc', success=False)
        assert 'holds 2 of token 7, fewer than 3' in output, output
        Node.state['items'] = 5
        signed = self.run_cli('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                              '--endpoint', self.base + '/rpc')['artifact']
        assert signed['signed_payload'] == VECTORS['erc1155']['raw'], signed['signed_payload']
        self.run_cli('send', 'broadcast-signed', signed['id'], '--endpoint', self.base + '/rpc', '--yes')
        assert Node.submitted == [VECTORS['erc721']['raw'], VECTORS['erc1155']['raw']], Node.submitted

        # History: each token its own asset, its quantity the amount; the
        # explorer's rows for the two sends replace the pending ones.
        Node.state['lists'] = {
            'tokennfttx': [
                transfer_row(APES, '1234', OWNER, RECIPIENT, VECTORS['erc721']['hash'], name='Apes', symbol='APE'),
                transfer_row(APES, '5678', STRANGER, OWNER, '0x' + 'a1' * 32, name='Apes', symbol='APE')],
            'token1155tx': [
                transfer_row(ITEMS, '7', OWNER, RECIPIENT, VECTORS['erc1155']['hash'], value='3'),
                transfer_row(ITEMS, '7', STRANGER, OWNER, '0x' + 'b2' * 32, value='4')],
        }
        self.run_cli('history', 'Collector', '--save')
        rows = self.run_cli('txs', '--page', '--wallet', 'Collector')['page']['records']
        nft_rows = sorted((r['deploymentId'], r['kind'], r['amount'], r['assetDisplayName'], r['symbol'], r['status'])
                          for r in rows if r['deploymentId'].count(':') == 3)
        assert nft_rows == sorted([
            (f'ethereum:erc-721:{APES}:1234', 'send', '1', 'Apes #1234', 'APE', 'confirmed'),
            (f'ethereum:erc-721:{APES}:5678', 'receive', '1', 'Apes #5678', 'APE', 'confirmed'),
            (f'ethereum:erc-1155:{ITEMS}:7', 'send', '3', '#7', 'NFT', 'confirmed'),
            (f'ethereum:erc-1155:{ITEMS}:7', 'receive', '4', '#7', 'NFT', 'confirmed')]), rows

        # A collection tracked by hand as a token never becomes an amount,
        # whatever decimals() answers: its balance read is refused, and so is
        # a token send of it.
        self.run_cli('token', 'add', '--chain', 'ethereum', '--symbol', 'APE', '--name', 'Apes',
                     '--contract', APES, '--decimals', '0')
        self.run_cli('refresh', '--wallet', 'Collector')
        with sqlite3.connect(pathlib.Path(self.directory.name) / 'spectra.sqlite') as db:
            wallet = json.loads(db.execute('SELECT payload FROM wallets').fetchone()[0])
        held = {h.get('contractAddress'): h['amount'] for h in wallet['holdings']}
        assert held.get(APES) in (None, '0'), wallet['holdings']
        discovered = self.run_cli('token', 'discover', '--wallet', 'Collector')['holdings']
        assert not [h for h in discovered if h['contract'] in (APES, ITEMS)], discovered
        self.refuses('NFT collection', 'send', 'build', '--from', 'Collector', '--to', RECIPIENT,
                     '--contract', APES, '--amount', '1')


if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('Run without -O or PYTHONOPTIMIZE: assertions must remain enabled.')
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]], verbosity=2)
