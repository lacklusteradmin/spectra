#!/usr/bin/env python3
"""A 2-of-3 Bitcoin multisig and its PSBT across three data directories,
against one loopback Esplora.

A coordinator watches the account from its descriptor. Each process builds
on what the last one recorded: a payer is handed the next receive address,
discovery finds the used receive and change addresses, the balance sums
them, and a PSBT's change goes to the next change address.
Cosigner A, whose phrase upgrades its watched copy of the account in place,
imports the PSBT and signs; cosigner B signs A's copy; the coordinator joins
both copies and broadcasts once, and the session keeps the transaction id.
Submitting and discarding a session ask for `--yes`.

Core's tests own the rules: coin selection, fees, review digests, spent
inputs and foreign coins (core/src/service/tests/multisig_psbt.rs,
core/src/send/tests/psbt.rs).
"""
import hashlib
from urllib.parse import unquote, urlsplit

from harness import Handler, Spectra, fixture, scratch, serve, signed_by

FIXTURE = fixture('multisig-psbt.json')
PHRASES = FIXTURE['phrases']
NETWORK = next(entry for entry in FIXTURE['networks'] if entry['chain'] == 'bitcoin')
GENESIS = '000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f'
USAGE = 2


def txid(raw):
    """A transaction's id as a node names it: the double SHA-256 of its
    serialization without witnesses, byte-reversed."""
    tx = bytes.fromhex(raw)
    if tx[4:6] == b'\x00\x01':
        def count(at):
            first = tx[at]
            size = {0xfd: 2, 0xfe: 4, 0xff: 8}.get(first, 0)
            return (int.from_bytes(tx[at + 1:at + 1 + size], 'little') if size else first), at + 1 + size

        inputs, at = count(6)
        for _ in range(inputs):
            script, at = count(at + 36)
            at += script + 4
        outputs, at = count(at)
        for _ in range(outputs):
            script, at = count(at + 8)
            at += script
        tx = tx[:4] + tx[6:at] + tx[-4:]
    return hashlib.sha256(hashlib.sha256(tx).digest()).digest()[::-1].hex()


class Esplora(Handler):
    def text(self, value):
        self.reply_bytes(value.encode(), content_type='text/plain')

    def do_GET(self):
        parts = unquote(urlsplit(self.path).path).strip('/').split('/')
        held = self.state['funds'].get(parts[1], []) if len(parts) > 1 else []
        if parts == ['block-height', '0']:
            return self.text(GENESIS)
        if parts[0] == 'address' and len(parts) == 2:
            stats = {'funded_txo_sum': sum(v for _, _, v in held), 'spent_txo_sum': 0, 'tx_count': len(held)}
            return self.reply({'address': parts[1], 'chain_stats': stats,
                               'mempool_stats': {'funded_txo_sum': 0, 'spent_txo_sum': 0, 'tx_count': 0}})
        if parts[0] == 'address' and parts[2:] == ['utxo']:
            return self.reply([{'txid': t, 'vout': n, 'value': v, 'status': {'confirmed': True, 'block_height': 1}}
                               for t, n, v in held])
        if parts[0] == 'address' and parts[2:3] == ['txs']:
            return self.reply([])
        self.send_error(404, self.path)

    def do_POST(self):
        body = self.raw_body().decode()
        if self.path.rstrip('/') == '/tx':
            self.state['broadcasts'].append(body)
            return self.text(txid(body))
        self.send_error(404, self.path)


def descriptor(cosigners):
    keys = ','.join(f"[{c['fingerprint']}/{c['origin']}]{c['xpub']}/<0;1>/*" for c in cosigners)
    return f'wsh(sortedmulti(2,{keys}))'


