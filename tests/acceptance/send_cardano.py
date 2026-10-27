#!/usr/bin/env python3
"""Cardano staged send against a loopback Koios, across processes: an ADA
payment is built, signed without being submitted, and broadcast once,
exactly as signed. Inputs, witnesses, tokens, expiry and the network check
are tested in core."""
from harness import Handler, Spectra, fixture, scratch, serve

vector = fixture('cardano-emurgo-witness.json')
PARAMS = [dict(epoch_no=660, min_fee_a=44, min_fee_b=155381, coins_per_utxo_size='4310', max_tx_size=16384,
               max_val_size=5000)]


class Node(Handler):
    def do_GET(self):
        if self.path.endswith('/epoch_params?order=epoch_no.desc&limit=1'):
            return self.reply(PARAMS)
        if self.path.endswith('/genesis'):
            return self.reply([dict(networkmagic='764824073', networkid='Mainnet')])
        assert self.path.endswith('/tip'), self.path
        self.reply([dict(abs_slot=0)])

    def do_POST(self):
        if self.path.endswith('/submittx'):
            self.state['submitted'].append(self.raw_body().hex())
            return self.reply('ab' * 32, 202)
        request = self.body()
        assert self.path.endswith('/address_utxos') and request['_extended'] is True, self.path
        self.reply([dict(tx_hash='00'*32, tx_index=0, value='1170000', is_spent=False, asset_list=[])])


with serve(Node, submitted=[]) as node, scratch('cardano') as directory:
    submitted, endpoint = node.state['submitted'], node.url
    spectra = Spectra(directory, SPECTRA_SEED=vector['mnemonic'], SPECTRA_PASSWORD='cardano-test-password')
    spectra.run('wallet', 'import', '--chain', 'cardano', '--name', 'Cardano', '--no-password')
    spectra.run('endpoints', '--chain', 'cardano', '--api', 'koios',
                '--capabilities', 'balance,utxo,fee,verification,broadcast', '--add', endpoint)
    spectra.run('endpoints', '--chain', 'cardano', '--custom-only', 'true')
    prepared = spectra.run('send', 'build', 'Cardano', '--to', vector['address'], '--amount', '1',
                           '--endpoint', endpoint)['artifact']
    signed = spectra.run('send', 'sign', prepared['id'], '--review-digest', prepared['review_digest'],
                         '--endpoint', endpoint)['artifact']
    assert signed['transaction_hash'] and not submitted, signed
    assert spectra.run('send', 'inspect', signed['id'])['artifact'] == signed
    spectra.run('send', 'broadcast-signed', signed['id'], '--endpoint', endpoint, '--yes')
    assert submitted == [signed['signed_payload']], submitted
    print('Cardano staged send: built, signed without submitting, broadcast once as signed')
