//! Certificate signing and verification in the Rust app: the host's certificate hold and
//! signing (`host/sign.rs`), the trusted list (`host/trust.rs`), the engine's `sign-prepare`,
//! `sign-list` and signature-field ops, and `tandem-sign` underneath. Ported from the
//! Electron-era suites tests/p5-sign-crypto, p5-sign, p5-sign-field, p5-verify,
//! p5-verify-crypto, sign-differential, sign-worker, one check of p5-drive-field and one of
//! p1-ux, which drove `build/sign/*.js` (the WASM build of the same crate).
//!
//! OpenSSL (Homebrew's, or an OpenSSL 3 on PATH) and pyHanko (`.venv-qa/bin/python` with
//! `scripts/verify-sign.py`) stay test-only oracles, as they were for the JS: a check that
//! needs one says so on stderr and passes when it is missing. The engine's own strict reader
//! (tandem_pdf_engine::document) stands in for MuPDF where a test looks inside an output.
// Some helpers serve only the Unix tests (modes, links).
#![cfg_attr(not(unix), allow(unused_imports, dead_code))]

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tandem_app::engine::{EngineHost, Reply, entropy, now_ms};
use tandem_app::host::sign::{CertInfo, SignFail, SignOffsets, signing_when_ok};
use tandem_app::host::util::{mkdtemp, temp_dir};
use tandem_app::host::{Host, HostConfig};
use tandem_app::platform::{
    Answers, Filter, MessageBox, Pick, Platform, PrintOutcome, Question, SavePurpose, Scripted,
};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Object, ObjectId, decode_text_string};
use tandem_pdf_engine::stream::StreamLimits;
use tandem_sign::cert::{Cert, certs_from_pem};
use tandem_sign::cms::{self, MAX_SIG_CONTENTS_BYTES};
use tandem_sign::drbg::HmacDrbg;
use tandem_sign::identity::Identity;

const PASSWORD: &str = "pdf4all";
const SENTINEL: &[u8] = b"[0 1000000000 1000000000 1000000000]";

// ------------------------------------------------------------------------------------ helpers

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(rel: &str) -> PathBuf {
    root().join("fixtures").join(rel)
}

fn read_fixture(rel: &str) -> Vec<u8> {
    fs::read(fixture(rel)).unwrap_or_else(|e| panic!("fixtures/{rel}: {e}"))
}

fn ca_pem() -> PathBuf {
    fixture("p0b/.ca/ca.pem")
}

fn s(path: &Path) -> &str {
    path.to_str().expect("an ASCII temp path")
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `value` with every number as a float, so 72 and 72.0 compare equal.
fn norm(value: &Value) -> Value {
    match value {
        Value::Number(n) => json!(n.as_f64().unwrap()),
        Value::Array(items) => Value::Array(items.iter().map(norm).collect()),
        Value::Object(map) => {
            Value::Object(map.iter().map(|(k, v)| (k.clone(), norm(v))).collect())
        }
        other => other.clone(),
    }
}

/// `actual` equals `expected`, numbers compared as numbers.
#[track_caller]
fn same(actual: &Value, expected: Value) {
    assert_eq!(norm(actual), norm(&expected), "{actual} vs {expected}");
}

/// A temp folder of the test's own, removed when the test ends.
struct Work(PathBuf);

impl Work {
    fn new(tag: &str) -> Work {
        Work(mkdtemp(&temp_dir().join(format!("tandem-sign-{tag}-"))).unwrap())
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The signing key material: not in git (fixtures/p0b/make-signed.sh, fixtures/p5/make-p12.sh),
/// so a test that needs it says so and passes without it, as tandem-sign's tests do.
fn keys() -> bool {
    let missing: Vec<&str> = [
        "p0b/.ca/ca.pem",
        "p0b/.ca/ca.key",
        "p0b/.ca/leaf.pem",
        "p0b/.ca/leaf.key",
        "p0b/.ca/ec.pem",
        "p5/leaf.p12",
        "p5/leaf-ec.p12",
    ]
    .into_iter()
    .filter(|rel| !fixture(rel).is_file())
    .collect();
    if !missing.is_empty() {
        eprintln!("skipped: signing key material is missing: {missing:?}");
    }
    missing.is_empty()
}

/// A smoke host (memory-only stores, no questions), as a drive runs it.
fn smoke_host(work: &Work) -> Host {
    Host::new(
        HostConfig {
            user_data: work.path("udata"),
            smoke: true,
            echo_stdout: false,
        },
        Box::new(Scripted::default()),
    )
}

/// A smoke host that trusts the fixture CA.
fn trusting_host(work: &Work) -> Host {
    let mut host = smoke_host(work);
    assert_eq!(host.trust_add(Some(s(&ca_pem()))), Ok((1, 1)));
    host
}

fn answers(host: &mut Host) -> &mut Answers {
    host.platform.answers().expect("a scripted platform")
}

/// Picks `cert` and reads it with `password` (`pick-certificate` + `certificate-info`).
fn load(host: &mut Host, cert: &Path, password: &str) -> Result<CertInfo, SignFail> {
    assert!(host.pick_certificate(Some(s(cert))).is_ok());
    host.certificate_info(password)
}

// The engine, through the app's own wrapper.

fn call(engine: &mut EngineHost, request: Value) -> Value {
    engine.dispatch(request, Vec::new()).0
}

fn ok(engine: &mut EngineHost, request: Value) -> Reply {
    engine
        .request(request.clone(), Vec::new())
        .unwrap_or_else(|e| panic!("{request}: {e} {}", e.reply))
}

fn open(engine: &mut EngineHost, name: &str, bytes: Vec<u8>) -> Reply {
    engine
        .request(
            json!({"op": "open", "name": name, "pdf": {"$buffer": 0}}),
            vec![bytes],
        )
        .unwrap_or_else(|e| panic!("open {name}: {e}"))
}

fn opened(name: &str, bytes: Vec<u8>) -> (EngineHost, Reply) {
    let mut engine = EngineHost::new();
    let reply = open(&mut engine, name, bytes);
    (engine, reply)
}

fn signatures_of(reply: &Reply) -> u64 {
    reply.value["info"]["signatures"]
        .as_u64()
        .unwrap_or_else(|| panic!("no info.signatures in {}", reply.value))
}

fn fields(engine: &mut EngineHost, page: u64) -> Vec<Value> {
    ok(engine, json!({"op": "fields", "page": page})).value["items"]
        .as_array()
        .unwrap()
        .clone()
}

fn by_name(engine: &mut EngineHost, name: &str, page: u64) -> Value {
    fields(engine, page)
        .into_iter()
        .find(|item| item["name"] == name)
        .unwrap_or_else(|| panic!("no field {name} on page {page}"))
}

fn save(engine: &mut EngineHost) -> Vec<u8> {
    ok(engine, json!({"op": "save"}))
        .take_bytes("bytes")
        .unwrap()
}

/// The journal fields every reply carries.
fn journal(engine: &EngineHost) -> Value {
    let state = engine.session_state();
    json!({"revision": state["revision"], "dirty": state["dirty"],
        "canUndo": state["canUndo"], "canRedo": state["canRedo"]})
}

/// A refusal with `reason` that leaves the journal as it was (and clean).
fn refuses(engine: &mut EngineHost, request: Value, reason: &str) -> Value {
    let before = journal(engine);
    let reply = call(engine, request.clone());
    assert_eq!(reply["ok"], false, "{request} was accepted");
    assert_eq!(reply["reason"], reason, "{request}: {reply}");
    assert_eq!(journal(engine), before);
    assert_eq!(before["dirty"], false);
    reply
}

/// `sign-prepare`'s reply, bytes and hole.
struct Prepared {
    reply: Value,
    bytes: Vec<u8>,
    offsets: SignOffsets,
}

fn prepare_request(fields: Value) -> Value {
    let mut request = json!({"op": "sign-prepare", "name": "P0B Signer", "reason": "field test",
        "page": 0, "rect": null, "certify": false, "when": now_ms()});
    if let (Value::Object(request), Value::Object(fields)) = (&mut request, fields) {
        request.extend(fields);
    }
    request
}

fn prepare(engine: &mut EngineHost, fields: Value) -> Prepared {
    let mut reply = ok(engine, prepare_request(fields));
    let at = |key: &str| reply.value[key].as_u64().unwrap();
    let offsets = SignOffsets {
        contents_at: at("contentsAt"),
        contents_len: at("contentsLen"),
        byte_range_at: at("byteRangeAt"),
    };
    let bytes = reply.take_bytes("bytes").unwrap();
    Prepared {
        reply: reply.value,
        bytes,
        offsets,
    }
}

/// The placeholders sit where the reply says (tests/p5-sign.test.mjs `exactOnce`).
fn exact_once(p: &Prepared) {
    let o = p.offsets;
    let (at, len, range) = (
        o.contents_at as usize,
        o.contents_len as usize,
        o.byte_range_at as usize,
    );
    assert_eq!(&p.bytes[range..range + 36], SENTINEL);
    assert_eq!(p.bytes[at - 1], b'<');
    assert_eq!(p.bytes[at + len], b'>');
    assert_eq!(len as u64, MAX_SIG_CONTENTS_BYTES * 2);
    assert!(p.bytes[at..at + len].iter().all(|&b| b == b'0'));
}

/// Signs `p` through the host with `cert` and returns the written file's bytes.
fn host_sign(host: &mut Host, work: &Work, cert: &Path, password: &str, p: &Prepared) -> Vec<u8> {
    load(host, cert, password).expect("the certificate reads");
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let out = work.path(&format!(
        "signed-{}.pdf",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    answers(host).save_to = Some(out.clone());
    let signed = host
        .sign_file(p.bytes.clone(), p.offsets, now_ms(), None)
        .expect("signed");
    assert!(!host.holding_certificate(), "signing spends the key");
    assert!(!signed.opened, "a smoke run stays on the original");
    fs::read(out).unwrap()
}

/// What the Verify dialog reads: `sign-list` on the reopened bytes, each entry's contents as
/// hex (session/verify.rs `entries_for_host`), and the source bytes it reported.
struct Listed {
    value: Value,
    pdf: Vec<u8>,
    entries: Vec<Value>,
}

fn sign_list(bytes: &[u8]) -> Listed {
    let (mut engine, _) = opened("check.pdf", bytes.to_vec());
    let listed = ok(&mut engine, json!({"op": "sign-list"}));
    let entries = listed.value["signatures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            let mut entry = entry.clone();
            if let Some(contents) = listed.bytes_at(entry.get("contents")) {
                entry["contents"] = json!(hex(contents));
            }
            entry
        })
        .collect();
    Listed {
        pdf: listed.bytes("bytes").unwrap_or_default().to_vec(),
        value: listed.value,
        entries,
    }
}

fn verdicts(host: &mut Host, pdf: &[u8], entries: &[Value]) -> Vec<Value> {
    host.verify_signatures(pdf, entries).unwrap()["verdicts"]
        .as_array()
        .unwrap()
        .clone()
}

/// The verdicts for every signature in `bytes`, as Verify gives them.
fn verify_file(host: &mut Host, bytes: &[u8]) -> Vec<Value> {
    let listed = sign_list(bytes);
    verdicts(host, &listed.pdf, &listed.entries)
}

/// Each signature widget: whether it holds /V, and its /AP /N stream as text.
fn widget(bytes: &[u8], number: u32) -> (bool, Option<String>) {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    let widget = doc
        .resolve_id(ObjectId::new(number, 0).unwrap())
        .as_dict()
        .unwrap_or_else(|| panic!("object {number} is no dictionary"));
    let has_v = widget.get(b"V".as_slice()).is_some();
    let ap = widget
        .get(b"AP".as_slice())
        .and_then(|ap| doc.resolve_dict(ap))
        .and_then(|ap| ap.get(b"N".as_slice()))
        .and_then(Object::as_reference)
        .map(|n| latin1(&doc.decoded_stream(n, StreamLimits::default()).unwrap()));
    (has_v, ap)
}

// The test-only oracles.

/// An OpenSSL 3 command, or `None` (the check is skipped).
fn openssl() -> Option<&'static Path> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        [
            "/opt/homebrew/bin/openssl",
            "/usr/local/bin/openssl",
            "openssl",
        ]
        .into_iter()
        .map(PathBuf::from)
        .find(|bin| {
            Command::new(bin)
                .arg("version")
                .output()
                .is_ok_and(|o| o.stdout.starts_with(b"OpenSSL 3"))
        })
    })
    .as_deref()
    .or_else(|| {
        eprintln!("skipped: no OpenSSL 3 (/opt/homebrew/bin/openssl or on PATH)");
        None
    })
}

