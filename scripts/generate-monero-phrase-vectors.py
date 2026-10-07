#!/usr/bin/env python3
"""Independent Monero phrase fixtures for core/tests/fixtures/monero-phrases.json.

Two implementations Spectra shares no code with:

* monero-python 1.1.1 (`pip install monero==1.1.1`) encodes 25-word seeds in
  its wordlists and derives keys and addresses. Its Japanese list matches
  Monero's words but not Monero's three-character prefix, so its Japanese
  checksums are not Monero's and Japanese is left out.
* The Polyseed reference implementation (tevador/polyseed dd998e2) through
  scripts/polyseed-vectors.c, which creates and decodes 16-word phrases:

    git clone https://github.com/tevador/polyseed && cd polyseed
    git checkout dd998e2b610cc5f1b79f005ed6c8edb676da743f
    clang -DPOLYSEED_STATIC -Iinclude -Isrc .../scripts/polyseed-vectors.c src/*.c \\
        -framework CoreFoundation -o polyseed-vectors

Usage: generate-monero-phrase-vectors.py path/to/polyseed-vectors > fixture.json
"""
import hashlib, json, subprocess, sys

from monero.seed import Seed

ELECTRUM_LANGUAGES = {
    'English': 'en', 'Chinese (simplified)': 'zh-hans', 'Dutch': 'nl', 'Esperanto': 'eo',
    'French': 'fr', 'German': 'de', 'Italian': 'it', 'Lojban': 'jbo', 'Portuguese': 'pt',
    'Russian': 'ru', 'Spanish': 'es',
}
POLYSEED_LANGUAGES = {
    'English': 'en', 'Japanese': 'ja', 'Korean': 'ko', 'Spanish': 'es', 'French': 'fr',
    'Italian': 'it', 'Czech': 'cs', 'Portuguese': 'pt', 'Chinese (Simplified)': 'zh-hans',
    'Chinese (Traditional)': 'zh-hant',
}
L = 2**252 + 27742317777372353535851937790883648493


def reduced_key(label):
    """A deterministic spend key below the group order, so it round-trips."""
    value = int.from_bytes(hashlib.sha512(label.encode()).digest(), 'little') % L
    return value.to_bytes(32, 'little').hex()


def keys(seed):
    return {
        'spend_key': seed.secret_spend_key(),
        'view_key': seed.secret_view_key(),
        'address': str(seed.public_address()),
        'stagenet_address': str(seed.public_address(net='stage')),
    }


def harness(binary, *args):
    out = subprocess.run([binary, *args], capture_output=True, text=True, check=True).stdout
    return dict(line.split('=', 1) for line in out.strip().splitlines() if '=' in line)


def main():
    binary = sys.argv[1]
    electrum = []
    for wordlist, code in ELECTRUM_LANGUAGES.items():
        seed = Seed(reduced_key(f'spectra-monero-{code}'), wordlist)
        electrum.append({'language': code, 'phrase': seed.phrase, **keys(seed)})
    polyseed = []
    for index, (name, code) in enumerate(POLYSEED_LANGUAGES.items()):
        secret = hashlib.sha256(f'spectra-polyseed-{code}'.encode()).digest()[:19].hex()
        created = 1_700_000_000 + index * 9_000_000
        made = harness(binary, 'create', name, secret, str(created))
        seed = Seed(made['key'])
        polyseed.append({'language': code, 'phrase': made['phrase'], 'key': made['key'],
                         'birthday': int(made['birthday']), 'created': created, **keys(seed)})
    encrypted = harness(binary, 'create', 'English', '11' * 19, '1700000000', 'password')
    print(json.dumps({
        'provenance': 'monero-python 1.1.1; tevador/polyseed dd998e2 via scripts/polyseed-vectors.c',
        'electrum': electrum,
        'polyseed': polyseed,
        'encrypted_polyseed': encrypted['phrase'],
    }, ensure_ascii=False, indent=2))


main()
