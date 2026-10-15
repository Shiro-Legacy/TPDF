//! Experimental safe Rust PDF engine with bounded rendering and text-edit sessions.
#![forbid(unsafe_code)]

pub mod app;
pub mod builtin_metrics;
pub mod builtin_outlines;
pub mod cmap;
pub mod content;
pub mod document;
pub mod embedded_glyph;
mod encrypt;
pub mod font_metrics;
pub mod font_resources;
pub mod font_text;
pub mod form_resources;
pub mod generated_appearance;
pub mod geometry;
pub mod image_resources;
pub mod image_sampling;
pub mod lexer;
pub mod object;
pub mod operators;
pub mod optional_content;
pub mod page_content;
pub mod page_geometry;
pub mod page_glyph;
pub mod page_program;
pub mod page_raster;
pub mod pages;
pub mod parser;
pub mod path;
pub mod program;
pub mod repair;
pub mod security;
pub mod serialize;
pub mod simple_encoding;
pub mod stream;
pub mod stream_object;
pub mod text_decode;
pub mod text_font;
pub mod text_position;
pub mod truetype_cmap;
pub mod truetype_metrics;
pub mod truetype_outline;
pub mod view;
pub mod writer;

#[cfg(feature = "raster")]
pub mod raster;

pub mod scan_raster;
pub mod stroke;

pub mod session;
