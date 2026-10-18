use super::{ColorSpace, ToRgb, U8Lookup};
use crate::cache::Cache;
use crate::function::Function;
use hayro_syntax::object::{Array, Name, Object};

#[derive(Debug, Clone)]
pub(crate) struct DeviceN {
    alternate_space: ColorSpace,
    pub(super) num_components: u8,
    tint_transform: Function,
    is_none: bool,
    lookup: U8Lookup<[u8; 3]>,
}

impl DeviceN {
    pub(super) fn new(array: &Array<'_>, cache: &Cache, depth: u8) -> Option<Self> {
        let mut iter = array.flex_iter();
        // Skip `/DeviceN`
        let _ = iter.next::<Name<'_>>()?;
        // Skip `Name`.
        let names = iter
            .next::<Array<'_>>()?
            .iter::<Name<'_>>()
            .collect::<Vec<_>>();
        let num_components = u8::try_from(names.len()).ok()?;
        let all_none = names.iter().all(|n| n.as_str() == "None");
        let alternate_space = ColorSpace::nested(iter.next::<Object<'_>>()?, cache, depth)?;
        let tint_transform = Function::new_with(&iter.next::<Object<'_>>()?, cache)?;

        if num_components == 0 {
            return None;
        }

        Some(Self {
            alternate_space,
            num_components,
            tint_transform,
            is_none: all_none,
            lookup: U8Lookup::default(),
        })
    }

    fn evaluate(&self, input: &[u8]) -> Vec<u8> {
        // Tandem patch: a pixel equal to the one before reuses its values, so
        // flat areas (and the padding of short image data) cost one
        // evaluation.
        let mut out = Vec::new();
        let mut last: Option<(&[u8], usize)> = None;
        for n in input.chunks_exact(self.num_components as usize) {
            if let Some((previous, at)) = last
                && previous == n
            {
                let start = out.len();
                out.extend_from_within(at..start);
                last = Some((n, start));
                continue;
            }
            let at = out.len();
            let input = n.iter().map(|value| *value as f32 / 255.0).collect();
            let mut values = self
                .tint_transform
                .eval(input)
                .unwrap_or(self.alternate_space.initial_color());
            // Tandem patch: as many values as the alternate space has
            // components, cut or padded with 0 (as reference viewers read it).
            values.resize(self.alternate_space.num_components() as usize, 0.0);
            out.extend_from_slice(&self.alternate_space.encode_values(&values));
            last = Some((n, at));
        }
        out
    }

    fn convert_inner(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        let evaluated = self.evaluate(input);
        self.alternate_space.convert(&evaluated, output)
    }

    fn u8_lookup(&self) -> Option<&[[u8; 3]; 256]> {
        self.lookup
            .get_or_init(|input, output| self.convert_inner(input, output))
    }
}

impl ToRgb for DeviceN {
    fn convert(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        if self.num_components == 1 {
            let lookup = self.u8_lookup()?;
            for (input, output) in input.iter().zip(output.chunks_exact_mut(3)) {
                output.copy_from_slice(&lookup[*input as usize]);
            }

            Some(())
        } else {
            self.convert_inner(input, output)
        }
    }

    fn convert_in_place(&self, input: &mut [u8]) -> Option<()> {
        if self.num_components != 3 {
            return None;
        }

        let evaluated = self.evaluate(input);
        self.alternate_space.convert(&evaluated, input)
    }

    fn is_none(&self) -> bool {
        self.is_none
    }
}
