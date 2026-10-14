// SPDX-License-Identifier: AGPL-3.0-or-later
//! Test-only line protocol: six f64 hex bit patterns, width height sample_x sample_y.
use std::io::{self, Read};
use tandem_pdf_engine::{
    geometry::Affine,
    image_sampling::{ImageSamplingLimits, ImageSamplingSession},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut data = String::new();
    io::stdin().take((8 << 20) + 1).read_to_string(&mut data)?;
    if data.len() > 8 << 20 {
        return Err("probe input limit".into());
    }
    let mut out = String::new();
    for line in data.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 10 {
            return Err("probe field count".into());
        }
        let mut matrix = [0.; 6];
        for (i, value) in matrix.iter_mut().enumerate() {
            *value = f64::from_bits(u64::from_str_radix(fields[i], 16)?);
        }
        let values: Vec<u32> = fields[6..]
            .iter()
            .map(|v| v.parse())
            .collect::<Result<_, _>>()?;
        let mut session = ImageSamplingSession::new(ImageSamplingLimits::default());
        let value = Affine::from_pdf(matrix)
            .map_err(|_| "transform")
            .and_then(|m| {
                session
                    .prepare(m, values[0], values[1])
                    .map_err(|_| "prepare")
            })
            .and_then(|p| {
                session
                    .sample(&p, values[2], values[3])
                    .map_err(|_| "sample")
            });
        use std::fmt::Write;
        match value {
            Ok(Some(t)) => writeln!(out, "{} {}", t.row(), t.column())?,
            Ok(None) => writeln!(out, "outside")?,
            Err(_) => writeln!(out, "refused")?,
        }
    }
    print!("{out}");
    Ok(())
}
