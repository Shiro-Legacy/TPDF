use crate::util::hash128;
use hayro_syntax::object::stream::DecodeFailure;
use hayro_syntax::object::{Array, Dict, MaybeRef, Name, Null, ObjRef, Object, Stream};
use kurbo::{Affine, Rect};
use rustc_hash::FxHashMap;
use std::any::Any;
use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::ops::Deref;
use std::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

type CacheMap = FxHashMap<u128, Option<Box<dyn Any + Send + Sync>>>;
// Tandem patch: and the content budget of what is interpreted with it.
#[derive(Clone)]
pub(crate) struct Cache(Arc<Mutex<CacheMap>>, Arc<ContentBudget>);

impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}

impl Cache {
    pub(crate) fn new() -> Self {
        Self(
            Arc::new(Mutex::new(FxHashMap::default())),
            Arc::new(ContentBudget::default()),
        )
    }

    pub(crate) fn get_or_insert_with<T: Clone + Send + Sync + 'static>(
        &self,
        id: u128,
        f: impl FnOnce() -> Option<T>,
    ) -> Option<T> {
        let mut locked = self.0.lock().unwrap();

        // We can't use `get_or_insert_with` here, because if the closure makes another access to the
        // cache, we end up with a deadlock.
        match locked.entry(id) {
            Entry::Occupied(o) => o
                .get()
                .as_ref()
                .and_then(|val| val.downcast_ref::<T>().cloned()),
            Entry::Vacant(_) => {
                drop(locked);
                let val = f();
                self.0.lock().unwrap().insert(
                    id,
                    val.clone()
                        .map(|val| Box::new(val) as Box<dyn Any + Send + Sync>),
                );

                val
            }
        }
    }
}

// Tandem patch: values kept within a byte budget.
/// The key holding the bytes kept by `get_or_insert_sized`.
const SIZED_TOTAL: u128 = u128::MAX;

impl Cache {
    /// The value kept for `id`, or `f()`'s, which is kept only while the
    /// values kept this way take at most `budget` bytes (by `size`).
    pub(crate) fn get_or_insert_sized<T: Clone + Send + Sync + 'static>(
        &self,
        id: u128,
        budget: usize,
        size: impl FnOnce(&T) -> usize,
        f: impl FnOnce() -> Option<T>,
    ) -> Option<T> {
        let lock = || {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        };
        if let Some(Some(value)) = lock().get(&id)
            && let Some(value) = value.downcast_ref::<T>()
        {
            return Some(value.clone());
        }
        let value = f()?;
        let bytes = size(&value);
        let mut locked = lock();
        let total = locked
            .entry(SIZED_TOTAL)
            .or_insert_with(|| Some(Box::new(0usize)))
            .as_mut()
            .and_then(|total| total.downcast_mut::<usize>())
            .filter(|total| total.saturating_add(bytes) <= budget)
            .map(|total| {
                *total += bytes;
            });
        if total.is_some() {
            locked.insert(id, Some(Box::new(value.clone())));
        }
        Some(value)
    }
}

// Tandem patch: the bytes of sampled function tables built with one cache.
/// The key holding the bytes reserved by `reserve_sampled`.
const SAMPLED_TOTAL: u128 = u128::MAX - 1;
/// The key marking a reservation refused.
const OVER_BUDGET: u128 = u128::MAX - 2;
/// Bytes of sampled (type 0) function tables one cache (a render, or the
/// text of a document) may build.
pub(crate) const MAX_SAMPLED_BYTES: usize = 64 << 20;

// Tandem patch: the operators of the type 4 functions built with one cache.
/// The key holding the operators counted by `reserve_program_ops`.
const PROGRAM_TOTAL: u128 = u128::MAX - 3;
/// PostScript operators the type 4 functions built with one cache (a render,
/// or the text of a document) may keep together, 32 bytes each. One
/// program has at most 2^14 (`type4.rs`), but a stitching function may name
/// 4,096 of them, each kept for the render: 256 programs of 2^14 held 218 MB
/// from a 74 KB file. Real programs have tens of operators.
pub(crate) const MAX_PROGRAM_OPS: usize = 1 << 18;

