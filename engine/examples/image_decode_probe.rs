// SPDX-License-Identifier: AGPL-3.0-or-later
//! Test-only original image-object decoder, atomic PPM stdout on success.
use std::io::{self, Read, Write};
use tandem_pdf_engine::{
    document::{Document, DocumentLimits},
    image_resources::{ImageLimits, ImageLoader},
    object::ObjectId,
};
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let n: u32 = args.next().ok_or("object number required")?.parse()?;
    if args.next().is_some() {
        return Err("one argument required".into());
    }
    let mut bytes = Vec::new();
    io::stdin().take((16 << 20) + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 16 << 20 {
        return Err("input bound".into());
    }
    let doc = Document::open(&bytes, DocumentLimits::default())?;
    let asset = ImageLoader::new(&doc, ImageLimits::default()).load(ObjectId::new(n, 0)?)?;
    let mut result = format!("P6\n{} {}\n255\n", asset.width(), asset.height()).into_bytes();
    for row in 0..asset.height() {
        for col in 0..asset.width() {
            result.extend_from_slice(&asset.pixel(row, col).ok_or("pixel indexing invariant")?);
        }
    }
    io::stdout().write_all(&result)?;
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("image-decode: {e}");
        std::process::exit(2);
    }
}
