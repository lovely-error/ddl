// The DDL compiler, as a library.
//
// `main.rs` is a thin argument parser over this; everything else -- tests,
// and any tool that wants to compile DDL without spawning a process -- links
// here. Nothing in the compilation path prints or exits: a stage either
// returns a value or returns diagnostics, and the caller decides.
#![feature(decl_macro)]
#![feature(str_from_raw_parts)]

pub mod diag;
pub mod dot;
pub mod driver;
pub mod fmt;

#[allow(unsafe_op_in_unsafe_fn)]
pub mod lex;
#[allow(unsafe_op_in_unsafe_fn)]
pub mod parse;

pub mod ir;
pub mod ir_adapt;
pub mod ir_comb;
pub mod ir_export;
pub mod ir_fsm;
pub mod ir_graph;
pub mod ir_match;
pub mod ir_pipe;
mod ir_scope;
pub mod source;
pub mod symbols;
pub mod ty;
pub mod verilog;

#[cfg(test)]
mod tests_m2;
