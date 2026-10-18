use crate::Renderer;
use hayro_interpret::Paint;
use hayro_interpret::encode::{EncodedShadingType, texture_dimensions};
use hayro_interpret::gradient::SvgGradientKind;
use hayro_interpret::pattern::{Pattern, TilingPattern};
use hayro_interpret::util::x_y_advances;
use hayro_interpret::CacheKey;
use kurbo::{Affine, BezPath, Rect, Shape};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use vello_cpu::color::palette::css::TRANSPARENT;
use vello_cpu::color::{AlphaColor, DynamicColor, Srgb};
use vello_cpu::peniko::{ColorStop, Gradient, ImageQuality, ImageSampler};
use vello_cpu::{Image, ImageSource, PaintType, PixelMetadata, Pixmap, peniko};

impl Renderer<'_> {
    #[must_use]
    pub(super) fn set_paint(
        &mut self,
        paint: &Paint<'_>,
        path_bbox: impl Fn() -> Rect,
        is_stroke: bool,
    ) -> Option<BezPath> {
        let mut paint_transform = Affine::IDENTITY;
        let mut clip_path = None;

        let paint: PaintType = match paint.clone() {
            Paint::Color(c) => {
                let c = c.to_rgba().to_rgba8();
                AlphaColor::from_rgba8(c[0], c[1], c[2], c[3]).into()
            }
            Paint::Pattern(p) => {
                let path_transform = self.ctx.transform();

                match *p {
                    Pattern::Shading(s) => {
                        const NATIVE_GRADIENT_TOLERANCE: f32 = 0.01;

                        clip_path = s.shading.clip_path.clone();
                        let mut bbox = (*path_transform * path_bbox().to_path(0.0)).bounding_box();

                        if is_stroke {
                            // Try to account for stroke in bbox.
                            let (a1, a2) = x_y_advances(path_transform);
                            let factor = a1.length().max(a2.length()) * self.ctx.stroke().width;
                            bbox = bbox.inflate(factor, factor);
                        }

                        bbox = bbox.intersect(Rect::new(
                            0.0,
                            0.0,
                            self.ctx.width() as f64,
                            self.ctx.height() as f64,
                        ));
                        // Tandem patch: meshes sampled within the area drawn.
                        let encoded = s.encode_within(Some(bbox));

                        if let EncodedShadingType::RadialAxial(gradient) = &encoded.shading_type
                            && let Some(native) =
                                gradient.as_svg_gradient(&encoded, bbox, NATIVE_GRADIENT_TOLERANCE)
                        {
                            paint_transform = path_transform.inverse()
                                * Affine::translate((-0.5, -0.5))
                                * native.transform;

                            let stops = native
                                .stops
                                .iter()
                                .map(|stop| ColorStop {
                                    offset: stop.offset,
                                    color: DynamicColor::from_alpha_color(AlphaColor::<Srgb>::new(
                                        stop.color,
                                    )),
                                })
                                .collect::<Vec<_>>();

                            let gradient = match native.kind {
                                SvgGradientKind::Linear { start, end } => {
                                    Gradient::new_linear(start, end)
                                }
                                SvgGradientKind::Radial {
                                    start_center,
                                    start_radius,
                                    end_center,
                                    end_radius,
                                } => Gradient::new_two_point_radial(
                                    start_center,
                                    start_radius,
                                    end_center,
                                    end_radius,
                                ),
                            }
                            .with_extend(peniko::Extend::Pad)
                            .with_stops(stops.as_slice());

                            PaintType::Gradient(gradient)
                        } else {
                            // Tandem patch: the texture counts against the
                            // render's pixmap budget (sampled more coarsely to
                            // fit it, or left out once it is spent), and paints
                            // of one shading defined by its dictionary (not a
                            // mesh) over one area share it.
                            let by_dict =
                                !matches!(encoded.shading_type, EncodedShadingType::Sampled(_));
                            let shared = (by_dict && s.transfer_function.is_none()).then(|| {
                                let mut hasher = rustc_hash::FxHasher::default();
                                (s.cache_key(), s.opacity.to_bits()).hash(&mut hasher);
                                [bbox.x0, bbox.y0, bbox.x1, bbox.y1]
                                    .map(f64::to_bits)
                                    .hash(&mut hasher);
                                (u128::from(hasher.finish()), 0, 0, 4)
                            });
                            let hit = shared.and_then(|key| self.global.shared.borrow().get(&key).cloned());
                            let (pixmap, transform) = if let Some((pixmap, _, transform)) = hit {
                                (pixmap, transform)
                            } else {
                                let (width, height) = texture_dimensions(bbox, 1.0);
                                let Some(scale) = self
                                    .global
                                    .pixmap_fit(f64::from(width), f64::from(height))
                                else {
                                    self.ctx.set_paint(TRANSPARENT);
                                    return clip_path;
                                };
                                let scale = scale as f32;
                                let (width, height) = texture_dimensions(bbox, scale);
                                let mut image = Vec::with_capacity(width as usize * height as usize);
                                let mut may_have_transparency = false;
                                let (width, height, transform) =
                                    encoded.sample_texture(bbox, scale, |sample| {
                                        let pixel = AlphaColor::<Srgb>::new(sample)
                                            .premultiply()
                                            .to_rgba8();
                                        may_have_transparency |= pixel.a != 255;
                                        image.push(pixel);
                                    });

                                let pixmap = Arc::new(Pixmap::from_parts(
                                    bytemuck::cast_vec(image),
                                    width as u16,
                                    height as u16,
                                    PixelMetadata {
                                        may_have_transparency,
                                        ..Default::default()
                                    },
                                ));
                                if let Some(key) = shared {
                                    let entry = (pixmap.clone(), ImageQuality::Medium, transform);
                                    self.global.shared.borrow_mut().insert(key, entry);
                                }
                                (pixmap, transform)
                            };
                            paint_transform = path_transform.inverse() * transform;

                            let image = Image {
                                image: ImageSource::Pixmap(pixmap),
                                sampler: ImageSampler {
                                    x_extend: peniko::Extend::Repeat,
                                    y_extend: peniko::Extend::Repeat,
                                    quality: ImageQuality::Medium,
                                    alpha: 1.0,
                                },
                            };

                            PaintType::Image(image)
                        }
                    }
                    Pattern::Tiling(t) => {
                        const MAX_PIXMAP_SIZE: f32 = 3000.0;
                        // TODO: Raise this limit and perform downsampling if reached
                        // (see pdftc_100k_0138.pdf).
                        const MIN_PIXMAP_SIZE: f32 = 1.0;

                        let bbox = t.bbox;
                        let max_x_scale = MAX_PIXMAP_SIZE / bbox.width() as f32;
                        let min_x_scale = MIN_PIXMAP_SIZE / bbox.width() as f32;
                        let max_y_scale = MAX_PIXMAP_SIZE / bbox.height() as f32;
                        let min_y_scale = MIN_PIXMAP_SIZE / bbox.height() as f32;

                        let (mut xs, mut ys) = {
                            let (x, y) = x_y_advances(&(t.matrix));
                            (x.length() as f32, y.length() as f32)
                        };
                        xs = xs.max(min_x_scale).min(max_x_scale);
                        ys = ys.max(min_y_scale).min(max_y_scale);

                        // Tandem patch: when the steps leave space between
                        // cells and one cell alone meets the area drawn, that
                        // cell is drawn at its own size instead of a step-sized
                        // pixmap; a repeated step pixmap stays within
                        // MAX_TILE_PIXELS, MAX_TILE_EDGE and the render's
                        // pixmap budget.
                        if t.x_step != 0.0
                            && t.y_step != 0.0
                            && (f64::from(t.x_step.abs()) > bbox.width()
                                || f64::from(t.y_step.abs()) > bbox.height())
                        {
                            let mut region =
                                (*path_transform * path_bbox().to_path(0.1)).bounding_box();
                            if is_stroke {
                                let (a1, a2) = x_y_advances(path_transform);
                                let factor = a1.length().max(a2.length()) * self.ctx.stroke().width;
                                region = region.inflate(factor, factor);
                            }
                            let region = region.inflate(1.0, 1.0).intersect(Rect::new(
                                0.0,
                                0.0,
                                self.ctx.width() as f64,
                                self.ctx.height() as f64,
                            ));
                            if let Some(cell) = visible_cell(&t, region) {
                                let (paint, transform) =
                                    self.single_cell(&t, cell, xs, ys, path_transform, is_stroke);
                                self.ctx.set_paint_transform(transform);
                                self.ctx.set_paint(paint);
                                return clip_path;
                            }
                        }

                        let fit = (MAX_TILE_PIXELS
                            / (f64::from(xs * t.x_step).abs() * f64::from(ys * t.y_step).abs()))
                        .sqrt()
                        .min(1.0) as f32;
                        if fit.is_finite() && fit > 0.0 {
                            xs *= fit;
                            ys *= fit;
                        }
                        let edge_fit =
                            |s: f32, step: f32| (MAX_TILE_EDGE / (s * step).abs()).min(1.0);
                        xs *= edge_fit(xs, t.x_step);
                        ys *= edge_fit(ys, t.y_step);
                        let Some(budget) = self.global.pixmap_fit(
                            f64::from(tile_edge(xs * t.x_step)),
                            f64::from(tile_edge(ys * t.y_step)),
                        ) else {
                            self.ctx.set_paint(TRANSPARENT);
                            return clip_path;
                        };
                        xs *= budget as f32;
                        ys *= budget as f32;

                        let x_step = xs * t.x_step;
                        let y_step = ys * t.y_step;

                        let pix_width = tile_edge(x_step);
                        let pix_height = tile_edge(y_step);

                        let mut ctx = self.child_context(pix_width, pix_height);
                        let mut renderer = Renderer::new(&mut ctx, self.global);
                        renderer.inside_pattern = true;
                        let initial_transform = Affine::scale_non_uniform(xs as f64, ys as f64)
                            * Affine::translate((-bbox.x0, -bbox.y0));
                        t.interpret(&mut renderer, initial_transform, is_stroke);
                        let mut pix = Pixmap::new(pix_width, pix_height);
                        renderer.ctx.flush();
                        let mut resources = vello_cpu::Resources::default();
                        renderer.ctx.render(&mut pix, &mut resources);

                        // Tandem patch: a negative step places cells as its
                        // magnitude does (reference viewers); hayro mirrored the cell.
                        paint_transform =
                            path_transform.inverse() * t.matrix * initial_transform.inverse();

                        let image = Image {
                            image: ImageSource::Pixmap(Arc::new(pix)),
                            sampler: ImageSampler {
                                x_extend: peniko::Extend::Repeat,
                                y_extend: peniko::Extend::Repeat,
                                quality: ImageQuality::Medium,
                                alpha: 1.0,
                            },
                        };

                        PaintType::Image(image)
                    }
                }
            }
        };

        self.ctx.set_paint_transform(paint_transform);
        self.ctx.set_paint(paint);

        clip_path
    }
}

