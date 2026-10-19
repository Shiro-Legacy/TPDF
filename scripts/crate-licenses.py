#!/usr/bin/env python3
"""Collects the licence texts of every crate that ships in the Rust Mac app.

usage: crate-licenses.py CRATES_WORKSPACE OUT_DIR

"Ships" = reachable from tandem-desktop through normal (not build or dev)
dependencies on aarch64-apple-darwin, which is what is linked into the binary.
Writes OUT_DIR/THIRD-PARTY.txt (name, version, licence, source) and copies
each crate's LICENSE*/COPYING*/NOTICE* files into OUT_DIR/crates/<name>-<version>/.
Exits 1 if a shipped crate's licence is outside the allowed set and not in
ACCEPTED below (each acceptance is a recorded owner question).
"""
import json
import os
import shutil
import subprocess
import sys

ALLOWED = ("MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "0BSD", "Unlicense", "MIT-0", "CC0-1.0")
# Shipped through Tauri, unmodified; see docs/rust-and-ux-plan.md "Questions for the owner".
ACCEPTED = {"option-ext": "MPL-2.0", "Unicode-3.0": "Unicode-3.0"}


def acceptable(name, lic):
    if not lic:
        return False
    lic = lic.replace("/", " OR ")
    for alt in lic.split(" OR "):
        parts = [p.strip(" ()") for p in alt.replace(" AND ", "&").split("&")]
        if all(p in ALLOWED or p == "Unicode-3.0" for p in parts):
            return True
    return name in ACCEPTED


def main():
    ws, out = sys.argv[1], sys.argv[2]
    meta = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--filter-platform", "aarch64-apple-darwin"], cwd=ws))
    pkgs = {(p["name"], p["version"]): p for p in meta["packages"]}
    # `cargo tree` honours the feature resolver (build-only features stay out); cargo metadata
    # unifies them. Walk normal edges and skip everything under a proc-macro (compile time only).
    tree = subprocess.check_output(
        ["cargo", "tree", "-p", "tandem-desktop", "-e", "normal", "--target", "aarch64-apple-darwin",
         "--prefix", "depth", "--format", "{p}"], cwd=ws, text=True)
    seen, skip_below = set(), None
    for line in tree.splitlines():
        digits = len(line) - len(line.lstrip("0123456789"))
        if not digits:
            continue
        depth, rest = int(line[:digits]), line[digits:]
        if skip_below is not None and depth > skip_below:
            continue
        skip_below = None
        name, version = rest.split()[0], rest.split()[1].lstrip("v")
        if "(proc-macro)" in rest:
            skip_below = depth
            continue
        seen.add((name, version))
    rows, bad = [], []
    os.makedirs(os.path.join(out, "crates"), exist_ok=True)
    for key in sorted(seen):
        p = pkgs.get(key)
        if p is None:
            continue
        if p.get("source") is None:  # our own crates and the engine (MIT, see LICENSE and NOTICE)
            continue
        lic = p.get("license") or "(see files)"
        rows.append(f"{p['name']} {p['version']}\t{lic}\t{p.get('repository') or ''}")
        if not acceptable(p["name"], p.get("license")):
            bad.append(f"{p['name']} {p['version']}: {lic}")
        src = os.path.dirname(p["manifest_path"])
        dest = os.path.join(out, "crates", f"{p['name']}-{p['version']}")
        found = False
        for f in os.listdir(src):
            if f.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE")):
                os.makedirs(dest, exist_ok=True)
                path = os.path.join(src, f)
                if os.path.isfile(path):
                    shutil.copy(path, dest)
                    found = True
        # Crates published without their licence files: the texts tandem-sign keeps
        # (crates/tandem-sign/licenses), else a pointer to the standard texts below.
        kept = os.path.join(ws, "tandem-sign", "licenses", f"{p['name']}-{p['version']}")
        if not found and os.path.isdir(kept):
            shutil.copytree(kept, dest, dirs_exist_ok=True)
            found = True
        if not found:
            os.makedirs(dest, exist_ok=True)
            with open(os.path.join(dest, "LICENSE-NOTE.txt"), "w") as fh:
                fh.write(f"{p['name']} {p['version']} is published under {lic} without a licence file.\n"
                         f"Its authors: {', '.join(p.get('authors') or ['see the repository'])}.\n"
                         f"Source: {p.get('repository') or 'crates.io'}\n"
                         "The standard licence texts are in ../../standard/.\n")
    std = os.path.join(os.path.dirname(os.path.abspath(__file__)), "licenses-standard")
    if os.path.isdir(std):
        shutil.copytree(std, os.path.join(out, "standard"), dirs_exist_ok=True)
    with open(os.path.join(out, "THIRD-PARTY.txt"), "w") as fh:
        fh.write("Crates linked into Tandem PDF for Mac (name version, licence, source).\n\n")
        fh.write("\n".join(rows) + "\n")
    print(f"{len(rows)} third-party crates; licence files in {out}/crates")
    if bad:
        print("Outside the allowed licences and not accepted:\n  " + "\n  ".join(bad))
        sys.exit(1)


if __name__ == "__main__":
    main()
