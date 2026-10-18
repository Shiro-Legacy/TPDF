use std::fmt;

use crate::document::{DocumentError, DocumentLimits};
use crate::parser::ParseLimits;
use crate::serialize::SerializeError;

#[derive(Clone, Copy, Debug)]
pub struct RepairLimits {
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    pub max_objects: usize,
    pub max_values: usize,
    pub max_object_streams: usize,
    pub max_decoded_bytes: usize,
    pub max_revisions: usize,
    pub parse: ParseLimits,
}

impl Default for RepairLimits {
    fn default() -> Self {
        todo!("clean-room: spec 03")
    }
}

impl RepairLimits {
    pub fn for_document(_limits: DocumentLimits) -> Self {
        todo!("clean-room: spec 03")
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepairReport {
    pub objects_recovered: usize,
    pub objects_overridden: usize,
    pub objects_dropped: usize,
    pub object_streams_expanded: usize,
    pub streams_length_recovered: usize,
    pub references_repaired: usize,
    pub catalog_from_scan: bool,
    pub page_tree_repaired: bool,
    pub linearized: bool,
    pub xref_intact: bool,
    pub objects_unreadable: usize,
    pub xref_sections: usize,
    pub xref_size: u64,
    pub damaged: bool,
    pub strict_error: Option<String>,
    pub locked: bool,
    pub password_dependent: bool,
}

#[derive(Debug)]
pub struct RepairedPdf {
    pub bytes: Vec<u8>,
    pub report: RepairReport,
}

#[derive(Debug)]
pub enum RepairError {
    Invalid(&'static str),
    Unsupported(&'static str),
    LimitExceeded(&'static str),
    Serialize(SerializeError),
    Document(DocumentError),
}

impl fmt::Display for RepairError {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!("clean-room: spec 03")
    }
}

impl std::error::Error for RepairError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        todo!("clean-room: spec 03")
    }
}

impl From<SerializeError> for RepairError {
    fn from(_error: SerializeError) -> Self {
        todo!("clean-room: spec 03")
    }
}

#[derive(Debug)]
pub struct TolerantPdf {
    pub bytes: Vec<u8>,
    pub repair: Option<RepairReport>,
    pub strict_error: Option<DocumentError>,
    pub original: Option<Vec<u8>>,
    pub needs_password: bool,
}

pub fn open_tolerant(
    _input: Vec<u8>,
    _limits: DocumentLimits,
    _password: &[u8],
) -> Result<TolerantPdf, RepairError> {
    todo!("clean-room: spec 03")
}

pub fn rebuild(_input: &[u8], _limits: RepairLimits) -> Result<RepairedPdf, RepairError> {
    todo!("clean-room: spec 03")
}

pub fn rebuild_with_passwords(
    _input: &[u8],
    _limits: RepairLimits,
    _passwords: &[&[u8]],
) -> Result<RepairedPdf, RepairError> {
    todo!("clean-room: spec 03")
}

pub(crate) fn untyped_kid_is_node(_kids: Option<usize>, _contents: bool, _media: bool) -> bool {
    todo!("clean-room: spec 03")
}
