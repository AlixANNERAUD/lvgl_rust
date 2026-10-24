//! The access audit (feature `cli`, maintainers only): builds a call graph
//! of LVGL from its sources compiled to LLVM IR, and derives the access of
//! every public function from the deletion sinks it can reach.

pub mod graph;
pub mod ir;
