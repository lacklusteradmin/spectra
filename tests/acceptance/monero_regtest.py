#!/usr/bin/env python3
"""Real-node integration, entirely offline: --monerod /path/to/verified/monerod.

Separate from make verify because the official daemon is an optional test tool.
Production has no fakechain bypass; only this loopback proxy normalizes nettype.

After a send the official daemon accepts, the wallet pays its fresh receive
subaddress and four more in one block — 0/199 and 0/300, 49/199 and 50/0,
each second one inside wallet2's lookahead only once the first is found. The
phrase restored in another directory finds every one and rotates past them;
the view key alone, in a third, finds what was received without seeing what
was spent, refuses to send, and is upgraded in place by the phrase.
"""
import argparse
import contextlib
import json
import pathlib
import socket
import subprocess
import time
import urllib.request

from harness import ARGS, Handler, Spectra, fixture, scratch, serve

# The Monero seed of the wallet the recorded fixtures were made with: Monero
# reads its own 25-word seed, not BIP-39.
MONERO_SEED = ('syndrome portents apex vivid flippant dizzy bumper duplex enjoy deodorant bunch pigment wolf muppet tuition wept ailments kiwi roles against today morsel eternal excess wolf')
# A scan or a decoy selection against the daemon outlasts the harness's default.
SCAN = 180


def rpc(daemon, method, params=None):
    request = {'jsonrpc': '2.0', 'id': '0', 'method': method, 'params': params or {}}
    return json.load(urllib.request.urlopen(daemon + '/json_rpc', json.dumps(request).encode(), timeout=120))['result']


@contextlib.contextmanager
def regtest(monerod, directory):
    """The official daemon on an offline regtest chain, for the block's
    length: its RPC URL."""
    with socket.socket() as port:
        port.bind(('127.0.0.1', 0))
        rpc_port = port.getsockname()[1]
    daemon = f'http://127.0.0.1:{rpc_port}'
    with open(directory / 'daemon.log', 'w') as log:
        process = subprocess.Popen([monerod, '--regtest', '--offline', '--fixed-difficulty', '1',
            '--no-igd', '--no-zmq', '--p2p-bind-port', '0', '--hide-my-port', '--non-interactive', '--disable-dns-checkpoints',
            '--check-updates', 'disabled', '--rpc-bind-ip', '127.0.0.1',
            '--rpc-bind-port', str(rpc_port), '--data-dir', str(directory / 'node')], stdout=log, stderr=log)
        try:
            for _ in range(100):
                try:
                    info = rpc(daemon, 'get_info')
                    assert info['nettype'] == 'fakechain' and info['offline']
                    break
                except (OSError, KeyError):
                    assert process.poll() is None, (directory / 'daemon.log').read_text()
                    time.sleep(.1)
            else:
                raise AssertionError('monerod did not start')
            yield daemon
        finally:
            process.terminate()
            try: process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.kill(); process.wait()


class Proxy(Handler):
    """The daemon, reporting itself a mainnet node."""

    def do_POST(self):
        body = self.raw_body()
        if self.path == '/send_raw_transaction': self.state['submissions'].append(body)
        raw = urllib.request.urlopen(self.state['daemon'] + self.path, body, timeout=120).read()
        if self.path == '/get_info':
            info = json.loads(raw)
            assert info['nettype'] == 'fakechain' and info['offline']
            info['nettype'] = 'mainnet'
            raw = json.dumps(info).encode()
        self.reply_bytes(raw, content_type='application/json')


