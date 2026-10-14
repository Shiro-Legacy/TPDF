// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bounded inspection of PDF syntax, not a complete document renderer or validator.
#![forbid(unsafe_code)]
mod cli_builtin;
mod cli_builtin_outlines;
mod cli_cmap;
mod cli_encoding;
mod cli_fonts;
mod cli_glyph;
mod cli_outline;
mod cli_page_geometry;
mod cli_page_glyph;
mod cli_page_program;
mod cli_page_raster;
mod cli_path;
mod cli_program;
#[cfg(feature = "raster")]
mod cli_raster;
mod cli_text;
use std::env;
use std::fs::File;
use std::io::{self, Read, Write};
use std::process::ExitCode;
use tandem_pdf_engine::content::{ContentLimits, ContentOperand, ContentParser};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::lexer::{LexLimits, Lexer, TokenKind};
use tandem_pdf_engine::object::{Object, ObjectId};
use tandem_pdf_engine::operators::validate_operation;
use tandem_pdf_engine::page_content::{PageContentLimits, load_page_content};
use tandem_pdf_engine::pages::{PageLimits, collect_pages};
use tandem_pdf_engine::parser::{ParseLimits, parse_complete};
use tandem_pdf_engine::serialize::{WriteLimits, serialize_object};
use tandem_pdf_engine::stream::{Filter, StreamLimits, decode_filters};
use tandem_pdf_engine::stream_object::decode_document_stream;
use tandem_pdf_engine::writer::{DocumentWriteLimits, rewrite_document};

const MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        result.push(DIGITS[(byte >> 4) as usize] as char);
        result.push(DIGITS[(byte & 15) as usize] as char);
    }
    result
}

