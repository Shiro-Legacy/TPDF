#!/bin/bash
# P5 U2 verification fixtures. Needs openssl 3 and .venv-qa (pyHanko). No network.
# Certs: bash fixtures/p0b/make-signed.sh && bash fixtures/p5/make-p12.sh first.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
if [[ ! -f fixtures/p0b/.ca/ca.pem || ! -f fixtures/p0b/.ca/ec.pem ]]; then
  echo "missing fixtures/p0b/.ca — run bash fixtures/p0b/make-signed.sh && bash fixtures/p5/make-p12.sh" >&2
  exit 1
fi
exec .venv-qa/bin/python fixtures/p5/make-verify-fixtures.py
