// SPDX-License-Identifier: AGPL-3.0-or-later
//! `DocumentInfo` on layouts the strict reader refuses: `versions` counts
//! the file's own xref sections as MuPDF's `countVersions` does (a hybrid
//! /XRefStm with its table, a linearized file's first-page section left
//! out, one section after a repair), not those of the rebuild opened.
use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppEngine, AppSession};

const OBJECTS: [&str; 5] = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
    "<< /Length 36 >>\nstream\nBT /F1 12 Tf 20 100 Td (Hello) Tj ET\nendstream",
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
];

fn row(offset: usize) -> String {
    format!("{offset:010} 00000 n \n")
}

fn classic() -> Vec<u8> {
    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in OBJECTS.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        out.extend_from_slice(row(offset).as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    out
}

fn last_startxref(bytes: &[u8]) -> usize {
    let at = bytes.windows(9).rposition(|w| w == b"startxref").unwrap() + 9;
    let digits: String = (bytes[at..].iter())
        .map(|&b| char::from(b))
        .skip_while(char::is_ascii_whitespace)
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().unwrap()
}

/// `base` with object 5 redefined in one more classic section.
fn updated(base: Vec<u8>, size: usize) -> Vec<u8> {
    let prev = last_startxref(&base);
    let mut out = base;
    let at = out.len();
    out.extend_from_slice(
        b"5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>\nendobj\n",
    );
    let xref = out.len();
    out.extend_from_slice(format!("xref\n5 1\n{}", row(at)).as_bytes());
    out.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R /Prev {prev} >>\nstartxref\n{xref}\n%%EOF\n")
            .as_bytes(),
    );
    out
}

/// MS Office's layout: object 5 in object stream 6, which the classic
/// table marks free and its /XRefStm (object 7) places.
fn hybrid() -> Vec<u8> {
    let mut out = b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = [0usize; 8];
    for (index, body) in OBJECTS.iter().enumerate().take(4) {
        offsets[index + 1] = out.len();
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
    }
    let data = format!("5 0 {}", OBJECTS[4]);
    offsets[6] = out.len();
    out.extend_from_slice(
        format!(
            "6 0 obj\n<< /Type /ObjStm /N 1 /First 4 /Length {} >>\nstream\n{data}\nendstream\nendobj\n",
            data.len()
        )
        .as_bytes(),
    );
    let mut rows = Vec::new();
    for number in 0..8u32 {
        let (kind, container) = if number == 5 { (2u8, 6u32) } else { (0, 0) };
        rows.push(kind);
        rows.extend_from_slice(&container.to_be_bytes());
        rows.extend_from_slice(&0u16.to_be_bytes());
    }
    offsets[7] = out.len();
    out.extend_from_slice(
        format!(
            "7 0 obj\n<< /Type /XRef /Size 8 /W [1 4 2] /Length {} >>\nstream\n",
            rows.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&rows);
    out.extend_from_slice(b"\nendstream\nendobj\n");
    let xref = out.len();
    out.extend_from_slice(b"xref\n0 8\n0000000000 65535 f \n");
    for (number, &offset) in offsets.iter().enumerate().skip(1) {
        match number {
            5 => out.extend_from_slice(b"0000000000 00000 f \n"),
            _ => out.extend_from_slice(row(offset).as_bytes()),
        }
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size 8 /Root 1 0 R /XRefStm {} >>\nstartxref\n{xref}\n%%EOF\n",
            offsets[7]
        )
        .as_bytes(),
    );
    out
}

/// Parameter dictionary 6, then a first-page section (3 and 6) whose /Prev
/// points forward to the main section at the end.
fn linearized() -> Vec<u8> {
    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let lin_at = out.len();
    let lin = |length: usize| {
        format!(
            "6 0 obj\n<< /Linearized 1 /L {length:010} /H [0 0] /O 3 /E 0 /N 1 /T 0 >>\nendobj\n"
        )
    };
    let first = |page: usize, main: usize| {
        format!(
            "xref\n3 1\n{}6 1\n{}trailer\n<< /Size 7 /Root 1 0 R /Prev {main:010} >>\nstartxref\n0\n%%EOF\n",
            row(page),
            row(lin_at)
        )
    };
    out.extend_from_slice(lin(0).as_bytes());
    let first_at = out.len();
    out.extend_from_slice(first(0, 0).as_bytes());
    let mut offsets = [0usize; 6];
    for number in [3, 1, 2, 4, 5] {
        offsets[number] = out.len();
        let body = OBJECTS[number - 1];
        out.extend_from_slice(format!("{number} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let main = out.len();
    out.extend_from_slice(
        format!(
            "xref\n0 3\n0000000000 65535 f \n{}{}4 2\n{}{}trailer\n<< /Size 6 >>\nstartxref\n{first_at}\n%%EOF\n",
            row(offsets[1]),
            row(offsets[2]),
            row(offsets[4]),
            row(offsets[5])
        )
        .as_bytes(),
    );
    let head = lin(out.len()) + &first(offsets[3], main);
    out[lin_at..lin_at + head.len()].copy_from_slice(head.as_bytes());
    out
}

/// Every in-use row of the last xref table moved by seven bytes.
fn shifted(bytes: Vec<u8>) -> Vec<u8> {
    let at = bytes.windows(6).rposition(|w| w == b"\nxref\n").unwrap();
    let text = String::from_utf8_lossy(&bytes[at..]).into_owned();
    let moved: Vec<String> = (text.split('\n'))
        .map(|line| match line.strip_suffix(" 00000 n ") {
            Some(offset) if offset.len() == 10 => row(offset.parse::<usize>().unwrap() + 7)
                .trim_end_matches('\n')
                .to_string(),
            _ => line.to_string(),
        })
        .collect();
    [&bytes[..at], moved.join("\n").as_bytes()].concat()
}

fn info(bytes: Vec<u8>) -> Value {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let request = json!({"op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = engine.dispatch(&mut session, request, vec![bytes]);
    assert_eq!(reply["ok"], true, "{reply}");
    reply["info"].clone()
}

#[test]
fn versions_count_the_file_as_mupdf_does() {
    // (layout, bytes, MuPDF's countVersions - 1, MuPDF's wasRepaired),
    // measured with MuPDF 1.28.1 through engine.ts.
    let cases = [
        ("classic", classic(), 0, false),
        ("classic+1", updated(classic(), 6), 1, false),
        ("classic+2", updated(updated(classic(), 6), 6), 2, false),
        ("hybrid", hybrid(), 0, false),
        ("hybrid+1", updated(hybrid(), 8), 1, false),
        ("hybrid+2", updated(updated(hybrid(), 8), 8), 2, false),
        ("linearized", linearized(), 0, false),
        ("linearized+1", updated(linearized(), 7), 1, false),
        (
            "linearized+2",
            updated(updated(linearized(), 7), 7),
            2,
            false,
        ),
        (
            "junk",
            [b"junk\n".as_slice(), &classic()].concat(),
            0,
            false,
        ),
        (
            "junk+1",
            [b"junk\n".as_slice(), &updated(classic(), 6)].concat(),
            1,
            false,
        ),
        ("shifted", shifted(classic()), 0, true),
        ("shifted+1", updated(shifted(classic()), 6), 0, true),
    ];
    for (layout, bytes, versions, repaired) in cases {
        let info = info(bytes);
        assert_eq!(
            (info["versions"].as_u64(), info["repaired"].as_bool()),
            (Some(versions), Some(repaired)),
            "{layout}: {info}"
        );
    }
}
