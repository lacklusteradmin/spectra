#!/usr/bin/env python3
"""Offline regression: an unreadable replacement list must never look empty."""
import pathlib
import sqlite3
import subprocess
import sys
import tempfile

binary = str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix="spectra-history-check-") as directory:
    def run(*args):
        return subprocess.run(
            [binary, "--data-dir", directory, "--json", *args],
            capture_output=True, text=True, check=False,
        )

    initialized = run("txs", "--replaceable")
    assert initialized.returncode == 0, initialized.stderr
    with sqlite3.connect(pathlib.Path(directory) / "spectra.sqlite") as db:
        raw = '{"id":42}'
        db.execute(
            "INSERT INTO history_records(id,chain_name,created_at,payload) VALUES(?,?,?,?)",
            ("fault", "Bitcoin", 0, raw),
        )
    for args in [("txs", "--replaceable"), ("txs", "--poll-chain", "Ethereum")]:
        result = run(*args)
        assert result.returncode != 0, result.stdout
    with sqlite3.connect(pathlib.Path(directory) / "spectra.sqlite") as db:
        assert db.execute("SELECT payload FROM history_records WHERE id='fault'").fetchone() == (raw,)