// Move owned content values into the already-tested object serializer. Binary
// strings/names are moved, not decoded as text or cloned. The parser has already
// bounded nesting and aggregate nodes before this adapter is reached.
fn content_object(value: ContentOperand) -> Object {
    match value {
        ContentOperand::Null => Object::Null,
        ContentOperand::Boolean(v) => Object::Boolean(v),
        ContentOperand::Integer(v) => Object::Integer(v),
        ContentOperand::Real(v) => Object::Real(v),
        ContentOperand::Name(v) => Object::Name(v),
        ContentOperand::String(v) => Object::String(v),
        ContentOperand::Array(v) => Object::Array(v.into_iter().map(content_object).collect()),
        ContentOperand::Dictionary(v) => Object::Dictionary(
            v.into_iter()
                .map(|(key, value)| (key, content_object(value)))
                .collect(),
        ),
    }
}
fn append_inspection_output(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), String> {
    const LIMIT: usize = 64 * 1024 * 1024;
    let end = output
        .len()
        .checked_add(bytes.len())
        .filter(|&n| n <= LIMIT)
        .ok_or("inspection CLI output exceeds64MiB")?;
    if end > output.capacity() {
        let capacity = end
            .max(output.capacity().saturating_mul(2).max(64))
            .min(LIMIT);
        output
            .try_reserve_exact(capacity - output.len())
            .map_err(|_| "inspection CLI output allocation failed")?;
    }
    output.extend_from_slice(bytes);
    Ok(())
}
fn run() -> Result<(), String> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let mode = args.first().and_then(|v| v.to_str()).unwrap_or("");
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!(
            "Usage: pdf-inspect --tokens|--object|--canonical|--document|--rewrite|--content|--check-content|--raster=WIDTHxHEIGHT|--path|--outline=QUERIES|--tt-cmap=all|--font-glyphs=N:G|--program=FONTSFILE|--text=FONTSFILE|--font-metrics|--builtin-metrics|--builtin=FACE|--fonts|--encoding|--encoding-text=RAWFILE|--cmap=QUERIES|--cmap-text=RAWFILE|--builtin-outlines=FACE|--page-outlines[=N]|--page-geometry|--page-render=N@DPI|--page-glyph=N|--page-glyphs|--page-program=N|--page-programs|--page-content=N|--decode=FILTERS|--stream=N:G [FILE|-]\n\
            Inspect at most 16 MiB of PDF syntax; defaults to stdin.\n\
            --tokens emits TSV compatible with the pinned MuPDF lexer oracle.\n\
            --object parses exactly one direct object and prints its Rust debug form.\n\
            --canonical parses one direct object and emits canonical PDF object syntax.\n\
            --content emits operator-hex, operand count, and canonical-operands-hex as TSV.\n\
            --check-content validates local operator operand schemas; it does not execute content.\n\
            --builtin-outlines=FACE inspects immutable pinned Helvetica or Times-Roman assets (no input).\n\
            --page-outlines[=N] includes explicitly pinned Base14 outlines for all pages or one page.\n\
            --page-render=N@DPI emits opaque PPM pixels for the supported filled-page subset.\n\
            --page-glyph=N emits supported embedded glyph outlines placed on one-based page N.\n\
            --page-glyphs emits page glyph traces and explicit refusals in one session.\n\
            --tt-cmap=all|N inspects raw TrueType character maps; --font-glyphs=N:G inspects a PDF font.\n\
            --page-program=N emits a full mixed trace for one-based page N (pinned builtin metrics enabled).\n\
            --page-programs emits compact traces/refusals for every page in one bounded session.\n\
            --page-geometry emits typed page transform coefficients and bounds.\n\
            --page-content=N emits logical decoded content for one-based page N.\n\
            --raster=WIDTHxHEIGHT emits PPM for the orthogonal subset (raster feature).\n\
            --path executes a standalone path-only program to an event trace.\n\
            --outline=QUERIES|all inspects simple unhinted TrueType glyph outlines.\n\
            --program=FONTSFILE traces mixed path/text content with shared state.\n\
            --text=FONTSFILE positions a standalone text program using explicit font bindings.\n\
            --builtin=FACE inspects a supported builtin face with an explicit encoding.\n\
            --builtin-metrics lists eligible document builtin advances.\n\
            --font-metrics lists explicit simple-font advances and provenance.\n\
            --fonts lists page font resources and declared simple-font mappings.\n\
            --encoding-text=RAWFILE decodes bytes through an explicit simple encoding.\n\
            --encoding inspects an explicit named encoding and optional Differences.\n\
            --cmap=QUERIES parses a strict ToUnicode map and executes a bounded query file.\n\
            --cmap-text=RAWFILE decodes bytes through a strict map, preserving source/UTF8 spans.\n\
            --document reads supported PDFs and emits page geometry as JSON.\n\
            --decode=flate,asciihex,ascii85,runlength,lzw,lzw0 applies the named filters in order.\n\
            --stream=N:G decodes the indirect stream with object number N and generation G.\n\
            --rewrite emits a fresh PDF to stdout; signed/linearized inputs are refused.\n\
            Page rendering is limited to the explicitly supported engine subset."
        );
        return Ok(());
    }
    if args.is_empty()
        || args.len() > 2
        || (!matches!(
            mode,
            "--tokens"
                | "--object"
                | "--canonical"
                | "--document"
                | "--rewrite"
                | "--content"
                | "--check-content"
                | "--path"
                | "--font-metrics"
                | "--builtin-metrics"
                | "--fonts"
                | "--encoding"
                | "--page-geometry"
                | "--page-programs"
                | "--page-glyphs"
                | "--page-outlines"
        ) && !mode.starts_with("--decode=")
            && !mode.starts_with("--stream=")
            && !mode.starts_with("--page-content=")
            && !mode.starts_with("--page-program=")
            && !mode.starts_with("--page-glyph=")
            && !mode.starts_with("--page-render=")
            && !mode.starts_with("--page-outlines=")
            && !mode.starts_with("--builtin-outlines=")
            && !mode.starts_with("--cmap=")
            && !mode.starts_with("--cmap-text=")
            && !mode.starts_with("--encoding-text=")
            && !mode.starts_with("--raster=")
            && !mode.starts_with("--builtin=")
            && !mode.starts_with("--text=")
            && !mode.starts_with("--program=")
            && !mode.starts_with("--outline=")
            && !mode.starts_with("--tt-cmap=")
            && !mode.starts_with("--font-glyphs="))
    {
        return Err("usage: pdf-inspect --tokens|--object|--canonical|--document|--rewrite|--content|--check-content|--raster=WIDTHxHEIGHT|--path|--outline=QUERIES|--tt-cmap=all|--font-glyphs=N:G|--program=FONTSFILE|--text=FONTSFILE|--font-metrics|--builtin-metrics|--builtin=FACE|--fonts|--encoding|--encoding-text=RAWFILE|--cmap=QUERIES|--cmap-text=RAWFILE|--builtin-outlines=FACE|--page-outlines[=N]|--page-geometry|--page-render=N@DPI|--page-glyph=N|--page-glyphs|--page-program=N|--page-programs|--page-content=N|--decode=FILTERS|--stream=N:G [FILE|-] (see --help)".into());
    }
    if let Some(face) = mode.strip_prefix("--builtin-outlines=") {
        if args.len() != 1 {
            return Err("builtin-outlines takes no input file".into());
        }
        let data = cli_builtin_outlines::inspect(face)?;
        return io::stdout()
            .lock()
            .write_all(&data)
            .map_err(|e| e.to_string());
    }
    let reader: Box<dyn Read> = match args.get(1) {
        Some(path) if path != "-" => {
            Box::new(File::open(path).map_err(|err| format!("open: {err}"))?)
        }
        _ => Box::new(io::stdin()),
    };
    let mut bytes = Vec::new();
    reader
        .take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| format!("read: {err}"))?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(format!("input exceeds {MAX_INPUT_BYTES} bytes"));
    }
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    if mode == "--page-outlines" {
        let data = cli_page_glyph::inspect_policy(
            &bytes,
            None,
            tandem_pdf_engine::page_glyph::PageOutlinePolicy::AllowPinnedBase14,
        )?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if let Some(page) = mode.strip_prefix("--page-outlines=") {
        let index = page
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .ok_or("page must be a positive one-based integer")?;
        let data = cli_page_glyph::inspect_policy(
            &bytes,
            Some(index),
            tandem_pdf_engine::page_glyph::PageOutlinePolicy::AllowPinnedBase14,
        )?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if let Some(selector) = mode.strip_prefix("--page-render=") {
        let data = cli_page_raster::inspect(&bytes, selector)?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if mode == "--page-glyphs" {
        let data = cli_page_glyph::inspect(&bytes, None)?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if let Some(page) = mode.strip_prefix("--page-glyph=") {
        let index = page
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .ok_or("page must be a positive one-based integer")?;
        let data = cli_page_glyph::inspect(&bytes, Some(index))?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if let Some(selector) = mode.strip_prefix("--tt-cmap=") {
        let data = cli_glyph::cmap(&bytes, selector)?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if let Some(selector) = mode.strip_prefix("--font-glyphs=") {
        let data = cli_glyph::font(&bytes, selector)?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if args[0] == "--tokens" {
        let mut lexer = Lexer::new(&bytes, LexLimits::default());
        while let Some(token) = lexer.next_token().map_err(|err| err.to_string())? {
            let line = match token.kind {
                TokenKind::Integer(value) => format!("integer\t{value}"),
                TokenKind::Real(value) => format!("real\t{value}"),
                TokenKind::Name(value) => format!("name\t{}", hex(&value)),
                TokenKind::String(value) => format!("string\t{}", hex(&value)),
                TokenKind::Keyword(value) => format!("keyword\t{}", hex(&value)),
                TokenKind::ArrayStart => "array-start".into(),
                TokenKind::ArrayEnd => "array-end".into(),
                TokenKind::DictStart => "dict-start".into(),
                TokenKind::DictEnd => "dict-end".into(),
            };
            writeln!(out, "{line}").map_err(|err| format!("write: {err}"))?;
        }
    } else if mode == "--page-programs" {
        let data = cli_page_program::inspect(&bytes, None)?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if let Some(page) = mode.strip_prefix("--page-program=") {
        let index = page
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .ok_or("page-program requires a one-based page number")?;
        let data = cli_page_program::inspect(&bytes, Some(index))?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if mode == "--page-geometry" {
        let data = cli_page_geometry::inspect(&bytes)?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if let Some(dimensions) = mode.strip_prefix("--raster=") {
        #[cfg(feature = "raster")]
        {
            let data = cli_raster::inspect(&bytes, dimensions)?;
            out.write_all(&data).map_err(|e| e.to_string())?;
        }
        #[cfg(not(feature = "raster"))]
        {
            let _ = dimensions;
            return Err("raster support requires a build with --features raster".into());
        }
    } else if let Some(query_path) = mode.strip_prefix("--outline=") {
        let data = cli_outline::inspect(&bytes, query_path)?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if let Some(font_path) = mode.strip_prefix("--program=") {
        let data = cli_program::inspect(&bytes, font_path)?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if let Some(font_path) = mode.strip_prefix("--text=") {
        let data = cli_text::inspect(&bytes, font_path)?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if mode == "--path" {
        let data = cli_path::inspect(&bytes)?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if mode == "--builtin-metrics" {
        let data = cli_fonts::inspect_builtin_metrics(&bytes)?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if let Some(face) = mode.strip_prefix("--builtin=") {
        let data = cli_builtin::inspect(&bytes, face)?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if mode == "--font-metrics" {
        let data = cli_fonts::inspect_metrics(&bytes)?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if mode == "--fonts" {
        let data = cli_fonts::inspect(&bytes)?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if mode == "--encoding" {
        let data = cli_encoding::inspect(&bytes)?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if let Some(raw_path) = mode.strip_prefix("--encoding-text=") {
        let data = cli_encoding::decode_text(&bytes, raw_path)?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if let Some(raw_path) = mode.strip_prefix("--cmap-text=") {
        let data = cli_cmap::decode_text(&bytes, raw_path)?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if let Some(query_path) = mode.strip_prefix("--cmap=") {
        let data = cli_cmap::inspect(&bytes, query_path)?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if let Some(page) = mode.strip_prefix("--page-content=") {
        let index = page
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .ok_or("page-content requires a one-based page number")?;
        let document =
            Document::open(&bytes, DocumentLimits::default()).map_err(|e| e.to_string())?;
        let pages = collect_pages(&document, PageLimits::default()).map_err(|e| e.to_string())?;
        let page = pages
            .get(index)
            .ok_or("page-content index is outside the document")?;
        let content = load_page_content(&document, page, PageContentLimits::default())
            .map_err(|e| e.to_string())?;
        out.write_all(&content.bytes).map_err(|e| e.to_string())?;
    } else if mode == "--check-content" {
        let mut parser = ContentParser::new(&bytes, ContentLimits::default());
        let mut operations = 0;
        while let Some(operation) = parser.next_operation().map_err(|e| e.to_string())? {
            validate_operation(&operation).map_err(|e| e.to_string())?;
            operations += 1;
        }
        writeln!(
            out,
            "{{\"operations\":{operations},\"validation\":\"local-operand-schema\"}}"
        )
        .map_err(|e| e.to_string())?;
    } else if mode == "--content" {
        let mut parser = ContentParser::new(&bytes, ContentLimits::default());
        let mut output = Vec::new();
        while let Some(operation) = parser.next_operation().map_err(|e| e.to_string())? {
            append_inspection_output(
                &mut output,
                format!(
                    "{}\t{}\t",
                    hex(&operation.operator),
                    operation.operands.len()
                )
                .as_bytes(),
            )?;
            for (index, operand) in operation.operands.into_iter().enumerate() {
                if index > 0 {
                    append_inspection_output(&mut output, b"20")?;
                }
                let canonical = serialize_object(
                    &content_object(operand),
                    WriteLimits {
                        max_output_bytes: 32 * 1024 * 1024,
                        max_objects: 1_000_000,
                        ..WriteLimits::default()
                    },
                )
                .map_err(|e| e.to_string())?;
                append_inspection_output(&mut output, hex(&canonical).as_bytes())?;
            }
            append_inspection_output(&mut output, b"\n")?;
        }
        out.write_all(&output).map_err(|e| e.to_string())?;
    } else if mode == "--rewrite" {
        let document =
            Document::open(&bytes, DocumentLimits::default()).map_err(|e| e.to_string())?;
        let data = rewrite_document(&document, DocumentWriteLimits::default())
            .map_err(|e| e.to_string())?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if mode == "--document" {
        let document =
            Document::open(&bytes, DocumentLimits::default()).map_err(|e| e.to_string())?;
        let pages = collect_pages(&document, PageLimits::default()).map_err(|e| e.to_string())?;
        write!(out, "{{\"objects\":{},\"pages\":[", document.object_count())
            .map_err(|e| e.to_string())?;
        for (index, page) in pages.iter().enumerate() {
            if index > 0 {
                write!(out, ",").map_err(|e| e.to_string())?;
            }
            let b = page.display_bounds();
            write!(
                out,
                "{{\"object\":{},\"generation\":{},\"rotation\":{},\"bounds\":[{},{},{},{}]}}",
                page.id.number(),
                page.id.generation(),
                page.rotation,
                b[0],
                b[1],
                b[2],
                b[3]
            )
            .map_err(|e| e.to_string())?;
        }
        write!(out, "],\"streams\":[").map_err(|e| e.to_string())?;
        for (index, object) in document
            .objects()
            .filter(|object| object.stream.is_some())
            .enumerate()
        {
            if index > 0 {
                write!(out, ",").map_err(|e| e.to_string())?;
            }
            write!(
                out,
                "{{\"object\":{},\"generation\":{}}}",
                object.id.number(),
                object.id.generation()
            )
            .map_err(|e| e.to_string())?;
        }
        writeln!(out, "]}}").map_err(|e| e.to_string())?;
    } else if let Some(id) = mode.strip_prefix("--stream=") {
        let (number, generation) = id
            .split_once(':')
            .ok_or("stream requires object:generation")?;
        let number = number
            .parse::<u32>()
            .map_err(|_| "invalid stream object number")?;
        let generation = generation
            .parse::<u16>()
            .map_err(|_| "invalid stream generation")?;
        let id = ObjectId::new(number, generation).map_err(|e| e.to_string())?;
        let document =
            Document::open(&bytes, DocumentLimits::default()).map_err(|e| e.to_string())?;
        let data = decode_document_stream(&document, id, StreamLimits::default())
            .map_err(|e| e.to_string())?;
        out.write_all(&data).map_err(|e| e.to_string())?;
    } else if let Some(filters) = mode.strip_prefix("--decode=") {
        let filters = filters
            .split(',')
            .map(|name| match name {
                "flate" => Ok(Filter::Flate),
                "asciihex" => Ok(Filter::AsciiHex),
                "ascii85" => Ok(Filter::Ascii85),
                "runlength" => Ok(Filter::RunLength),
                "lzw" => Ok(Filter::Lzw { early_change: true }),
                "lzw0" => Ok(Filter::Lzw {
                    early_change: false,
                }),
                _ => Err(format!("unknown filter {name:?}")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let decoded =
            decode_filters(&bytes, &filters, StreamLimits::default()).map_err(|e| e.to_string())?;
        out.write_all(&decoded).map_err(|e| e.to_string())?;
    } else {
        let object =
            parse_complete(&bytes, ParseLimits::default()).map_err(|err| err.to_string())?;
        if mode == "--canonical" {
            let bytes =
                serialize_object(&object, WriteLimits::default()).map_err(|e| e.to_string())?;
            out.write_all(&bytes).map_err(|e| e.to_string())?;
            writeln!(out).map_err(|e| e.to_string())?;
        } else {
            writeln!(out, "{object:?}").map_err(|err| format!("write: {err}"))?;
        }
    }
    out.flush().map_err(|err| format!("write: {err}"))?;
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pdf-inspect: {error}");
            ExitCode::from(2)
        }
    }
}
