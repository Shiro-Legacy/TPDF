//! Temporary: lane 2's UI entry points, until lane 2 merges (pruned then).
#![allow(unused_variables)]
use super::Session;

impl Session {
    #[allow(clippy::too_many_arguments)]
    pub fn pointer(&mut self, phase: &str, page: usize, x: f64, y: f64, shift: bool, alt: bool, meta: bool) {
        todo!("lane 2: pointer")
    }
    pub fn key(&mut self, key: &str, shift: bool, alt: bool, meta: bool) {
        todo!("lane 2: key")
    }
}
