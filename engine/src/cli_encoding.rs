// SPDX-License-Identifier: MIT
//! Inspect explicit simple encodings; missing font context never selects a base.
use tandem_pdf_engine::object::Object;
use tandem_pdf_engine::parser::{ParseLimits, parse_complete};
use tandem_pdf_engine::simple_encoding::{
    EncodingLimits, GlyphUnicode, NamedEncoding, SimpleEncoding,
};

pub(super) fn load(bytes: &[u8]) -> Result<(NamedEncoding, SimpleEncoding), String> {
    let object = parse_complete(bytes, ParseLimits::default()).map_err(|e| e.to_string())?;
    load_object(&object)
}

pub(super) fn load_object(object: &Object) -> Result<(NamedEncoding, SimpleEncoding), String> {
    let (base, differences) = match object {
        Object::Name(name) => (name.as_slice(), None),
        Object::Dictionary(dictionary) => {
            for (key, value) in dictionary {
                if !matches!(key.as_slice(), b"Type" | b"BaseEncoding" | b"Differences")
                    && !matches!(value, Object::Null)
                {
                    return Err("unsupported non-null encoding dictionary key".into());
                }
            }
            match dictionary.get(b"Type".as_slice()) {
                None | Some(Object::Null) => {}
                Some(Object::Name(name)) if name == b"Encoding" => {}
                _ => return Err("encoding Type must be /Encoding".into()),
            }
            let name = match dictionary.get(b"BaseEncoding".as_slice()) {
                None | Some(Object::Null) => {
                    return Err(
                        "encoding requires font built-in context when BaseEncoding is absent"
                            .into(),
                    );
                }
                Some(Object::Name(name)) => name,
                _ => return Err("encoding BaseEncoding must be a direct name".into()),
            };
            let differences = dictionary
                .get(b"Differences".as_slice())
                .filter(|v| !matches!(v, Object::Null));
            (name.as_slice(), differences)
        }
        _ => return Err("encoding must be an explicit name or dictionary".into()),
    };
    let base = NamedEncoding::from_pdf_name(base).ok_or("unsupported named simple encoding")?;
    let encoding = SimpleEncoding::from_named(base);
    let encoding = if let Some(differences) = differences {
        encoding
            .with_object_differences(differences, EncodingLimits::default())
            .map_err(|e| e.to_string())?
    } else {
        encoding
    };
    Ok((base, encoding))
}

pub(super) fn inspect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let (base, encoding) = load(bytes)?;
    let mut output = Vec::new();
    // Supported base names are fixed ASCII constants from the enum.
    let name = std::str::from_utf8(base.pdf_name()).map_err(|_| "non-ASCII encoding constant")?;
    super::append_inspection_output(
        &mut output,
        format!("{{\"base\":\"{name}\",\"slots\":").as_bytes(),
    )?;
    write_slots(&mut output, &encoding)?;
    super::append_inspection_output(&mut output, b"}\n")?;
    Ok(output)
}

pub(super) fn write_slots(output: &mut Vec<u8>, encoding: &SimpleEncoding) -> Result<(), String> {
    super::append_inspection_output(output, b"[")?;
    for code in 0..=u8::MAX {
        if code > 0 {
            super::append_inspection_output(output, b",")?;
        }
        let glyph = encoding
            .glyph_name(code)
            .map(|name| format!("\"{}\"", super::hex(name)))
            .unwrap_or_else(|| "null".into());
        super::append_inspection_output(
            output,
            format!("{{\"code\":{code},\"glyph_name_hex\":{glyph},").as_bytes(),
        )?;
        match encoding.lookup_unicode(code) {
            GlyphUnicode::UndefinedCode => super::append_inspection_output(
                output,
                b"\"kind\":\"undefined\",\"scalars\":null}",
            )?,
            GlyphUnicode::UnknownGlyphName(_) => {
                super::append_inspection_output(output, b"\"kind\":\"unknown\",\"scalars\":null}")?
            }
            GlyphUnicode::Unicode(scalars) => {
                super::append_inspection_output(output, b"\"kind\":\"unicode\",\"scalars\":[")?;
                for (index, scalar) in scalars.iter().enumerate() {
                    if index > 0 {
                        super::append_inspection_output(output, b",")?;
                    }
                    super::append_inspection_output(
                        output,
                        (*scalar as u32).to_string().as_bytes(),
                    )?;
                }
                super::append_inspection_output(output, b"]}")?;
            }
        }
    }
    super::append_inspection_output(output, b"]")?;
    Ok(())
}
