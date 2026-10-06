//! The integration tests of trss-jobs, one module per file, linked as one
//! binary (docs/adr/0014-one-integration-test-binary-per-crate.md).
//! `tests/extract_process.rs` stays a binary of its own: its tests write
//! scripts and run them, which fails ("text file busy") when another thread
//! of the process starts a process meanwhile.

mod airtime_sample;
mod blogger;
mod choose;
mod cleanup;
mod drive_fonts;
mod find;
mod follow;
mod naver;
mod place;
mod placement;
mod recheck;
mod replace;
mod replace_many;
mod runner;
mod screen;
mod tistory;
mod unpack;
mod upload;
mod winpng;
