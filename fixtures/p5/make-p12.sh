#!/bin/bash
# Two PKCS#12 files from the P0-b leaf. Password: pdf4all. Keys stay gitignored.
# Usage: bash fixtures/p5/make-p12.sh
set -euo pipefail
# Git Bash on Windows would rewrite -subj "/CN=..." as a Windows path (file paths still convert).
export MSYS2_ARG_CONV_EXCL="/CN="
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
CA="$ROOT/fixtures/p0b/.ca"
OUT="$ROOT/fixtures/p5"
test -f "$CA/leaf.key" || { echo "missing $CA; run bash fixtures/p0b/make-signed.sh"; exit 1; }
openssl pkcs12 -export -inkey "$CA/leaf.key" -in "$CA/leaf.pem" -certfile "$CA/ca.pem" \
  -name "P0B Signer" -passout pass:pdf4all -out "$OUT/leaf.p12"
openssl pkcs12 -export -legacy -inkey "$CA/leaf.key" -in "$CA/leaf.pem" -certfile "$CA/ca.pem" \
  -name "P0B Signer" -passout pass:pdf4all -out "$OUT/leaf-legacy.p12"
# P-256, for the unsupported-key refusal. Key stays in the gitignored CA dir.
openssl ecparam -name prime256v1 -genkey -noout -out "$CA/ec.key"
openssl req -new -key "$CA/ec.key" -subj "/CN=P0B EC" -out "$CA/ec.csr"
openssl x509 -req -in "$CA/ec.csr" -CA "$CA/ca.pem" -CAkey "$CA/ca.key" -CAcreateserial -sha256 -days 3650 -out "$CA/ec.pem" 2>/dev/null
openssl pkcs12 -export -inkey "$CA/ec.key" -in "$CA/ec.pem" -passout pass:pdf4all -out "$OUT/leaf-ec.p12"
rm -f "$CA/ec.csr"
# PEM: unencrypted key+certs, and the same key under aes-256-cbc.
cat "$CA/leaf.key" "$CA/leaf.pem" "$CA/ca.pem" > "$OUT/leaf.pem"
openssl pkey -in "$CA/leaf.key" -aes-256-cbc -passout pass:pdf4all -out "$OUT/leaf-enc.key.pem"
cat "$OUT/leaf-enc.key.pem" "$CA/leaf.pem" "$CA/ca.pem" > "$OUT/leaf-enc.pem"
rm -f "$OUT/leaf-enc.key.pem"
echo "wrote $OUT/leaf.p12 (PBES2), leaf-legacy.p12 (RC2/3DES), leaf-ec.p12, leaf.pem, leaf-enc.pem"
