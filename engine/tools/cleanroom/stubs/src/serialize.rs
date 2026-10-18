use crate::object::{Dictionary, Object, ObjectId};
use std::collections::BTreeSet;
use std::fmt;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SerializeStyle {
    #[default]
    Canonical,
    Compact,
}

#[derive(Clone, Copy, Debug)]
pub struct WriteLimits {
    pub max_output_bytes: usize,
    pub max_depth: usize,
    pub max_objects: usize,
}

impl Default for WriteLimits {
    fn default() -> Self {
        todo!("clean-room: spec 01")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SerializeError {
    OutputLimit,
    DepthLimit,
    ObjectLimit,
    NullInName,
    NonfiniteReal,
    AllocationFailed,
}

impl fmt::Display for SerializeError {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!("clean-room: spec 01")
    }
}

impl std::error::Error for SerializeError {}

pub(crate) struct Writer {}

impl Writer {
    pub(crate) fn with_style(_limits: WriteLimits, _style: SerializeStyle) -> Self {
        todo!("clean-room: spec 01")
    }

    pub(crate) fn keep_order(&mut self) {
        todo!("clean-room: spec 01")
    }

    pub(crate) fn null_references(&mut self, _ids: BTreeSet<ObjectId>) {
        todo!("clean-room: spec 01")
    }

    pub(crate) fn len(&self) -> usize {
        todo!("clean-room: spec 01")
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        todo!("clean-room: spec 01")
    }

    pub(crate) fn append(&mut self, _bytes: &[u8]) -> Result<(), SerializeError> {
        todo!("clean-room: spec 01")
    }

    pub(crate) fn value(&mut self, _value: &Object, _depth: usize) -> Result<(), SerializeError> {
        todo!("clean-room: spec 01")
    }

    pub(crate) fn dictionary_entries<'a>(
        &mut self,
        _entries: impl IntoIterator<Item = (&'a [u8], &'a Object)>,
    ) -> Result<(), SerializeError> {
        todo!("clean-room: spec 01")
    }

    pub(crate) fn stream_dictionary(
        &mut self,
        _values: &Dictionary,
        _length: usize,
    ) -> Result<(), SerializeError> {
        todo!("clean-room: spec 01")
    }
}

pub(crate) fn compact_real(_value: f64) -> String {
    todo!("clean-room: spec 01")
}

pub fn serialize_object(_value: &Object, _limits: WriteLimits) -> Result<Vec<u8>, SerializeError> {
    todo!("clean-room: spec 01")
}

pub fn serialize_object_with_style(
    _value: &Object,
    _limits: WriteLimits,
    _style: SerializeStyle,
) -> Result<Vec<u8>, SerializeError> {
    todo!("clean-room: spec 01")
}
