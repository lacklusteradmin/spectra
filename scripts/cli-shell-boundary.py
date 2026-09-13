#!/usr/bin/env python3
"""Raw derivation inputs and refresh intents require no app or external network."""
import json, os, pathlib, subprocess, sys, tempfile
binary = str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='spectra-shell-boundary-') as directory:
    root = pathlib.Path(directory)
    seed = root/'seed.txt'
    seed.write_text('abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about')
    def run(*args, success=True):
        p = subprocess.run([binary, '--data-dir', str(root/'state'), '--json', *args], capture_output=True, text=True)
        assert (p.returncode == 0) == success, (args, p.stdout, p.stderr)
        return json.loads(p.stdout) if success else None
    def import_with(name, fields, success=True, chain='ethereum'):
        path = root/'input.json'; path.write_text(json.dumps(fields))
        return run('wallet', 'import', '--chain', chain, '--name', name, '--seed-file', str(seed), '--no-password', '--derivation-input-file', str(path), success=success)
    spaced = import_with('Spaced', {'passphrase':' secret '})['wallet']
    trimmed = import_with('Trimmed', {'passphrase':'secret'})['wallet']
    assert spaced['address'] != trimmed['address'], 'passphrase whitespace changed silently'
    run('send','identity','--from','Spaced')
    run('send','identity','--from','Trimmed')
    for fields in [{'iterationCount':s} for s in ['abc','0','4294967296','2048']] + [{'curve':'ed25519'}, {'saltPrefix':'custom'}, {'hmacKey':'custom'}]:
        import_with('Refused', fields, success=False)
    import_with('Monero refusal', {'passphrase':'secret'}, success=False, chain='monero')
    assert len(run('wallet','list')['wallets']) == 2, 'rejected inputs persisted wallets'
    seed.write_text('  ' + ' \t\n'.join(seed.read_text().upper().split()) + '  ')
    canonical = import_with('Canonical mnemonic', {'passphrase':'secret'})['wallet']
    assert canonical['address'] == trimmed['address'], 'raw mnemonic derived a different identity'
    run('send','identity','--from','Canonical mnemonic')
    conditions = dict(appIsActive=True, isNetworkReachable=False, isConstrainedNetwork=False, isExpensiveNetwork=False, isLowPowerMode=False, batteryLevel=1, wantsPriceRefresh=True)
    for intent in ['user','scheduled','foreground','balancesUpdated',{'afterSend':{'chain_id':'ethereum'}},{'chain':{'chain_id':'bitcoin'}}]:
        result = run('diagnostics','refresh','--intent',json.dumps(intent),'--conditions',json.dumps(conditions))['refresh']
        assert result['pending'] is None and result['failures'] == []
    run('diagnostics','refresh','--intent',json.dumps({'afterSend':{'chain_id':'missing'}}),'--conditions',json.dumps(conditions),success=False)
    result = run('diagnostics','refresh','--intent',json.dumps({'deepRescan':{'chain_id':'bitcoin'}}),'--conditions',json.dumps(conditions))['refresh']
    assert result['failures'] and result['pending'] is None, 'offline rescan falsely succeeded'
    for chain in ['ethereum','missing']:
        run('diagnostics','refresh','--intent',json.dumps({'deepRescan':{'chain_id':chain}}),'--conditions',json.dumps(conditions),success=False)
print('shell boundary: exact passphrases, unsupported input refusal, no partial imports, offline refresh and invalid-network refusal passed')