fn run_openssl(args: &[&str]) -> Output {
    Command::new(openssl().unwrap())
        .args(args)
        // Windows OpenSSL reads its arguments in the ANSI code page unless told they are
        // UTF-8: a non-ASCII password or subject would arrive as other bytes or `?`.
        .env("OPENSSL_WIN32_UTF8", "1")
        .output()
        .unwrap()
}

/// Runs openssl and insists it succeeded.
fn ossl(args: &[&str]) -> Output {
    let out = run_openssl(args);
    assert!(
        out.status.success(),
        "openssl {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// `openssl cms -verify` of a detached CMS over `content` under the fixture CA.
fn openssl_verifies(work: &Work, name: &str, cms: &[u8], content: &[u8]) -> bool {
    let sig = work.path(&format!("{name}.der"));
    let data = work.path(&format!("{name}.bin"));
    // A file of its own, not /dev/null, which Windows OpenSSL cannot open.
    let out = work.path(&format!("{name}.out"));
    fs::write(&sig, cms).unwrap();
    fs::write(&data, content).unwrap();
    run_openssl(&[
        "cms",
        "-verify",
        "-inform",
        "DER",
        "-in",
        s(&sig),
        "-content",
        s(&data),
        "-CAfile",
        s(&ca_pem()),
        "-binary",
        "-purpose",
        "any",
        "-out",
        s(&out),
    ])
    .status
    .success()
}

static SERIAL: AtomicU64 = AtomicU64::new(0x7e57_0000);

/// A leaf the fixture CA issues; `keygen` are `openssl genpkey` arguments, `ext` the v3
/// section and `x509` extra `openssl x509` arguments (default: 30 days). Returns the key, the
/// certificate and a `.pem` holding both (what "Sign digitally" reads).
struct Issued {
    key: PathBuf,
    cert: PathBuf,
    pem: PathBuf,
}

fn issue_with(work: &Work, name: &str, keygen: &[&str], ext: &str, x509: &[&str]) -> Issued {
    let serial = SERIAL.fetch_add(1, Ordering::Relaxed).to_string();
    let base: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let base = format!("{base}-{serial}");
    let key = work.path(&format!("{base}.key"));
    let csr = work.path(&format!("{base}.csr"));
    let cert = work.path(&format!("{base}.crt"));
    let cnf = work.path(&format!("{base}.cnf"));
    fs::write(&cnf, format!("[v3]\n{ext}\n")).unwrap();
    ossl(&[&["genpkey"][..], keygen, &["-out", s(&key)][..]].concat());
    let subject = format!("/CN={name}");
    ossl(&[
        "req",
        "-new",
        "-utf8",
        "-key",
        s(&key),
        "-subj",
        &subject,
        "-out",
        s(&csr),
    ]);
    let ca_key = fixture("p0b/.ca/ca.key");
    let days: &[&str] = if x509.is_empty() {
        &["-days", "30"]
    } else {
        x509
    };
    ossl(
        &[
            &[
                "x509",
                "-req",
                "-in",
                s(&csr),
                "-CA",
                s(&ca_pem()),
                "-CAkey",
                s(&ca_key),
                "-set_serial",
                &serial,
                "-sha256",
                "-extfile",
                s(&cnf),
                "-extensions",
                "v3",
                "-out",
                s(&cert),
            ][..],
            days,
        ]
        .concat(),
    );
    let pem = work.path(&format!("{base}.pem"));
    fs::write(
        &pem,
        [fs::read(&key).unwrap(), fs::read(&cert).unwrap()].concat(),
    )
    .unwrap();
    Issued { key, cert, pem }
}

const SIGNING_EXT: &str = "basicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature,nonRepudiation\nsubjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid";

fn issue(work: &Work, name: &str, keygen: &[&str]) -> Issued {
    issue_with(work, name, keygen, SIGNING_EXT, &[])
}

const RSA_2048: &[&str] = &["-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048"];
const EC_P256: &[&str] = &["-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256"];
const EC_P384: &[&str] = &["-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-384"];

/// `scripts/verify-sign.py` (pyHanko) on `pdf`, or `None` when .venv-qa is missing.
fn pyhanko(work: &Work, name: &str, pdf: &[u8], args: &[&str]) -> Option<Output> {
    let py = root().join(if cfg!(windows) {
        ".venv-qa/Scripts/python.exe"
    } else {
        ".venv-qa/bin/python"
    });
    if !py.exists() {
        eprintln!("skipped pyHanko: {} is missing", py.display());
        return None;
    }
    let file = work.path(name);
    fs::write(&file, pdf).unwrap();
    Some(
        Command::new(py)
            .arg("scripts/verify-sign.py")
            .arg(&file)
            .args(args)
            .current_dir(root())
            .output()
            .unwrap(),
    )
}

/// pyHanko accepts `pdf` with `count` intact signatures and prints each of `lines`.
fn pyhanko_accepts(work: &Work, name: &str, pdf: &[u8], count: usize, lines: &[&str]) {
    let ca = ca_pem();
    let n = count.to_string();
    let args = ["--ca", s(&ca), "--expect-signatures", &n];
    let Some(run) = pyhanko(work, name, pdf, &args) else {
        return;
    };
    let out = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "{name}: {out}{}",
        String::from_utf8_lossy(&run.stderr)
    );
    for line in lines {
        assert!(out.lines().any(|l| l == *line), "{name}: {line} in {out}");
    }
}

// A small DER reader (tests/sign-oracle.mjs), independent of tandem-sign's.

#[derive(Clone, Copy, Debug)]
struct Tlv {
    tag: u8,
    start: usize,
    value_start: usize,
    end: usize,
}

fn tlv_at(buf: &[u8], at: usize) -> Tlv {
    let tag = buf[at];
    let mut len = usize::from(buf[at + 1]);
    let mut cursor = at + 2;
    if len & 0x80 != 0 {
        let count = len & 0x7f;
        len = 0;
        for _ in 0..count {
            len = (len << 8) | usize::from(buf[cursor]);
            cursor += 1;
        }
    }
    Tlv {
        tag,
        start: at,
        value_start: cursor,
        end: cursor + len,
    }
}

fn kids(buf: &[u8], node: Tlv) -> Vec<Tlv> {
    let mut out = Vec::new();
    let mut cursor = node.value_start;
    while cursor < node.end {
        let child = tlv_at(buf, cursor);
        cursor = child.end;
        out.push(child);
    }
    out
}

fn oid_of(buf: &[u8], node: Tlv) -> String {
    let body = &buf[node.value_start..node.end];
    let mut arcs = vec![u64::from(body[0] / 40), u64::from(body[0] % 40)];
    let mut value = 0u64;
    for &byte in &body[1..] {
        value = (value << 7) | u64::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            arcs.push(value);
            value = 0;
        }
    }
    arcs.iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

/// The SignerInfo's parts.
fn signer_info(cms: &[u8]) -> Vec<Tlv> {
    let top = kids(cms, tlv_at(cms, 0));
    let signed_data = kids(cms, top[1])[0];
    let set = *kids(cms, signed_data)
        .iter()
        .rfind(|part| part.tag == 0x31)
        .unwrap();
    kids(cms, kids(cms, set)[0])
}

/// The signed attributes: (OID, the attribute, its first value).
fn signed_attributes(cms: &[u8]) -> Vec<(String, Tlv, Tlv)> {
    let attrs = *signer_info(cms)
        .iter()
        .find(|part| part.tag == 0xa0)
        .unwrap();
    kids(cms, attrs)
        .into_iter()
        .map(|attr| {
            let parts = kids(cms, attr);
            (oid_of(cms, parts[0]), attr, kids(cms, parts[1])[0])
        })
        .collect()
}

/// The CMS without the zero padding of its hole.
fn cms_of(contents: &[u8]) -> Vec<u8> {
    contents[..tlv_at(contents, 0).end].to_vec()
}

/// The bytes a signature covers.
fn covered(pdf: &[u8], range: &Value) -> Vec<u8> {
    let r: Vec<usize> = range
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_u64().unwrap() as usize)
        .collect();
    [&pdf[..r[1]], &pdf[r[2]..r[2] + r[3]]].concat()
}

