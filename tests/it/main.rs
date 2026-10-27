//! The one integration-test binary: its modules share the fixtures.
//! Every test here shows PDFs, so needs the pdfcraft backend.
#![cfg(feature = "pdfcraft")]

mod fixtures;
mod pdfcraft_pin;
mod view;
mod view_pager;
mod view_tmux;
