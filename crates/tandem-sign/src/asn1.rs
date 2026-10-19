// SPDX-License-Identifier: MIT
//! A small DER reader that never panics and a DER writer.
//!
//! The reader walks tag-length-value nodes and hands out raw byte ranges, so
//! the verifier can compare and hash the exact bytes a signer wrote (signed
//! attributes, issuer names) instead of a re-encoding. It accepts only
//! definite lengths of at most four length bytes and single-byte tags, which
//! is all that CMS and X.509 use in practice. Typed decoding of certificates
//! is left to `x509-cert`.

use const_oid::ObjectIdentifier;

/// One node: `bytes[start..end]` is the whole node, `bytes[value_start..end]`
/// its contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tlv {
    pub tag: u8,
    pub start: usize,
    pub value_start: usize,
    pub end: usize,
}

impl Tlv {
    /// The node's contents.
    pub fn value<'a>(&self, bytes: &'a [u8]) -> &'a [u8] {
        bytes.get(self.value_start..self.end).unwrap_or(&[])
    }
    /// The whole node, header included.
    pub fn raw<'a>(&self, bytes: &'a [u8]) -> &'a [u8] {
        bytes.get(self.start..self.end).unwrap_or(&[])
    }
}

/// The node starting at `offset`, or `None` when it does not fit in `bytes`.
pub fn parse(bytes: &[u8], offset: usize) -> Option<Tlv> {
    let tag = *bytes.get(offset)?;
    let first = *bytes.get(offset.checked_add(1)?)? as usize;
    let mut cursor = offset + 2;
    let length = if first & 0x80 == 0 {
        first
    } else {
        let count = first & 0x7f;
        if count == 0 || count > 4 {
            return None;
        }
        let mut length = 0usize;
        for _ in 0..count {
            length = (length << 8) | *bytes.get(cursor)? as usize;
            cursor += 1;
        }
        length
    };
    let end = cursor.checked_add(length)?;
    if end > bytes.len() {
        return None;
    }
    Some(Tlv {
        tag,
        start: offset,
        value_start: cursor,
        end,
    })
}

/// The children of a constructed node, or `None` when one of them overruns it.
pub fn children(bytes: &[u8], node: &Tlv) -> Option<Vec<Tlv>> {
    let mut out = Vec::new();
    let mut cursor = node.value_start;
    while cursor < node.end {
        let child = parse(bytes, cursor)?;
        if child.end > node.end || child.end <= cursor {
            return None;
        }
        out.push(child);
        cursor = child.end;
    }
    Some(out)
}

/// Whether `node` holds the content bytes of `oid` (the tag is not checked).
pub fn is_oid(bytes: &[u8], node: &Tlv, oid: &ObjectIdentifier) -> bool {
    node.value(bytes) == oid.as_bytes()
}

/// The dotted form of an OID's content bytes, for reasons and messages.
pub fn oid_text(bytes: &[u8], node: &Tlv) -> String {
    let body = node.value(bytes);
    let Some(&first) = body.first() else {
        return String::new();
    };
    let mut arcs: Vec<u64> = vec![u64::from(first / 40), u64::from(first % 40)];
    let mut value: u64 = 0;
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

/// Big-endian unsigned value of an INTEGER's content, `None` past `u64`.
pub fn small_uint(bytes: &[u8], node: &Tlv) -> Option<u64> {
    let body = node.value(bytes);
    if body.is_empty() || body.len() > 9 || body[0] & 0x80 != 0 {
        return None;
    }
    let body = if body.len() == 9 {
        if body[0] != 0 {
            return None;
        }
        &body[1..]
    } else {
        body
    };
    Some(body.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b)))
}

// ---- writer -------------------------------------------------------------

/// A DER length.
pub fn len_bytes(n: usize) -> Vec<u8> {
    if n < 0x80 {
        return vec![n as u8];
    }
    let mut bytes = Vec::new();
    let mut x = n;
    while x > 0 {
        bytes.insert(0, (x & 0xff) as u8);
        x >>= 8;
    }
    let mut out = vec![0x80 | bytes.len() as u8];
    out.extend(bytes);
    out
}

/// A node with `tag` around `content`.
pub fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend(len_bytes(content.len()));
    out.extend_from_slice(content);
    out
}

/// A SEQUENCE of already-encoded parts.
pub fn seq(parts: &[&[u8]]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}

/// A DER SET OF: the parts sorted by their encodings.
pub fn set_of(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut sorted: Vec<&Vec<u8>> = parts.iter().collect();
    sorted.sort();
    let body: Vec<u8> = sorted.into_iter().flatten().copied().collect();
    tlv(0x31, &body)
}

/// An OBJECT IDENTIFIER.
pub fn oid(oid: &ObjectIdentifier) -> Vec<u8> {
    tlv(0x06, oid.as_bytes())
}

/// A small non-negative INTEGER.
pub fn uint(value: u64) -> Vec<u8> {
    let mut bytes: Vec<u8> = value.to_be_bytes().to_vec();
    while bytes.len() > 1 && bytes[0] == 0 && bytes[1] & 0x80 == 0 {
        bytes.remove(0);
    }
    if bytes[0] & 0x80 != 0 {
        bytes.insert(0, 0);
    }
    tlv(0x02, &bytes)
}

/// An AlgorithmIdentifier, with an explicit NULL parameter when `null`.
pub fn algorithm(id: &ObjectIdentifier, null: bool) -> Vec<u8> {
    if null {
        seq(&[&oid(id), &[0x05, 0x00]])
    } else {
        seq(&[&oid(id)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oid_round_trip_and_set_sorting() {
        let id = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
        let encoded = oid(&id);
        let node = parse(&encoded, 0).unwrap();
        assert_eq!(node.tag, 0x06);
        assert_eq!(node.end, encoded.len());
        assert_eq!(tlv(node.tag, node.value(&encoded)), encoded);
        assert_eq!(oid_text(&encoded, &node), "1.2.840.113549.1.9.4");
        let set = set_of(&[uint(2), uint(1)]);
        assert_eq!(set[0], 0x31);
        assert_eq!(set[4], 1);
        assert_eq!(set[7], 2);
    }

    #[test]
    fn lengths_and_integers() {
        assert_eq!(len_bytes(0x7f), vec![0x7f]);
        assert_eq!(len_bytes(0x80), vec![0x81, 0x80]);
        assert_eq!(len_bytes(0x1234), vec![0x82, 0x12, 0x34]);
        assert_eq!(uint(1), vec![2, 1, 1]);
        assert_eq!(uint(0x80), vec![2, 2, 0, 0x80]);
        assert_eq!(uint(0), vec![2, 1, 0]);
        let long = tlv(0x04, &[7u8; 300]);
        let node = parse(&long, 0).unwrap();
        assert_eq!(node.value(&long).len(), 300);
        assert_eq!(
            small_uint(&uint(2048), &parse(&uint(2048), 0).unwrap()),
            Some(2048)
        );
    }

    #[test]
    fn hostile_lengths_do_not_panic() {
        assert_eq!(parse(&[0x30], 0), None);
        assert_eq!(parse(&[0x30, 0x80], 0), None);
        assert_eq!(parse(&[0x30, 0x85, 1, 2, 3, 4, 5], 0), None);
        assert_eq!(parse(&[0x30, 0x84, 0xff, 0xff, 0xff, 0xff], 0), None);
        assert_eq!(parse(&[0x30, 0x05, 1], 0), None);
        let outer = [0x30, 0x03, 0x04, 0x05, 0x00];
        assert_eq!(children(&outer, &parse(&outer, 0).unwrap()), None);
    }
}
