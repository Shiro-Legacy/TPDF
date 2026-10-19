//! The headless Tandem PDF app: `tandem-headless --smoke …` and
//! `tandem-headless --drive=<script.jsonl|->` (the protocol of
//! spec-drive-state.md §1–2; see `tandem_app::drive`).
//!
//! The document session plugs in here: replace `Unwired::new` with the
//! session's constructor once `session::Session` implements `drive::Driven`.

fn main() {
    tandem_app::drive::main(tandem_app::drive::Unwired::new)
}