def main():
    parser = argparse.ArgumentParser(usage='%(prog)s [path/to/spectra] --monerod PATH', description=__doc__)
    parser.add_argument('--monerod', required=True, help='the official daemon, verified')
    monerod = str(pathlib.Path(parser.parse_args(ARGS).monerod).resolve())
    with scratch('monero-regtest') as directory, regtest(monerod, directory) as daemon, \
            serve(Proxy, daemon=daemon, submissions=[]) as proxy:
        submissions, endpoint = proxy.state['submissions'], proxy.url
        spectra = Spectra(directory, SCAN, SPECTRA_SEED=MONERO_SEED, SPECTRA_PASSWORD='fixture-password')
        wallet, restore, watch = spectra / 'wallet', spectra / 'restored', spectra / 'viewonly'
        wallet.run('wallet', 'import', '--chain', 'monero', '--name', 'LocalXmr')
        wallet.run('endpoints','--chain','monero','--api','monero-daemon-rpc','--capabilities','fee,broadcast,verification','--add', endpoint)
        address = wallet.run('send', 'identity', 'LocalXmr')['address']
        rpc(daemon, 'generateblocks', {'wallet_address': address, 'amount_of_blocks': 180})
        synced = wallet.run('send', 'sync-monero', 'LocalXmr')['sync']
        assert synced['complete'] and synced['unlocked_piconeros'] > 0
        assert wallet.run('send', 'monero-status', 'LocalXmr')['sync'] == synced
        def build():
            return wallet.run('send', 'build', 'LocalXmr', '--to', address,
                              '--amount', '0.001', '--endpoint', endpoint)['artifact']
        prepared, competing = build(), build()
        signed = wallet.run('send', 'sign', prepared['id'], '--review-digest', prepared['review_digest'], '--endpoint', endpoint)['artifact']
        assert not submissions, 'build/sign must not broadcast'
        wallet.fail('send', 'sign', competing['id'], '--review-digest', competing['review_digest'], '--endpoint', endpoint)
        assert wallet.run('send', 'inspect', prepared['id'])['artifact'] == signed
        accepted = wallet.run('send', 'broadcast-signed', prepared['id'], '--endpoint', endpoint, '--yes')['artifact']
        assert accepted['attempts'][-1]['outcome'] == 'Accepted', accepted
        assert len(submissions) == 1
        pool = json.load(urllib.request.urlopen(daemon+'/get_transaction_pool', b'{}', timeout=30))
        assert pool['transactions'][0]['id_hash'] == signed['transaction_hash']
        print('PASS: official monerod accepted local CLSAG/Bulletproof+ signing; no early broadcast, durable restart and input reservation verified')

        vectors = fixture('monero-subaddresses.json')['networks'][0]
        subaddress = {(v['account'], v['address']): v['encoded'] for v in vectors['subaddresses']}
        assert vectors['primary'] == address
        # More outputs on chain for decoy selection to draw rings from.
        rpc(daemon, 'generateblocks', {'wallet_address': address, 'amount_of_blocks': 200})
        wallet.run('send', 'sync-monero', 'LocalXmr')
        # The primary address has received, so the next payer gets 0/1.
        fresh = wallet.run('wallet', 'receive', 'LocalXmr')['address']
        assert fresh == subaddress[(0, 1)], fresh
        targets = [(0, 1), (0, 199), (0, 300), (49, 199), (50, 0)]
        def build_to(destination, amount):
            # Decoy selection is random and a regtest chain is small:
            # a round limit is retried, nothing else.
            for _ in range(5):
                built = json.loads(wallet.process('send', 'build', 'LocalXmr', '--to', destination,
                                                  '--amount', amount, '--endpoint', endpoint).stdout)
                if 'artifact' in built:
                    return built['artifact']
                assert 'decoy selection round limit' in built['error'], built
            raise AssertionError('decoy selection kept failing')
        for number, target in enumerate(targets):
            artifact = build_to(subaddress[target], f'0.00{11 + number}')
            wallet.run('send', 'sign', artifact['id'], '--review-digest', artifact['review_digest'], '--endpoint', endpoint)
            wallet.run('send', 'broadcast-signed', artifact['id'], '--endpoint', endpoint, '--yes')
        pool = json.load(urllib.request.urlopen(daemon+'/get_transaction_pool', b'{}', timeout=30))
        assert len(pool['transactions']) == len(targets), pool
        # One block holds all five; ten more unlock them.
        rpc(daemon, 'generateblocks', {'wallet_address': address, 'amount_of_blocks': 11})
        used = [{'account': 0, 'address': 300}, {'account': 49, 'address': 199}, {'account': 50, 'address': 0}]
        original = wallet.run('send', 'sync-monero', 'LocalXmr')['sync']
        assert original['complete'] and original['spends_known'] and original['used_subaddresses'] == used, original
        rotated = wallet.run('wallet', 'receive', 'LocalXmr')['address']
        assert rotated not in subaddress.values(), rotated

        # The phrase, restored elsewhere, finds every subaddress again.
        restore.run('wallet', 'import', '--chain', 'monero', '--name', 'RestoredXmr')
        restore.run('endpoints','--chain','monero','--api','monero-daemon-rpc','--capabilities','fee,broadcast,verification','--add', endpoint)
        restored = restore.run('send', 'sync-monero', 'RestoredXmr')['sync']
        assert restored['used_subaddresses'] == used, restored
        assert restored['unlocked_piconeros'] == original['unlocked_piconeros'], (restored, original)
        assert restore.run('wallet', 'receive', 'RestoredXmr')['address'] == rotated
        # A restart scans on from where it stopped.
        assert restore.run('send', 'monero-status', 'RestoredXmr')['sync'] == restored

        # The view key alone: what arrived, never what left.
        view = vectors['private_view_key']
        other = view[:-2] + ('00' if view[-2:] != '00' else '01')
        watch.fail('wallet', 'watch', '--chain', 'monero', '--name', 'ViewXmr', '--address', address,
                      '--view-key', other)
        watch.fail('wallet', 'watch', '--chain', 'monero', '--name', 'ViewXmr', '--address', subaddress[(0, 1)],
                      '--view-key', view)
        watch.run('wallet', 'watch', '--chain', 'monero', '--name', 'ViewXmr', '--address', address,
                     '--view-key', view)
        watch.run('endpoints','--chain','monero','--api','monero-daemon-rpc','--capabilities','fee,broadcast,verification','--add', endpoint)
        watched = watch.run('send', 'sync-monero', 'ViewXmr')['sync']
        assert watched['complete'] and not watched['spends_known'], watched
        assert watched['used_subaddresses'] == used, watched
        assert watched['unlocked_piconeros'] > restored['unlocked_piconeros'], (watched, restored)
        assert watch.run('wallet', 'receive', 'ViewXmr')['address'] == rotated
        watch.fail('send', 'build', 'ViewXmr', '--to', address, '--amount', '0.001', '--endpoint', endpoint)
        assert watch.run('send', 'monero-status', 'ViewXmr')['sync'] == watched
        rpc(daemon, 'generateblocks', {'wallet_address': address, 'amount_of_blocks': 1})
        later = watch.run('send', 'sync-monero', 'ViewXmr')['sync']
        assert later['scanned_height'] == watched['scanned_height'] + 1, (later, watched)
        # The phrase gives the watched wallet its spend key: it scans
        # again and sees what it spent.
        upgraded = watch.run('wallet', 'import', '--chain', 'monero', '--name', 'Upgraded')
        assert upgraded['wallet']['name'] == 'ViewXmr', upgraded
        synced = watch.run('send', 'sync-monero', 'ViewXmr')['sync']
        again = restore.run('send', 'sync-monero', 'RestoredXmr')['sync']
        assert synced['spends_known'] and synced['unlocked_piconeros'] == again['unlocked_piconeros'], \
            (synced, again)
        print('PASS: subaddresses recovered past wallet2 lookahead edges across accounts, receive rotation, '
              'restart, and a view-only wallet that cannot sign and upgrades in place')


if __name__ == '__main__':
    main()
