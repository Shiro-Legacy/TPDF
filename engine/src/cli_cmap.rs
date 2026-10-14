// SPDX-License-Identifier: AGPL-3.0-or-later
//! Test/inspection CLI adapter. Query files are explicit user inputs; embedded
//! CMaps never request filesystem or system-CMap resources through this path.
use std::fs::File;
use std::io::Read;
use tandem_pdf_engine::cmap::{CMapLimits, CharacterCode, CodeDecodeError, parse_to_unicode};

fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
fn query_bytes(hex: &[u8], decode: bool) -> Result<Vec<u8>, String> {
    if decode && hex == b"-" {
        return Ok(Vec::new());
    }
    if hex.is_empty() || hex.len() > 8 || hex.len() % 2 != 0 {
        return Err(
            "CMap queries require1–4 complete hex bytes, or decode - for empty input".into(),
        );
    }
    hex.chunks_exact(2)
        .map(|pair| {
            let high = nibble(pair[0]).ok_or("invalid CMap query hex")?;
            let low = nibble(pair[1]).ok_or("invalid CMap query hex")?;
            Ok((high << 4) | low)
        })
        .collect()
}

pub(super) fn inspect(cmap_bytes: &[u8], query_path: &str) -> Result<Vec<u8>, String> {
    const QUERY_BYTES: usize = 8 * 1024 * 1024;
    const MAX_QUERIES: usize = 262_144;
    let cmap = parse_to_unicode(cmap_bytes, CMapLimits::default()).map_err(|e| e.to_string())?;
    let mut queries = Vec::new();
    File::open(query_path)
        .map_err(|e| format!("open CMap query file: {e}"))?
        .take(QUERY_BYTES as u64 + 1)
        .read_to_end(&mut queries)
        .map_err(|e| format!("read CMap queries: {e}"))?;
    if queries.len() > QUERY_BYTES {
        return Err("CMap query file exceeds8MiB".into());
    }
    let mut output = Vec::new();
    super::append_inspection_output(&mut output, b"{\"usecmap_ignored\":false,\"queries\":[")?;
    let mut count = 0;
    for line in queries.split(|&b| b == b'\n') {
        let mut words = line
            .split(u8::is_ascii_whitespace)
            .filter(|word| !word.is_empty());
        let Some(kind) = words.next() else { continue };
        let decode = match kind {
            b"lookup" => false,
            b"decode" => true,
            _ => return Err("unknown CMap query kind".into()),
        };
        let hex = words.next().ok_or("missing CMap query bytes")?;
        if words.next().is_some() {
            return Err("extra CMap query field".into());
        }
        let bytes = query_bytes(hex, decode)?;
        if count == MAX_QUERIES {
            return Err("CMap query count exceeds262144".into());
        }
        if count > 0 {
            super::append_inspection_output(&mut output, b",")?;
        }
        count += 1;
        let kind = if decode { "decode" } else { "lookup" };
        let input = super::hex(&bytes);
        let prefix = format!("{{\"kind\":\"{kind}\",\"input\":\"{input}\"");
        super::append_inspection_output(&mut output, prefix.as_bytes())?;
        let decoded = if decode {
            match cmap.decode_code_prefix(&bytes) {
                Ok(value) => Some(value),
                Err(error) => {
                    let tag = match error {
                        CodeDecodeError::EmptyInput | CodeDecodeError::IncompleteCode => {
                            "Incomplete"
                        }
                        CodeDecodeError::OutsideCodeSpace => "Invalid",
                    };
                    super::append_inspection_output(
                        &mut output,
                        format!(",\"error\":\"{tag}\"}}").as_bytes(),
                    )?;
                    None
                }
            }
        } else {
            Some((
                CharacterCode::from_bytes(&bytes).map_err(|e| format!("CMap query code: {e:?}"))?,
                bytes.len(),
            ))
        };
        let Some((code, consumed)) = decoded else {
            continue;
        };
        super::append_inspection_output(
            &mut output,
            format!(
                ",\"width\":{},\"value\":{},\"scalars\":",
                consumed,
                code.value()
            )
            .as_bytes(),
        )?;
        if let Some(unicode) = cmap.lookup(code) {
            super::append_inspection_output(&mut output, b"[")?;
            for (index, value) in unicode.scalars().iter().enumerate() {
                if index > 0 {
                    super::append_inspection_output(&mut output, b",")?;
                }
                super::append_inspection_output(
                    &mut output,
                    (*value as u32).to_string().as_bytes(),
                )?;
            }
            super::append_inspection_output(&mut output, b"]")?;
        } else {
            super::append_inspection_output(&mut output, b"null")?;
        }
        if decode {
            super::append_inspection_output(
                &mut output,
                format!(",\"consumed\":{consumed},\"matched_codespace\":true").as_bytes(),
            )?;
        }
        super::append_inspection_output(&mut output, b",\"valid_scalars\":true}")?;
    }
    super::append_inspection_output(&mut output, b"]}\n")?;
    Ok(output)
}

pub(super) fn decode_text(cmap_bytes: &[u8], raw_path: &str) -> Result<Vec<u8>, String> {
    use tandem_pdf_engine::text_decode::{TextDecodeLimits, decode_mapped_text};
    let limits = TextDecodeLimits::default();
    let cmap = parse_to_unicode(cmap_bytes, CMapLimits::default()).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    File::open(raw_path)
        .map_err(|e| format!("open mapped-text input: {e}"))?
        .take(limits.max_input_bytes as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|e| format!("read mapped-text input: {e}"))?;
    let decoded = decode_mapped_text(&cmap, &raw, limits).map_err(|e| e.to_string())?;
    text_json(&decoded)
}

pub(super) fn text_json(
    decoded: &tandem_pdf_engine::text_decode::DecodedText<'_>,
) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    super::append_inspection_output(
        &mut output,
        format!(
            "{{\"text_utf8_hex\":\"{}\",\"codes\":[",
            super::hex(decoded.text().as_bytes())
        )
        .as_bytes(),
    )?;
    for (index, code) in decoded.codes().iter().enumerate() {
        if index > 0 {
            super::append_inspection_output(&mut output, b",")?;
        }
        let source = code.source_span();
        let utf8 = code.utf8_span();
        super::append_inspection_output(
            &mut output,
            format!(
                "{{\"code\":\"{}\",\"source\":[{},{}],\"utf8\":[{},{}]}}",
                super::hex(&code.code().to_bytes()),
                source.start,
                source.end,
                utf8.start,
                utf8.end
            )
            .as_bytes(),
        )?;
    }
    super::append_inspection_output(&mut output, b"]}\n")?;
    Ok(output)
}
