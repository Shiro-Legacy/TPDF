// SPDX-License-Identifier: MIT
//! Images whose RunLengthDecode data ends without its end-of-data byte.
//! Reference viewers decode the runs they find; hayro refuses the stream and
//! draws nothing. The view appends the byte to such an image's stored data.
use crate::object::{Object, ObjectId};

use super::Fixes;

/// RunLengthDecode's end-of-data length byte.
const EOD: u8 = 128;

impl Fixes<'_, '_> {
    /// Ends the run-length data of image `id` (its only filter) with the
    /// end-of-data byte when every run is whole and the byte is missing,
    /// once, within the view's copy budget.
    pub(super) fn fix_run_length(&mut self, id: ObjectId) {
        if !self.recolor.ended.insert(id) || self.streams.contains_key(&id) {
            return;
        }
        let doc = self.doc;
        let Ok(dict) = doc.stream_dict(id) else {
            return;
        };
        let only = match doc.dict_get(dict, b"Filter") {
            Object::Name(name) => name.as_slice() == b"RunLengthDecode",
            Object::Array(filters) => {
                matches!(filters.as_slice(), [one] if doc.resolve(one).as_name() == Some(b"RunLengthDecode"))
            }
            _ => false,
        };
        if !only {
            return;
        }
        let Ok(Some(data)) = doc.stream_data(id) else {
            return;
        };
        if !runs_end_unmarked(data) {
            return;
        }
        if self.copy_budget(data.len().saturating_add(1)).is_none() {
            return;
        }
        let mut ended = data.to_vec();
        ended.push(EOD);
        self.streams.insert(id, (dict.clone(), ended));
    }
}

/// Whether `data` is whole runs up to its end with no end-of-data byte.
fn runs_end_unmarked(data: &[u8]) -> bool {
    let mut at = 0usize;
    while let Some(&length) = data.get(at) {
        let step = match length {
            EOD => return false,
            0..EOD => usize::from(length) + 2,
            _ => 2,
        };
        at = at.saturating_add(step);
    }
    at == data.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_whole_runs_without_the_end_byte_are_ended() {
        assert!(runs_end_unmarked(&[1, 7, 8, 255, 9]));
        assert!(!runs_end_unmarked(&[1, 7, 8, 255, 9, 128]));
        assert!(!runs_end_unmarked(&[128, 1, 2]));
        // A literal run cut short, or a repeat without its byte.
        assert!(!runs_end_unmarked(&[3, 7, 8]));
        assert!(!runs_end_unmarked(&[1, 7, 8, 255]));
        assert!(runs_end_unmarked(&[]));
    }
}
