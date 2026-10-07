#!/usr/bin/env python3
"""SEP-53 message signatures from the Stellar SDK for Python.

python3 -m venv /tmp/spectra-py-vectors && /tmp/spectra-py-vectors/bin/pip install stellar-sdk==16.1.0
/tmp/spectra-py-vectors/bin/python scripts/generate-stellar-message-vectors.py
"""
import base64
import json
import pathlib

from stellar_sdk import Keypair

MESSAGES = ['Hello World', 'Spectra 证明 ✓\nline two', '']
seed = bytes([7] * 32)
keypair = Keypair.from_raw_ed25519_seed(seed)
vectors = []
for message in MESSAGES:
    signature = keypair.sign_message(message)
    keypair.verify_message(message, signature)  # raises on a bad signature
    vectors.append({'address': keypair.public_key, 'key': seed.hex(), 'message': message,
                    'signature': base64.b64encode(signature).decode()})
path = pathlib.Path(__file__).resolve().parents[1] / 'core/tests/fixtures/message-signatures-stellar.json'
path.write_text(json.dumps({'provenance': 'stellar-sdk 16.1.0 (Python)', 'stellar': vectors},
                           ensure_ascii=False, indent=2) + '\n')
