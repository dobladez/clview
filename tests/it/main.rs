//! The one integration-test binary: its modules share the fixtures. Every test here shows PDFs,
//! so needs the PDF backend (the `pdf` feature).
#![cfg(feature = "pdf")]

mod fixtures;
mod pdfcraft;
mod pdfcraft_pin;
mod resvg;
mod tar;
mod view;
mod view_pager;
mod view_tmux;
mod zip;
