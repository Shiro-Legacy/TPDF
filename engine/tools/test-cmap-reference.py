#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Adapter safety/ABI assertions; mapping differential cases live separately."""
import argparse
import hashlib
import json
import pathlib
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("oracle", type=pathlib.Path)
    args = parser.parse_args()
    oracle = args.oracle.resolve()
    cmap = b"""begincmap
1 begincodespacerange <80000000> <FFFFFFFF> endcodespacerange
2 beginbfchar <80000000> <D800DC000041> <FFFFFFFF> <FFFF> endbfchar
endcmap
"""
    cases = 0
    with tempfile.TemporaryDirectory(prefix="tandem-cmap-oracle-") as temporary:
        path = pathlib.Path(temporary) / "queries"

        def run(queries, source=cmap):
            path.write_bytes(queries)
            return subprocess.run(
                [str(oracle), str(path)], input=source,
                capture_output=True, timeout=20, check=False,
            )

        result = run(b"lookup 80000000\nlookup ffffffff\ndecode 80000000\ndecode ffffffff\ndecode -\ndecode 80\nlookup 00000000\n")
        assert result.returncode == 0, result.stderr
        rows = json.loads(result.stdout)["queries"]
        assert rows[0]["value"] == 0x80000000 and rows[0]["scalars"] == [0x10000, 65]
        assert rows[1]["value"] == 0xffffffff and rows[1]["scalars"] == [0xffff]
        assert all(row["consumed"] == 4 and row["matched_codespace"] for row in rows[2:4])
        assert all(row["consumed"] == 1 and not row["matched_codespace"] for row in rows[4:6])
        assert rows[6]["scalars"] is None
        cases += 7
        for bad in [b"lookup 0", b"lookup 0000000000", b"lookup -", b"decode gg", b"lookup 01 extra", b"lookup 01\x00x\n", b"decode ", b"\n", b"Lookup 01"]:
            result = run(bad)
            assert result.returncode == 1 and not result.stdout, (bad, result)
            cases += 1
        recovery = b"begincmap 1 begincodespacerange <00><ff> endcodespacerange 2 beginbfchar <01><10400> <02><D800> endbfchar /Identity-H usecmap endcmap"
        result = run(b"lookup 01\nlookup 02\n", recovery)
        assert result.returncode == 0, result.stderr
        parsed = json.loads(result.stdout)
        assert parsed["usecmap_ignored"] and parsed["lookup_widthless"]
        assert parsed["queries"][0]["scalars"] == [0x1040]
        assert parsed["queries"][1]["scalars"] == [0xd800]
        assert not parsed["queries"][1]["valid_scalars"]
        cases += 3
        for queries, source in [(b"lookup 01\n" * 262145, cmap), (b"", b" " * (4 * 1024 * 1024 + 1))]:
            result = run(queries, source)
            assert result.returncode == 1 and not result.stdout
            cases += 1
        # JSON output limit: each mapping has 256 valid Unicode scalars. Stop
        # before publishing any partial output even though prior rows succeeded.
        large_sequence = b"begincmap 1 begincodespacerange <00><ff> endcodespacerange 1 beginbfchar <01><" + b"0041" * 256 + b"> endbfchar endcmap"
        result = run(b"lookup 01\n" * 100000, large_sequence)
        assert result.returncode == 2 and not result.stdout, (result.returncode, result.stderr)
        assert b"output limit" in result.stderr
        cases += 1
    print(json.dumps({"adapter_assertions": cases, "oracle_sha256": hashlib.sha256(oracle.read_bytes()).hexdigest()}))


if __name__ == "__main__":
    main()
