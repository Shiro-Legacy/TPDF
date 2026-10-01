# fixtures/p5

Regenerate with OpenSSL 3 (`-legacy` is an OpenSSL 3 flag; it is absent on OpenSSL 1.1). No network. Password for every file is `pdf4all`. Keys come from `fixtures/p0b/.ca/` (run `bash fixtures/p0b/make-signed.sh` first). The p12 and pem outputs contain the leaf key and are gitignored.

```
bash fixtures/p5/make-p12.sh
.venv-qa/bin/python fixtures/p5/make-certified-p1.py
bash fixtures/p5/make-verify-fixtures.sh
python3 fixtures/p5/make-empty-sigfield.py
```

| file | what |
|---|---|
| `leaf.p12` | openssl default: PBES2 / AES-256, PBKDF2-HMAC-SHA256 |
| `leaf-legacy.p12` | `openssl pkcs12 -legacy`: certs RC2-40, key 3DES. Repo Node 22 cannot read it; Electron's Node can |
| `leaf-ec.p12` | P-256 leaf. The app refuses it as `unsupported-key` |
| `leaf.pem` | unencrypted key + leaf + CA |
| `leaf-enc.pem` | same key, `aes-256-cbc`, plus the two certs |
| `certified-p1.pdf` | pyHanko DocMDP P=1 (no changes). Signing time is in the file, so bytes change every run |
| `signed-ec.pdf` | ECDSA P-256 leaf `fixtures/p0b/.ca/ec.pem`, signed by P0B Test CA |
| `signed-sha1.pdf` | RSA leaf, `md_algorithm=sha1`. Valid, reason `weak-digest` |
| `signed-sha1-notime.pdf` | `node fixtures/p5/make-sha1-notime.mjs` (after build): `signed-sha1.pdf` with `/M` blanked, hole re-signed SHA-1 without signingTime. Valid, trusted, reason `weak-digest no-signing-time` |
| `signed-expired.pdf` | leaf notBefore/notAfter in January 2020. Valid signature, trust `expired` |
| `signed-false-ca.pdf` | Forged Bob, issued by Honest Alice (`CA:FALSE`), issued by P0B Test CA. Valid signature, trust `untrusted` |
| `false-ca/` | alice/bob2/expired material from the §8 probe recipe. `*.key` are private |
| `empty-sigfield.pdf` | `python3 fixtures/p5/make-empty-sigfield.py` (stdlib + `fixtures/minipdf.py`, deterministic, 1581 B). Three empty `/Sig` fields: Signature1 (obj 7, no /AP) and Signature2 (obj 8, grey /AP) on page 0, Signature3 (obj 10) on the `/Rotate 90` page 1; `/SigFlags 1` |
