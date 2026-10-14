// SPDX-License-Identifier: AGPL-3.0-or-later
//! Test-only owned mask provenance and complete effective alpha output.
use std::io::{self, Read, Write};
use tandem_pdf_engine::{document::Document, image_resources::ImageLoader, object::ObjectId};
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let n: u32 = std::env::args()
        .nth(1)
        .ok_or("object number required")?
        .parse()?;
    let mut bytes = Vec::new();
    io::stdin().take((16 << 20) + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 16 << 20 {
        return Err("input bound".into());
    }
    let doc = Document::open(&bytes, Default::default())?;
    let mut loader = ImageLoader::new(&doc, Default::default());
    let asset = loader.load(ObjectId::new(n, 0)?)?;
    let mask = asset.opaque_mask().ok_or("mask provenance required")?;
    let mut out = format!(
        "{{\"base_object\":{},\"mask_object\":{},\"width\":{},\"height\":{},\"alpha_hex\":\"",
        asset.object_id().number(),
        mask.object_id().number(),
        mask.width(),
        mask.height()
    );
    for y in 0..mask.height() {
        for x in 0..mask.width() {
            use std::fmt::Write;
            write!(out, "{:02x}", mask.pixel(y, x).ok_or("mask indexing")?[0])?;
        }
    }
    out.push_str("\"}\n");
    io::stdout().write_all(out.as_bytes())?;
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("soft-mask: {e}");
        std::process::exit(2);
    }
}