impl Cache {
    /// Counts `bytes` of sampled function tables against
    /// [`MAX_SAMPLED_BYTES`]: false, and the cache over budget, past it.
    pub(crate) fn reserve_sampled(&self, bytes: usize) -> bool {
        let mut locked = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fits = locked
            .entry(SAMPLED_TOTAL)
            .or_insert_with(|| Some(Box::new(0usize)))
            .as_mut()
            .and_then(|total| total.downcast_mut::<usize>())
            .filter(|total| total.saturating_add(bytes) <= MAX_SAMPLED_BYTES)
            .map(|total| *total += bytes)
            .is_some();
        if !fits {
            locked.insert(OVER_BUDGET, None);
        }
        fits
    }

    /// Counts `ops` PostScript operators of a type 4 function against
    /// [`MAX_PROGRAM_OPS`]: false, and the cache over budget, past it.
    pub(crate) fn reserve_program_ops(&self, ops: usize) -> bool {
        let mut locked = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fits = locked
            .entry(PROGRAM_TOTAL)
            .or_insert_with(|| Some(Box::new(0usize)))
            .as_mut()
            .and_then(|total| total.downcast_mut::<usize>())
            .filter(|total| total.saturating_add(ops) <= MAX_PROGRAM_OPS)
            .map(|total| *total += ops)
            .is_some();
        if !fits {
            locked.insert(OVER_BUDGET, None);
        }
        fits
    }

    /// Whether something built with this cache passed its budget.
    pub(crate) fn over_budget(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(&OVER_BUDGET)
            || self.content_refusal().is_some()
    }
}

// Tandem patch: the decoded content held and read with one cache.
//
// hayro decodes a content stream (a page's, a form's, a soft mask's group, a
// tiling pattern's cell, a Type3 glyph's procedure) whole and holds it while
// what it invokes runs; forms inherit the caller's pattern, Type3 font and
// soft mask, so a pattern cell or glyph can run, and hold its content, inside
// a form that holds its own (forms of 120 MiB chained through inherited
// patterns or Type3 fonts held 128 MiB a level). Counting where hayro decodes
// covers every way one run invokes another.

/// Decoded content bytes the runs interpreted with one cache may hold at
/// once (the engine's walk holds a page to the same).
pub const MAX_HELD_CONTENT_BYTES: usize = hayro_syntax::object::stream::MAX_DECODED_BYTES;
/// Content bytes one page may read, each run's counted every time it runs.
pub const MAX_READ_CONTENT_BYTES: u64 = 1 << 30;
/// Samples of images too large to decode whole (past the image sample
/// budget, or past what hayro-syntax holds) one page may decode reduced, each
/// image counted each time it is drawn at another size: such an image is
/// decoded and reduced in full every time (a 68000² one-bit image took 38 s
/// natively, a 24000² one drawn at ten sizes 18 s on WASM). 2^30 is a
/// 24000² image once, or a 32768² one.
pub const MAX_REDUCED_SAMPLES: u64 = 1 << 30;

/// Why the content of a page was not all interpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentRefusal {
    /// A run would have held more than [`MAX_HELD_CONTENT_BYTES`] of
    /// decoded content together with the runs that invoked it.
    Held,
    /// The page read more than [`MAX_READ_CONTENT_BYTES`] of content.
    Read,
}

#[derive(Default)]
pub(crate) struct ContentBudget {
    held: AtomicUsize,
    /// The most held at once since the budget started.
    peak: AtomicUsize,
    read: AtomicU64,
    /// 0: none, 1: held, 2: read.
    refused: AtomicU8,
    /// Samples of images decoded reduced (`MAX_REDUCED_SAMPLES`).
    reduced_samples: AtomicU64,
    /// The most bytes one reduced decode kept for itself.
    reduced_peak: AtomicUsize,
}

impl ContentBudget {
    fn refuse(&self, why: ContentRefusal) {
        let code = match why {
            ContentRefusal::Held => 1,
            ContentRefusal::Read => 2,
        };
        let _ = self
            .refused
            .compare_exchange(0, code, Ordering::Relaxed, Ordering::Relaxed);
    }
}

