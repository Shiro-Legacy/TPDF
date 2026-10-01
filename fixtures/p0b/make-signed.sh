#!/bin/bash
# P0-b probe 3: self-signed CA + two leaves. Keys stay in fixtures/p0b/.ca/ (gitignored).
# Usage: bash fixtures/p0b/make-signed.sh
# Tools: openssl 3.x (Apache-2.0). No network.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
CA="$ROOT/fixtures/p0b/.ca"
mkdir -p "$CA"
cd "$CA"
if [[ -f ca.pem && -f leaf.pem && -f other-ca.pem && -f other-leaf.pem ]]; then
  echo "certs already in $CA"
  exit 0
fi
cat > ca.cnf <<'EOF'
[req]
distinguished_name=dn
x509_extensions=v3
[dn]
[v3]
basicConstraints=critical,CA:TRUE
keyUsage=critical,keyCertSign,cRLSign
EOF
cat > leaf.cnf <<'EOF'
[v3]
basicConstraints=CA:FALSE
keyUsage=critical,digitalSignature,nonRepudiation
EOF
openssl genrsa -out ca.key 2048 2>/dev/null
openssl req -x509 -new -key ca.key -sha256 -days 3650 -subj "/CN=P0B Test CA" -config ca.cnf -extensions v3 -out ca.pem
openssl genrsa -out leaf.key 2048 2>/dev/null
openssl req -new -key leaf.key -subj "/CN=P0B Signer" -out leaf.csr
openssl x509 -req -in leaf.csr -CA ca.pem -CAkey ca.key -CAcreateserial -sha256 -days 3650 -extfile leaf.cnf -extensions v3 -out leaf.pem 2>/dev/null
openssl genrsa -out other-ca.key 2048 2>/dev/null
openssl req -x509 -new -key other-ca.key -sha256 -days 3650 -subj "/CN=P0B Other CA" -config ca.cnf -extensions v3 -out other-ca.pem
openssl genrsa -out other-leaf.key 2048 2>/dev/null
openssl req -new -key other-leaf.key -subj "/CN=P0B Untrusted Signer" -out other-leaf.csr
openssl x509 -req -in other-leaf.csr -CA other-ca.pem -CAkey other-ca.key -CAcreateserial -sha256 -days 3650 -extfile leaf.cnf -extensions v3 -out other-leaf.pem 2>/dev/null
rm -f leaf.csr other-leaf.csr ca.cnf leaf.cnf
echo "wrote $CA/{ca,leaf,other-ca,other-leaf}.{pem,key}"
openssl x509 -in ca.pem -noout -subject
openssl x509 -in leaf.pem -noout -subject -issuer
