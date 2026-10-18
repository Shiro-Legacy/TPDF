use crate::context::Context;
use crate::device::Device;
use crate::font::{Glyph, GlyphRun, PositionedGlyph, ShownCode, TextRun, UNITS_PER_EM};
use crate::interpret::state::TextStateFont;
use crate::{DrawMode, FillRule, Paint};
use hayro_syntax::object;
use hayro_syntax::page::Resources;
use kurbo::{Affine, BezPath};

pub(crate) fn show_text_string<'a>(
    ctx: &mut Context<'a>,
    device: &mut impl Device<'a>,
    resources: &Resources<'a>,
    text: &object::String<'_>,
) {
    begin_glyph_run(ctx);
    append_text_string(ctx, resources, text);
    show_glyph_run(ctx, device);
}

pub(crate) fn begin_glyph_run(ctx: &mut Context<'_>) {
    ctx.glyph_scratch.clear();
}

pub(crate) fn append_text_string<'a>(
    ctx: &mut Context<'a>,
    resources: &Resources<'a>,
    text: &object::String<'_>,
) {
    let Some(font) = ctx.get().text_state.font.clone() else {
        warn!("tried to show text without active font");

        return;
    };

    let bytes = text.as_bytes();

    // In case we have a fallback font (which occurs if either no font was set at all
    // in the content stream, or an invalid one), we only want to show the glyphs
    // using Helvetica if the bytes are actually valid ASCII.
    let show_glyphs = matches!(font, TextStateFont::Font(_))
        || (matches!(font, TextStateFont::Fallback(_)) && bytes.is_ascii());

    let mut cur_idx = 0;
    while cur_idx < bytes.len() {
        let (code, adv) = font.read_code(bytes, cur_idx);
        // Tandem patch: the code's bytes, kept with its glyph.
        let code_bytes = bytes.get(cur_idx..cur_idx + adv).unwrap_or_default();
        cur_idx += adv;

        if show_glyphs && ctx.ocg_state.is_visible() {
            let origin_displacement = font.origin_displacement(code);
            let transform = ctx.get().text_state.full_transform()
                * Affine::scale(1.0 / UNITS_PER_EM as f64)
                * Affine::translate(origin_displacement);
            let glyph = font.get_glyph(font.map_code(code), code, ctx, resources);
            let code = ShownCode::new(code_bytes, font.code_advance(code), origin_displacement);

            ctx.glyph_scratch.push(PositionedGlyph {
                glyph,
                transform,
                code,
            });
        }

        ctx.get_mut().text_state.apply_code_advance(code, adv);
    }
}

pub(crate) fn apply_glyph_run_adjustment(ctx: &mut Context<'_>, adjustment: f32) {
    ctx.get_mut().text_state.apply_adjustment(adjustment);
}

pub(crate) fn next_line(ctx: &mut Context<'_>, tx: f64, ty: f64) {
    let new_matrix = ctx.get_mut().text_state.text_line_matrix * Affine::translate((tx, ty));
    ctx.get_mut().text_state.text_line_matrix = new_matrix;
    ctx.get_mut().text_state.text_matrix = new_matrix;
}

