//! Atomics used by the registry: loom's under `cfg(loom)`, core's otherwise.
//! Only loads and stores are used (no read-modify-write), so the registry
//! works on targets without compare-and-swap (`thumbv6m`, `riscv32imc`).

#[cfg(not(loom))]
#[cfg_attr(lvgl_rust_views_only, allow(unused_imports))]
pub(crate) use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering, fence};
#[cfg(loom)]
#[cfg_attr(lvgl_rust_views_only, allow(unused_imports))]
pub(crate) use loom::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering, fence};
