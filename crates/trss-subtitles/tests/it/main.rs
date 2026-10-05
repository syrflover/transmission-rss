//! The integration tests of trss-subtitles, one module per file, linked as one
//! binary (docs/adr/0014-one-integration-test-binary-per-crate.md).
//! The modules that need `test-hooks` are left out without it.

mod auth_sample;
#[cfg(feature = "test-hooks")]
mod erulabo_sample;
mod find_sample;
#[cfg(feature = "test-hooks")]
mod winpng_sample;