fn contents_of(entry: &Value) -> Vec<u8> {
    let text = entry["contents"].as_str().unwrap();
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

/// Wraps a detached CMS into the smallest file the verifier accepts (sign-differential `wrap`).
fn wrap(cms: &[u8], content: &[u8]) -> (Vec<u8>, Value) {
    let upper: String = cms.iter().map(|b| format!("{b:02X}")).collect();
    let (head, tail) = content.split_at(9);
    let src = [head, b"<", upper.as_bytes(), b">", tail].concat();
    let b = head.len();
    let c = b + upper.len() + 2;
    let entry = json!({"field": "Sig1", "page": 0, "rect": null, "empty": false,
        "subFilter": "adbe.pkcs7.detached", "byteRange": [0, b, c, tail.len()], "contents": hex(cms)});
    (src, entry)
}

fn drbg() -> HmacDrbg {
    HmacDrbg::new(&entropy(48)).unwrap()
}

/// edit-tj.pdf prepared for an invisible signature.
fn prepared_edit_tj(name: &str) -> Prepared {
    let (mut engine, _) = opened("edit-tj.pdf", read_fixture("edit-tj.pdf"));
    prepare(
        &mut engine,
        json!({"name": name, "reason": "differential test"}),
    )
}

// ------------------------------------------------------------- certificates and the hold

// was tests/p5-sign-crypto.test.mjs: 'main owns the clock: more than 10 minutes off is invalid'
#[test]
fn the_signing_time_must_be_within_ten_minutes_of_now() {
    let now = now_ms();
    assert!(signing_when_ok(now, now));
    assert!(signing_when_ok(now - 9.0 * 60_000.0, now));
    assert!(!signing_when_ok(now - 11.0 * 60_000.0, now));
    assert!(!signing_when_ok(now + 11.0 * 60_000.0, now));
    assert!(!signing_when_ok(f64::NAN, now));
}

// was tests/p5-sign-crypto.test.mjs: 'certificate-info then certificate-clear then sign-file is
// no-certificate'; tests/p1-ux.test.mjs: 'review fix 12: Help does not understate…' (the hold
// lets go right after signing); tests/sign-worker.test.mjs: 'open, sign once, and the handle is
// spent; a wrong password and a dropped handle are refused'.
#[test]
fn a_cleared_spent_or_replaced_certificate_signs_nothing() {
    if !keys() {
        return;
    }
    let work = Work::new("hold");
    let mut host = trusting_host(&work);
    let p12 = fixture("p5/leaf.p12");
    let p = prepared_edit_tj("Hold");

    // Clear, then sign: nothing is held.
    let info = load(&mut host, &p12, PASSWORD).unwrap();
    assert_eq!((info.cn.as_str(), info.certs), ("P0B Signer", 2));
    assert!(host.holding_certificate());
    host.certificate_clear();
    assert!(!host.holding_certificate());
    let refused = host.sign_file(p.bytes.clone(), p.offsets, now_ms(), None);
    assert_eq!(refused.unwrap_err().reason, "no-certificate");

    // A failed retry drops the key a previous attempt left.
    load(&mut host, &p12, PASSWORD).unwrap();
    assert_eq!(
        host.certificate_info("nope").unwrap_err().reason,
        "bad-password"
    );
    assert!(!host.holding_certificate());
    // Picking another file drops the held key too.
    load(&mut host, &p12, PASSWORD).unwrap();
    assert!(
        host.pick_certificate(Some(s(&fixture("p5/leaf-ec.p12"))))
            .is_ok()
    );
    assert!(!host.holding_certificate());

    // One shot: the key signs once, then the next sign-file has none.
    let signed = host_sign(&mut host, &work, &p12, PASSWORD, &p);
    let v = verify_file(&mut host, &signed);
    assert_eq!(
        (&v[0]["status"], &v[0]["trust"], &v[0]["signerCn"]),
        (&json!("valid"), &json!("trusted"), &json!("P0B Signer"))
    );
    let again = host.sign_file(p.bytes.clone(), p.offsets, now_ms(), None);
    assert_eq!(again.unwrap_err().reason, "no-certificate");
}

// tests/p5-sign-crypto.test.mjs: 'a held key is dropped after the idle timer' is not ported
// (P-blocked): the five-minute idle drop (host/sign.rs CERT_IDLE, the pub(crate) CertState and
// Held::since) has no test hook short of a real five-minute wait.

// was tests/p5-sign-crypto.test.mjs: 'a P-521 key is unsupported-key (signing keys are RSA, P-256
// or P-384)' and 'RSA under 2048 bits is unsupported-key'
#[test]
fn small_rsa_and_p521_signing_keys_are_unsupported() {
    if !keys() {
        return;
    }
    if openssl().is_none() {
        return;
    }
    let work = Work::new("keys");
    let mut host = smoke_host(&work);
    for (label, keygen) in [
        (
            "P521",
            &["-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-521"][..],
        ),
        (
            "Small",
            &["-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:1024"][..],
        ),
    ] {
        let issued = issue(&work, label, keygen);
        let err = load(&mut host, &issued.pem, "").unwrap_err();
        assert_eq!(err.reason, "unsupported-key", "{label}");
        assert!(!host.holding_certificate());
    }
}

// was tests/p5-sign-crypto.test.mjs: 'expired and not-yet-valid leaves are certificate-expired'
// (the crate's sentences: tandem-sign tests/fixtures.rs::expired_and_not_yet_valid_leaves);
// tests/sign-worker.test.mjs: 'an out-of-date time is refused at open and at signing' (the open
// half; at signing the host first refuses a time more than ten minutes off, and the crate's
// check at the signing time is tests/generated.rs::an_expired_link_is_expired_and_signing_refuses_an_expired_leaf).
#[test]
fn out_of_date_certificates_are_refused_with_their_date() {
    if !keys() {
        return;
    }
    if openssl().is_none() {
        return;
    }
    let work = Work::new("dates");
    let mut host = smoke_host(&work);
    for (label, window, date, status) in [
        (
            "Expired",
            [
                "-not_before",
                "20200101000000Z",
                "-not_after",
                "20200102000000Z",
            ],
            "2020-01-02",
            "this certificate expired on 2020-01-02",
        ),
        (
            "Future",
            [
                "-not_before",
                "20300101000000Z",
                "-not_after",
                "20300102000000Z",
            ],
            "2030-01-01",
            "this certificate is not valid until 2030-01-01",
        ),
    ] {
        let issued = issue_with(&work, label, EC_P256, SIGNING_EXT, &window);
        let err = load(&mut host, &issued.pem, "").unwrap_err();
        assert_eq!(
            (err.reason, err.date.as_deref(), err.status.as_deref()),
            ("certificate-expired", Some(date), Some(status)),
            "{label}"
        );
        assert_eq!(
            err.to_json(),
            json!({"ok": false, "reason": "certificate-expired", "date": date, "status": status})
        );
        assert!(!host.holding_certificate());
    }
}

// was tests/p5-sign-crypto.test.mjs: 'sign-file hole: a flipped Contents byte is unreadable, and
// an oversized CMS is cms-too-large' (the crate's half: tandem-sign
// tests/fixtures.rs::signs_a_prepared_pdf_that_then_verifies_and_tampering_fails)
#[test]
fn the_hole_is_checked_and_an_oversized_cms_is_refused() {
    if !keys() {
        return;
    }
    let work = Work::new("hole");
    let mut host = smoke_host(&work);
    let p12 = fixture("p5/leaf.p12");
    let p = prepared_edit_tj("Hole");

    let mut flipped = p.bytes.clone();
    flipped[p.offsets.contents_at as usize] = b'1';
    load(&mut host, &p12, PASSWORD).unwrap();
    answers(&mut host).save_to = Some(work.path("flipped.pdf"));
    let err = host.sign_file(flipped, p.offsets, now_ms(), None);
    assert_eq!(err.unwrap_err().reason, "unreadable");
    assert!(!work.path("flipped.pdf").exists());

    // A chain too long for the 8 KiB the PDF reserves.
    let ca = fs::read(ca_pem()).unwrap();
    let mut long = [
        fs::read(fixture("p0b/.ca/leaf.key")).unwrap(),
        fs::read(fixture("p0b/.ca/leaf.pem")).unwrap(),
    ]
    .concat();
    for _ in 0..12 {
        long.extend_from_slice(&ca);
    }
    let long_pem = work.path("long-chain.pem");
    fs::write(&long_pem, long).unwrap();
    assert!(load(&mut host, &long_pem, "").unwrap().certs > 10);
    answers(&mut host).save_to = Some(work.path("too-large.pdf"));
    let err = host.sign_file(p.bytes.clone(), p.offsets, now_ms(), None);
    assert_eq!(err.unwrap_err().reason, "cms-too-large");
    assert!(!work.path("too-large.pdf").exists());

    let signed = host_sign(&mut host, &work, &p12, PASSWORD, &p);
    let range = p.offsets.byte_range_at as usize;
    assert_ne!(&signed[range..range + 36], SENTINEL);
    assert_eq!(signed.len(), p.bytes.len());
}

// was tests/sign-worker.test.mjs: 'a certificate not meant for signing documents opens with
// forSigning false (review p2b L4)'; in a window the host asks before signing with it.
#[test]
fn a_certificate_not_meant_for_signing_warns_and_asks_first() {
    if !keys() {
        return;
    }
    if openssl().is_none() {
        return;
    }
    let work = Work::new("tls");
    let tls = issue_with(
        &work,
        "TLS Only",
        EC_P256,
        "basicConstraints=CA:FALSE\nkeyUsage=critical,keyEncipherment\nextendedKeyUsage=serverAuth",
        &["-days", "2"],
    );
    let mut host = smoke_host(&work);
    let info = load(&mut host, &tls.pem, "").unwrap();
    assert_eq!(
        (info.cn.as_str(), info.warning),
        ("TLS Only", Some("not-for-signing"))
    );
    let leaf = load(&mut host, &fixture("p5/leaf.p12"), PASSWORD).unwrap();
    assert_eq!(leaf.warning, None);

    // In a window: "Sign anyway?" — Cancel keeps nothing, "Sign anyway" holds the key.
    for (reply, held) in [(1, false), (0, true)] {
        let (platform, asked) = Asking::new(tls.pem.clone(), reply);
        let mut gui = Host::new(
            HostConfig {
                user_data: work.path(&format!("gui-{reply}")),
                smoke: false,
                echo_stdout: false,
            },
            Box::new(platform),
        );
        assert!(gui.pick_certificate(None).is_ok());
        let got = gui.certificate_info("");
        assert_eq!(got.is_ok(), held, "{got:?}");
        if !held {
            assert_eq!(got.unwrap_err().reason, "cancelled");
        }
        assert_eq!(gui.holding_certificate(), held);
        assert_eq!(*asked.lock().unwrap(), [Question::NotForSigning]);
    }
}

/// The questions a window asked.
type Asked = std::sync::Arc<std::sync::Mutex<Vec<Question>>>;

/// A window's platform: the certificate sheet answers `pick`, every question `reply`.
struct Asking {
    pick: PathBuf,
    reply: usize,
    asked: Asked,
}

impl Asking {
    fn new(pick: PathBuf, reply: usize) -> (Asking, Asked) {
        let asked = Asked::default();
        (
            Asking {
                pick,
                reply,
                asked: std::sync::Arc::clone(&asked),
            },
            asked,
        )
    }
}

impl Platform for Asking {
    fn pick_file(&mut self, _pick: Pick, _filters: &[Filter]) -> Option<PathBuf> {
        Some(self.pick.clone())
    }
    fn pick_images(&mut self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn pick_folder(&mut self) -> Option<PathBuf> {
        None
    }
    fn pick_save(&mut self, _: SavePurpose, _: &Path, _: &[Filter]) -> Option<PathBuf> {
        None
    }
    fn ask(&mut self, question: &MessageBox) -> usize {
        self.asked.lock().unwrap().push(question.question);
        self.reply
    }
    fn open_external(&mut self, _uri: &str) {}
    fn print(&mut self, _pdf: &[u8], _job_name: &str) -> PrintOutcome {
        PrintOutcome::NoPrinter
    }
    fn add_recent_document(&mut self, _path: &Path) {}
    fn clear_recent_documents(&mut self) {}
    fn set_represented_file(&mut self, _path: Option<&Path>, _title: &str) {}
    fn set_document_state(&mut self, _edited: bool, _save: bool, _save_as: bool) {}
}

// was tests/sign-differential.test.mjs: 'a PKCS#12 file asking for too many KDF iterations is
// refused before the work (too-costly)'
#[test]
fn a_too_costly_certificate_file_is_refused_before_the_work() {
    if !keys() {
        return;
    }
    if openssl().is_none() {
        return;
    }
    let work = Work::new("costly");
    let issued = issue(&work, "Costly Signer", EC_P256);
    let out = work.path("costly.p12");
    // No MAC; the key bag asks for 2,000,001 PBKDF2 iterations, one over the per-derivation cap.
    ossl(&[
        "pkcs12",
        "-export",
        "-inkey",
        s(&issued.key),
        "-in",
        s(&issued.cert),
        "-nomac",
        "-keypbe",
        "AES-128-CBC",
        "-certpbe",
        "NONE",
        "-iter",
        "2000001",
        "-passout",
        "pass:pw",
        "-out",
        s(&out),
    ]);
    let mut host = smoke_host(&work);
    let started = Instant::now();
    assert_eq!(
        load(&mut host, &out, "pw").unwrap_err().reason,
        "too-costly"
    );
    assert!(
        started.elapsed().as_millis() < 2000,
        "refused without doing the work"
    );
}

// was tests/sign-differential.test.mjs: 'OpenSSL-made PKCS#12 and PEM variants read under Rust like
// node:crypto reads their keys'. The host only holds an identity whose key matches one of the
// file's certificates (tandem-sign identity.rs `Found::finish`).
#[test]
fn openssl_made_certificate_files_read() {
    if !keys() {
        return;
    }
    if openssl().is_none() {
        return;
    }
    let work = Work::new("containers");
    let mut host = smoke_host(&work);
    let rsa = issue(&work, "Container Signer", RSA_2048);
    let ec = issue(&work, "Container EC", EC_P384);
    let ca = ca_pem();
    let p12s: [(&str, Vec<&str>, &Issued); 5] = [
        (
            "p12 AES-128 + SHA-256 MAC",
            vec![
                "-keypbe",
                "AES-128-CBC",
                "-certpbe",
                "AES-128-CBC",
                "-macalg",
                "sha256",
            ],
            &rsa,
        ),
        ("p12 3DES/RC2 (legacy)", vec!["-legacy"], &rsa),
        (
            "p12 3DES both + SHA-1 MAC",
            vec![
                "-keypbe",
                "PBE-SHA1-3DES",
                "-certpbe",
                "PBE-SHA1-3DES",
                "-macalg",
                "sha1",
            ],
            &rsa,
        ),
        (
            "p12 SHA-512 MAC, CA chain",
            vec!["-macalg", "sha512", "-certfile", s(&ca)],
            &rsa,
        ),
        ("p12 P-384", vec![], &ec),
    ];
    for (i, (label, args, issued)) in p12s.iter().enumerate() {
        let out = work.path(&format!("container-{i}.p12"));
        let mut cmd = vec![
            "pkcs12",
            "-export",
            "-inkey",
            s(&issued.key),
            "-in",
            s(&issued.cert),
        ];
        cmd.extend(args);
        cmd.extend(["-passout", "pass:pässwörd", "-out", s(&out)]);
        ossl(&cmd);
        let info = load(&mut host, &out, "pässwörd").unwrap_or_else(|e| panic!("{label}: {e:?}"));
        let is_rsa = issued.cert == rsa.cert;
        let want_cn = if is_rsa {
            "Container Signer"
        } else {
            "Container EC"
        };
        assert_eq!(info.cn, want_cn, "{label}");
        assert_eq!(
            info.certs,
            if label.contains("CA chain") { 2 } else { 1 },
            "{label}"
        );
        let id = Identity::from_p12(&fs::read(&out).unwrap(), "pässwörd").unwrap();
        if is_rsa {
            assert_eq!((id.key.kind(), id.key.bits()), ("rsa", 2048), "{label}");
        } else {
            assert_eq!(
                (id.key.kind(), id.key.curve()),
                ("ec", Some("P-384")),
                "{label}"
            );
        }
        assert_eq!(
            load(&mut host, &out, "wrong").unwrap_err().reason,
            "bad-password",
            "{label}"
        );
    }
    let pems: [(&str, Vec<&str>, &Issued); 7] = [
        (
            "PKCS#8 PBES2 AES-128 HMAC-SHA1",
            vec![
                "pkcs8",
                "-topk8",
                "-in",
                s(&rsa.key),
                "-v2",
                "aes-128-cbc",
                "-v2prf",
                "hmacWithSHA1",
                "-passout",
                "pass:pw",
            ],
            &rsa,
        ),
        (
            "PKCS#8 PBE-SHA1-3DES",
            vec![
                "pkcs8",
                "-topk8",
                "-in",
                s(&rsa.key),
                "-v1",
                "PBE-SHA1-3DES",
                "-passout",
                "pass:pw",
            ],
            &rsa,
        ),
        (
            "traditional RSA AES-256",
            vec![
                "rsa",
                "-in",
                s(&rsa.key),
                "-aes256",
                "-traditional",
                "-passout",
                "pass:pw",
            ],
            &rsa,
        ),
        (
            "traditional RSA 3DES",
            vec![
                "rsa",
                "-in",
                s(&rsa.key),
                "-des3",
                "-traditional",
                "-passout",
                "pass:pw",
            ],
            &rsa,
        ),
        (
            "traditional EC AES-128",
            vec!["ec", "-in", s(&ec.key), "-aes128", "-passout", "pass:pw"],
            &ec,
        ),
        (
            "plain PKCS#1",
            vec!["rsa", "-in", s(&rsa.key), "-traditional"],
            &rsa,
        ),
        ("plain SEC1", vec!["ec", "-in", s(&ec.key)], &ec),
    ];
    for (i, (label, args, issued)) in pems.iter().enumerate() {
        let key = work.path(&format!("variant-{i}.key"));
        let mut cmd = args.clone();
        cmd.extend(["-out", s(&key)]);
        ossl(&cmd);
        let pem = work.path(&format!("variant-{i}.pem"));
        fs::write(
            &pem,
            [fs::read(&key).unwrap(), fs::read(&issued.cert).unwrap()].concat(),
        )
        .unwrap();
        let plain = label.starts_with("plain");
        let info = load(&mut host, &pem, if plain { "" } else { "pw" })
            .unwrap_or_else(|e| panic!("{label}: {e:?}"));
        let cert = Cert::parse(&certs_from_pem(&fs::read(&issued.cert).unwrap())[0]).unwrap();
        assert_eq!(info.cn, cert.subject_cn(), "{label}");
        if !plain {
            assert_eq!(
                load(&mut host, &pem, "nope").unwrap_err().reason,
                "bad-password",
                "{label}"
            );
        }
    }
}

// --------------------------------------------------------- Rust-signed under the oracles

// was tests/sign-differential.test.mjs: 'Rust-signed (RSA PKCS#12 | P-256 PKCS#12 | P-384 PEM) is
// PAdES B-B and verifies under openssl, node:crypto and pyHanko; tampering fails in all';
// tests/p5-sign-crypto.test.mjs: 'P-256 p12 signs with ECDSA, and openssl verifies it' and
// 'cmsDetached verifies here and with openssl'.
#[test]
fn rust_signed_copies_are_pades_b_b_and_verify_under_openssl_and_pyhanko() {
    if !keys() {
        return;
    }
    let work = Work::new("pades");
    let mut cases = vec![
        (
            "RSA PKCS#12",
            fixture("p5/leaf.p12"),
            PASSWORD,
            fixture("p0b/.ca/leaf.pem"),
            true,
        ),
        // ec.pem has no keyUsage, which pyHanko does not trust; its integrity is still checked.
        (
            "P-256 PKCS#12",
            fixture("p5/leaf-ec.p12"),
            PASSWORD,
            fixture("p0b/.ca/ec.pem"),
            false,
        ),
    ];
    let have_openssl = openssl().is_some();
    if have_openssl {
        let p384 = issue(&work, "P384 Signer", EC_P384);
        cases.push(("P-384 PEM", p384.pem, "", p384.cert, true));
    }
    let mut host = trusting_host(&work);
    for (label, cert, password, leaf_pem, py_trusted) in cases {
        let tag: String = label.chars().filter(char::is_ascii_alphanumeric).collect();
        let p = prepared_edit_tj("Differential");
        let pdf = host_sign(&mut host, &work, &cert, password, &p);
        let listed = sign_list(&pdf);
        let entry = &listed.entries[0];
        assert_eq!(entry["subFilter"], "ETSI.CAdES.detached", "{label}");
        let v = &verdicts(&mut host, &listed.pdf, &listed.entries)[0];
        assert_eq!(
            (&v["status"], &v["coverage"], &v["trust"]),
            (&json!("valid"), &json!("entire-file"), &json!("trusted")),
            "{label}: {v}"
        );
        assert!(
            v.get("reason").is_none(),
            "{label}: the claimed time is /M: {v}"
        );
        // The verdict's signing time is the signed /M (D:YYYYMMDDHHmmSSZ).
        let m = entry["m"].as_str().unwrap();
        let iso = format!(
            "{}-{}-{}T{}:{}:{}.000Z",
            &m[2..6],
            &m[6..8],
            &m[8..10],
            &m[10..12],
            &m[12..14],
            &m[14..16]
        );
        assert_eq!(v["signingTime"], iso, "{label}");

        // pyHanko trusts the CA only for the leaves it would trust (as the JS ran it).
        let py_ca = if py_trusted {
            ca_pem()
        } else {
            work.path("no-such-ca.pem")
        };
        let cms = cms_of(&contents_of(entry));
        let content = covered(&pdf, &entry["byteRange"]);
        if have_openssl {
            assert!(
                openssl_verifies(&work, &format!("ok-{tag}"), &cms, &content),
                "{label}"
            );
        }
        // PAdES B-B: ESS signing-certificate-v2 names the leaf by its SHA-256; no signing-time.
        let attrs = signed_attributes(&cms);
        let attr = |id: &str| {
            attrs
                .iter()
                .find(|(oid, _, _)| oid == id)
                .map(|(_, node, value)| (*node, *value))
        };
        let (_, ess) = attr("1.2.840.113549.1.9.16.2.47").expect("signing-certificate-v2");
        let cert_id = kids(&cms, kids(&cms, kids(&cms, ess)[0])[0]);
        assert_eq!(
            cert_id[0].tag, 0x04,
            "default hash (SHA-256): no AlgorithmIdentifier"
        );
        let leaf_der = certs_from_pem(&fs::read(&leaf_pem).unwrap()).remove(0);
        assert_eq!(
            &cms[cert_id[0].value_start..cert_id[0].end],
            Sha256::digest(&leaf_der).as_slice(),
            "{label}"
        );
        assert!(
            attr("1.2.840.113549.1.9.5").is_none(),
            "PAdES: no signing-time"
        );
        let (_, content_type) = attr("1.2.840.113549.1.9.3").unwrap();
        assert_eq!(
            oid_of(&cms, content_type),
            "1.2.840.113549.1.7.1",
            "content-type id-data"
        );
        if let Some(run) = pyhanko(
            &work,
            &format!("ok-{tag}.pdf"),
            &pdf,
            &["--form", "--ca", s(&py_ca)],
        ) {
            let out = String::from_utf8_lossy(&run.stdout);
            let want = format!(
                "INTACT ENTIRE_FILE {} docmdp-ok PAdES-B-B",
                if py_trusted { "trusted" } else { "untrusted" }
            );
            assert!(
                run.status.success() && out.contains(&want),
                "{label}: {out}"
            );
        }

        // A byte of the binary header comment: every reader still parses it, but it changed.
        let mut tampered = pdf.clone();
        assert_eq!(tampered[9], b'%');
        tampered[11] ^= 0x01;
        let bad = &verdicts(&mut host, &tampered, &listed.entries)[0];
        assert_eq!(
            (&bad["status"], &bad["reason"]),
            (&json!("invalid"), &json!("digest")),
            "{label}"
        );
        if have_openssl {
            let changed = covered(&tampered, &entry["byteRange"]);
            assert!(!openssl_verifies(
                &work,
                &format!("bad-{tag}"),
                &cms,
                &changed
            ));
        }
        if let Some(run) = pyhanko(
            &work,
            &format!("bad-{tag}.pdf"),
            &tampered,
            &["--form", "--ca", s(&py_ca)],
        ) {
            assert_eq!(run.status.code(), Some(1), "{label}");
            assert!(String::from_utf8_lossy(&run.stdout).contains("INVALID"));
        }
    }
}

// was tests/sign-differential.test.mjs: 'the module and limits.ts agree on the reserved CMS size'
// (now: the engine's hole and tandem-sign's limit)
#[test]
fn the_engine_reserves_the_cms_size_tandem_sign_signs_into() {
    let p = prepared_edit_tj("Size");
    assert_eq!(MAX_SIG_CONTENTS_BYTES, 8192);
    assert_eq!(p.offsets.contents_len, MAX_SIG_CONTENTS_BYTES * 2);
}

// was tests/sign-differential.test.mjs: 'OpenSSL-made <RSA SHA-256 | RSA SHA-384 | RSA SHA-512 |
// RSA SHA-1 | RSA-PSS SHA-256 | RSA-PSS SHA-384 max salt | ECDSA P-256 SHA-256 | ECDSA P-384
// SHA-384 | ECDSA P-521 SHA-512 | ECDSA P-521 SHA-256> signature verifies under Rust; a flipped
// byte fails in both'
#[test]
fn openssl_made_signatures_verify_and_tampering_fails_in_both() {
    if !keys() {
        return;
    }
    if openssl().is_none() {
        return;
    }
    let work = Work::new("ossl");
    let rsa = |bits: &'static str| vec!["-algorithm", "RSA", "-pkeyopt", bits];
    let ec = |curve: &'static str| vec!["-algorithm", "EC", "-pkeyopt", curve];
    let pss = |salt: &'static str| vec!["-keyopt", "rsa_padding_mode:pss", "-keyopt", salt];
    let variants: Vec<(&str, Vec<&str>, &str, Vec<&str>)> = vec![
        ("RSA SHA-256", rsa("rsa_keygen_bits:2048"), "sha256", vec![]),
        ("RSA SHA-384", rsa("rsa_keygen_bits:2048"), "sha384", vec![]),
        ("RSA SHA-512", rsa("rsa_keygen_bits:3072"), "sha512", vec![]),
        ("RSA SHA-1", rsa("rsa_keygen_bits:2048"), "sha1", vec![]),
        (
            "RSA-PSS SHA-256",
            rsa("rsa_keygen_bits:2048"),
            "sha256",
            pss("rsa_pss_saltlen:32"),
        ),
        (
            "RSA-PSS SHA-384 max salt",
            rsa("rsa_keygen_bits:2048"),
            "sha384",
            pss("rsa_pss_saltlen:max"),
        ),
        (
            "ECDSA P-256 SHA-256",
            ec("ec_paramgen_curve:P-256"),
            "sha256",
            vec![],
        ),
        (
            "ECDSA P-384 SHA-384",
            ec("ec_paramgen_curve:P-384"),
            "sha384",
            vec![],
        ),
        (
            "ECDSA P-521 SHA-512",
            ec("ec_paramgen_curve:P-521"),
            "sha512",
            vec![],
        ),
        (
            "ECDSA P-521 SHA-256",
            ec("ec_paramgen_curve:P-521"),
            "sha256",
            vec![],
        ),
    ];
    // OpenSSL makes the keys, certificates and signatures in parallel; Rust checks them here.
    let made: Vec<(String, &str, Vec<u8>, Vec<u8>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = variants
            .iter()
            .map(|(label, keygen, md, keyopt)| {
                let work = &work;
                scope.spawn(move || {
                    let name = format!(
                        "OSSL {}",
                        label.replace(|c: char| !c.is_alphanumeric(), " ")
                    );
                    let issued = issue(work, &name, keygen);
                    let content =
                        format!("%PDF-1.7\n1 0 obj\n<<>>\nendobj\n% {label}\n%%EOF\n").into_bytes();
                    let tag: String = label.chars().filter(char::is_ascii_alphanumeric).collect();
                    let data = work.path(&format!("{tag}.bin"));
                    let out = work.path(&format!("{tag}.der"));
                    fs::write(&data, &content).unwrap();
                    let mut cmd = vec![
                        "cms",
                        "-sign",
                        "-binary",
                        "-in",
                        s(&data),
                        "-signer",
                        s(&issued.cert),
                        "-inkey",
                        s(&issued.key),
                        "-md",
                        md,
                    ];
                    cmd.extend(keyopt);
                    cmd.extend(["-outform", "DER", "-out", s(&out)]);
                    ossl(&cmd);
                    (name, *md, fs::read(&out).unwrap(), content)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut host = trusting_host(&work);
    for (name, md, cms, content) in made {
        let (src, entry) = wrap(&cms, &content);
        let v = &verdicts(&mut host, &src, std::slice::from_ref(&entry))[0];
        assert_eq!(
            (&v["status"], &v["trust"], &v["digest"], &v["signerCn"]),
            (&json!("valid"), &json!("trusted"), &json!(md), &json!(name)),
            "{name}: {v}"
        );
        let want_reason = (md == "sha1").then(|| json!("weak-digest"));
        assert_eq!(v.get("reason").cloned(), want_reason, "{name}: {v}");

        let mut flipped = src.clone();
        let at = flipped.len() - 8;
        flipped[at] ^= 0x01;
        let bad = &verdicts(&mut host, &flipped, std::slice::from_ref(&entry))[0];
        assert_eq!(
            (&bad["status"], &bad["reason"]),
            (&json!("invalid"), &json!("digest")),
            "{name}"
        );
        let tag: String = name.chars().filter(char::is_ascii_alphanumeric).collect();
        assert!(!openssl_verifies(
            &work,
            &format!("{tag}-bad"),
            &cms,
            &covered(&flipped, &entry["byteRange"])
        ));
        assert!(openssl_verifies(
            &work,
            &format!("{tag}-ok"),
            &cms,
            &content
        ));

        // A corrupted signature value is invalid in Rust and in OpenSSL.
        let signature = *signer_info(&cms)
            .iter()
            .rfind(|part| part.tag == 0x04)
            .unwrap();
        let mut broken = cms.clone();
        broken[signature.end - 2] ^= 0x01;
        let (src, entry) = wrap(&broken, &content);
        let v = &verdicts(&mut host, &src, &[entry])[0];
        assert_eq!(v["status"], "invalid", "{name}: {v}");
        assert!(!openssl_verifies(
            &work,
            &format!("{tag}-sig"),
            &broken,
            &content
        ));
    }
}

// ----------------------------------------------------------------------- verification

// was tests/p5-verify-crypto.test.mjs: 'p0b fixtures match the contract table and pyHanko
// coverage' (the crate's table alone: tandem-sign
// tests/fixtures.rs::p0b_fixtures_match_the_contract_table); here through sign-list and the host,
// beside pyHanko's line for each.
#[test]
fn the_signed_fixtures_verify_as_pyhanko_reads_them() {
    if !keys() {
        return;
    }
    let work = Work::new("p0b");
    let mut host = trusting_host(&work);
    for (name, status, coverage, trust, py) in [
        (
            "signed-intact",
            "valid",
            "entire-file",
            "trusted",
            "INTACT ENTIRE_FILE trusted",
        ),
        (
            "signed-tampered",
            "invalid",
            "entire-file",
            "none",
            "INVALID ENTIRE_FILE",
        ),
        (
            "signed-untrusted",
            "valid",
            "entire-file",
            "untrusted",
            "INTACT ENTIRE_FILE untrusted",
        ),
        (
            "certified-p2",
            "valid",
            "entire-file",
            "trusted",
            "INTACT ENTIRE_FILE trusted",
        ),
        (
            "certified-p2-filled",
            "valid",
            "partial",
            "trusted",
            "INTACT ENTIRE_REVISION trusted",
        ),
        (
            "certified-p2-annot",
            "valid",
            "partial",
            "trusted",
            "INTACT ENTIRE_REVISION trusted",
        ),
    ] {
        let rel = format!("p0b/{name}.pdf");
        let bytes = read_fixture(&rel);
        let v = &verify_file(&mut host, &bytes)[0];
        assert_eq!(
            (&v["status"], &v["coverage"], &v["trust"]),
            (&json!(status), &json!(coverage), &json!(trust)),
            "{name}: {v}"
        );
        if status == "invalid" {
            assert_eq!(v["reason"], "digest");
        }
        if trust == "trusted" {
            assert_eq!(v["issuerCn"], "P0B Test CA");
        }
        if name.contains("certified-p2") {
            same(&v["docmdp"], json!(2));
        }
        if let Some(run) = pyhanko(
            &work,
            &format!("{name}.pdf"),
            &bytes,
            &["--ca", s(&ca_pem())],
        ) {
            let out = String::from_utf8_lossy(&run.stdout);
            let line = out.lines().find(|l| l.starts_with("Sig1 ")).unwrap_or("");
            assert!(line.contains(py), "{name}: {out}");
        }
    }
}

// was tests/p5-verify.test.mjs: 'sign-list entries verify against intact, tampered, and a freshly
// signed document'; tests/p5-sign.test.mjs: 'P5 prepare → CMS patch → verify → reopen preserves
// read-only signed source'.
#[test]
fn a_freshly_signed_copy_verifies_trusted_and_reopens_read_only() {
    if !keys() {
        return;
    }
    let work = Work::new("fresh");
    let mut host = trusting_host(&work);
    for (name, status) in [("signed-intact", "valid"), ("signed-tampered", "invalid")] {
        let v = &verify_file(&mut host, &read_fixture(&format!("p0b/{name}.pdf")))[0];
        assert_eq!(v["status"], status, "{name}");
        if status == "valid" {
            assert_eq!(v["trust"], "trusted");
        }
    }

    let (mut engine, _) = opened("edit-tj.pdf", read_fixture("edit-tj.pdf"));
    let before = journal(&engine);
    let p = prepare(&mut engine, json!({"reason": "end-to-end"}));
    assert_eq!(
        journal(&engine),
        before,
        "signing never touches the open document"
    );
    let signed = host_sign(&mut host, &work, &fixture("p5/leaf.p12"), PASSWORD, &p);
    // The CMS covers everything but its hole, and verifies under an independent reader.
    let listed = sign_list(&signed);
    let entry = &listed.entries[0];
    let start = p.offsets.contents_at as usize - 1;
    let end = (p.offsets.contents_at + p.offsets.contents_len) as usize + 1;
    assert_eq!(
        covered(&signed, &entry["byteRange"]),
        [&signed[..start], &signed[end..]].concat()
    );
    let content = covered(&signed, &entry["byteRange"]);
    if openssl().is_some() {
        assert!(openssl_verifies(
            &work,
            "fresh",
            &cms_of(&contents_of(entry)),
            &content
        ));
    }
    let v = &verdicts(&mut host, &listed.pdf, &listed.entries)[0];
    assert_eq!(
        (&v["status"], &v["trust"], &v["signerCn"]),
        (&json!("valid"), &json!("trusted"), &json!("P0B Signer"))
    );
    let (mut reopened, info) = opened("signed.pdf", signed);
    assert_eq!(signatures_of(&info), 1);
    let before = journal(&reopened);
    assert_eq!(
        call(&mut reopened, json!({"op": "save"}))["reason"],
        "read-only"
    );
    assert_eq!(journal(&reopened), before);
}

// was tests/p5-verify-crypto.test.mjs: 'byte-range, digest, coverage, and subfilter failures' — the
// one case tandem-sign tests/fixtures.rs::byte_range_digest_coverage_and_subfilter_failures does
// not have: a nonzero byte in the hole's padding after the CMS.
#[test]
fn a_nonzero_byte_after_the_cms_is_a_byte_range_failure() {
    if !keys() {
        return;
    }
    let work = Work::new("pad");
    let mut host = trusting_host(&work);
    let src = read_fixture("p0b/signed-intact.pdf");
    let listed = sign_list(&src);
    let entry = &listed.entries[0];
    let mut dirty = contents_of(entry);
    *dirty.last_mut().unwrap() = 1;
    let r: Vec<usize> = entry["byteRange"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_u64().unwrap() as usize)
        .collect();
    let upper: String = dirty.iter().map(|b| format!("{b:02X}")).collect();
    assert_eq!(upper.len(), r[2] - r[1] - 2);
    let mut reseated = src.clone();
    reseated[r[1] + 1..r[2] - 1].copy_from_slice(upper.as_bytes());
    let mut moved = entry.clone();
    moved["contents"] = json!(hex(&dirty));
    let v = &verdicts(&mut host, &reseated, &[moved])[0];
    assert_eq!(v["reason"], "byte-range", "{v}");
}

// was tests/p5-verify-crypto.test.mjs: 'a not-yet-valid leaf is expired, and a SKI sid is unsupported'
#[test]
fn a_not_yet_valid_leaf_is_expired_and_a_key_id_signer_unsupported() {
    if !keys() {
        return;
    }
    if openssl().is_none() {
        return;
    }
    let work = Work::new("future");
    let issued = issue_with(
        &work,
        "Future",
        RSA_2048,
        SIGNING_EXT,
        &[
            "-not_before",
            "20300101000000Z",
            "-not_after",
            "20300102000000Z",
        ],
    );
    let identity = Identity::from_pem(
        &[fs::read(&issued.pem).unwrap(), fs::read(ca_pem()).unwrap()].concat(),
        "",
    )
    .unwrap();
    assert_eq!(identity.certs.len(), 2);
    let content = b"%PDF-1.4\n\n%%EOF\n";
    let cms = cms::sign_detached(&identity.key, &identity.certs, content, &mut drbg()).unwrap();
    let mut host = trusting_host(&work);
    let (src, entry) = wrap(&cms, content);
    let v = &verdicts(&mut host, &src, &[entry])[0];
    assert_eq!(
        (&v["status"], &v["trust"]),
        (&json!("valid"), &json!("expired")),
        "{v}"
    );

    // The signer named by subjectKeyIdentifier ([0]) instead of issuer and serial.
    let sid = signer_info(&cms)[1];
    let mut ski = cms.clone();
    ski[sid.start] = 0xa0;
    let (src, entry) = wrap(&ski, content);
    let v = &verdicts(&mut host, &src, &[entry])[0];
    assert_eq!(
        (&v["status"], &v["reason"]),
        (&json!("unsupported"), &json!("signer-id")),
        "{v}"
    );
}

// was tests/sign-differential.test.mjs: 'the trust store reads certificates like node:crypto does'
// (dedupe, file round trip and junk: host/trust.rs tests::round_trip_through_a_file, junk_is_skipped)
#[test]
fn the_trusted_list_reads_certificates_as_openssl_does() {
    if !keys() {
        return;
    }
    if openssl().is_none() {
        return;
    }
    let work = Work::new("trust");
    let files = [
        ca_pem(),
        fixture("p0b/.ca/leaf.pem"),
        fixture("p0b/.ca/ec.pem"),
        fixture("p5/false-ca/alice.pem"),
    ];
    let all = work.path("four.pem");
    fs::write(
        &all,
        files
            .iter()
            .flat_map(|f| fs::read(f).unwrap())
            .collect::<Vec<u8>>(),
    )
    .unwrap();
    let mut host = smoke_host(&work);
    assert_eq!(host.trust_add(Some(s(&all))), Ok((4, 4)));
    let listed = host.trust.certs();
    assert_eq!(listed.len(), 4);
    for (file, cert) in files.iter().zip(&listed) {
        let out = ossl(&[
            "x509",
            "-in",
            s(file),
            "-noout",
            "-fingerprint",
            "-sha256",
            "-subject",
            "-nameopt",
            "RFC2253",
            "-enddate",
            "-dateopt",
            "iso_8601",
            "-ext",
            "basicConstraints",
        ]);
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let value = |prefix: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(prefix))
                .unwrap_or_else(|| panic!("{prefix} in {text}"))
                .to_string()
        };
        assert_eq!(
            cert.fingerprint256,
            value("sha256 Fingerprint="),
            "{file:?}"
        );
        let subject = value("subject=");
        let cn = subject
            .split(',')
            .find_map(|part| part.strip_prefix("CN="))
            .unwrap();
        assert_eq!(cert.cn, cn, "{file:?}");
        // openssl: 2036-09-13 15:47:00Z; the list: 2036-09-13T15:47:00.000Z.
        let end = value("notAfter=").replace(' ', "T").replace('Z', ".000Z");
        assert_eq!(cert.not_after, end, "{file:?}");
        let is_ca = text.contains("CA:TRUE");
        assert_eq!(Cert::parse(&cert.raw).unwrap().is_ca, is_ca, "{file:?}");
    }
    let junk = work.path("junk.pem");
    fs::write(&junk, b"not a certificate").unwrap();
    assert_eq!(host.trust_add(Some(s(&junk))), Err("no-certificate"));
    assert_eq!(host.trust.certs().len(), 4);
}

// was tests/sign-differential.test.mjs: 'verdicts carry a plain-language sentence' (the crate's
// sentences: tandem-sign src/report.rs tests)
#[test]
fn verdicts_carry_a_plain_sentence() {
    if !keys() {
        return;
    }
    let work = Work::new("plain");
    let mut host = trusting_host(&work);
    let intact = read_fixture("p0b/signed-intact.pdf");
    let trusted = verify_file(&mut host, &intact).remove(0);
    let text = trusted["plain"].as_str().unwrap().to_string();
    assert!(text.starts_with("Signed by “P0B Signer”, dated "), "{text}");
    assert!(text.ends_with(
        ". The document has not changed since it was signed. The signer's certificate is trusted: it was issued through “P0B Test CA”, which is in your trusted list."
    ), "{text}");
    let tampered = verify_file(&mut host, &read_fixture("p0b/signed-tampered.pdf")).remove(0);
    assert!(
        tampered["plain"]
            .as_str()
            .unwrap()
            .contains("has changed since it was signed")
    );
    let p2 = verify_file(&mut host, &read_fixture("p0b/certified-p2-filled.pdf")).remove(0);
    let p2_text = p2["plain"].as_str().unwrap().to_string();
    assert!(
        p2_text.contains("Later changes were added after signing"),
        "{p2_text}"
    );
    assert!(
        p2_text.contains("allows only filling in forms and signing"),
        "{p2_text}"
    );
    assert!(host.trust_clear());
    let untrusted = verify_file(&mut host, &intact).remove(0);
    let u_text = untrusted["plain"].as_str().unwrap().to_string();
    assert!(u_text.starts_with(
        "The signer's certificate is not in your trusted list, so the name is not confirmed. Signed by “P0B Signer”, dated "
    ), "{u_text}");
    for t in [&text, &u_text, &p2_text] {
        for jargon in ["DocMDP", "P=", "bytes", "digest", "CMS"] {
            assert!(!t.contains(jargon), "{jargon} in {t}");
        }
    }
}

// was tests/sign-differential.test.mjs: 'an over-long or broken string in one entry never costs
// the other entries their verdicts'. (Its lone-surrogate case has no Rust form: entries come
// from the engine as Rust strings, which are always valid UTF-8.)
#[test]
fn a_broken_entry_never_costs_the_others_their_verdicts() {
    if !keys() {
        return;
    }
    let work = Work::new("broken");
    let mut host = trusting_host(&work);
    let listed = sign_list(&read_fixture("p0b/signed-intact.pdf"));
    let good = listed.entries[0].clone();
    let with = |field: &str, key: &str, value: Value| {
        let mut e = good.clone();
        e["field"] = json!(field);
        e[key] = value;
        e
    };
    let entries = vec![
        good.clone(),
        with("Long", "reason", json!("a".repeat(1001))),
        with(
            "Emoji",
            "reason",
            json!(format!("{}\u{1F600}", "a".repeat(1000))),
        ),
        with("Big", "page", json!(1e20)),
        Value::Null,
        good.clone(),
    ];
    let v = verdicts(&mut host, &listed.pdf, &entries);
    assert_eq!(v.len(), entries.len());
    assert_eq!(
        (&v[0]["status"], &v[5]["status"]),
        (&json!("valid"), &json!("valid"))
    );
    for (i, field) in [(1, "Long"), (2, "Emoji")] {
        assert_eq!(
            (&v[i]["field"], &v[i]["status"], &v[i]["reason"]),
            (&json!(field), &json!("unsupported"), &json!("malformed"))
        );
    }
    same(
        &json!([v[3]["field"], v[3]["page"], v[3]["status"]]),
        json!(["Big", 0, "valid"]),
    );
    assert_eq!(
        (&v[4]["status"], &v[4]["reason"]),
        (&json!("unsupported"), &json!("malformed"))
    );
    for verdict in &v {
        assert!(verdict["plain"].is_string(), "{verdict}");
    }
}

// was tests/sign-differential.test.mjs: 'a later revision that rewrites /M cannot change the signing
// date (review p2b M1)' (the crate's check: tandem-sign
// tests/generated.rs::a_later_update_cannot_change_the_signing_date); here with the host's words.
#[test]
fn a_later_revision_cannot_change_the_signing_date() {
    if !keys() {
        return;
    }
    let work = Work::new("m");
    let mut host = trusting_host(&work);
    let p = prepared_edit_tj("Dated");
    let pdf = host_sign(&mut host, &work, &fixture("p5/leaf.p12"), PASSWORD, &p);
    let listed = sign_list(&pdf);
    let signed = verdicts(&mut host, &listed.pdf, &listed.entries).remove(0);
    assert_eq!(signed["status"], "valid");
    let later = [pdf.as_slice(), b"\n% later revision\n%%EOF\n"].concat();
    for m in ["D:20000101000000Z", "D:20990101000000Z"] {
        let mut entry = listed.entries[0].clone();
        entry["m"] = json!(m);
        let v = verdicts(&mut host, &later, &[entry]).remove(0);
        assert_eq!(
            (&v["status"], &v["trust"], &v["coverage"]),
            (&json!("valid"), &json!("trusted"), &json!("partial")),
            "{m}"
        );
        assert_eq!(v["signingTime"], signed["signingTime"], "{m}");
        assert!(
            v["reason"].as_str().unwrap().contains("time-changed"),
            "{v}"
        );
        assert!(
            v["plain"].as_str().unwrap().contains(
                "A later change edited the signature's date; the date shown is the one that was signed."
            ),
            "{v}"
        );
        assert!(
            v["reasonWords"]
                .as_str()
                .unwrap()
                .contains("date edited after signing")
        );
    }
}

// was tests/sign-differential.test.mjs: 'verdicts carry cleaned names and reasons in words for the
// Verify table'
#[test]
fn verdicts_carry_cleaned_names_and_reasons_in_words() {
    if !keys() {
        return;
    }
    if openssl().is_none() {
        return;
    }
    let work = Work::new("names");
    let name = "Eve\u{202E}gnp.exe\u{202C} Doe";
    let issued = issue(&work, name, EC_P256);
    let identity = Identity::from_pem(&fs::read(&issued.pem).unwrap(), "").unwrap();
    let content = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\n%%EOF\n";
    let cms = cms::sign_detached(&identity.key, &identity.certs, content, &mut drbg()).unwrap();
    let (src, entry) = wrap(&cms, content);
    let mut host = trusting_host(&work);
    let v = verdicts(&mut host, &src, &[entry]).remove(0);
    assert_eq!(v["status"], "valid");
    assert_eq!(
        v["signerCn"], name,
        "STATE keeps the certificate name as it is"
    );
    assert_eq!(v["signerName"], "Evegnp.exe Doe");
    assert_eq!(v["issuerName"], "P0B Test CA");
    assert_eq!(v["reasonWords"], "no date given");
    assert!(v["plain"].as_str().unwrap().contains("“Evegnp.exe Doe”"));
}

// ------------------------------------------------------------- sign-prepare and fields

const FIXTURE: &str = "p5/empty-sigfield.pdf";
const FIXTURE_SHA: &str = "07e46851060fc7aa179c0f89a7d08ebdac26f682f4322edf20c57dad0523a206";

/// /AcroForm /Fields entry `name`, read back from `bytes`: its /V /Name and its /AP /N text.
fn signed_field(bytes: &[u8], name: &str) -> (String, String) {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    let catalog = doc.resolve_id(doc.root()).as_dict().unwrap();
    let acro = doc
        .resolve_dict(catalog.get(b"AcroForm".as_slice()).unwrap())
        .unwrap();
    let list = doc
        .resolve(acro.get(b"Fields".as_slice()).unwrap())
        .as_array()
        .unwrap();
    for field in list {
        let field = doc.resolve_dict(field).unwrap();
        match field.get(b"T".as_slice()) {
            Some(Object::String(t)) if decode_text_string(t) == name => {}
            _ => continue,
        }
        let sig = doc
            .resolve_dict(field.get(b"V".as_slice()).unwrap())
            .unwrap();
        let Some(Object::String(signer)) = sig.get(b"Name".as_slice()) else {
            panic!("{name} has no /Name");
        };
        let ap = field
            .get(b"AP".as_slice())
            .and_then(|ap| doc.resolve_dict(ap))
            .and_then(|ap| ap.get(b"N".as_slice()))
            .and_then(Object::as_reference)
            .map(|n| latin1(&doc.decoded_stream(n, StreamLimits::default()).unwrap()))
            .unwrap_or_default();
        return (decode_text_string(signer), ap);
    }
    panic!("no signed field {name}")
}

// was tests/p5-sign.test.mjs: 'P5 appearance shows every line unescaped, /Name keeps Unicode,
// certified accepts only invisible (review fix)' (the /Name part; the appearance text is engine
// tests/app_sign.rs::a_new_widget_is_visible_or_invisible_and_the_session_stays_as_it_was and the
// WinAnsi bytes engine src/app/ops/sign/prepare.rs::win_ansi_escapes_and_replaces);
// 'P5 U3: the new-widget path is unchanged beside empty fields (free name Signature4, field
// null)'; 'P5 certified P=1 source refuses without dirtying'.
#[test]
fn sign_prepare_keeps_unicode_names_and_names_new_widgets_freely() {
    let (mut engine, _) = opened("edit-tj.pdf", read_fixture("edit-tj.pdf"));
    let name = "Zoë (QA) — €";
    let p = prepare(
        &mut engine,
        json!({"name": name, "reason": "approved (final)", "rect": [72, 620, 340, 680]}),
    );
    let (signer, ap) = signed_field(&p.bytes, "Signature1");
    assert_eq!(signer, name, "/Name keeps Unicode");
    // The appearance is WinAnsi (— is 0x97, € is 0x80), one escape per parenthesis, lines
    // stepped by 0 -11 Td.
    let lines = "8 47 Td (Digitally signed by Zo\u{eb} \\(QA\\) \u{97} \u{80}) Tj 0 -11 Td (";
    assert!(ap.contains(lines), "{ap}");
    assert!(
        ap.contains(") Tj 0 -11 Td (Reason: approved \\(final\\)) Tj"),
        "{ap}"
    );

    let (mut engine, _) = opened("empty-sigfield.pdf", read_fixture(FIXTURE));
    for field in [None, Some(Value::Null)] {
        let mut fields = json!({});
        if let Some(field) = field {
            fields["field"] = field;
        }
        let p = prepare(&mut engine, fields);
        assert!(p.reply.get("field").is_none(), "{}", p.reply);
        assert!(latin1(&p.bytes).contains("/FT/Sig/T(Signature4)"));
    }
    let mixed = refuses(
        &mut engine,
        prepare_request(json!({"field": "Signature1", "rect": [72, 620, 340, 680]})),
        "invalid",
    );
    assert_eq!(
        mixed["error"],
        "sign-prepare: a field signature takes rect null and the field's page"
    );

    let (mut p1, _) = opened("certified-p1.pdf", read_fixture("p5/certified-p1.pdf"));
    let refusal = refuses(
        &mut p1,
        prepare_request(json!({"rect": [72, 620, 340, 680]})),
        "certified",
    );
    assert_eq!(refusal["dirty"], false);
}

// was tests/p5-sign-field.test.mjs: 'P5 U3 fixture: pinned bytes, three empty signature fields,
// writable'
#[test]
fn the_empty_sigfield_fixture_is_pinned_with_three_empty_fields() {
    let bytes = read_fixture(FIXTURE);
    assert_eq!(hex(&Sha256::digest(&bytes)), FIXTURE_SHA);
    let provenance = fs::read_to_string(fixture("p5/empty-sigfield.provenance.json")).unwrap();
    assert!(provenance.contains(FIXTURE_SHA));
    let (mut engine, info) = opened("empty-sigfield.pdf", bytes);
    assert_eq!(signatures_of(&info), 0);
    let shape = |items: Vec<Value>| -> Value {
        items
            .iter()
            .map(|i| json!([i["id"], i["name"], i["kind"], i["rect"], i["signed"]]))
            .collect()
    };
    same(
        &shape(fields(&mut engine, 0)),
        json!([
            [7, "Signature1", "signature", [72, 620, 340, 680], false],
            [8, "Signature2", "signature", [360, 620, 540, 680], false],
        ]),
    );
    same(
        &shape(fields(&mut engine, 1)),
        json!([[10, "Signature3", "signature", [500, 72, 560, 300], false]]),
    );
    save(&mut engine);
}

// was tests/p5-sign-field.test.mjs: 'P5 U3 sign-prepare {field}: adopts Signature1 in place, then
// signs', 'P5 U3 sign-prepare {field} on a signed source: refusals, then an incremental second
// field', 'P5 U3 sign-prepare {field}: a third signature on a twice-signed source reports revisions
// 3' and their legs of 'P5 U3 pyHanko accepts every field signature'.
#[test]
fn existing_fields_sign_in_place_one_revision_at_a_time() {
    if !keys() {
        return;
    }
    let work = Work::new("fields");
    let mut host = trusting_host(&work);
    let p12 = fixture("p5/leaf.p12");

    let (mut engine, _) = opened("empty-sigfield.pdf", read_fixture(FIXTURE));
    let before = journal(&engine);
    let p = prepare(&mut engine, json!({"field": "Signature1"}));
    same(
        &json!([p.reply["revisions"], p.reply["field"], p.reply["rect"]]),
        json!([1, "Signature1", [72, 620, 340, 680]]),
    );
    exact_once(&p);
    assert_eq!(
        journal(&engine),
        before,
        "signing never touches the open document"
    );
    let (v7, ap7) = widget(&p.bytes, 7);
    let (v8, ap8) = widget(&p.bytes, 8);
    let (v10, ap10) = widget(&p.bytes, 10);
    assert_eq!((v7, v8, v10), (true, false, false));
    assert!(ap7.unwrap().contains("Digitally signed by P0B Signer"));
    let ap8 = ap8.expect("Signature2 keeps its grey appearance");
    assert!(!ap8.contains("Digitally signed"));
    assert_eq!(ap10, None);

    let signed1 = host_sign(&mut host, &work, &p12, PASSWORD, &p);
    let (mut reopened, info) = opened("signed1.pdf", signed1.clone());
    assert_eq!(signatures_of(&info), 1);
    assert_eq!(by_name(&mut reopened, "Signature1", 0)["signed"], true);
    assert_eq!(by_name(&mut reopened, "Signature2", 0)["signed"], false);
    assert_eq!(
        call(&mut reopened, json!({"op": "save"}))["reason"],
        "read-only"
    );
    let listed = sign_list(&signed1);
    let entry = listed
        .entries
        .iter()
        .find(|e| e["field"] == "Signature1")
        .unwrap();
    assert_eq!(entry["empty"], false);
    pyhanko_accepts(
        &work,
        "signed1.pdf",
        &signed1,
        1,
        &["Signature1 INTACT ENTIRE_FILE trusted docmdp-ok"],
    );

    // On the signed source: refusals, then Signature2 as an incremental update.
    let (mut engine, _) = opened("signed1.pdf", signed1.clone());
    refuses(
        &mut engine,
        prepare_request(json!({"field": "Signature1"})),
        "already-signed",
    );
    let nope = refuses(
        &mut engine,
        prepare_request(json!({"field": "Nope"})),
        "no-such-field",
    );
    assert_eq!(
        nope["error"],
        "sign-prepare: page 0 has no signature field Nope"
    );
    refuses(
        &mut engine,
        prepare_request(json!({"field": "Signature2", "page": 1})),
        "no-such-field",
    );
    refuses(
        &mut engine,
        prepare_request(json!({"field": "Signature2", "rect": [1, 1, 2, 2]})),
        "invalid",
    );
    let p = prepare(&mut engine, json!({"field": "Signature2"}));
    same(
        &json!([p.reply["revisions"], p.reply["rect"]]),
        json!([2, [360, 620, 540, 680]]),
    );
    exact_once(&p);
    assert_eq!(&p.bytes[..signed1.len()], signed1.as_slice());
    let update = latin1(&p.bytes[signed1.len()..]);
    assert!(
        update.starts_with("8 0 obj") || update.contains("\n8 0 obj"),
        "widget 8 is rewritten in the update"
    );
    assert!(widget(&p.bytes, 8).0);
    let signed2 = host_sign(&mut host, &work, &p12, PASSWORD, &p);
    assert_eq!(signatures_of(&opened("signed2.pdf", signed2.clone()).1), 2);
    pyhanko_accepts(
        &work,
        "signed2.pdf",
        &signed2,
        2,
        &[
            "Signature1 INTACT ENTIRE_REVISION trusted docmdp-ok",
            "Signature2 INTACT ENTIRE_FILE trusted docmdp-ok",
        ],
    );

    let (mut engine, _) = opened("signed2.pdf", signed2);
    let p = prepare(&mut engine, json!({"field": "Signature3", "page": 1}));
    same(&p.reply["revisions"], json!(3));
    let signed3 = host_sign(&mut host, &work, &p12, PASSWORD, &p);
    assert_eq!(signatures_of(&opened("signed3.pdf", signed3.clone()).1), 3);
    for v in verify_file(&mut host, &signed3) {
        assert_eq!(
            (&v["status"], &v["trust"]),
            (&json!("valid"), &json!("trusted")),
            "{v}"
        );
    }
    pyhanko_accepts(
        &work,
        "signed3.pdf",
        &signed3,
        3,
        &["Signature3 INTACT ENTIRE_FILE trusted docmdp-ok"],
    );
}

// was tests/p5-drive-field.test.mjs: 'drive P5 U3: field-signed3.pdf reopens with 3 valid
// signatures' — the drive's own chain (tests/drive/sign-field.jsonl): a drawn, unsaved
// Signature4, then Signature1 on the signed copy, then Signature3 on the turned page; the last
// copy lists three signed fields and one empty, three versions, and every signature is valid and
// trusted.
#[test]
fn the_drive_s_three_field_signatures_reopen_valid_and_trusted() {
    if !keys() {
        return;
    }
    let work = Work::new("drive3");
    let mut host = trusting_host(&work);
    let p12 = fixture("p5/leaf.p12");
    let (mut engine, _) = opened("empty-sigfield.pdf", read_fixture(FIXTURE));
    let added = ok(
        &mut engine,
        json!({"op": "field-add", "page": 0, "kind": "signature", "rect": [72, 300, 292, 360]}),
    );
    assert_eq!(added.value["name"], "Signature4");
    let p = prepare(
        &mut engine,
        json!({"field": "Signature4", "reason": "drive field"}),
    );
    same(&p.reply["revisions"], json!(1));
    assert_eq!(
        journal(&engine)["dirty"],
        true,
        "the session keeps its journal"
    );
    let mut pdf = host_sign(&mut host, &work, &p12, PASSWORD, &p);
    for (field, page, revisions) in [("Signature1", 0, 2), ("Signature3", 1, 3)] {
        let (mut engine, _) = opened("field-signed.pdf", pdf);
        let p = prepare(
            &mut engine,
            json!({"field": field, "page": page, "reason": "drive field"}),
        );
        same(&p.reply["revisions"], json!(revisions));
        pdf = host_sign(&mut host, &work, &p12, PASSWORD, &p);
    }
    let listed = sign_list(&pdf);
    let mut signed: Vec<&str> = listed
        .entries
        .iter()
        .filter(|e| e["empty"] == false)
        .map(|e| e["field"].as_str().unwrap())
        .collect();
    signed.sort_unstable();
    assert_eq!(signed, ["Signature1", "Signature3", "Signature4"]);
    let empty: Vec<&Value> = listed
        .entries
        .iter()
        .filter(|e| e["empty"] == true)
        .map(|e| &e["field"])
        .collect();
    assert_eq!(empty, [&json!("Signature2")]);
    same(&listed.value["versions"], json!(3));
    for v in verdicts(&mut host, &listed.pdf, &listed.entries) {
        if v["field"] == "Signature2" {
            assert_eq!(v["status"], "empty");
        } else {
            assert_eq!(
                (&v["status"], &v["trust"]),
                (&json!("valid"), &json!("trusted")),
                "{v}"
            );
        }
    }
}

// was tests/p5-sign-field.test.mjs: 'P5 U3 sign-prepare {field} on the /Rotate 90 page: /Matrix and
// pixels inside the field' (the /Matrix and appearance; MuPDF's pixel count is not ported — the
// engine draws widget appearances in its view tests) and its pyHanko leg.
#[test]
fn a_field_on_a_turned_page_signs_with_a_turned_appearance() {
    if !keys() {
        return;
    }
    let work = Work::new("turned");
    let mut host = trusting_host(&work);
    let (mut engine, _) = opened("empty-sigfield.pdf", read_fixture(FIXTURE));
    let p = prepare(&mut engine, json!({"field": "Signature3", "page": 1}));
    same(&p.reply["rect"], json!([500, 72, 560, 300]));
    assert!(widget(&p.bytes, 10).1.unwrap().contains("Digitally signed"));
    assert!(latin1(&p.bytes).contains("/Matrix[0 1 -1 0 0 0]"));
    let signed = host_sign(&mut host, &work, &fixture("p5/leaf.p12"), PASSWORD, &p);
    pyhanko_accepts(
        &work,
        "turned.pdf",
        &signed,
        1,
        &["Signature3 INTACT ENTIRE_FILE trusted docmdp-ok"],
    );
}

// was tests/p5-sign-field.test.mjs: 'P5 U3 a certified (P=2) document: an existing field signs, a
// new visible one stays refused' and its pyHanko leg.
#[test]
fn a_certified_document_signs_an_existing_field_but_no_new_visible_one() {
    if !keys() {
        return;
    }
    let work = Work::new("certified");
    let mut host = trusting_host(&work);
    let p12 = fixture("p5/leaf.p12");
    let (mut engine, _) = opened("empty-sigfield.pdf", read_fixture(FIXTURE));
    let p = prepare(&mut engine, json!({"certify": true}));
    let certified = host_sign(&mut host, &work, &p12, PASSWORD, &p);
    let (mut engine, _) = opened("certified.pdf", certified);
    refuses(
        &mut engine,
        prepare_request(json!({"rect": [72, 300, 340, 360]})),
        "certified",
    );
    let p = prepare(&mut engine, json!({"field": "Signature1"}));
    same(&p.reply["revisions"], json!(2));
    let filled = host_sign(&mut host, &work, &p12, PASSWORD, &p);
    pyhanko_accepts(
        &work,
        "certified-filled.pdf",
        &filled,
        2,
        &[
            "Signature4 INTACT ENTIRE_REVISION trusted docmdp-ok",
            "Signature1 INTACT ENTIRE_FILE trusted docmdp-ok",
            "DocMDP 2",
        ],
    );
}

// was tests/p5-sign-field.test.mjs: 'P5 U3 field-add kind signature: free name, no /AP, survives
// save, signs in place' and its pyHanko leg.
#[test]
fn signature_fields_are_authored_saved_and_signed_in_place() {
    let work = Work::new("authored");
    let (mut engine, _) = opened("hello.pdf", read_fixture("hello.pdf"));
    let added = ok(
        &mut engine,
        json!({"op": "field-add", "page": 0, "kind": "signature", "rect": [72, 72, 272, 132]}),
    );
    assert_eq!(added.value["name"], "Signature1");
    let item = by_name(&mut engine, "Signature1", 0);
    same(
        &json!([item["kind"], item["signed"], item["rect"]]),
        json!(["signature", false, [72, 72, 272, 132]]),
    );
    let saved = save(&mut engine);
    let id = item["id"].as_u64().unwrap() as u32;
    assert_eq!(
        widget(&saved, id).1,
        None,
        "no /AP on an authored signature field"
    );
    let (mut re, info) = opened("hello-sig.pdf", saved);
    assert_eq!(signatures_of(&info), 0);
    let again = by_name(&mut re, "Signature1", 0);
    assert_eq!(
        again["id"], item["id"],
        "the object number is kept across a save"
    );
    same(&again["rect"], json!([72, 72, 272, 132]));
    let p = prepare(&mut re, json!({"field": "Signature1"}));
    same(&p.reply["rect"], json!([72, 72, 272, 132]));
    if keys() {
        let mut host = trusting_host(&work);
        let signed = host_sign(&mut host, &work, &fixture("p5/leaf.p12"), PASSWORD, &p);
        pyhanko_accepts(
            &work,
            "hello-field.pdf",
            &signed,
            1,
            &["Signature1 INTACT ENTIRE_FILE trusted docmdp-ok"],
        );
    }

    let (mut fresh, _) = opened("hello.pdf", read_fixture("hello.pdf"));
    let base = json!({"op": "field-add", "page": 0, "kind": "signature", "name": "S", "rect": [72, 72, 272, 132]});
    let with = |key: &str, value: Value| {
        let mut r = base.clone();
        r[key] = value;
        r
    };
    let small = call(&mut fresh, with("rect", json!([72, 72, 100, 80])));
    assert_eq!(
        (&small["detail"], &small["error"]),
        (
            &json!("invalid"),
            &json!("field-add: a signature field is at least 40x20 pt")
        )
    );
    for (key, value, error) in [
        (
            "default",
            json!("x"),
            "field-add: default is not a signature property",
        ),
        (
            "options",
            json!(["a"]),
            "field-add: options are a dropdown property",
        ),
        ("kids", json!([]), "field-add: kids are a radio property"),
        (
            "maxLen",
            json!(3),
            "field-add: maxLen and multiline are text properties",
        ),
    ] {
        assert_eq!(call(&mut fresh, with(key, value))["error"], error, "{key}");
    }
    let mut no_rect = base.clone();
    no_rect.as_object_mut().unwrap().remove("rect");
    assert_eq!(
        call(&mut fresh, no_rect)["detail"],
        "invalid",
        "rect is required"
    );
    let unnamed_text = call(
        &mut fresh,
        json!({"op": "field-add", "page": 0, "kind": "text", "rect": [72, 72, 272, 132]}),
    );
    assert_eq!(
        unnamed_text["detail"], "invalid-name",
        "only a signature field may omit its name"
    );
    assert_eq!(journal(&fresh)["dirty"], false);
    ok(&mut fresh, base.clone());
    assert_eq!(
        call(&mut fresh, with("rect", json!([72, 200, 272, 260])))["detail"],
        "duplicate-name"
    );

    let (mut fx, _) = opened("empty-sigfield.pdf", read_fixture(FIXTURE));
    let fourth = ok(
        &mut fx,
        json!({"op": "field-add", "page": 0, "kind": "signature", "name": "", "rect": [72, 300, 292, 360]}),
    );
    assert_eq!(fourth.value["name"], "Signature4");
    let saved = save(&mut fx);
    let (mut re, _) = opened("fx.pdf", saved);
    same(
        &by_name(&mut re, "Signature4", 0)["rect"],
        json!([72, 300, 292, 360]),
    );
}

// was tests/p5-sign-field.test.mjs: 'P5 U3 field-props / field-delete / field-set on an EMPTY
// signature field'
#[test]
fn an_empty_signature_field_moves_and_deletes_but_keeps_its_name() {
    let (mut engine, _) = opened("empty-sigfield.pdf", read_fixture(FIXTURE));
    let reread = |engine: &mut EngineHost| {
        let saved = save(engine);
        (opened("re.pdf", saved.clone()).0, saved)
    };
    let sig = by_name(&mut engine, "Signature1", 0);
    let id = sig["id"].clone();
    ok(
        &mut engine,
        json!({"op": "field-props", "page": 0, "widgetId": id, "rect": [72, 500, 340, 580]}),
    );
    let (mut re, saved) = reread(&mut engine);
    same(
        &by_name(&mut re, "Signature1", 0)["rect"],
        json!([72, 500, 340, 580]),
    );
    assert_eq!(widget(&saved, 7).1, None);
    ok(&mut engine, json!({"op": "undo"}));
    same(
        &by_name(&mut engine, "Signature1", 0)["rect"],
        json!([72, 620, 340, 680]),
    );

    // Signature2 carries a grey /AP; a move drops it.
    let sig2 = by_name(&mut engine, "Signature2", 0)["id"].clone();
    ok(
        &mut engine,
        json!({"op": "field-props", "page": 0, "widgetId": sig2, "rect": [360, 500, 540, 560],
            "label": "Approver", "required": true}),
    );
    let (mut re, saved) = reread(&mut engine);
    let props = by_name(&mut re, "Signature2", 0);
    same(
        &json!([props["rect"], props["label"], props["required"]]),
        json!([[360, 500, 540, 560], "Approver", true]),
    );
    assert_eq!(widget(&saved, 8).1, None);
    ok(&mut engine, json!({"op": "undo"}));

    let clean = journal(&engine);
    for (request, detail, error) in [
        (
            json!({"op": "field-props", "page": 0, "widgetId": id, "name": "Other"}),
            "signature",
            "field-props: a signature field keeps its name",
        ),
        (
            json!({"op": "field-props", "page": 0, "widgetId": id, "rect": [72, 620, 100, 630]}),
            "invalid",
            "field-props: a signature field is at least 40x20 pt",
        ),
        (
            json!({"op": "field-props", "page": 0, "widgetId": id, "default": "x"}),
            "invalid",
            "field-props: default is not a signature property",
        ),
        (
            json!({"op": "field-props", "page": 0, "widgetId": id, "options": ["a"]}),
            "invalid",
            "field-props: options are a dropdown property",
        ),
        (
            json!({"op": "field-set", "page": 0, "widgetId": id, "value": "x"}),
            "signature",
            "field-set: click the signature field to sign it",
        ),
    ] {
        let reply = call(&mut engine, request.clone());
        assert_eq!(
            (&reply["ok"], &reply["detail"], &reply["error"]),
            (&json!(false), &json!(detail), &json!(error)),
            "{request}"
        );
        assert_eq!(journal(&engine), clean);
    }

    ok(
        &mut engine,
        json!({"op": "field-delete", "page": 0, "widgetId": id}),
    );
    let (mut re, _) = reread(&mut engine);
    let names: Vec<Value> = fields(&mut re, 0)
        .iter()
        .map(|i| i["name"].clone())
        .collect();
    assert_eq!(names, [json!("Signature2")]);
    ok(&mut engine, json!({"op": "undo"}));
    let back: Value = fields(&mut engine, 0)
        .iter()
        .map(|i| json!([i["id"], i["name"]]))
        .collect();
    same(&back, json!([[7, "Signature1"], [8, "Signature2"]]));
}

/// A one-page scratch PDF from object bodies numbered from 1 (1 catalog, 2 pages, 3 page).
fn build_pdf(objects: &[&str]) -> Vec<u8> {
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{body}\nendobj\n", i + 1));
    }
    let xref = out.len();
    out.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    ));
    for offset in offsets {
        out.push_str(&format!("{offset:010} 00000 n \n"));
    }
    out.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    ));
    out.into_bytes()
}

