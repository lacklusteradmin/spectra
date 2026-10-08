#!/usr/bin/env python3
"""Cardano on loopback: the extended witness matches the SDK's, inputs holding
native assets pay too and return their assets as change, a native asset
travels with its minimum ADA, and changed, incomplete or insufficient inputs
are refused before signing."""
import http.server
import json
import os
import pathlib
import sqlite3
import subprocess
import sys
import tempfile
import threading

root = pathlib.Path(__file__).resolve().parents[1]
binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else root / 'target/debug/spectra').resolve())
vector = json.loads((root / 'core/tests/fixtures/cardano-emurgo-witness.json').read_text())
live = dict(token_only=False, omit_assets=False, changed=False, rich_tokens=False, reads=0, submitted=[],
            magic='764824073')
asset = dict(policy_id='a'*56, asset_name='01', quantity='1')
PARAMS = [dict(epoch_no=660, min_fee_a=44, min_fee_b=155381, coins_per_utxo_size='4310', max_tx_size=16384,
               max_val_size=5000)]

class Node(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def reply(self, data, status=200):
        raw = json.dumps(data).encode()
        self.send_response(status); self.send_header('Content-Length', str(len(raw)))
        self.end_headers(); self.wfile.write(raw)
    def do_GET(self):
        if self.path.endswith('/epoch_params?order=epoch_no.desc&limit=1'):
            return self.reply(PARAMS)
        if self.path.endswith('/genesis'):
            return self.reply([dict(networkmagic=live['magic'], networkid='Mainnet')])
        assert self.path.endswith('/tip'), self.path
        self.reply([dict(abs_slot=0)])
    def do_POST(self):
        if self.path.endswith('/submittx'):
            live['submitted'].append(self.rfile.read(int(self.headers['Content-Length'])).hex())
            return self.reply('ab' * 32, 202)
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        if self.path.endswith('/address_info'):
            return self.reply([dict(balance='3170000')])
        if self.path.endswith('/asset_info'):
            assert request['_asset_list'] == [['a' * 56, '01']], request
            return self.reply([dict(policy_id='a' * 56, asset_name='01', token_registry_metadata=None,
                                    cip68_metadata=None)])
        assert self.path.endswith('/address_utxos'), self.path
        assert request['_extended'] is True
        live['reads'] += 1
        token_ada = '5000000' if live['rich_tokens'] else '2000000'
        entries = [dict(tx_hash='11'*32, tx_index=0, value=token_ada, is_spent=False, asset_list=[asset])]
        if not live['token_only']:
            entries.append(dict(tx_hash='00'*32, tx_index=0, value='1170000', is_spent=False,
                asset_list=[asset] if live['changed'] else []))
        if live['omit_assets']:
            for entry in entries: del entry['asset_list']
        self.reply(entries)

server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Node)
worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
try:
    with tempfile.TemporaryDirectory(prefix='spectra-cardano-') as directory:
        endpoint = f'http://127.0.0.1:{server.server_port}'
        env = {**os.environ, 'SPECTRA_SEED': vector['mnemonic'], 'CARDANO_KEY': vector['privateKey'],
            'SPECTRA_PASSWORD': 'cardano-test-password', 'SHORT_CARDANO_KEY': vector['privateKey'][:64],
            'SPECTRA_LOOPBACK_ONLY': str(pathlib.Path(directory) / 'network.jsonl')}
        def run(*args, success=True):
            result = subprocess.run([binary, '--data-dir', directory, '--json', *args],
                env=env, capture_output=True, text=True, timeout=45)
            assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
            return json.loads(result.stdout)
        # The extended key is the phrase's account key: it previews the
        # phrase's address, and once the phrase is imported it is that wallet.
        previewed = run('wallet', 'import', '--chain', 'cardano', '--private-key-env', 'CARDANO_KEY', '--preview')
        assert previewed['addresses'] == [vector['address']], previewed
        wallet = run('wallet', 'import', '--chain', 'cardano', '--name', 'Cardano', '--no-password')['wallet']
        assert wallet['address'] == vector['address'], wallet
        run('wallet', 'import', '--chain', 'cardano', '--name', 'Extended', '--private-key-env', 'CARDANO_KEY', success=False)
        run('wallet', 'import', '--chain', 'cardano', '--name', 'Short key', '--private-key-env', 'SHORT_CARDANO_KEY', success=False)
        assert run('send', 'identity', '--from', 'Cardano')['address'] == vector['address']
        run('endpoints', '--chain', 'cardano', '--api', 'koios',
            '--capabilities', 'balance,utxo,fee,verification,broadcast,token-balance,token-discovery', '--add', endpoint)
        run('endpoints', '--chain', 'cardano', '--custom-only', 'true')
        def build(*extra, success=True):
            return run('send', 'build', '--from', 'Cardano', '--to', vector['address'],
                '--amount', '1', '--endpoint', endpoint, *extra, success=success)
        # ADA alone pays first: 1 ADA from the 1.17 ADA output, the change too
        # small to stand joining the fee, exactly as the SDK signs it.
        prepared = build()['artifact']
        details = json.loads(prepared['prepared_details'])['Cardano']
        assert details['inputs'] == [dict(tx_hash='00'*32, tx_index=0, lovelace=1170000, assets=[])], details
        assert details['outputs'] == [dict(address=vector['address'], lovelace=1000000, assets=[])], details
        assert details['fee'] == 170000, details
        signed = run('send', 'sign', prepared['id'], '--review-digest', prepared['review_digest'], '--endpoint', endpoint)['artifact']
        assert json.loads(signed['signed_payload'])['cbor_hex'] == vector['cliTransaction'], signed
        assert signed['transaction_hash'], signed
        pending = build()['artifact']
        live['changed'] = True
        run('send', 'sign', pending['id'], '--review-digest', pending['review_digest'], '--endpoint', endpoint, success=False)
        live['changed'] = False
        # An output holding a native asset pays too, and the asset comes back
        # with the ADA it needs; with too little ADA to carry it, nothing is built.
        live['token_only'] = True
        build(success=False)
        live['rich_tokens'] = True
        mixed = json.loads(build()['artifact']['prepared_details'])['Cardano']
        token = 'a' * 56 + '.01'
        assert [i['tx_hash'] for i in mixed['inputs']] == ['11'*32], mixed
        assert mixed['outputs'][1]['assets'] == [dict(asset=token, quantity=1)], mixed
        assert mixed['outputs'][1]['address'] == vector['address'], mixed
        assert sum(o['lovelace'] for o in mixed['outputs']) + mixed['fee'] == 5000000, mixed
        # The asset itself travels with its minimum ADA, reviewed as such.
        sent = run('send', 'build', '--from', 'Cardano', '--to', vector['address'], '--amount', '1',
                   '--endpoint', endpoint, '--contract', token, '--decimals', '0')['artifact']
        outputs = json.loads(sent['prepared_details'])['Cardano']['outputs']
        assert outputs[0]['assets'] == [dict(asset=token, quantity=1)], outputs
        terms = sent['review']['transfer_terms']
        assert terms['carried_native'] == format(outputs[0]['lovelace'] / 1e6, '.6f').rstrip('0').rstrip('.'), terms
        assert (terms['debited'], terms['received']) == ('1', '1'), terms
        run('send', 'build', '--from', 'Cardano', '--to', vector['address'], '--amount', '2',
            '--endpoint', endpoint, '--contract', token, '--decimals', '0', success=False)
        signed = run('send', 'sign', sent['id'], '--review-digest', sent['review_digest'], '--endpoint', endpoint)['artifact']
        # A node on another network does not receive it.
        live['magic'] = '1'
        run('send', 'broadcast-signed', signed['id'], '--endpoint', endpoint, '--yes', success=False)
        live['magic'] = '764824073'
        run('send', 'broadcast-signed', signed['id'], '--endpoint', endpoint, '--yes')
        assert live['submitted'] == [json.loads(signed['signed_payload'])['cbor_hex']], live['submitted']
        live['token_only'] = False; live['rich_tokens'] = False
        # The asset is discovered from the outputs and tracked by policy and name.
        held = run('token', 'discover', '--wallet', 'Cardano')['holdings']
        assert [(h['contract'], h['balance'], h['decimals']) for h in held] == [(token, '1', 0)], held
        run('token', 'add', '--chain', 'cardano', '--symbol', 'TEST', '--name', 'Test asset', '--contract',
            token.upper(), '--decimals', '0')
        refreshed = run('refresh', '--wallet', 'Cardano')
        assert refreshed['errors'] == 0, refreshed
        with sqlite3.connect(pathlib.Path(directory) / 'spectra.sqlite') as db:
            wallet = json.loads(db.execute("SELECT payload FROM wallets WHERE payload LIKE '%Cardano%'").fetchone()[0])
        assert {h.get('contractAddress'): h['amount'] for h in wallet['holdings']}.get(token) == '1', wallet['holdings']
        # Held under the network's own name for it.
        assert {h.get('contractAddress'): h['tokenStandard'] for h in wallet['holdings']}.get(token) == \
            'Cardano Native Token', wallet['holdings']
        live['omit_assets'] = True; build(success=False)
        assert live['reads'] >= 8
        print('Cardano offline CLI: SDK-matching witness, token-bearing inputs returning their assets, '
              'native assets with their minimum ADA, changed/incomplete/insufficient inputs refused')
finally:
    server.shutdown(); server.server_close(); worker.join()
