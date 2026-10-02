#!/usr/bin/env python3
"""Wallet deletion preserves keys on database failure and retries durable cleanup."""
import json
import os
import pathlib
import sqlite3
import subprocess
import sys
import tempfile

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/spectra").resolve())
mnemonic = "test test test test test test test test test test test junk"

with tempfile.TemporaryDirectory(prefix="spectra-wallet-deletion-") as directory:
    environment = dict(os.environ, SPECTRA_SEED=mnemonic)

    def run(*arguments, status=0):
        result = subprocess.run(
            [binary, "--data-dir", directory, "--json", *arguments],
            env=environment, text=True, capture_output=True, timeout=30,
        )
        assert result.returncode == status, (arguments, result.returncode, result.stdout, result.stderr)
        return json.loads(result.stdout)

    imported = run("wallet", "import", "--chain", "Ethereum", "--no-password")
    wallet_id = imported["wallet"]["id"]
    database_path = pathlib.Path(directory) / "spectra.sqlite"
    with sqlite3.connect(database_path) as database:
        database.execute("CREATE TRIGGER reject_delete BEFORE DELETE ON wallets BEGIN SELECT RAISE(ABORT, 'injected'); END")
    run("wallet", "delete", wallet_id, "--yes", status=1)
    assert run("wallet", "list")["wallets"][0]["id"] == wallet_id
    assert mnemonic in json.dumps(run("wallet", "export", wallet_id, "--yes"))
    with sqlite3.connect(database_path) as database:
        assert database.execute("SELECT COUNT(*) FROM wallet_secret_deletions").fetchone()[0] == 0
        database.execute("DROP TRIGGER reject_delete")
        # Failure after successful secret cleanup leaves durable retry work.
        database.execute("CREATE TRIGGER reject_ack BEFORE DELETE ON wallet_secret_deletions BEGIN SELECT RAISE(ABORT, 'injected'); END")
    assert run("wallet", "delete", wallet_id, "--yes")["ok"]
    with sqlite3.connect(database_path) as database:
        assert database.execute("SELECT COUNT(*) FROM wallets").fetchone()[0] == 0
        assert database.execute("SELECT wallet_id FROM wallet_secret_deletions").fetchone()[0] == wallet_id
        database.execute("DROP TRIGGER reject_ack")
    # The next CLI invocation retries cleanup before loading its state.
    assert run("wallet", "list")["wallets"] == []
    with sqlite3.connect(database_path) as database:
        assert database.execute("SELECT COUNT(*) FROM wallet_secret_deletions").fetchone()[0] == 0
    assert not list((pathlib.Path(directory) / "secrets" / "seed").iterdir())

print("Wallet database rollback and durable secret cleanup checks passed")
