//! Compile-fail cases: programs the access model must reject, each with
//! the compiler error it gets (`tests/compile-fail/**/*.stderr`), and
//! their accepted counterparts (`tests/compile-pass/`).
//!
//! The expected messages are rustc's, so they change with the compiler:
//! the test is ignored by a plain `cargo test` and run by
//! `mise run test-compile-fail`, on the toolchain that task pins (refresh
//! the snapshots with `TRYBUILD=overwrite` when raising it).
//!
//! Some refusals are `const` assertions (a closure with captures given to
//! `on`, `timer` or `post`), which fail only when the generic function is
//! instantiated, during code generation: `cargo check` accepts those
//! programs. trybuild builds every case with `cargo build` as soon as one
//! pass case exists, so the compile-pass case is also what makes those
//! cases fail.

#[test]
#[ignore = "rustc's messages change between versions: run by `mise run test-compile-fail`"]
#[cfg(not(lvgl_rust_views_only))]
fn compile_fail() {
    let cases = trybuild::TestCases::new();
    // Keep a pass case: without one, trybuild only checks.
    cases.pass("tests/compile-pass/*.rs");
    cases.compile_fail("tests/compile-fail/*.rs");
    #[cfg(feature = "async")]
    cases.compile_fail("tests/compile-fail/async/*.rs");
    #[cfg(feature = "heap")]
    cases.compile_fail("tests/compile-fail/heap/*.rs");
}
