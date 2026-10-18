// SPDX-License-Identifier: MIT
//! Path geometry for redact-apply: a painted path read from its operators
//! into subpaths, then cut by mark regions (convex quadrilaterals in the
//! path's own space) so that nothing of it lies inside a region while what
//! it paints outside the regions is unchanged:
//!
//! - a fill or clip keeps every subpath outside a region, and a chain of a
//!   subpath inside one is replaced by a walk along the region's border
//!   from where it entered to where it left (the filled or clipped area
//!   outside the region is the same; inside it is the black box's);
//! - a stroke keeps the parts of its subpaths outside the regions, as open
//!   subpaths cut at the region's border.
//!
//! Curves a region's box touches are flattened first (to `tolerance`);
//! other curves keep their control points. Work is bounded by a budget of
//! segments; past it the caller removes the path whole.
use super::super::annots::Point;

/// One segment from the current point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    Line(Point),
    Curve(Point, Point, Point),
}

impl Segment {
    fn end(&self) -> Point {
        match *self {
            Self::Line(p) | Self::Curve(_, _, p) => p,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Subpath {
    pub start: Point,
    pub segments: Vec<Segment>,
    pub closed: bool,
}

/// What reading a path's operators found.
#[derive(Clone, Debug, PartialEq)]
pub enum Parsed {
    Path(Vec<Subpath>),
    /// Operators other than path construction between its start and its
    /// paint (a `cm` inside a path, say): the geometry is not one space.
    Mixed,
}

/// The convex region a page-space mark rect covers in a path's space:
/// its corners in order around it, counter-clockwise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub corners: [Point; 4],
}

impl Region {
    /// The rect's corners through `to_local`; None when they do not span an
    /// area (a singular map).
    pub fn of(rect: [f64; 4], to_local: &[f64; 6]) -> Option<Self> {
        let apply = |(x, y): Point| {
            (
                x * to_local[0] + y * to_local[2] + to_local[4],
                x * to_local[1] + y * to_local[3] + to_local[5],
            )
        };
        let mut corners = [
            apply((rect[0], rect[1])),
            apply((rect[2], rect[1])),
            apply((rect[2], rect[3])),
            apply((rect[0], rect[3])),
        ];
        let area = signed_area(&corners);
        if !area.is_finite() || area == 0.0 {
            return None;
        }
        if area < 0.0 {
            corners.reverse();
        }
        Some(Self { corners })
    }

    fn bounds(&self) -> [f64; 4] {
        bounds(self.corners.iter().copied())
    }

    fn edge(&self, i: usize) -> (Point, Point) {
        (self.corners[i % 4], self.corners[(i + 1) % 4])
    }

    /// The part of the line from `a` to `b` inside (closed), as parameters
    /// `t_in <= t_out` with the edges they lie on; None when it misses.
    fn clip(&self, a: Point, b: Point) -> Option<(f64, usize, f64, usize)> {
        let (mut t_in, mut e_in, mut t_out, mut e_out) = (0.0f64, usize::MAX, 1.0f64, usize::MAX);
        let d = (b.0 - a.0, b.1 - a.1);
        for i in 0..4 {
            let (v, w) = self.edge(i);
            let e = (w.0 - v.0, w.1 - v.1);
            let num = cross(e, (a.0 - v.0, a.1 - v.1));
            let den = cross(e, d);
            if den == 0.0 {
                if num < 0.0 {
                    return None;
                }
                continue;
            }
            let t = -num / den;
            if den > 0.0 {
                if t > t_in {
                    t_in = t;
                    e_in = i;
                }
            } else if t < t_out {
                t_out = t;
                e_out = i;
            }
        }
        (t_in <= t_out).then_some((t_in, e_in, t_out, e_out))
    }

    /// Whether `p` is inside (closed).
    fn contains(&self, p: Point) -> bool {
        (0..4).all(|i| {
            let (v, w) = self.edge(i);
            cross((w.0 - v.0, w.1 - v.1), (p.0 - v.0, p.1 - v.1)) >= 0.0
        })
    }

    /// The edge `p` lies nearest to.
    fn nearest_edge(&self, p: Point) -> usize {
        let mut best = (f64::INFINITY, 0);
        for i in 0..4 {
            let (v, w) = self.edge(i);
            let e = (w.0 - v.0, w.1 - v.1);
            let len = e.0.hypot(e.1);
            let distance = if len > 0.0 {
                (cross(e, (p.0 - v.0, p.1 - v.1)) / len).abs()
            } else {
                f64::INFINITY
            };
            if distance < best.0 {
                best = (distance, i);
            }
        }
        best.1
    }

    /// The corners passed walking counter-clockwise along the border from
    /// `a` (on edge `ea`) to `b` (on edge `eb`), `b` included.
    fn walk(&self, a: Point, ea: usize, b: Point, eb: usize) -> Vec<Point> {
        let ea = if ea < 4 { ea } else { self.nearest_edge(a) };
        let eb = if eb < 4 { eb } else { self.nearest_edge(b) };
        let mut out = Vec::new();
        if (a.0 - b.0).abs() <= f64::EPSILON * a.0.abs().max(1.0)
            && (a.1 - b.1).abs() <= f64::EPSILON * a.1.abs().max(1.0)
        {
            return out;
        }
        let along = |p: Point, i: usize| {
            let (v, w) = self.edge(i);
            (p.0 - v.0) * (w.0 - v.0) + (p.1 - v.1) * (w.1 - v.1)
        };
        if ea == eb && along(b, eb) >= along(a, ea) {
            out.push(b);
            return out;
        }
        let mut i = ea;
        for _ in 0..4 {
            i = (i + 1) % 4;
            out.push(self.corners[i]);
            if i == eb {
                break;
            }
        }
        out.push(b);
        out
    }
}

fn cross(a: Point, b: Point) -> f64 {
    a.0 * b.1 - a.1 * b.0
}

fn signed_area(points: &[Point; 4]) -> f64 {
    let mut sum = 0.0;
    for i in 0..4 {
        let (a, b) = (points[i], points[(i + 1) % 4]);
        sum += cross(a, b);
    }
    sum / 2.0
}

pub fn bounds(points: impl Iterator<Item = Point>) -> [f64; 4] {
    let mut r = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for (x, y) in points {
        r = [r[0].min(x), r[1].min(y), r[2].max(x), r[3].max(y)];
    }
    r
}

fn overlaps(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] <= b[2] && a[2] >= b[0] && a[1] <= b[3] && a[3] >= b[1]
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// Segments a cut may examine in all; past it the cut gives up.
pub struct Budget(pub usize);

impl Budget {
    fn spend(&mut self, n: usize) -> Option<()> {
        self.0 = self.0.checked_sub(n)?;
        Some(())
    }
}

/// Line segments a curve is flattened into at most.
const MAX_CURVE_STEPS: usize = 256;

/// A cubic's points at `steps` even parameters after its start.
fn flatten(p0: Point, c1: Point, c2: Point, p3: Point, tolerance: f64) -> Vec<Point> {
    let dd = |a: Point, b: Point, c: Point| {
        let x = a.0 - 2.0 * b.0 + c.0;
        let y = a.1 - 2.0 * b.1 + c.1;
        x.hypot(y)
    };
    let d = dd(p0, c1, c2).max(dd(c1, c2, p3));
    let steps = if tolerance > 0.0 && d.is_finite() {
        ((0.75 * d / tolerance).sqrt().ceil() as usize).clamp(1, MAX_CURVE_STEPS)
    } else {
        MAX_CURVE_STEPS
    };
    (1..=steps)
        .map(|i| {
            let t = i as f64 / steps as f64;
            let u = 1.0 - t;
            let (a, b, c, e) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            (
                a * p0.0 + b * c1.0 + c * c2.0 + e * p3.0,
                a * p0.1 + b * c1.1 + c * c2.1 + e * p3.1,
            )
        })
        .collect()
}

/// A cubic split at `t`: the two halves' control points.
fn split(p0: Point, c1: Point, c2: Point, p3: Point, t: f64) -> ([Point; 4], [Point; 4]) {
    let a = lerp(p0, c1, t);
    let b = lerp(c1, c2, t);
    let c = lerp(c2, p3, t);
    let d = lerp(a, b, t);
    let e = lerp(b, c, t);
    let f = lerp(d, e, t);
    ([p0, a, d, f], [f, e, c, p3])
}

fn cubic_at(q: &[Point; 4], t: f64) -> Point {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (
        a * q[0].0 + b * q[1].0 + c * q[2].0 + d * q[3].0,
        a * q[0].1 + b * q[1].1 + c * q[2].1 + d * q[3].1,
    )
}

/// Samples per curve when looking for where it crosses a region's edges.
const CROSSING_SAMPLES: usize = 64;

/// The parameters (0..1, ascending) where a cubic crosses the lines of the
/// region's edges.
fn crossings(region: &Region, q: &[Point; 4]) -> Vec<f64> {
    let mut out = Vec::new();
    for i in 0..4 {
        let (v, w) = region.edge(i);
        let e = (w.0 - v.0, w.1 - v.1);
        let side = |t: f64| {
            let p = cubic_at(q, t);
            cross(e, (p.0 - v.0, p.1 - v.1))
        };
        let mut t0 = 0.0;
        let mut f0 = side(t0);
        for k in 1..=CROSSING_SAMPLES {
            let t1 = k as f64 / CROSSING_SAMPLES as f64;
            let f1 = side(t1);
            if f1 == 0.0 && k < CROSSING_SAMPLES {
                out.push(t1);
            } else if f0 * f1 < 0.0 {
                let (mut lo, mut hi, mut flo) = (t0, t1, f0);
                for _ in 0..60 {
                    let mid = (lo + hi) / 2.0;
                    let fm = side(mid);
                    if fm * flo <= 0.0 {
                        hi = mid;
                    } else {
                        lo = mid;
                        flo = fm;
                    }
                }
                out.push((lo + hi) / 2.0);
            }
            t0 = t1;
            f0 = f1;
        }
    }
    out.retain(|t| *t > 1e-12 && *t < 1.0 - 1e-12);
    out.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    out.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    out
}

/// Strictly inside, a hair in from the edges.
fn strictly_inside(region: &Region, p: Point) -> bool {
    (0..4).all(|i| {
        let (v, w) = region.edge(i);
        let e = (w.0 - v.0, w.1 - v.1);
        let len = e.0.hypot(e.1);
        len > 0.0 && cross(e, (p.0 - v.0, p.1 - v.1)) / len > 1e-9
    })
}

/// The subpath's segments, the closing one added when `close`, each curve
/// the region's box touches split where it crosses the region's edges: a
/// part inside stays a curve (it goes), a part outside stays a curve unless
/// one of its control points lies inside, when it is flattened.
fn prepare(
    subpath: &Subpath,
    region: &Region,
    close: bool,
    tolerance: f64,
    budget: &mut Budget,
) -> Option<Vec<(Point, Segment)>> {
    let area = region.bounds();
    let mut out = Vec::new();
    let mut at = subpath.start;
    for segment in &subpath.segments {
        match *segment {
            Segment::Line(p) => out.push((at, Segment::Line(p))),
            Segment::Curve(c1, c2, p) => {
                let hull = bounds([at, c1, c2, p].into_iter());
                if !overlaps(&hull, &area) {
                    out.push((at, *segment));
                    at = segment.end();
                    continue;
                }
                budget.spend(CROSSING_SAMPLES)?;
                let mut rest = [at, c1, c2, p];
                let mut done = 0.0;
                let mut parts = Vec::new();
                for t in crossings(region, &[at, c1, c2, p]) {
                    let local = ((t - done) / (1.0 - done)).clamp(0.0, 1.0);
                    let (left, right) = split(rest[0], rest[1], rest[2], rest[3], local);
                    parts.push(left);
                    rest = right;
                    done = t;
                }
                parts.push(rest);
                for q in parts {
                    let inside = region.contains(cubic_at(&q, 0.5));
                    let poking =
                        !inside && (strictly_inside(region, q[1]) || strictly_inside(region, q[2]));
                    if poking {
                        let mut from = q[0];
                        for point in flatten(q[0], q[1], q[2], q[3], tolerance) {
                            out.push((from, Segment::Line(point)));
                            from = point;
                        }
                    } else {
                        out.push((q[0], Segment::Curve(q[1], q[2], q[3])));
                    }
                }
            }
        }
        at = segment.end();
    }
    if close {
        out.push((at, Segment::Line(subpath.start)));
    }
    budget.spend(out.len())?;
    Some(out)
}

/// One piece of a segment: outside the region (kept, with where it starts
/// and the edge it starts on, when it leaves the region there), or inside
/// it (where it starts and the edge).
enum Piece {
    Out(Point, usize, Segment),
    In(Point, usize),
}

/// A segment split by the region, in order.
fn pieces(region: &Region, from: Point, segment: Segment) -> Vec<Piece> {
    let to = match segment {
        Segment::Line(to) => to,
        Segment::Curve(..) => {
            // Split where it crosses: wholly on one side.
            let q = match segment {
                Segment::Curve(a, b, c) => [from, a, b, c],
                Segment::Line(_) => [from; 4],
            };
            return match region.contains(cubic_at(&q, 0.5)) {
                true => vec![Piece::In(from, usize::MAX)],
                false => vec![Piece::Out(from, usize::MAX, segment)],
            };
        }
    };
    let Some((t_in, e_in, t_out, e_out)) = region.clip(from, to) else {
        return vec![Piece::Out(from, usize::MAX, segment)];
    };
    let mut out = Vec::new();
    let a = if t_in <= 0.0 {
        from
    } else {
        lerp(from, to, t_in)
    };
    let b = if t_out >= 1.0 {
        to
    } else {
        lerp(from, to, t_out)
    };
    if t_in > 0.0 {
        out.push(Piece::Out(from, usize::MAX, Segment::Line(a)));
    }
    out.push(Piece::In(a, if t_in > 0.0 { e_in } else { usize::MAX }));
    if t_out < 1.0 {
        out.push(Piece::Out(b, e_out, Segment::Line(to)));
    }
    out
}

/// A filled or clipping subpath (closed) without what lies inside
/// `region`: each chain inside replaced by the region's border from where
/// it entered to where it left. None when it lies wholly inside.
fn fill_cut(
    subpath: &Subpath,
    region: &Region,
    tolerance: f64,
    budget: &mut Budget,
) -> Option<Option<Subpath>> {
    let segments = prepare(subpath, region, true, tolerance, budget)?;
    let mut all: Vec<Piece> = Vec::new();
    for (from, segment) in segments {
        all.extend(pieces(region, from, segment));
    }
    budget.spend(all.len())?;
    let Some(first) = all.iter().position(|p| matches!(p, Piece::Out(..))) else {
        return Some(None);
    };
    if all.iter().all(|p| matches!(p, Piece::Out(..))) {
        return Some(Some(subpath.clone()));
    }
    let n = all.len();
    let Some(&Piece::Out(start, start_edge, _)) = all.get(first) else {
        return Some(None);
    };
    let mut out = Subpath {
        start,
        segments: Vec::new(),
        closed: true,
    };
    // Inside since this point (and edge).
    let mut entered: Option<(Point, usize)> = None;
    for k in 0..n {
        match all.get((first + k) % n) {
            Some(Piece::Out(from, edge, segment)) => {
                if let Some((a, ea)) = entered.take() {
                    for p in region.walk(a, ea, *from, *edge) {
                        out.segments.push(Segment::Line(p));
                    }
                }
                out.segments.push(*segment);
            }
            Some(Piece::In(from, edge)) => {
                entered.get_or_insert((*from, *edge));
            }
            None => {}
        }
    }
    if let Some((a, ea)) = entered {
        for p in region.walk(a, ea, start, start_edge) {
            out.segments.push(Segment::Line(p));
        }
    }
    Some(Some(out))
}

/// A stroked subpath's parts outside `region`, as open subpaths (a closed
/// subpath the region misses stays as it is).
fn stroke_cut(
    subpath: &Subpath,
    region: &Region,
    tolerance: f64,
    budget: &mut Budget,
) -> Option<Vec<Subpath>> {
    let segments = prepare(subpath, region, subpath.closed, tolerance, budget)?;
    let mut all: Vec<Piece> = Vec::new();
    for (from, segment) in segments {
        all.extend(pieces(region, from, segment));
    }
    budget.spend(all.len())?;
    if all.iter().all(|p| matches!(p, Piece::Out(..))) {
        return Some(vec![subpath.clone()]);
    }
    // A closed subpath is read from just after a part inside, so the run
    // around its start stays one piece.
    let n = all.len();
    let first = match subpath.closed {
        true => all
            .iter()
            .position(|p| matches!(p, Piece::In(..)))
            .map_or(0, |i| i + 1),
        false => 0,
    };
    let mut out: Vec<Subpath> = Vec::new();
    let mut current: Option<Subpath> = None;
    for k in 0..n {
        match all.get((first + k) % n) {
            Some(Piece::Out(from, _, segment)) => {
                let run = current.get_or_insert_with(|| Subpath {
                    start: *from,
                    segments: Vec::new(),
                    closed: false,
                });
                run.segments.push(*segment);
            }
            Some(Piece::In(..)) => out.extend(current.take()),
            None => {}
        }
    }
    out.extend(current);
    Some(out)
}

/// How a path is painted, for the cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Use {
    /// Filled or used as a clip: the area outside the regions is kept.
    Area,
    /// Stroked: the lines outside the regions are kept.
    Lines,
}

/// The subpaths cut by every region in turn; None past the budget.
pub fn cut(
    subpaths: &[Subpath],
    regions: &[Region],
    usage: Use,
    tolerance: f64,
    budget: &mut Budget,
) -> Option<Vec<Subpath>> {
    let mut current: Vec<Subpath> = subpaths.to_vec();
    for region in regions {
        let mut next = Vec::with_capacity(current.len());
        for subpath in &current {
            let reach = bounds(std::iter::once(subpath.start).chain(
                subpath.segments.iter().flat_map(|s| match *s {
                    Segment::Line(p) => vec![p],
                    Segment::Curve(a, b, c) => vec![a, b, c],
                }),
            ));
            budget.spend(1)?;
            if !overlaps(&reach, &region.bounds()) {
                next.push(subpath.clone());
                continue;
            }
            match usage {
                Use::Area => next.extend(fill_cut(subpath, region, tolerance, budget)?),
                Use::Lines => next.extend(stroke_cut(subpath, region, tolerance, budget)?),
            }
        }
        current = next;
    }
    Some(current)
}

/// Reads path construction operations (operator, operands); `close_last`
/// closes the last subpath (`s`, `b`, `b*`).
pub fn parse(ops: &[(Vec<u8>, Vec<f64>)], close_last: bool) -> Parsed {
    let mut subpaths: Vec<Subpath> = Vec::new();
    let mut current: Option<Subpath> = None;
    // Where a segment after `h` starts its new subpath.
    let mut reopen: Option<Point> = None;
    let mut at: Option<Point> = None;
    let push = |subpaths: &mut Vec<Subpath>, current: &mut Option<Subpath>| {
        if let Some(subpath) = current.take() {
            subpaths.push(subpath);
        }
    };
    for (op, n) in ops {
        let v = |i: usize| n.get(i).copied().filter(|v| v.is_finite());
        let pair = |i: usize| Some((v(i)?, v(i + 1)?));
        // A segment without a subpath starts one at the current point (after
        // `h`, the closed subpath's start), else at its own end.
        let mut begin = |current: &mut Option<Subpath>, p: Point| {
            if current.is_none() {
                let start = reopen.take().or(at).unwrap_or(p);
                *current = Some(Subpath {
                    start,
                    segments: Vec::new(),
                    closed: false,
                });
            }
        };
        match op.as_slice() {
            b"m" => {
                let Some(p) = pair(0) else { continue };
                push(&mut subpaths, &mut current);
                reopen = None;
                current = Some(Subpath {
                    start: p,
                    segments: Vec::new(),
                    closed: false,
                });
                at = Some(p);
            }
            b"l" => {
                let Some(p) = pair(0) else { continue };
                begin(&mut current, p);
                if let Some(c) = current.as_mut() {
                    c.segments.push(Segment::Line(p));
                }
                at = Some(p);
            }
            b"c" | b"v" | b"y" => {
                let points = match op.as_slice() {
                    b"c" => (|| Some((pair(0)?, pair(2)?, pair(4)?)))(),
                    b"v" => (|| {
                        let (c2, p) = (pair(0)?, pair(2)?);
                        Some((at.unwrap_or(c2), c2, p))
                    })(),
                    _ => (|| {
                        let (c1, p) = (pair(0)?, pair(2)?);
                        Some((c1, p, p))
                    })(),
                };
                let Some((c1, c2, p)) = points else { continue };
                begin(&mut current, p);
                if let Some(c) = current.as_mut() {
                    c.segments.push(Segment::Curve(c1, c2, p));
                }
                at = Some(p);
            }
            b"re" => {
                let (Some(x), Some(y), Some(w), Some(h)) = (v(0), v(1), v(2), v(3)) else {
                    continue;
                };
                push(&mut subpaths, &mut current);
                subpaths.push(Subpath {
                    start: (x, y),
                    segments: vec![
                        Segment::Line((x + w, y)),
                        Segment::Line((x + w, y + h)),
                        Segment::Line((x, y + h)),
                    ],
                    closed: true,
                });
                reopen = Some((x, y));
                at = Some((x, y));
            }
            b"h" => {
                if let Some(mut c) = current.take() {
                    c.closed = true;
                    reopen = Some(c.start);
                    at = Some(c.start);
                    subpaths.push(c);
                }
            }
            b"W" | b"W*" => {}
            _ => return Parsed::Mixed,
        }
    }
    push(&mut subpaths, &mut current);
    if close_last {
        if let Some(last) = subpaths.last_mut() {
            last.closed = true;
        }
    }
    Parsed::Path(subpaths)
}

/// PDF numbers: at most six decimals, no exponent, no negative zero.
pub fn number(value: f64) -> String {
    if !value.is_finite() {
        return "0".into();
    }
    let mut text = format!("{value:.6}");
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    if text == "-0" || text.is_empty() {
        text = "0".into();
    }
    text
}

/// The subpaths as construction operators.
pub fn write(subpaths: &[Subpath], out: &mut Vec<u8>) {
    let point = |out: &mut Vec<u8>, (x, y): Point| {
        out.extend_from_slice(number(x).as_bytes());
        out.push(b' ');
        out.extend_from_slice(number(y).as_bytes());
        out.push(b' ');
    };
    for subpath in subpaths {
        point(out, subpath.start);
        out.extend_from_slice(b"m\n");
        for segment in &subpath.segments {
            match *segment {
                Segment::Line(p) => {
                    point(out, p);
                    out.extend_from_slice(b"l\n");
                }
                Segment::Curve(a, b, c) => {
                    point(out, a);
                    point(out, b);
                    point(out, c);
                    out.extend_from_slice(b"c\n");
                }
            }
        }
        if subpath.closed {
            out.extend_from_slice(b"h\n");
        }
    }
}

/// Every coordinate of the subpaths is finite and within `limit`.
pub fn within(subpaths: &[Subpath], limit: f64) -> bool {
    let ok = |(x, y): Point| x.is_finite() && y.is_finite() && x.abs() <= limit && y.abs() <= limit;
    subpaths.iter().all(|s| {
        ok(s.start)
            && s.segments.iter().all(|seg| match *seg {
                Segment::Line(p) => ok(p),
                Segment::Curve(a, b, c) => ok(a) && ok(b) && ok(c),
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Region {
        Region::of([10.0, 10.0, 20.0, 20.0], &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]).unwrap()
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Subpath {
        match parse(&[(b"re".to_vec(), vec![x, y, w, h])], false) {
            Parsed::Path(mut p) => p.remove(0),
            Parsed::Mixed => unreachable!(),
        }
    }

    fn points(s: &Subpath) -> Vec<Point> {
        std::iter::once(s.start)
            .chain(s.segments.iter().map(Segment::end))
            .collect()
    }

    fn inside(region: &Region, p: Point) -> bool {
        let shrunk = [
            region.bounds()[0] + 1e-9,
            region.bounds()[1] + 1e-9,
            region.bounds()[2] - 1e-9,
            region.bounds()[3] - 1e-9,
        ];
        p.0 > shrunk[0] && p.0 < shrunk[2] && p.1 > shrunk[1] && p.1 < shrunk[3]
    }

    #[test]
    fn a_fill_keeps_its_outside_and_loses_its_inside() {
        // A triangle with one vertex at (15, 15), inside the region.
        let tri = Subpath {
            start: (0.0, 0.0),
            segments: vec![Segment::Line((15.0, 15.0)), Segment::Line((30.0, 0.0))],
            closed: true,
        };
        let out = cut(&[tri], &[square()], Use::Area, 0.01, &mut Budget(1000)).unwrap();
        assert_eq!(out.len(), 1);
        let pts = points(&out[0]);
        assert!(pts.iter().all(|p| !inside(&square(), *p)), "{pts:?}");
        assert!(pts.contains(&(10.0, 10.0)) && pts.contains(&(20.0, 10.0)));
    }

    #[test]
    fn a_subpath_inside_goes_and_one_around_stays() {
        let out = cut(
            &[rect(12.0, 12.0, 2.0, 2.0), rect(0.0, 0.0, 30.0, 30.0)],
            &[square()],
            Use::Area,
            0.01,
            &mut Budget(1000),
        )
        .unwrap();
        assert_eq!(out, vec![rect(0.0, 0.0, 30.0, 30.0)]);
    }

    #[test]
    fn a_stroke_is_cut_at_the_border() {
        let line = Subpath {
            start: (0.0, 15.0),
            segments: vec![Segment::Line((30.0, 15.0))],
            closed: false,
        };
        let out = cut(&[line], &[square()], Use::Lines, 0.01, &mut Budget(1000)).unwrap();
        assert_eq!(
            out.iter().map(points).collect::<Vec<_>>(),
            vec![
                vec![(0.0, 15.0), (10.0, 15.0)],
                vec![(20.0, 15.0), (30.0, 15.0)]
            ]
        );
    }

    #[test]
    fn a_curve_through_a_region_is_flattened_and_cut() {
        let curve = Subpath {
            start: (0.0, 0.0),
            segments: vec![Segment::Curve((10.0, 40.0), (20.0, -10.0), (30.0, 30.0))],
            closed: false,
        };
        let out = cut(&[curve], &[square()], Use::Lines, 0.01, &mut Budget(10_000)).unwrap();
        for s in &out {
            for p in points(s) {
                assert!(!inside(&square(), p), "{p:?}");
            }
        }
        // A curve far away keeps its control points.
        let far = Subpath {
            start: (100.0, 100.0),
            segments: vec![Segment::Curve(
                (110.0, 140.0),
                (120.0, 90.0),
                (130.0, 130.0),
            )],
            closed: false,
        };
        let out = cut(
            std::slice::from_ref(&far),
            &[square()],
            Use::Lines,
            0.01,
            &mut Budget(10),
        )
        .unwrap();
        assert_eq!(out, vec![far]);
    }

    #[test]
    fn numbers_have_no_exponent() {
        assert_eq!(number(1e-9), "0");
        assert_eq!(number(-0.0000001), "0");
        assert_eq!(number(12.5), "12.5");
        assert_eq!(number(1e20), "100000000000000000000");
        assert_eq!(number(f64::NAN), "0");
    }

    #[test]
    fn mixed_operators_are_not_one_path() {
        let ops = vec![
            (b"m".to_vec(), vec![0.0, 0.0]),
            (b"cm".to_vec(), vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        ];
        assert_eq!(parse(&ops, false), Parsed::Mixed);
    }
}