// was tests/p5-sign-field.test.mjs: 'P5 U3 a signed signature field cannot be changed (scratch
// document, not in /Fields)' and 'debt1 row 11: /V on the field /Parent reads signed and
// sign-prepare refuses already-signed' (the refusal alone is engine
// tests/app_sign.rs::field_signatures_are_refused_in_check_field_order, case "P.S").
#[test]
fn a_signed_field_cannot_be_changed_and_a_parent_v_reads_signed() {
    let (mut engine, info) = opened(
        "signed-scratch.pdf",
        build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [] >> >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R] >>",
            "<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signed) /Rect [72 600 272 660] /F 4 /P 3 0 R /V << /Type /Sig >> >>",
        ]),
    );
    assert_eq!(signatures_of(&info), 0, "the document stays writable");
    let item = by_name(&mut engine, "Signed", 0);
    assert_eq!(item["signed"], true);
    let before = journal(&engine);
    for request in [
        json!({"op": "field-props", "page": 0, "widgetId": item["id"], "rect": [72, 72, 272, 132]}),
        json!({"op": "field-delete", "page": 0, "widgetId": item["id"]}),
    ] {
        let reply = call(&mut engine, request);
        assert_eq!(reply["detail"], "signature");
        assert!(
            reply["error"]
                .as_str()
                .unwrap()
                .ends_with("a signed field cannot be changed"),
            "{reply}"
        );
    }
    assert_eq!(journal(&engine), before);
    assert_eq!(before["dirty"], false);

    // A /Sig parent holding /V over a kid widget, beside an unsigned field.
    let (mut engine, _) = opened(
        "parent-v.pdf",
        build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R 6 0 R] >> >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [5 0 R 6 0 R] >>",
            "<< /FT /Sig /T (P) /Kids [5 0 R] /V << /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached >> >>",
            "<< /Type /Annot /Subtype /Widget /Parent 4 0 R /T (Signature1) /Rect [72 112 340 172] /F 4 /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signature2) /Rect [360 112 540 172] /F 4 /P 3 0 R >>",
        ]),
    );
    assert_eq!(by_name(&mut engine, "P.Signature1", 0)["signed"], true);
    assert_eq!(by_name(&mut engine, "Signature2", 0)["signed"], false);
    refuses(
        &mut engine,
        prepare_request(json!({"field": "P.Signature1"})),
        "already-signed",
    );
}

