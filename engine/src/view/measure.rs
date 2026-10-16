// SPDX-License-Identifier: AGPL-3.0-or-later
//! Glyph measurement for text edits: codes shown with a page's own font,
//! interpreted by hayro exactly as the view draws them, so an edit's width
//! and missing-glyph checks use the font the page renders with.
use super::walk::{FontBudget, FontSizes};
use super::{PageFrame, ViewDocument, ViewError};
use hayro::kurbo::{Affine, BezPath, Rect};
use hayro_interpret::font::{Glyph, GlyphRun};
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, SoftMask, interpret,
};
use hayro_syntax::content::TypedIter;
use hayro_syntax::object::dict::keys::RESOURCES;
use hayro_syntax::object::{Dict, ObjectIdentifier, Stream};
use hayro_syntax::page::{Page, Resources};
use std::cell::RefCell;

/// One code as the view draws it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphMeasure {
    /// Advance in 1000-unit text space; None when hayro has none (Type3).
    pub advance: Option<f64>,
    /// The font has no glyph for the code (glyph 0).
    pub missing: bool,
}

struct Capture {
    runs: Vec<Vec<GlyphMeasure>>,
}

impl<'a> Device<'a> for Capture {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, _: DrawProps<'a>, _: &DrawMode) {
        let glyphs = run
            .glyphs()
            .iter()
            .map(|glyph| match &**glyph {
                Glyph::Outline(outline) => GlyphMeasure {
                    advance: outline.advance_width().map(f64::from),
                    missing: outline.glyph_id().to_u32() == 0,
                },
                Glyph::Type3(_) => GlyphMeasure {
                    advance: None,
                    missing: false,
                },
            })
            .collect();
        self.runs.push(glyphs);
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// A name as PDF source: `/` and `#xx` for anything not regular.
fn name_source(name: &[u8]) -> Vec<u8> {
    let mut out = vec![b'/'];
    for &b in name {
        let regular = b > 0x20
            && b < 0x7f
            && !matches!(
                b,
                b'#' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
            );
        if regular {
            out.push(b);
        } else {
            out.extend_from_slice(format!("#{b:02x}").as_bytes());
        }
    }
    out
}

impl ViewDocument {
    /// MuPDF's page transform (`page.getTransform()`): user space to the
    /// view's y-down page space.
    pub fn page_transform(&self, page: usize) -> Result<[f64; 6], ViewError> {
        Ok(PageFrame::new(self.page(page)?).ctm.as_coeffs())
    }

    /// A stream object's decoded bytes as the view reads them (hayro, which
    /// reads damage a strict decoder refuses); None when it is not a stream
    /// or cannot be decoded.
    pub fn decoded_stream(&self, number: u32, generation: u16) -> Option<Vec<u8>> {
        let id = ObjectIdentifier {
            obj_number: i32::try_from(number).ok()?,
            gen_number: i32::from(generation),
        };
        let stream = self.pdf.xref().get::<Stream<'_>>(id)?;
        stream.decoded().ok().map(|bytes| bytes.into_owned())
    }

    /// A measurer for `page`: its font loads are shared by every call.
    pub fn measurer(&self, page: usize) -> Result<Measurer<'_>, ViewError> {
        Ok(Measurer {
            page: self.page(page)?,
            cache: InterpreterCache::default(),
            font_sizes: &self.font_sizes,
            fonts: RefCell::new(FontBudget::default()),
        })
    }
}

/// Measures codes with a page's fonts (`ViewDocument::measurer`).
pub struct Measurer<'v> {
    page: &'v Page<'v>,
    cache: InterpreterCache<'v>,
    font_sizes: &'v FontSizes,
    /// The fonts `cache` keeps, held to `walk::MAX_FONT_BYTES`.
    fonts: RefCell<FontBudget>,
}

impl Measurer<'_> {
    /// Each of `strings` (codes, in the font's own encoding) shown at size 1
    /// with the font `font` of the page's resources, or of the Form reached
    /// from them through the XObject names `forms`. One entry per code; None
    /// for a string hayro did not draw code for code (an unloadable font).
    pub fn measure(
        &self,
        forms: &[Vec<u8>],
        font: &[u8],
        strings: &[Vec<u8>],
    ) -> Vec<Option<Vec<GlyphMeasure>>> {
        let page = self.page;
        let mut resources = page.resources().clone();
        for name in forms {
            let Some(stream) = resources.x_objects.get::<Stream<'_>>(name.as_slice()) else {
                return vec![None; strings.len()];
            };
            resources = match stream.dict().get(RESOURCES) {
                Some(dict) => Resources::new(dict),
                None => resources,
            };
        }
        // A font past the budget is not loaded, as an unloadable one.
        let fits = resources.fonts.get::<Dict<'_>>(font).is_none_or(|dict| {
            (self.fonts.borrow_mut())
                .select(&dict, self.font_sizes)
                .is_ok()
        });
        if !fits {
            return vec![None; strings.len()];
        }
        let bounds = Rect::new(-1e9, -1e9, 1e9, 1e9);
        let mut out = Vec::with_capacity(strings.len());
        for codes in strings {
            if codes.is_empty() {
                out.push(Some(Vec::new()));
                continue;
            }
            let mut content = b"BT ".to_vec();
            content.extend_from_slice(&name_source(font));
            content.extend_from_slice(b" 1 Tf 0 Tr <");
            for byte in codes {
                content.extend_from_slice(format!("{byte:02x}").as_bytes());
            }
            content.extend_from_slice(b"> Tj ET");
            let settings = super::fonts::settings(false, false);
            let mut ctx =
                Context::new(Affine::IDENTITY, bounds, &self.cache, page.xref(), settings);
            let mut capture = Capture { runs: Vec::new() };
            interpret(TypedIter::new(&content), &resources, &mut ctx, &mut capture);
            let run = capture.runs.into_iter().next();
            out.push(run.filter(|run| run.len() == codes.len()));
        }
        out
    }
}
