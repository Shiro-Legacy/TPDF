// SPDX-License-Identifier: MIT
//! Redaction: marks (Redact annotations) and what they cover. `marks` reads
//! and writes the marks redact-add, redact-search and redact-set place;
//! `content` walks page content into draws with byte spans and page-space
//! bounds; `region` maps marks onto those draws, onto annotations, and
//! finds marked objects drawn elsewhere without a mark; `inventory` counts
//! the surfaces a sanitize clears; `inspect` lists what apply would remove
//! under boxes (redact-inspect, the "show what's underneath" tint). The ops are `app::ops::redact`.
pub mod apply;
pub mod content;
pub mod fonts;
pub mod geometry;
pub mod images;
pub mod inspect;
pub mod inventory;
pub mod marks;
pub mod private;
pub mod region;
pub mod sanitize;
pub mod verify;
