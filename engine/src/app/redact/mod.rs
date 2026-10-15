// SPDX-License-Identifier: AGPL-3.0-or-later
//! Redaction: marks (Redact annotations) and what they cover. `marks` reads
//! and writes the marks redact-add, redact-search and redact-set place;
//! `content` walks page content into draws with byte spans and page-space
//! bounds; `region` maps marks onto those draws, onto annotations, and
//! finds marked objects drawn elsewhere without a mark; `inventory` counts
//! the surfaces a sanitize clears. The ops are `app::ops::redact`.
pub mod apply;
pub mod content;
pub mod fonts;
pub mod geometry;
pub mod images;
pub mod inventory;
pub mod marks;
pub mod region;
pub mod sanitize;
pub mod verify;