address = {(a['branch'], a['index']): a['address'] for a in NETWORK['addresses']}
recipient = NETWORK['psbt']['recipient']['address']
funds = {
    address[(0, 0)]: [('aa' * 32, 0, 100_000)],
    address[(0, 2)]: [('bb' * 32, 1, 300_000)],
    address[(1, 0)]: [('cc' * 32, 2, 50_000)],
}
with serve(Esplora, funds=funds, broadcasts=[]) as node, scratch('multisig') as directory:
    spectra = Spectra(directory, SPECTRA_PASSWORD='multisig-password')
    coordinator = spectra / 'coordinator'
    broadcasts = node.state['broadcasts']

    vault = descriptor(NETWORK['cosigners'])
    for signer in [coordinator, spectra / 'a', spectra / 'b']:
        signer.run('endpoints', '--chain', 'bitcoin', '--api', 'esplora',
                   '--capabilities', 'balance,history,utxo,fee,broadcast,verification', '--add', node.url)
        signer.run('wallet', 'watch', '--chain', 'bitcoin', '--name', 'Vault', '--multisig', vault)

    # The keypool is kept between processes: 0/0 is the account's own
    # address, so a payer gets 0/1; discovery records the used 0/2 and
    # 1/0, the balance sums them, and change goes to 1/1.
    assert coordinator.run('wallet', 'receive', 'Vault')['address'] == address[(0, 1)]
    found = coordinator.run('pool', 'discover', 'Vault')['addresses']
    assert {address[(0, 2)], address[(1, 0)]} <= set(found), found
    assert coordinator.run('balance', 'Vault')['smallestUnit'] == '450000'
    created = coordinator.run('multisig', 'create', 'Vault', '--to', recipient,
                              '--amount', '0.0032', '--fee-rate', '2')['session']
    assert [i['address'] for i in created['inputs']] == [address[(0, 2)], address[(0, 0)]], created
    change = created['outputs'][1]
    assert change['isChange'] and change['address'] == address[(1, 1)], created

    # Submitting and discarding ask for --yes, and without it change
    # nothing.
    for verb in ['submit', 'discard']:
        refusal = coordinator.fail('multisig', verb, created['id'], code=USAGE)
        assert refusal['error'] == f'{verb} requires --yes', refusal
    assert [s['id'] for s in coordinator.run('multisig', 'list', 'Vault')['sessions']] == [created['id']]

    # Each cosigner's phrase upgrades its watched copy; A signs the
    # coordinator's PSBT, B signs A's copy.
    copy = created['data']
    for data, cosigner in [('a', 0), ('b', 1)]:
        signer = spectra / data
        upgraded = signer.run('wallet', 'import', '--chain', 'bitcoin', '--name', 'Other', '--upgrade', 'Vault',
                              env={'SPECTRA_SEED': PHRASES[cosigner]})
        assert upgraded['upgraded'] and upgraded['wallet']['name'] == 'Vault', upgraded
        imported = signer.run('multisig', 'import', 'Vault', '--data', copy)['session']
        assert imported['reviewDigest'] == created['reviewDigest'], imported
        signed = signer.run('multisig', 'sign', imported['id'], '--review-digest', imported['reviewDigest'])['session']
        assert signed['complete'] == (cosigner == 1), signed
        coordinator.run('multisig', 'import', 'Vault', '--data', signed['data'])
        copy = signed['data']

    # The coordinator's session joined both copies; it broadcasts once
    # and keeps the transaction id.
    joined = coordinator.run('multisig', 'show', created['id'])['session']
    assert joined['complete'], joined
    assert signed_by(joined) == sorted(c['fingerprint'] for c in NETWORK['cosigners'][:2]), joined
    raw = coordinator.run('multisig', 'finalize', created['id'])['raw']
    assert not broadcasts
    sent = coordinator.run('multisig', 'submit', created['id'], '--yes')['session']
    assert broadcasts == [raw] and sent['submittedTxid'] == created['transactionId'], sent
    listed = coordinator.run('multisig', 'list', 'Vault')['sessions']
    assert [s['submittedTxid'] for s in listed] == [created['transactionId']], listed
print('PASS multisig: descriptor watch, discovery kept across processes, a PSBT signed by two cosigners in '
      'their own data directories, joined and broadcast once; submit and discard ask for --yes')
