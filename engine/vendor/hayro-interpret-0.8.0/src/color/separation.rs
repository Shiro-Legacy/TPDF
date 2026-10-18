use super::{ColorSpace, ToRgb, U8Lookup};
use crate::cache::Cache;
use crate::function::Function;
use hayro_syntax::object::{Array, Name, Object};
use smallvec::smallvec;

#[derive(Debug, Clone)]
pub(crate) struct Separation {
    alternate_space: ColorSpace,
    tint_transform: Function,
    is_none_separation: bool,
    lookup: U8Lookup<[u8; 3]>,
}

impl Separation {
    pub(super) fn new(array: &Array<'_>, cache: &Cache, depth: u8) -> Option<Self> {
        let mut iter = array.flex_iter();
        // Skip `/Separation`
        let _ = iter.next::<Name<'_>>()?;
        let name = iter.next::<Name<'_>>()?;
        let alternate_space = ColorSpace::nested(iter.next::<Object<'_>>()?, cache, depth)?;
        let tint_transform = Function::new_with(&iter.next::<Object<'_>>()?, cache)?;
        // Either I did something wrong, or no other viewers properly handles
        // `All`, so let's just ignore it as well.
        let is_none_separation = name.as_str() == "None";

        Some(Self {
            alternate_space,
            tint_transform,
            is_none_separation,
            lookup: U8Lookup::default(),
        })
    }

    fn convert_inner(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        let evaluated = input
            .iter()
            .flat_map(|n| {
                let mut values = self
                    .tint_transform
                    .eval(smallvec![*n as f32 / 255.0])
                    .unwrap_or(self.alternate_space.initial_color());
                // Tandem patch: as many values as the alternate space has
                // components, cut or padded with 0 (as reference viewers read it).
                values.resize(self.alternate_space.num_components() as usize, 0.0);
                self.alternate_space.encode_values(&values)
            })
            .collect::<Vec<u8>>();
        self.alternate_space.convert(&evaluated, output)
    }

    fn u8_lookup(&self) -> Option<&[[u8; 3]; 256]> {
        self.lookup
            .get_or_init(|input, output| self.convert_inner(input, output))
    }
}

impl ToRgb for Separation {
    fn convert(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
        let lookup = self.u8_lookup()?;
        for (input, output) in input.iter().zip(output.chunks_exact_mut(3)) {
            output.copy_from_slice(&lookup[*input as usize]);
        }

        Some(())
    }

    fn is_none(&self) -> bool {
        self.is_none_separation
    }
}
