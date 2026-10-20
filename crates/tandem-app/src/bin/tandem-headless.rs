//! The headless Tandem PDF app: `tandem-headless --smoke …` and
//! `tandem-headless --drive=<script.jsonl|->` (the protocol of
//! spec-drive-state.md §1–2; see `tandem_app::drive`), driving the document
//! session (`tandem_app::session::Session`).

fn main() {
    tandem_app::drive::main(tandem_app::session::Session::for_drive)
}
