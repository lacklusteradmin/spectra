#!/usr/bin/env python3
"""ERC-721 and ERC-1155 transfers as staged sends against a loopback EVM
node, across processes: each is built, signed and broadcast, and the node
receives exactly the transaction ethers.js signs for the same fields
(core/tests/fixtures/nft-transfer-vectors.json). Standards, holdings,
refusals, reviews and history are tested in core."""
from harness import JsonRpc, RpcError, Spectra, fixture, scratch, serve

VECTORS = fixture('nft-transfer-vectors.json')['signed']
OWNER, RECIPIENT = VECTORS['signer'], VECTORS['recipient']
APES, ITEMS = VECTORS['erc721']['to'], VECTORS['erc1155']['to']
SIGNED = {VECTORS['erc721']['raw']: VECTORS['erc721']['hash'], VECTORS['erc1155']['raw']: VECTORS['erc1155']['hash']}


def word(value):
    return '0x' + format(value, '064x')


class Node(JsonRpc):
    """An Ethereum node where the wallet owns Apes #1234 and five of item 7."""

    def eth_call(self, to, data):
        """An `eth_call`'s answer; anything else reverts."""
        selector, argument = data[2:10], data[10:]
        if selector == '01ffc9a7':
            return word(argument[:8] == {APES: '80ac58cd', ITEMS: 'd9b67a26'}.get(to))
        if selector == '6352211e' and to == APES and int(argument[:64], 16) == 1234:
            return word(int(OWNER, 16))
        if selector == '00fdd58e' and to == ITEMS:
            return word(5 if int(argument[64:128], 16) == 7 else 0)
        raise RpcError(3, 'execution reverted')

    def call(self, method, params):
        if method == 'eth_call':
            return self.eth_call(params[0]['to'].lower(), params[0]['data'])
        if method == 'eth_sendRawTransaction':
            raw = params[0]
            # Only the transactions ethers signs for the same fields.
            assert raw in SIGNED, raw
            self.state['submitted'].append(raw)
            return SIGNED[raw]
        values = {'eth_chainId': '0x1', 'eth_getBalance': hex(10 * 10**18), 'eth_getCode': '0x',
                  'eth_getTransactionCount': '0x3', 'eth_estimateGas': '0x15f90', 'eth_blockNumber': '0x20',
                  'eth_feeHistory': {'baseFeePerGas': ['0x3b9aca00'], 'reward': [['0x77359400']]}}
        assert method in values, method
        return values[method]


def send(spectra, rpc, submitted, contract, token_id, quantity, vector):
    """Builds, signs and broadcasts the transfer: the node receives the
    signed bytes, which are ethers' for the same fields."""
    before = len(submitted)
    built = spectra.run('wallet', 'send-nft', 'Collector', '--contract', contract, '--token-id', token_id,
                        '--quantity', quantity, '--to', RECIPIENT)['artifact']
    signed = spectra.run('send', 'sign', built['id'], '--review-digest', built['review_digest'],
                         '--endpoint', rpc)['artifact']
    assert signed['signed_payload'] == vector['raw'], signed['signed_payload']
    assert len(submitted) == before, 'signing broadcasts nothing'
    spectra.run('send', 'broadcast-signed', signed['id'], '--endpoint', rpc, '--yes')


if __name__ == '__main__':
    with serve(Node, submitted=[]) as node, scratch('nfts') as directory:
        submitted, rpc, spectra = node.state['submitted'], node.url + '/rpc', Spectra(directory)
        imported = spectra.run('wallet', 'import', '--chain', 'ethereum', '--name', 'Collector', '--no-password',
                               '--private-key-env', 'KEY', env={'KEY': '01' * 32})
        assert imported['wallet']['address'].lower() == OWNER, imported
        spectra.run('endpoints', 'add', '--chain', 'ethereum', '--api', 'evm-json-rpc', '--capabilities',
                    'balance,fee,broadcast,verification,token-balance', rpc)
        spectra.run('endpoints', 'custom-only', '--chain', 'ethereum', 'true')
        send(spectra, rpc, submitted, APES, '1234', '1', VECTORS['erc721'])
        send(spectra, rpc, submitted, ITEMS, '7', '3', VECTORS['erc1155'])
        assert submitted == [VECTORS['erc721']['raw'], VECTORS['erc1155']['raw']], submitted
    print('ERC-721 and ERC-1155 transfers broadcast as signed passed')
