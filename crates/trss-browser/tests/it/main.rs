//! The integration tests of trss-browser, one module per file, linked as one
//! binary (docs/adr/0014-one-integration-test-binary-per-crate.md).

mod support;

mod compose;
mod docker;
mod launcher;
mod pool;