// was tests/p5-sign-field.test.mjs: 'debt1 row 12: /Rotate inherited from /Pages is the page
// rotation and turns the signature /AP' (the /Matrix alone is engine
// tests/app_sign.rs::the_appearance_turns_with_the_page_and_shows_the_signer_s_local_time)
#[test]
fn an_inherited_rotate_is_the_page_rotation() {
    let (mut engine, reply) = opened(
        "inherit-rotate.pdf",
        build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /Rotate 90 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        ]),
    );
    let rotations = reply
        .value
        .get("rotations")
        .or_else(|| reply.value["info"].get("rotations"))
        .cloned();
    assert_eq!(rotations, Some(json!([90])), "{}", reply.value);
    let added = call(
        &mut engine,
        json!({"op": "text-add", "page": 0, "point": [100, 100], "text": "x", "font": "Helvetica",
            "size": 12, "color": [0, 0, 0]}),
    );
    assert_eq!(added["reason"], "unsupported-transform", "{added}");
    let p = prepare(&mut engine, json!({"rect": [100, 100, 300, 200]}));
    assert!(latin1(&p.bytes).contains("/Matrix[0 1 -1 0 0 0]"));
}

// The test's own DER reader (the oracle above) reads what tandem-sign writes.
#[test]
fn the_der_reader_reads_tandem_sign_s_oids() {
    let oid = tandem_sign::asn1::oid(&tandem_sign::oids::MESSAGE_DIGEST);
    let node = tlv_at(&oid, 0);
    assert_eq!((node.tag, node.end), (0x06, oid.len()));
    assert_eq!(oid_of(&oid, node), "1.2.840.113549.1.9.4");
}