/// Decoded content, counted as held for as long as it lives.
pub(crate) struct HeldContent<'a> {
    data: Cow<'a, [u8]>,
    held: usize,
    budget: Arc<ContentBudget>,
}

impl Deref for HeldContent<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.data
    }
}

impl Drop for HeldContent<'_> {
    fn drop(&mut self) {
        self.budget.held.fetch_sub(self.held, Ordering::Relaxed);
    }
}

impl Cache {
    /// `stream`'s decoded content, read within [`MAX_READ_CONTENT_BYTES`]
    /// and held within what the runs holding theirs leave of
    /// [`MAX_HELD_CONTENT_BYTES`] (bytes hayro borrows from the file,
    /// unfiltered, are read but not held). `None` when it does not decode,
    /// as before, and (the content refused) when it would pass a budget, or
    /// once the content was refused: nothing more is decoded then.
    pub(crate) fn decode_content<'a>(&self, stream: &Stream<'a>) -> Option<HeldContent<'a>> {
        let budget = &self.1;
        if budget.refused.load(Ordering::Relaxed) != 0 {
            return None;
        }
        let held = budget.held.load(Ordering::Relaxed);
        let room = MAX_HELD_CONTENT_BYTES.saturating_sub(held);
        match stream.decoded_within(room) {
            Ok(data) => {
                if !self.read_content(data.len()) {
                    return None;
                }
                let held = match &data {
                    Cow::Owned(data) => data.len(),
                    Cow::Borrowed(_) => 0,
                };
                Some(self.hold_content(data, held))
            }
            // Alone, a stream past hayro-syntax's own limit is not read, as
            // before.
            Err(DecodeFailure::TooLarge) if held > 0 => {
                budget.refuse(ContentRefusal::Held);
                None
            }
            Err(_) => None,
        }
    }

    /// `data`, of which `held` bytes are counted as held while it lives:
    /// what hayro decoded, all of a page's content (hayro-syntax hands it
    /// over decoded, kept or copied), nothing of bytes borrowed from the
    /// file.
    pub(crate) fn hold_content<'a>(&self, data: Cow<'a, [u8]>, held: usize) -> HeldContent<'a> {
        let budget = &self.1;
        let now = budget.held.fetch_add(held, Ordering::Relaxed) + held;
        budget.peak.fetch_max(now, Ordering::Relaxed);
        if now > MAX_HELD_CONTENT_BYTES {
            budget.refuse(ContentRefusal::Held);
        }
        HeldContent {
            data,
            held,
            budget: budget.clone(),
        }
    }

    /// Counts `len` content bytes read (decoded, or borrowed, for a run)
    /// against [`MAX_READ_CONTENT_BYTES`]: false (the content refused, the
    /// run not to be interpreted) past it, or once the content was refused.
    pub(crate) fn read_content(&self, len: usize) -> bool {
        let budget = &self.1;
        if budget.refused.load(Ordering::Relaxed) != 0 {
            return false;
        }
        let len = u64::try_from(len).unwrap_or(u64::MAX);
        let read = budget
            .read
            .fetch_add(len, Ordering::Relaxed)
            .saturating_add(len);
        if read > MAX_READ_CONTENT_BYTES {
            budget.refuse(ContentRefusal::Read);
            return false;
        }
        true
    }

    /// Why content interpreted with this cache was refused, if it was.
    pub(crate) fn content_refusal(&self) -> Option<ContentRefusal> {
        match self.1.refused.load(Ordering::Relaxed) {
            1 => Some(ContentRefusal::Held),
            2 => Some(ContentRefusal::Read),
            _ => None,
        }
    }

    /// Starts the content budget of another page: what it read and why it
    /// was refused are forgotten (what is held stays held).
    pub(crate) fn start_content(&self) {
        self.1.read.store(0, Ordering::Relaxed);
        self.1.refused.store(0, Ordering::Relaxed);
        self.1.reduced_samples.store(0, Ordering::Relaxed);
        self.1.reduced_peak.store(0, Ordering::Relaxed);
        let held = self.1.held.load(Ordering::Relaxed);
        self.1.peak.store(held, Ordering::Relaxed);
    }

    /// Counts `samples` of an image decoded reduced against
    /// [`MAX_REDUCED_SAMPLES`]: false past it (the image is over budget).
    pub(crate) fn reserve_reduced_samples(&self, samples: u64) -> bool {
        let total = &self.1.reduced_samples;
        let before = total.fetch_add(samples, Ordering::Relaxed);
        before.saturating_add(samples) <= MAX_REDUCED_SAMPLES
    }

    /// Records the bytes one reduced decode kept for itself.
    pub(crate) fn note_reduced_working_set(&self, bytes: usize) {
        self.1.reduced_peak.fetch_max(bytes, Ordering::Relaxed);
    }

    /// The most bytes one reduced decode kept for itself.
    pub(crate) fn reduced_working_peak(&self) -> usize {
        self.1.reduced_peak.load(Ordering::Relaxed)
    }

    /// The most decoded content bytes held at once since the budget started.
    pub(crate) fn content_peak(&self) -> usize {
        self.1.peak.load(Ordering::Relaxed)
    }
}

