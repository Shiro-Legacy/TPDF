//! Reference-viewer comparison driver for the app save (password from TPDF_PDF_PASSWORD):
//!   app_save_probe FILE OUT.pdf
//! The input opens as the app opens it (strict, else repaired) and is written
//! with `WriteOptions::APP`; an encrypted input is written decrypted.
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::repair::open_tolerant;
use tandem_pdf_engine::writer::{DocumentWriteLimits, Encryption, WriteOptions, write_document};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let [_, input, output] = args.as_slice() else {
        return Err("usage: app_save_probe FILE OUT.pdf".into());
    };
    let password = std::env::var("TPDF_PDF_PASSWORD").unwrap_or_default();
    let limits = DocumentLimits::default();
    let opened = open_tolerant(std::fs::read(input)?, limits, password.as_bytes())?;
    let doc = Document::open_with_password(&opened.bytes, limits, password.as_bytes())?;
    let options = WriteOptions {
        encryption: Encryption::Plaintext,
        ..WriteOptions::APP
    };
    std::fs::write(
        output,
        write_document(&doc, DocumentWriteLimits::default(), options)?,
    )?;
    Ok(())
}
