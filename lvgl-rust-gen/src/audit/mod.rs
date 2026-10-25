//! The access audit (feature `cli`, maintainers only): builds a call graph
//! of LVGL from its sources compiled to LLVM IR, and derives the access of
//! every public function from the deletion sinks it can reach and from the
//! memory held across event sends (`held`), and what handlers and senders
//! of each event code may do (`events`).

pub mod events;
pub mod graph;
pub mod held;
pub mod ir;