/// A trait for objects that can generate a unique cache key.
pub trait CacheKey {
    /// Returns the cache key for this object.
    fn cache_key(&self) -> u128;
}

impl<T: CacheKey, U: CacheKey> CacheKey for (T, U) {
    fn cache_key(&self) -> u128 {
        hash128(&(self.0.cache_key(), self.1.cache_key()))
    }
}

impl CacheKey for Dict<'_> {
    fn cache_key(&self) -> u128 {
        hash128(self.data())
    }
}

impl CacheKey for Stream<'_> {
    fn cache_key(&self) -> u128 {
        self.dict().cache_key()
    }
}

impl CacheKey for Null {
    fn cache_key(&self) -> u128 {
        hash128(self)
    }
}

impl CacheKey for bool {
    fn cache_key(&self) -> u128 {
        hash128(self)
    }
}

impl CacheKey for hayro_syntax::object::Number {
    fn cache_key(&self) -> u128 {
        hash128(&self.as_f64().to_bits())
    }
}

impl CacheKey for hayro_syntax::object::String<'_> {
    fn cache_key(&self) -> u128 {
        hash128(self.as_ref())
    }
}

impl CacheKey for Name<'_> {
    fn cache_key(&self) -> u128 {
        hash128(self)
    }
}

impl CacheKey for Array<'_> {
    fn cache_key(&self) -> u128 {
        hash128(self.data())
    }
}

impl CacheKey for Object<'_> {
    fn cache_key(&self) -> u128 {
        match self {
            Object::Null(n) => n.cache_key(),
            Object::Boolean(b) => b.cache_key(),
            Object::Number(n) => n.cache_key(),
            Object::String(s) => s.cache_key(),
            Object::Name(n) => n.cache_key(),
            Object::Dict(d) => d.cache_key(),
            Object::Array(a) => a.cache_key(),
            Object::Stream(s) => s.cache_key(),
        }
    }
}

impl CacheKey for ObjRef {
    fn cache_key(&self) -> u128 {
        hash128(self)
    }
}

impl<T: CacheKey> CacheKey for MaybeRef<T> {
    fn cache_key(&self) -> u128 {
        match self {
            Self::Ref(r) => r.cache_key(),
            Self::NotRef(o) => o.cache_key(),
        }
    }
}

impl CacheKey for Affine {
    fn cache_key(&self) -> u128 {
        let c = self.as_coeffs();
        hash128(&[
            c[0].to_bits(),
            c[1].to_bits(),
            c[2].to_bits(),
            c[3].to_bits(),
            c[4].to_bits(),
            c[5].to_bits(),
        ])
    }
}

impl CacheKey for Rect {
    fn cache_key(&self) -> u128 {
        hash128(&[
            self.x0.to_bits(),
            self.y0.to_bits(),
            self.x1.to_bits(),
            self.y1.to_bits(),
        ])
    }
}

impl CacheKey for u128 {
    fn cache_key(&self) -> u128 {
        hash128(self)
    }
}