pub(crate) fn show_glyph_run<'a>(ctx: &mut Context<'a>, device: &mut impl Device<'a>) {
    if ctx.glyph_scratch.is_empty() {
        return;
    }

    let render_mode = ctx.get().text_state.render_mode;
    let stroke_props = ctx.stroke_props();
    let fill_props = ctx.draw_props(false);
    let stroke_draw_props = ctx.draw_props(true);

    // Tandem patch: the run with its codes and font, once whatever the
    // rendering mode, for text extraction.
    {
        let text_state = &ctx.get().text_state;
        let font = match &text_state.font {
            Some(TextStateFont::Font(font)) => font.dict(),
            _ => None,
        };
        let horizontal = text_state.font.as_ref().is_none_or(|f| f.is_horizontal());
        let fill = match &fill_props.paint {
            Paint::Color(color) => Some(color.to_rgba().to_rgba8()),
            Paint::Pattern(_) => None,
        };
        device.show_text(&TextRun {
            glyphs: &ctx.glyph_scratch,
            font,
            render_mode: render_mode.number(),
            horizontal,
            transform: ctx.get().ctm,
            fill,
            text_object: ctx.text_objects,
        });
    }

    let clip_path = {
        let run = GlyphRun {
            glyphs: &ctx.glyph_scratch,
        };

        let clip_path = if matches!(
            render_mode,
            TextRenderingMode::Clip
                | TextRenderingMode::FillAndClip
                | TextRenderingMode::StrokeAndClip
                | TextRenderingMode::FillAndStrokeAndClip
        ) {
            let mut clip_path = BezPath::new();
            for glyph in run.glyphs() {
                clip_glyph(&mut clip_path, glyph);
            }
            Some(clip_path)
        } else {
            None
        };

        match render_mode {
            TextRenderingMode::Fill => {
                device.draw_glyph_run(&run, fill_props, &DrawMode::Fill(FillRule::NonZero));
            }
            TextRenderingMode::Stroke => {
                device.draw_glyph_run(&run, stroke_draw_props, &DrawMode::Stroke(stroke_props));
            }
            TextRenderingMode::FillStroke => {
                device.draw_glyph_run(&run, fill_props, &DrawMode::Fill(FillRule::NonZero));
                device.draw_glyph_run(&run, stroke_draw_props, &DrawMode::Stroke(stroke_props));
            }
            TextRenderingMode::Invisible => {
                // Still call draw_glyph_run for invisible text, so that it can
                // for example be used for text extraction.
                device.draw_glyph_run(&run, fill_props, &DrawMode::Invisible);
            }
            TextRenderingMode::FillAndClip => {
                device.draw_glyph_run(&run, fill_props, &DrawMode::Fill(FillRule::NonZero));
            }
            TextRenderingMode::StrokeAndClip => {
                device.draw_glyph_run(&run, stroke_draw_props, &DrawMode::Stroke(stroke_props));
            }
            TextRenderingMode::FillAndStrokeAndClip => {
                device.draw_glyph_run(&run, fill_props, &DrawMode::Fill(FillRule::NonZero));
                device.draw_glyph_run(&run, stroke_draw_props, &DrawMode::Stroke(stroke_props));
            }
            TextRenderingMode::Clip => {}
        }

        clip_path
    };

    if let Some(clip_path) = clip_path {
        ctx.get_mut().text_state.clip_paths.extend(clip_path);
    }

    ctx.glyph_scratch.clear();
}

fn clip_glyph(clip_path: &mut BezPath, glyph: &PositionedGlyph<'_>) {
    match &**glyph {
        Glyph::Outline(outline_glyph) => {
            let outline = glyph.transform() * outline_glyph.outline();
            let has_outline = outline.segments().next().is_some();

            if has_outline {
                clip_path.extend(outline);
            }
        }
        Glyph::Type3(_) => {
            warn!("text rendering mode clip not implemented for shape glyphs");
        }
    }
}

impl TextRenderingMode {
    /// Tandem patch: the mode's number (ISO 32000-2 9.3.6).
    pub(crate) fn number(self) -> u8 {
        match self {
            Self::Fill => 0,
            Self::Stroke => 1,
            Self::FillStroke => 2,
            Self::Invisible => 3,
            Self::FillAndClip => 4,
            Self::StrokeAndClip => 5,
            Self::FillAndStrokeAndClip => 6,
            Self::Clip => 7,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) enum TextRenderingMode {
    #[default]
    Fill,
    Stroke,
    FillStroke,
    Invisible,
    FillAndClip,
    StrokeAndClip,
    FillAndStrokeAndClip,
    Clip,
}
