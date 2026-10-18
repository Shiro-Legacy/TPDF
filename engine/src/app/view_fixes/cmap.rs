// SPDX-License-Identifier: MIT
//! An embedded CMap whose stream dictionary sets /WMode against its
//! program's: reference viewers take the dictionary's,
//! hayro the program's. The view writes the dictionary's writing mode into
//! the program.
use crate::lexer::{LexLimits, Lexer, TokenKind};
use crate::object::Object;

use super::Fixes;

impl Fixes<'_, '_> {
    /// Gives the embedded CMap of a Type0 `font` its dictionary's /WMode.
    pub(super) fn fix_cmap_wmode(&mut self, font: &Object) {
        let doc = self.doc;
        let Some(font) = self.view_resolve(font).as_dict() else {
            return;
        };
        if doc.dict_get(font, b"Subtype").as_name() != Some(b"Type0") {
            return;
        }
        let Some(Object::Reference(id)) = font.get(b"Encoding".as_slice()) else {
            return;
        };
        let id = *id;
        let Some(mut dict) = self.stream_dict(id).cloned() else {
            return;
        };
        let Some(wmode) = doc.dict_get(&dict, b"WMode").as_i64() else {
            return;
        };
        if !self.recolor.done.insert(id) {
            return;
        }
        let Some(program) = self.decoded(id) else {
            return;
        };
        let wanted: &[u8] = if wmode == 0 { b"0" } else { b"1" };
        let Some(patched) = with_wmode(&program, wanted) else {
            return;
        };
        for key in [b"Filter".as_slice(), b"DecodeParms", b"DL"] {
            dict.remove(key);
        }
        self.streams.insert(id, (dict, patched));
    }
}

/// `program` with its `/WMode n def` set to `wanted`, or, without one,
/// with it added in front when `wanted` is not 0; None when it already
/// reads so or cannot be read.
fn with_wmode(program: &[u8], wanted: &[u8]) -> Option<Vec<u8>> {
    let mut lexer = Lexer::new(program, LexLimits::default());
    let mut previous: [Option<TokenKind>; 2] = [None, None];
    let mut value = None;
    while let Ok(Some(token)) = lexer.next_token() {
        if let TokenKind::Integer(_) = token.kind {
            value = Some(token.span.clone());
        }
        let defined = matches!(&token.kind, TokenKind::Keyword(word) if word == b"def");
        if let ([Some(TokenKind::Name(name)), Some(TokenKind::Integer(_))], true) =
            (&previous, defined)
        {
            if name == b"WMode" {
                let span = value?;
                if program.get(span.clone())? == wanted {
                    return None;
                }
                let mut out = program.get(..span.start)?.to_vec();
                out.extend_from_slice(wanted);
                out.extend_from_slice(program.get(span.end..)?);
                return Some(out);
            }
        }
        previous = [previous[1].take(), Some(token.kind)];
    }
    if wanted == b"0" {
        return None;
    }
    let mut out = b"/WMode 1 def\n".to_vec();
    out.extend_from_slice(program);
    Some(out)
}