// Tandem patch: bounds on tiling pattern pixmaps.

/// Pixels of transparent margin around a single cell; the pad extend repeats
/// them outside it.
const CELL_MARGIN: f64 = 2.0;
/// The largest step pixmap of a repeating tiling pattern (64 MiB of RGBA).
const MAX_TILE_PIXELS: f64 = (1u32 << 24) as f64;
/// The longest edge of a step pixmap, well inside vello's u16 coordinates.
const MAX_TILE_EDGE: f32 = 16384.0;

/// A step pixmap edge in pixels.
fn tile_edge(step: f32) -> u16 {
    let px = step.abs().round();
    if px.is_finite() {
        px.clamp(1.0, MAX_TILE_EDGE) as u16
    } else {
        1
    }
}

/// The offset in pattern space of the only cell of `t` that meets the device
/// `region`; `Some(None)` when no cell does, `None` when several may.
fn visible_cell(t: &TilingPattern<'_>, region: Rect) -> Option<Option<(f64, f64)>> {
    if region.width() <= 0.0 || region.height() <= 0.0 {
        return Some(None);
    }
    let inverse = t.matrix.inverse();
    if t.matrix.determinant() == 0.0 || !inverse.is_finite() {
        return None;
    }
    let area = (inverse * region.to_path(0.1)).bounding_box();
    // Cells sit at whole multiples of the steps, whatever their signs.
    let (sx, sy) = (f64::from(t.x_step.abs()), f64::from(t.y_step.abs()));
    // Cells i with b0 + i·step < hi and b1 + i·step > lo.
    let range = |lo: f64, hi: f64, b0: f64, b1: f64, step: f64| {
        (
            ((lo - b1) / step).floor() + 1.0,
            ((hi - b0) / step).ceil() - 1.0,
        )
    };
    let (x0, x1) = range(area.x0, area.x1, t.bbox.x0, t.bbox.x1, sx);
    let (y0, y1) = range(area.y0, area.y1, t.bbox.y0, t.bbox.y1, sy);
    if !(x0.is_finite() && x1.is_finite() && y0.is_finite() && y1.is_finite()) {
        return None;
    }
    if x1 < x0 || y1 < y0 {
        return Some(None);
    }
    if x1 > x0 || y1 > y0 {
        return None;
    }
    Some(Some((x0 * sx, y0 * sy)))
}

