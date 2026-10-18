// SPDX-License-Identifier: AGPL-3.0-or-later
//! Transfer functions of graphics states (/TR, /TR2). Reference viewers
//! ignore them when they paint (PDF 2.0 deprecates them, PDF/A forbids them);
//! hayro applies them to the colour it draws, so an inverting function turned
//! fills into their complement. The view leaves them out of each graphics state
//! the resources walk meets; a soft mask's own /TR stays.
use crate::object::{Dictionary, Object};

use super::Fixes;

impl Fixes<'_, '_> {
    /// Takes the transfer functions out of the graphics states of an
    /// /ExtGState resource dictionary, editing `holder` in place when it is
    /// direct. Whether `holder` changed.
    pub(super) fn map_gstates(&mut self, holder: &mut Object) -> bool {
        let Object::Reference(id) = holder else {
            return self.map_gstate_entries(holder);
        };
        let id = *id;
        if !self.recolor.done.insert(id) {
            return false;
        }
        if let Some(mut value) = self.current(id) {
            if self.map_gstate_entries(&mut value) {
                self.objects.insert(id, value);
            }
        }
        false
    }

    fn map_gstate_entries(&mut self, value: &mut Object) -> bool {
        let Some(states) = value.as_dict_mut() else {
            return false;
        };
        let mut changed = false;
        for state in states.values_mut() {
            match state {
                Object::Reference(id) => {
                    let id = *id;
                    if !self.recolor.done.insert(id) {
                        continue;
                    }
                    if let Some(mut value) = self.current(id) {
                        if value.as_dict_mut().is_some_and(|d| self.untransfer(d)) {
                            self.objects.insert(id, value);
                        }
                    }
                }
                Object::Dictionary(state) => changed |= self.untransfer(state),
                _ => {}
            }
        }
        changed
    }

    /// Removes a graphics state's /TR and /TR2 unless they are the identity.
    fn untransfer(&self, state: &mut Dictionary) -> bool {
        let identity = |value: &Object| {
            matches!(
                self.doc.resolve(value).as_name(),
                Some(b"Identity" | b"Default")
            )
        };
        let mut changed = false;
        for key in [b"TR".as_slice(), b"TR2"] {
            if state.get(key).is_some_and(|value| !identity(value)) {
                state.remove(key);
                changed = true;
            }
        }
        changed
    }
}
