// SPDX-License-Identifier: AGPL-3.0-or-later
//! Document-bound Gray8/RGB8 and Indexed RGB8 assets with metered decoding.
//! Indexed color-key transparency is binary; general soft alpha remains refused.
//! CalRGB samples are converted to sRGB with exact math and Bradford adaptation
//! from the declared white point to D65. MuPDF's LittleCMS path agrees within
//! one level per channel. Admitted spaces are matrix/gamma only, so an image's
//! standard rendering intent cannot change output.
use crate::{
    document::{Document, DocumentError},
    object::{Object, ObjectId},
    stream::{
        Filter, FilterSpec, PredictorParams, StreamError, StreamLimits,
        decode_filter_specs_with_usage,
    },
};
use std::{collections::BTreeMap, fmt, mem::size_of, sync::Arc};
type Dictionary = BTreeMap<Vec<u8>, Object>;
#[derive(Debug)]
pub enum ImageErrorKind {
    Document(DocumentError),
    Decode(StreamError),
    Invalid(&'static str),
    Unsupported(&'static str),
    Limit(&'static str),
    ReferenceCycle(ObjectId),
    Allocation,
    Invariant(&'static str),
}
#[derive(Debug)]
pub struct ImageError {
    pub object: ObjectId,
    /// Originating base image when this failure concerns its SMask dependency.
    pub base_image: Option<ObjectId>,
    pub field: &'static str,
    pub kind: ImageErrorKind,
}
impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(base) = self.base_image {
            write!(f, "image {} {} SMask -> ", base.number(), base.generation())?;
        }
        write!(
            f,
            "image {} {} {}: {:?}",
            self.object.number(),
            self.object.generation(),
            self.field,
            self.kind
        )
    }
}
impl std::error::Error for ImageError {}
macro_rules! ledger {
 ($($field:ident => $cap:ident = $default:expr),* $(,)?)=>{
  #[derive(Clone,Copy,Debug)] pub struct ImageLimits { $(pub $cap:usize,)* pub max_dimension:usize,pub max_image_pixels:usize,pub max_row_bytes:usize,pub max_image_bytes:usize,pub max_filters:usize,pub max_stage_bytes:usize,pub max_decode_bytes:usize }
  #[derive(Clone,Copy,Debug,Default,Eq,PartialEq)] pub struct ImageUsage { $(pub $field:usize,)* }
  impl Default for ImageLimits { fn default()->Self { Self { $($cap:$default,)* max_dimension:16384,max_image_pixels:4194304,max_row_bytes:49152,max_image_bytes:12<<20,max_filters:8,max_stage_bytes:16<<20,max_decode_bytes:64<<20 } } }
  impl ImageLimits {
   fn intersect(self,other:Self)->Self { Self { $($cap:self.$cap.min(other.$cap),)* max_dimension:self.max_dimension.min(other.max_dimension).min(16384),max_image_pixels:self.max_image_pixels.min(other.max_image_pixels).min(4194304),max_row_bytes:self.max_row_bytes.min(other.max_row_bytes).min(49152),max_image_bytes:self.max_image_bytes.min(other.max_image_bytes).min(12<<20),max_filters:self.max_filters.min(other.max_filters).min(8),max_stage_bytes:self.max_stage_bytes.min(other.max_stage_bytes).min(16<<20),max_decode_bytes:self.max_decode_bytes.min(other.max_decode_bytes).min(64<<20) } }
   fn page_ceiling(self,page:Self,used:ImageUsage)->Self { let page=Self { $($cap:used.$field.saturating_add(page.$cap),)* ..page }; self.intersect(page) }
  }
 }
}
ledger! { attempts=>max_attempts=100000,unique_images=>max_unique_images=4096,metadata_items=>max_metadata_items=1000000,reference_hops=>max_reference_hops=1000000,lookup_work=>max_lookup_work=100000000,encoded_bytes=>max_encoded_bytes=256<<20,decoded_work_bytes=>max_decoded_work_bytes=512<<20,decoded_rows=>max_decoded_rows=1000000,decoded_pixels=>max_decoded_pixels=64000000,retained_sample_bytes=>max_retained_sample_bytes=192<<20,allocated_bytes=>max_allocated_bytes=2usize<<30,filter_specs=>max_filter_specs=32768,mask_dependency_attempts=>max_mask_dependency_attempts=100000,mask_certificate_attempts=>max_mask_certificate_attempts=4096,opacity_scan_samples=>max_opacity_scan_samples=64000000 }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageComponents {
    Gray,
    Rgb,
    IndexedRgb,
}
impl ImageComponents {
    pub fn count(self) -> usize {
        match self {
            Self::Gray | Self::IndexedRgb => 1,
            Self::Rgb => 3,
        }
    }
}
#[derive(Debug)]
pub struct ImageAsset {
    object: ObjectId,
    width: usize,
    height: usize,
    row_bytes: usize,
    components: ImageComponents,
    inverted: [bool; 3],
    samples: Vec<u8>,
    filters: Vec<FilterSpec>,
    opaque_mask: Option<Arc<ImageAsset>>,
    palette: Vec<u8>,
    color_key: Option<[u8; 2]>,
    calibrated: Option<Box<CalRgb>>,
}
/// Per-channel linearization table and CalRGB-to-linear-sRGB matrix.
#[derive(Debug)]
struct CalRgb {
    linear: [[f64; 256]; 3],
    matrix: [[f64; 3]; 3],
}
impl CalRgb {
    // ponytail: three powf per sampled texel; add an encode LUT if large CalRGB images get slow.
    fn convert(&self, rgb: [u8; 3]) -> [u8; 3] {
        let linear = [0, 1, 2].map(|i| self.linear[i][usize::from(rgb[i])]);
        self.matrix.map(|row| {
            let c = (row[0] * linear[0] + row[1] * linear[1] + row[2] * linear[2]).clamp(0., 1.);
            let encoded = if c <= 0.0031308 {
                12.92 * c
            } else {
                1.055 * c.powf(1. / 2.4) - 0.055
            };
            (encoded * 255.).round() as u8
        })
    }
}
fn mat_mul(a: [[f64; 3]; 3], b: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    [0, 1, 2].map(|i| [0, 1, 2].map(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}
fn mat_vec(a: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    a.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2])
}
const BRADFORD: [[f64; 3]; 3] = [
    [0.8951, 0.2664, -0.1614],
    [-0.7502, 1.7135, 0.0367],
    [0.0389, -0.0685, 1.0296],
];
const BRADFORD_INVERSE: [[f64; 3]; 3] = [
    [0.9869929, -0.1470543, 0.1599627],
    [0.4323053, 0.5183603, 0.0492912],
    [-0.0085287, 0.0400428, 0.9684867],
];
const XYZ_TO_SRGB: [[f64; 3]; 3] = [
    [3.2404542, -1.5371385, -0.4985314],
    [-0.9692660, 1.8760108, 0.0415560],
    [0.0556434, -0.2040259, 1.0572252],
];
const D65: [f64; 3] = [0.95047, 1., 1.08883];
impl CalRgb {
    fn new(white: [f64; 3], gamma: [f64; 3], matrix: [f64; 9]) -> Self {
        let source = mat_vec(BRADFORD, white);
        let target = mat_vec(BRADFORD, D65);
        let scale =
            [0, 1, 2].map(|i| [0, 1, 2].map(|j| if i == j { target[i] / source[i] } else { 0. }));
        let adapt = mat_mul(BRADFORD_INVERSE, mat_mul(scale, BRADFORD));
        // PDF lists the X, Y and Z contributions of A, then B, then C.
        let to_xyz = [0, 1, 2].map(|i| [matrix[i], matrix[3 + i], matrix[6 + i]]);
        let matrix = mat_mul(XYZ_TO_SRGB, mat_mul(adapt, to_xyz));
        let linear = [0, 1, 2].map(|c| {
            let mut table = [0.; 256];
            for (v, entry) in table.iter_mut().enumerate() {
                *entry = (v as f64 / 255.).powf(gamma[c]);
            }
            table
        });
        Self { linear, matrix }
    }
}
impl ImageAsset {
    /// Complete, owned Gray8 dependency whose effective samples are all 255.
    pub fn opaque_mask(&self) -> Option<&Arc<ImageAsset>> {
        self.opaque_mask.as_ref()
    }
    pub fn object_id(&self) -> ObjectId {
        self.object
    }
    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }
    pub fn row_bytes(&self) -> usize {
        self.row_bytes
    }
    pub fn components(&self) -> ImageComponents {
        self.components
    }
    pub fn samples(&self) -> &[u8] {
        &self.samples
    }
    pub fn decode_inverted(&self) -> &[bool] {
        &self.inverted[..self.components.count()]
    }
    pub fn filters(&self) -> &[FilterSpec] {
        &self.filters
    }
    pub fn pixel(&self, row: usize, column: usize) -> Option<[u8; 3]> {
        if row >= self.height || column >= self.width {
            return None;
        }
        let offset = row
            .checked_mul(self.row_bytes)?
            .checked_add(column.checked_mul(self.components.count())?)?;
        let channel = |i: usize| {
            let value = *self.samples.get(offset + i)?;
            Some(if self.inverted[i] { 255 - value } else { value })
        };
        Some(match self.components {
            ImageComponents::Gray => [channel(0)?; 3],
            ImageComponents::Rgb => {
                let rgb = [channel(0)?, channel(1)?, channel(2)?];
                match &self.calibrated {
                    Some(cal) => cal.convert(rgb),
                    None => rgb,
                }
            }
            ImageComponents::IndexedRgb => {
                let index =
                    usize::from(*self.samples.get(offset)?).min(self.palette.len() / 3 - 1) * 3;
                [
                    self.palette[index],
                    self.palette[index + 1],
                    self.palette[index + 2],
                ]
            }
        })
    }
    /// Color-key masking compares encoded samples before palette clipping.
    /// Only binary transparency is admitted; soft alpha still needs its own gate.
    pub fn pixel_rgba(&self, row: usize, column: usize) -> Option<[u8; 4]> {
        let [r, g, b] = self.pixel(row, column)?;
        let transparent = self.color_key.is_some_and(|[lo, hi]| {
            let value = self.samples[row * self.row_bytes + column];
            value >= lo && value <= hi
        });
        Some([r, g, b, if transparent { 0 } else { 255 }])
    }
}
pub struct ImageLoader<'d, 'i> {
    document: &'d Document<'i>,
    limits: ImageLimits,
    active: ImageLimits,
    usage: ImageUsage,
    cache: Vec<Arc<ImageAsset>>,
    opaque_certificates: Vec<ObjectId>,
}
impl<'d, 'i> ImageLoader<'d, 'i> {
    fn cal_rgb<'a>(&mut self, id: ObjectId, values: &'a [Object]) -> Result<CalRgb, ImageError>
    where
        'd: 'a,
    {
        let invalid = |what| Self::error(id, "ColorSpace", ImageErrorKind::Invalid(what));
        if values.len() != 2 {
            return Err(invalid("CalRGB array length"));
        }
        let Object::Dictionary(d) = self.resolve(id, &values[1], "ColorSpace")? else {
            return Err(invalid("CalRGB dictionary required"));
        };
        let mut numbers = |key: &'static [u8], default: &[f64]| -> Result<Vec<f64>, ImageError> {
            let Some(value) = self.get(id, d, key, "ColorSpace")? else {
                return Ok(default.to_vec());
            };
            let Object::Array(items) = value else {
                return Err(invalid("CalRGB number array required"));
            };
            if items.len() != default.len() {
                return Err(invalid("CalRGB array length"));
            }
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                self.item(id)?;
                out.push(match self.resolve(id, item, "ColorSpace")? {
                    Object::Integer(n) => *n as f64,
                    Object::Real(n) if n.is_finite() => *n,
                    _ => return Err(invalid("CalRGB number required")),
                });
            }
            Ok(out)
        };
        let white = numbers(b"WhitePoint", &[f64::NAN; 3])?;
        let black = numbers(b"BlackPoint", &[0.; 3])?;
        let gamma = numbers(b"Gamma", &[1.; 3])?;
        let matrix = numbers(b"Matrix", &[1., 0., 0., 0., 1., 0., 0., 0., 1.])?;
        if white[1] != 1. || white[0] <= 0. || white[2] <= 0. {
            return Err(invalid("CalRGB WhitePoint requires Y=1 and positive X/Z"));
        }
        if black.iter().any(|&v| v != 0.) {
            return Err(Self::error(
                id,
                "ColorSpace",
                ImageErrorKind::Unsupported("nonzero CalRGB BlackPoint"),
            ));
        }
        if gamma.iter().any(|&g| g <= 0. || g > 16.) {
            return Err(invalid("CalRGB Gamma must be in (0, 16]"));
        }
        Ok(CalRgb::new(
            [white[0], white[1], white[2]],
            [gamma[0], gamma[1], gamma[2]],
            matrix.try_into().expect("checked length"),
        ))
    }
    fn indexed_palette<'a>(
        &mut self,
        id: ObjectId,
        values: &'a [Object],
    ) -> Result<Vec<u8>, ImageError>
    where
        'd: 'a,
    {
        let invalid = || {
            Self::error(
                id,
                "ColorSpace",
                ImageErrorKind::Unsupported("Indexed DeviceRGB with 8-bit lookup required"),
            )
        };
        if values.len() != 4
            || !matches!(self.resolve(id,&values[0],"ColorSpace")?,Object::Name(n) if n==b"Indexed")
            || !matches!(self.resolve(id,&values[1],"ColorSpace")?,Object::Name(n) if n==b"DeviceRGB")
        {
            return Err(invalid());
        }
        let high = match self.resolve(id, &values[2], "ColorSpace")? {
            Object::Integer(n) if (0..=255).contains(n) => *n as usize,
            _ => return Err(invalid()),
        };
        let expected = (high + 1) * 3;
        let mut value = &values[3];
        let mut seen = [None; 64];
        let mut hops = 0;
        let bytes = loop {
            self.item(id)?;
            match value {
                Object::String(bytes) => break bytes.as_slice(),
                Object::Reference(next) => {
                    self.work(id, hops + 1)?;
                    if hops == seen.len() || seen[..hops].contains(&Some(*next)) {
                        return Err(invalid());
                    }
                    seen[hops] = Some(*next);
                    hops += 1;
                    charge(
                        &mut self.usage.reference_hops,
                        1,
                        self.active.max_reference_hops,
                        id,
                        "palette reference hops",
                    )?;
                    self.document_lookup(id)?;
                    let object = self
                        .document
                        .object(*next)
                        .map_err(|e| Self::error(id, "palette", ImageErrorKind::Document(e)))?;
                    if object.stream.is_some() {
                        let Object::Dictionary(dict) = &object.value else {
                            return Err(invalid());
                        };
                        // Initial stream lookup subset is deliberately unfiltered;
                        // arbitrary filter chains need separate metered decoding.
                        for key in [
                            b"Filter".as_slice(),
                            b"DecodeParms",
                            b"F",
                            b"FFilter",
                            b"FDecodeParms",
                        ] {
                            if self.get(id, dict, key, "palette")?.is_some() {
                                return Err(invalid());
                            }
                        }
                        let bytes = self
                            .document
                            .stream_data(*next)
                            .map_err(|e| Self::error(id, "palette", ImageErrorKind::Document(e)))?
                            .ok_or_else(invalid)?;
                        charge(
                            &mut self.usage.encoded_bytes,
                            bytes.len(),
                            self.active.max_encoded_bytes,
                            id,
                            "palette bytes",
                        )?;
                        break bytes;
                    }
                    value = &object.value;
                }
                _ => return Err(invalid()),
            }
        };
        if bytes.len() != expected {
            return Err(Self::error(
                id,
                "palette",
                ImageErrorKind::Invalid("lookup byte length"),
            ));
        }
        self.allocate(id, expected)?;
        charge(
            &mut self.usage.retained_sample_bytes,
            expected,
            self.active.max_retained_sample_bytes,
            id,
            "palette retention",
        )?;
        self.work(id, expected)?;
        let mut owned = Vec::new();
        owned
            .try_reserve_exact(expected)
            .map_err(|_| Self::error(id, "palette", ImageErrorKind::Allocation))?;
        owned.extend_from_slice(bytes);
        Ok(owned)
    }
    pub fn new(document: &'d Document<'i>, limits: ImageLimits) -> Self {
        Self {
            document,
            limits,
            active: limits.intersect(limits),
            usage: ImageUsage::default(),
            cache: Vec::new(),
            opaque_certificates: Vec::new(),
        }
    }
    pub fn begin_page(&mut self, limits: ImageLimits) {
        self.active = self.limits.page_ceiling(limits, self.usage);
    }
    pub fn usage(&self) -> ImageUsage {
        self.usage
    }
    fn error(id: ObjectId, field: &'static str, kind: ImageErrorKind) -> ImageError {
        ImageError {
            object: id,
            base_image: None,
            field,
            kind,
        }
    }
    fn work(&mut self, id: ObjectId, amount: usize) -> Result<(), ImageError> {
        charge(
            &mut self.usage.lookup_work,
            amount,
            self.active.max_lookup_work,
            id,
            "lookup work",
        )
    }
    fn document_lookup(&mut self, id: ObjectId) -> Result<(), ImageError> {
        // Two scalar comparisons per ObjectId for every possible index entry.
        let amount = self
            .document
            .object_count()
            .checked_add(1)
            .and_then(|n| n.checked_mul(2))
            .ok_or_else(|| Self::error(id, "lookup work", ImageErrorKind::Limit("lookup work")))?;
        self.work(id, amount)
    }
    fn allocate(&mut self, id: ObjectId, amount: usize) -> Result<(), ImageError> {
        charge(
            &mut self.usage.allocated_bytes,
            amount,
            self.active.max_allocated_bytes,
            id,
            "allocated bytes",
        )
    }
    fn item(&mut self, id: ObjectId) -> Result<(), ImageError> {
        charge(
            &mut self.usage.metadata_items,
            1,
            self.active.max_metadata_items,
            id,
            "metadata items",
        )
    }
    fn resolve<'a>(
        &mut self,
        id: ObjectId,
        mut value: &'a Object,
        field: &'static str,
    ) -> Result<&'a Object, ImageError>
    where
        'd: 'a,
    {
        let mut seen = [None; 64];
        let mut count = 0;
        while let Object::Reference(next) = value {
            self.work(id, count + 1)?;
            if seen[..count].contains(&Some(*next)) {
                return Err(Self::error(
                    id,
                    field,
                    ImageErrorKind::ReferenceCycle(*next),
                ));
            }
            if count == 64 {
                return Err(Self::error(
                    id,
                    field,
                    ImageErrorKind::Limit("reference depth"),
                ));
            }
            charge(
                &mut self.usage.reference_hops,
                1,
                self.active.max_reference_hops,
                id,
                "reference hops",
            )?;
            seen[count] = Some(*next);
            count += 1;
            self.document_lookup(id)?;
            let target = self
                .document
                .object(*next)
                .map_err(|e| Self::error(id, field, ImageErrorKind::Document(e)))?;
            if target.stream.is_some() {
                return Err(Self::error(
                    id,
                    field,
                    ImageErrorKind::Invalid("stream-valued metadata"),
                ));
            }
            value = &target.value;
        }
        Ok(value)
    }
    fn get<'a>(
        &mut self,
        id: ObjectId,
        d: &'a Dictionary,
        key: &'static [u8],
        field: &'static str,
    ) -> Result<Option<&'a Object>, ImageError>
    where
        'd: 'a,
    {
        self.item(id)?;
        self.work(id, mul(id, d.len().saturating_add(1), key.len() + 1)?)?;
        d.get(key)
            .map(|v| self.resolve(id, v, field))
            .transpose()
            .map(|v| v.filter(|v| !matches!(v, Object::Null)))
    }
    fn required<'a>(
        &mut self,
        id: ObjectId,
        d: &'a Dictionary,
        key: &'static [u8],
        field: &'static str,
    ) -> Result<&'a Object, ImageError>
    where
        'd: 'a,
    {
        self.get(id, d, key, field)?.ok_or_else(|| {
            Self::error(id, field, ImageErrorKind::Invalid("missing required value"))
        })
    }
    fn number<'a>(
        &mut self,
        id: ObjectId,
        d: &'a Dictionary,
        key: &'static [u8],
        field: &'static str,
    ) -> Result<usize, ImageError>
    where
        'd: 'a,
    {
        match self.required(id, d, key, field)? {
            Object::Integer(n) => usize::try_from(*n).map_err(|_| {
                Self::error(
                    id,
                    field,
                    ImageErrorKind::Invalid("nonnegative integer required"),
                )
            }),
            _ => Err(Self::error(
                id,
                field,
                ImageErrorKind::Invalid("integer required"),
            )),
        }
    }
    fn check_shape(
        &self,
        id: ObjectId,
        width: usize,
        height: usize,
        components: ImageComponents,
    ) -> Result<(usize, usize), ImageError> {
        if width == 0 || height == 0 {
            return Err(Self::error(
                id,
                "dimensions",
                ImageErrorKind::Invalid("positive dimensions required"),
            ));
        }
        let pixels = mul(id, width, height)?;
        let row = mul(id, width, components.count())?;
        let bytes = mul(id, row, height)?;
        for (value, cap, label) in [
            (width, self.active.max_dimension, "width"),
            (height, self.active.max_dimension, "height"),
            (pixels, self.active.max_image_pixels, "image pixels"),
            (row, self.active.max_row_bytes, "row bytes"),
            (bytes, self.active.max_image_bytes, "image bytes"),
        ] {
            if value > cap {
                return Err(Self::error(id, label, ImageErrorKind::Limit(label)));
            }
        }
        Ok((row, bytes))
    }
    // Only this resolver admits stream-valued metadata. It returns the actual
    // stream identity through aliases, with every hop charged before lookup.
    fn soft_mask_edge<'a>(
        &mut self,
        id: ObjectId,
        d: &'a Dictionary,
    ) -> Result<Option<ObjectId>, ImageError>
    where
        'd: 'a,
    {
        self.item(id)?;
        self.work(id, mul(id, d.len().saturating_add(1), 6)?)?;
        let Some(mut value) = d.get(b"SMask".as_slice()) else {
            return Ok(None);
        };
        // Each non-null source edge attempt is charged before resolution,
        // including dangling/cyclic edges and failures in later base decoding.
        if !matches!(value, Object::Null) {
            charge(
                &mut self.usage.mask_dependency_attempts,
                1,
                self.active.max_mask_dependency_attempts,
                id,
                "mask dependency attempts",
            )?;
        }

        let mut seen = [None; 64];
        let mut count = 0;
        while let Object::Reference(next) = value {
            self.work(id, count + 1)?;
            if seen[..count].contains(&Some(*next)) {
                return Err(Self::error(
                    id,
                    "SMask",
                    ImageErrorKind::ReferenceCycle(*next),
                ));
            }
            if count == seen.len() {
                return Err(Self::error(
                    id,
                    "SMask",
                    ImageErrorKind::Limit("reference depth"),
                ));
            }
            charge(
                &mut self.usage.reference_hops,
                1,
                self.active.max_reference_hops,
                id,
                "reference hops",
            )?;
            seen[count] = Some(*next);
            count += 1;
            self.document_lookup(id)?;
            let target = self.document.object(*next).map_err(|e| {
                let mut error = Self::error(*next, "SMask", ImageErrorKind::Document(e));
                error.base_image = Some(id);
                error
            })?;
            if target.stream.is_some() {
                return Ok(Some(*next));
            }
            value = &target.value;
        }
        if matches!(value, Object::Null) {
            Ok(None)
        } else {
            Err(Self::error(
                id,
                "SMask",
                ImageErrorKind::Invalid("indirect mask stream required"),
            ))
        }
    }
    fn mask_role<'a>(
        &mut self,
        id: ObjectId,
        d: &'a Dictionary,
        edge: Option<ObjectId>,
    ) -> Result<(), ImageError>
    where
        'd: 'a,
    {
        if edge.is_some() {
            return Err(Self::error(
                id,
                "SMask",
                ImageErrorKind::Unsupported("nested soft mask"),
            ));
        }
        for (key, field) in [(b"TR".as_slice(), "TR"), (b"SMaskInData", "SMaskInData")] {
            if self.get(id, d, key, field)?.is_some() {
                return Err(Self::error(
                    id,
                    field,
                    ImageErrorKind::Unsupported("mask role semantics"),
                ));
            }
        }
        if !matches!(self.required(id,d,b"ColorSpace","ColorSpace")?, Object::Name(n) if n == b"DeviceGray")
        {
            return Err(Self::error(
                id,
                "ColorSpace",
                ImageErrorKind::Unsupported("Gray8 soft mask required"),
            ));
        }
        Ok(())
    }
    fn admit_mask(
        &mut self,
        base: ObjectId,
        mask: ObjectId,
        width: usize,
        height: usize,
    ) -> Result<Arc<ImageAsset>, ImageError> {
        let result = (|| {
            if base == mask {
                return Err(Self::error(
                    mask,
                    "SMask",
                    ImageErrorKind::ReferenceCycle(mask),
                ));
            }
            // The role flag is an explicit one-edge frame: the internal mask
            // path rejects any further dependency before it can call us again.
            let asset = self.load_role(mask, true)?;
            if (asset.width, asset.height) != (width, height) {
                return Err(Self::error(
                    mask,
                    "dimensions",
                    ImageErrorKind::Unsupported("soft mask dimensions must match base"),
                ));
            }
            self.work(mask, self.opaque_certificates.len() + 1)?;
            if self.opaque_certificates.contains(&mask) {
                return Ok(asset);
            }
            charge(
                &mut self.usage.mask_certificate_attempts,
                1,
                self.active.max_mask_certificate_attempts,
                mask,
                "mask certificate attempts",
            )?;
            // Reserve the complete plane even if the first sample refuses.
            charge(
                &mut self.usage.opacity_scan_samples,
                asset.samples.len(),
                self.active.max_opacity_scan_samples,
                mask,
                "opacity scan samples",
            )?;
            if asset
                .samples
                .iter()
                .any(|&v| if asset.inverted[0] { v != 0 } else { v != 255 })
            {
                return Err(Self::error(
                    mask,
                    "opacity",
                    ImageErrorKind::Unsupported("nonopaque soft mask sample"),
                ));
            }
            if self.opaque_certificates.len() == self.opaque_certificates.capacity() {
                let capacity = self
                    .opaque_certificates
                    .capacity()
                    .saturating_mul(2)
                    .max(1)
                    .min(self.active.max_mask_certificate_attempts);
                if capacity <= self.opaque_certificates.len() {
                    return Err(Self::error(
                        mask,
                        "certificate cache",
                        ImageErrorKind::Limit("mask certificate attempts"),
                    ));
                }
                self.allocate(mask, mul(mask, capacity, size_of::<ObjectId>())?)?;
                self.opaque_certificates
                    .try_reserve_exact(capacity - self.opaque_certificates.len())
                    .map_err(|_| {
                        Self::error(mask, "certificate cache", ImageErrorKind::Allocation)
                    })?;
            }
            self.opaque_certificates.push(mask);
            Ok(asset)
        })();
        result.map_err(|mut e: ImageError| {
            e.base_image = Some(base);
            e
        })
    }
    pub fn load(&mut self, id: ObjectId) -> Result<Arc<ImageAsset>, ImageError> {
        self.load_role(id, false)
    }
    fn load_role(&mut self, id: ObjectId, mask_role: bool) -> Result<Arc<ImageAsset>, ImageError> {
        charge(
            &mut self.usage.attempts,
            1,
            self.active.max_attempts,
            id,
            "attempts",
        )?;
        self.work(id, self.cache.len() + 1)?;
        let cached = self.cache.iter().find(|a| a.object == id).cloned();
        if let Some(asset) = &cached {
            self.check_shape(id, asset.width, asset.height, asset.components)?;
            if asset.filters.len() > self.active.max_filters {
                return Err(Self::error(id, "filters", ImageErrorKind::Limit("filters")));
            }
            // An immutable, admitted ordinary asset with no dependency retains
            // the R23 zero-metadata-work cache contract. Mask roles and masked
            // bases must still authenticate their edge/role on every lookup.
            if !mask_role && asset.opaque_mask.is_none() {
                return Ok(Arc::clone(asset));
            }
        }
        self.document_lookup(id)?;
        let object = self
            .document
            .object(id)
            .map_err(|e| Self::error(id, "stream", ImageErrorKind::Document(e)))?;
        if object.stream.is_none() {
            return Err(Self::error(
                id,
                "stream",
                ImageErrorKind::Invalid("indirect image stream required"),
            ));
        }
        let Object::Dictionary(d) = &object.value else {
            return Err(Self::error(
                id,
                "stream",
                ImageErrorKind::Invalid("dictionary required"),
            ));
        };
        let edge = self.soft_mask_edge(id, d)?;
        if mask_role {
            self.mask_role(id, d, edge)?;
        }
        if let Some(asset) = cached {
            if let Some(mask) = edge {
                let dependency = self.admit_mask(id, mask, asset.width, asset.height)?;
                if !asset
                    .opaque_mask
                    .as_ref()
                    .is_some_and(|a| Arc::ptr_eq(a, &dependency))
                {
                    return Err(Self::error(
                        id,
                        "SMask",
                        ImageErrorKind::Invariant("cached dependency identity"),
                    ));
                }
            }
            return Ok(asset);
        }
        self.decode_image(id, d, edge, None)
    }
    /// Decode an inline image (BI/ID/EI) whose abbreviations the caller has
    /// expanded. `id` is a synthetic identity outside the document's numbers.
    pub fn load_inline(
        &mut self,
        id: ObjectId,
        d: &Dictionary,
        data: &[u8],
    ) -> Result<Arc<ImageAsset>, ImageError> {
        charge(
            &mut self.usage.attempts,
            1,
            self.active.max_attempts,
            id,
            "attempts",
        )?;
        self.decode_image(id, d, None, Some(data))
    }
    fn decode_image<'a>(
        &mut self,
        id: ObjectId,
        d: &'a Dictionary,
        edge: Option<ObjectId>,
        inline: Option<&'a [u8]>,
    ) -> Result<Arc<ImageAsset>, ImageError>
    where
        'd: 'a,
    {
        if let Some(value) = self.get(id, d, b"Type", "Type")? {
            if !matches!(value,Object::Name(n) if n==b"XObject") {
                return Err(Self::error(
                    id,
                    "Type",
                    ImageErrorKind::Invalid("XObject required"),
                ));
            }
        }
        if !matches!(self.required(id,d,b"Subtype","Subtype")?,Object::Name(n) if n==b"Image") {
            return Err(Self::error(
                id,
                "Subtype",
                ImageErrorKind::Unsupported("image required"),
            ));
        }
        for (key, field) in [
            (b"ImageMask".as_slice(), "ImageMask"),
            (b"Interpolate", "Interpolate"),
        ] {
            if let Some(value) = self.get(id, d, key, field)? {
                match value {
                    Object::Boolean(false) => {}
                    Object::Boolean(true) => {
                        return Err(Self::error(id, field, ImageErrorKind::Unsupported(field)));
                    }
                    _ => {
                        return Err(Self::error(
                            id,
                            field,
                            ImageErrorKind::Invalid("boolean required"),
                        ));
                    }
                }
            }
        }
        // /OC is evaluated by the page program, which hides the image.
        for key in [
            b"Alternates".as_slice(),
            b"OPI",
            b"F",
            b"FFilter",
            b"FDecodeParms",
            b"Matte",
        ] {
            if self.get(id, d, key, "image semantics")?.is_some() {
                return Err(Self::error(
                    id,
                    "image semantics",
                    ImageErrorKind::Unsupported("mask/external/alternate/color semantics"),
                ));
            }
        }
        if let Some(intent) = self.get(id, d, b"Intent", "Intent")? {
            if !matches!(intent, Object::Name(n) if matches!(n.as_slice(), b"AbsoluteColorimetric" | b"RelativeColorimetric" | b"Saturation" | b"Perceptual"))
            {
                return Err(Self::error(
                    id,
                    "Intent",
                    ImageErrorKind::Unsupported("nonstandard rendering intent"),
                ));
            }
        }
        if let Some(v) = self.get(id, d, b"SMaskInData", "SMaskInData")? {
            if !matches!(v, Object::Integer(0)) {
                return Err(Self::error(
                    id,
                    "SMaskInData",
                    ImageErrorKind::Unsupported("nonzero or malformed SMaskInData"),
                ));
            }
        }
        let width = self.number(id, d, b"Width", "Width")?;
        let height = self.number(id, d, b"Height", "Height")?;
        if self.number(id, d, b"BitsPerComponent", "BitsPerComponent")? != 8 {
            return Err(Self::error(
                id,
                "BitsPerComponent",
                ImageErrorKind::Unsupported("only 8-bit samples"),
            ));
        }
        let mut palette = Vec::new();
        let mut calibrated = None;
        let components = match self.required(id, d, b"ColorSpace", "ColorSpace")? {
            Object::Name(n) if n == b"DeviceGray" => ImageComponents::Gray,
            Object::Name(n) if n == b"DeviceRGB" => ImageComponents::Rgb,
            Object::Array(values)
                if values
                    .first()
                    .is_some_and(|v| matches!(v, Object::Name(n) if n == b"CalRGB")) =>
            {
                calibrated = Some(Box::new(self.cal_rgb(id, values)?));
                ImageComponents::Rgb
            }
            Object::Array(values) => {
                palette = self.indexed_palette(id, values)?;
                ImageComponents::IndexedRgb
            }
            _ => {
                return Err(Self::error(
                    id,
                    "ColorSpace",
                    ImageErrorKind::Unsupported("exact device Gray/RGB required"),
                ));
            }
        };
        let (row_bytes, raw_bytes) = self.check_shape(id, width, height, components)?;
        let color_key = if let Some(value) = self.get(id, d, b"Mask", "Mask")? {
            if components != ImageComponents::IndexedRgb || edge.is_some() {
                return Err(Self::error(
                    id,
                    "Mask",
                    ImageErrorKind::Unsupported("color key requires Indexed RGB without SMask"),
                ));
            }
            let Object::Array(values) = value else {
                return Err(Self::error(
                    id,
                    "Mask",
                    ImageErrorKind::Unsupported("only color-key arrays"),
                ));
            };
            if values.len() != 2 {
                return Err(Self::error(
                    id,
                    "Mask",
                    ImageErrorKind::Invalid("color-key pair length"),
                ));
            }
            let mut range = [0; 2];
            for (i, value) in values.iter().enumerate() {
                self.item(id)?;
                range[i] = match self.resolve(id, value, "Mask")? {
                    Object::Integer(n) if (0..=255).contains(n) => *n as u8,
                    _ => {
                        return Err(Self::error(
                            id,
                            "Mask",
                            ImageErrorKind::Invalid("8-bit color-key integer"),
                        ));
                    }
                };
            }
            if range[0] > range[1] {
                return Err(Self::error(
                    id,
                    "Mask",
                    ImageErrorKind::Invalid("reversed color-key range"),
                ));
            }
            Some(range)
        } else {
            None
        };
        let mut inverted = [false; 3];
        if let Some(value) = self.get(id, d, b"Decode", "Decode")? {
            let Object::Array(values) = value else {
                return Err(Self::error(
                    id,
                    "Decode",
                    ImageErrorKind::Invalid("array required"),
                ));
            };
            if values.len() != 2 * components.count() {
                return Err(Self::error(
                    id,
                    "Decode",
                    ImageErrorKind::Invalid("component pair count"),
                ));
            }
            if components == ImageComponents::IndexedRgb {
                for (value, expected) in values.iter().zip([0., 255.]) {
                    self.item(id)?;
                    let number = match self.resolve(id, value, "Decode")? {
                        Object::Integer(n) => *n as f64,
                        Object::Real(n) => *n,
                        _ => f64::NAN,
                    };
                    if number != expected {
                        return Err(Self::error(
                            id,
                            "Decode",
                            ImageErrorKind::Unsupported("Indexed Decode must be [0 255]"),
                        ));
                    }
                }
            } else {
                let mut pairs = [0.; 6];
                for (i, v) in values.iter().enumerate() {
                    self.item(id)?;
                    pairs[i] = match self.resolve(id, v, "Decode")? {
                        Object::Integer(0) => 0.,
                        Object::Integer(1) => 1.,
                        Object::Real(v) if *v == 0. || *v == 1. => *v,
                        _ => {
                            return Err(Self::error(
                                id,
                                "Decode",
                                ImageErrorKind::Unsupported("only zero/one endpoints"),
                            ));
                        }
                    };
                }
                for (i, entry) in inverted.iter_mut().take(components.count()).enumerate() {
                    match (pairs[2 * i], pairs[2 * i + 1]) {
                        (0., 1.) => {}
                        (1., 0.) => *entry = true,
                        _ => {
                            return Err(Self::error(
                                id,
                                "Decode",
                                ImageErrorKind::Unsupported("only normal/inverted pairs"),
                            ));
                        }
                    }
                }
            }
        }
        let filters = self.filters(id, d, width, components.count())?;
        let data = match inline {
            Some(data) => data,
            None => {
                self.document_lookup(id)?;
                self.document
                    .stream_data(id)
                    .map_err(|e| Self::error(id, "stream", ImageErrorKind::Document(e)))?
                    .ok_or_else(|| {
                        Self::error(id, "stream", ImageErrorKind::Invalid("missing stream data"))
                    })?
            }
        };
        charge(
            &mut self.usage.unique_images,
            1,
            self.active.max_unique_images,
            id,
            "unique images",
        )?;
        charge(
            &mut self.usage.encoded_bytes,
            data.len(),
            self.active.max_encoded_bytes,
            id,
            "encoded bytes",
        )?;
        charge(
            &mut self.usage.decoded_rows,
            height,
            self.active.max_decoded_rows,
            id,
            "decoded rows",
        )?;
        charge(
            &mut self.usage.decoded_pixels,
            mul(id, width, height)?,
            self.active.max_decoded_pixels,
            id,
            "decoded pixels",
        )?;
        charge(
            &mut self.usage.retained_sample_bytes,
            raw_bytes,
            self.active.max_retained_sample_bytes,
            id,
            "retained samples",
        )?;
        self.allocate(id, size_of::<ImageAsset>() + 2 * size_of::<usize>())?;
        let stages = filters
            .iter()
            .map(|s| 1 + usize::from(s.predictor.predictor != 1))
            .sum::<usize>()
            .max(1);
        let mut total = self
            .active
            .max_decode_bytes
            .min(self.active.max_decoded_work_bytes - self.usage.decoded_work_bytes);
        let mut stage = self.active.max_stage_bytes.min(total);
        if filters.is_empty() {
            total = total.min(data.len());
            stage = stage.min(data.len());
        }
        let allocation_factor = mul(id, 3, stages)?;
        stage = stage.min(
            (self.active.max_allocated_bytes - self.usage.allocated_bytes) / allocation_factor,
        );
        // This reservation covers every potential output stage, including predictors.
        self.allocate(id, mul(id, allocation_factor, stage)?)?;
        charge(
            &mut self.usage.decoded_work_bytes,
            total,
            self.active.max_decoded_work_bytes,
            id,
            "decoded work",
        )?;
        let decoded = decode_filter_specs_with_usage(
            data,
            &filters,
            StreamLimits {
                max_encoded_bytes: data.len(),
                max_decoded_bytes: stage,
                max_filters: self.active.max_filters,
                max_total_decoded_bytes: total,
            },
        )
        .map_err(|e| Self::error(id, "decode", ImageErrorKind::Decode(e)))?;
        self.usage.decoded_work_bytes -= total - decoded.usage.total_decoded_bytes;
        if decoded.bytes.len() != raw_bytes {
            return Err(Self::error(
                id,
                "samples",
                ImageErrorKind::Invalid("decoded image byte length"),
            ));
        }
        let opaque_mask = if let Some(mask) = edge {
            Some(self.admit_mask(id, mask, width, height)?)
        } else {
            None
        };
        if self.cache.len() == self.cache.capacity() {
            let capacity = self
                .cache
                .capacity()
                .saturating_mul(2)
                .max(1)
                .min(self.active.max_unique_images);
            if capacity <= self.cache.len() {
                return Err(Self::error(
                    id,
                    "cache",
                    ImageErrorKind::Limit("unique images"),
                ));
            }
            self.allocate(id, mul(id, capacity, size_of::<Arc<ImageAsset>>())?)?;
            self.cache
                .try_reserve_exact(capacity - self.cache.len())
                .map_err(|_| Self::error(id, "cache", ImageErrorKind::Allocation))?;
        }
        let asset = Arc::new(ImageAsset {
            object: id,
            width,
            height,
            row_bytes,
            components,
            inverted,
            samples: decoded.bytes,
            filters,
            opaque_mask,
            palette,
            color_key,
            calibrated,
        });
        self.cache.push(Arc::clone(&asset));
        Ok(asset)
    }
    fn filter<'a>(&mut self, id: ObjectId, value: &'a Object) -> Result<Filter, ImageError>
    where
        'd: 'a,
    {
        self.item(id)?;
        let value = self.resolve(id, value, "Filter")?;
        let Object::Name(name) = value else {
            return Err(Self::error(
                id,
                "Filter",
                ImageErrorKind::Invalid("name required"),
            ));
        };
        self.work(id, mul(id, name.len().saturating_add(1), 5)?)?;
        match name.as_slice() {
            b"FlateDecode" => Ok(Filter::Flate),
            b"ASCIIHexDecode" => Ok(Filter::AsciiHex),
            b"ASCII85Decode" => Ok(Filter::Ascii85),
            b"RunLengthDecode" => Ok(Filter::RunLength),
            b"LZWDecode" => Ok(Filter::Lzw { early_change: true }),
            _ => Err(Self::error(
                id,
                "Filter",
                ImageErrorKind::Unsupported("lossless full filter name required"),
            )),
        }
    }
    fn parameters<'a>(
        &mut self,
        id: ObjectId,
        mut filter: Filter,
        value: &'a Object,
        width: usize,
        components: usize,
    ) -> Result<FilterSpec, ImageError>
    where
        'd: 'a,
    {
        self.item(id)?;
        let value = self.resolve(id, value, "DecodeParms")?;
        let mut predictor = PredictorParams::default();
        let dictionary = match value {
            Object::Null => return Ok(FilterSpec { filter, predictor }),
            Object::Dictionary(d) => d,
            _ => {
                return Err(Self::error(
                    id,
                    "DecodeParms",
                    ImageErrorKind::Invalid("dictionary or null required"),
                ));
            }
        };
        for (key, value) in dictionary {
            self.item(id)?;
            self.work(id, mul(id, key.len().saturating_add(1), 5)?)?;
            let value = self.resolve(id, value, "DecodeParms")?;
            if matches!(value, Object::Null) {
                continue;
            }
            let Object::Integer(n) = value else {
                return Err(Self::error(
                    id,
                    "DecodeParms",
                    ImageErrorKind::Invalid("integer parameter required"),
                ));
            };
            if key == b"EarlyChange" && matches!(filter, Filter::Lzw { .. }) {
                filter = match n {
                    0 => Filter::Lzw {
                        early_change: false,
                    },
                    1 => Filter::Lzw { early_change: true },
                    _ => {
                        return Err(Self::error(
                            id,
                            "EarlyChange",
                            ImageErrorKind::Unsupported("zero or one required"),
                        ));
                    }
                };
                continue;
            }
            if !matches!(filter, Filter::Flate | Filter::Lzw { .. }) {
                return Err(Self::error(
                    id,
                    "DecodeParms",
                    ImageErrorKind::Unsupported("parameters require Flate/LZW"),
                ));
            }
            let n = usize::try_from(*n).map_err(|_| {
                Self::error(
                    id,
                    "DecodeParms",
                    ImageErrorKind::Invalid("nonnegative integer required"),
                )
            })?;
            match key.as_slice() {
                b"Predictor" => {
                    predictor.predictor = u8::try_from(n).map_err(|_| {
                        Self::error(
                            id,
                            "Predictor",
                            ImageErrorKind::Unsupported("predictor range"),
                        )
                    })?
                }
                b"Colors" => predictor.colors = n,
                b"Columns" => predictor.columns = n,
                b"BitsPerComponent" => {
                    predictor.bits_per_component = u8::try_from(n).map_err(|_| {
                        Self::error(
                            id,
                            "BitsPerComponent",
                            ImageErrorKind::Unsupported("component depth"),
                        )
                    })?
                }
                _ => {
                    return Err(Self::error(
                        id,
                        "DecodeParms",
                        ImageErrorKind::Unsupported("unknown parameter"),
                    ));
                }
            }
        }
        if predictor.predictor != 1
            && (predictor.colors != components
                || predictor.columns != width
                || predictor.bits_per_component != 8)
        {
            return Err(Self::error(
                id,
                "DecodeParms",
                ImageErrorKind::Invalid("predictor disagrees with image layout"),
            ));
        }
        Ok(FilterSpec { filter, predictor })
    }
    fn filters<'a>(
        &mut self,
        id: ObjectId,
        d: &'a Dictionary,
        width: usize,
        components: usize,
    ) -> Result<Vec<FilterSpec>, ImageError>
    where
        'd: 'a,
    {
        let value = self.get(id, d, b"Filter", "Filter")?;
        let count = match value {
            None => 0,
            Some(Object::Name(_)) => 1,
            Some(Object::Array(values)) => values.len(),
            _ => {
                return Err(Self::error(
                    id,
                    "Filter",
                    ImageErrorKind::Invalid("name/array/null required"),
                ));
            }
        };
        if count > self.active.max_filters {
            return Err(Self::error(id, "Filter", ImageErrorKind::Limit("filters")));
        }
        charge(
            &mut self.usage.filter_specs,
            count,
            self.active.max_filter_specs,
            id,
            "filter specs",
        )?;
        self.allocate(id, mul(id, count, size_of::<FilterSpec>())?)?;
        let mut filters: Vec<FilterSpec> = Vec::new();
        filters
            .try_reserve_exact(count)
            .map_err(|_| Self::error(id, "Filter", ImageErrorKind::Allocation))?;
        match value {
            Some(v @ Object::Name(_)) => filters.push(self.filter(id, v)?.into()),
            Some(Object::Array(values)) => {
                for v in values {
                    filters.push(self.filter(id, v)?.into());
                }
            }
            _ => {}
        }
        match self.get(id, d, b"DecodeParms", "DecodeParms")? {
            None => {}
            Some(v @ Object::Dictionary(_)) if count == 1 => {
                filters[0] = self.parameters(id, filters[0].filter, v, width, components)?
            }
            Some(Object::Array(values)) if count > 0 && values.len() == count => {
                for (spec, v) in filters.iter_mut().zip(values) {
                    *spec = self.parameters(id, spec.filter, v, width, components)?;
                }
            }
            _ => {
                return Err(Self::error(
                    id,
                    "DecodeParms",
                    ImageErrorKind::Invalid("parameter shape differs from filters"),
                ));
            }
        }
        Ok(filters)
    }
}
fn mul(id: ObjectId, a: usize, b: usize) -> Result<usize, ImageError> {
    a.checked_mul(b)
        .ok_or_else(|| ImageLoader::error(id, "budget", ImageErrorKind::Limit("arithmetic")))
}
fn charge(
    used: &mut usize,
    n: usize,
    cap: usize,
    id: ObjectId,
    field: &'static str,
) -> Result<(), ImageError> {
    *used = used
        .checked_add(n)
        .filter(|v| *v <= cap)
        .ok_or_else(|| ImageLoader::error(id, field, ImageErrorKind::Limit(field)))?;
    Ok(())
}
