//! Experimental safe Rust PDF engine with bounded rendering and text-edit sessions.
#![forbid(unsafe_code)]

pub mod app;
pub mod builtin_metrics;
pub mod builtin_outlines;
pub mod content;
pub mod document;
mod encrypt;
pub mod font_metrics;
pub mod incremental;
pub mod lexer;
pub mod object;
pub mod parser;
pub mod repair;
pub mod security;
pub mod serialize;
pub mod simple_encoding;
pub mod stream;
pub mod stream_object;
pub mod view;
pub mod writer;