impl Renderer<'_> {
    /// One cell of `t` at `cell`, as a padded image of its bounding box.
    fn single_cell(
        &self,
        t: &TilingPattern<'_>,
        cell: Option<(f64, f64)>,
        xs: f32,
        ys: f32,
        path_transform: &Affine,
        is_stroke: bool,
    ) -> (PaintType, Affine) {
        let Some((dx, dy)) = cell else {
            return (TRANSPARENT.into(), Affine::IDENTITY);
        };
        let bbox = t.bbox;
        let (mut xs, mut ys) = (f64::from(xs), f64::from(ys));
        let Some(budget) = self.global.pixmap_fit(
            (bbox.width() * xs).abs() + 2.0 * CELL_MARGIN,
            (bbox.height() * ys).abs() + 2.0 * CELL_MARGIN,
        ) else {
            return (TRANSPARENT.into(), Affine::IDENTITY);
        };
        xs *= budget;
        ys *= budget;
        let width = tile_edge((bbox.width() * xs + 2.0 * CELL_MARGIN).ceil() as f32);
        let height = tile_edge((bbox.height() * ys + 2.0 * CELL_MARGIN).ceil() as f32);
        let content = Affine::translate((CELL_MARGIN, CELL_MARGIN))
            * Affine::scale_non_uniform(xs, ys)
            * Affine::translate((-bbox.x0, -bbox.y0));

        let mut ctx = self.child_context(width, height);
        let mut renderer = Renderer::new(&mut ctx, self.global);
        renderer.inside_pattern = true;
        t.interpret(&mut renderer, content, is_stroke);
        let mut pix = Pixmap::new(width, height);
        renderer.ctx.flush();
        let mut resources = vello_cpu::Resources::default();
        renderer.ctx.render(&mut pix, &mut resources);

        let transform =
            path_transform.inverse() * t.matrix * Affine::translate((dx, dy)) * content.inverse();
        let image = Image {
            image: ImageSource::Pixmap(Arc::new(pix)),
            sampler: ImageSampler {
                x_extend: peniko::Extend::Pad,
                y_extend: peniko::Extend::Pad,
                quality: ImageQuality::Medium,
                alpha: 1.0,
            },
        };
        (PaintType::Image(image), transform)
    }
}
