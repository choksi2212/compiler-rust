//! Shared compiler stages used by the CLI and execution backends.
pub mod bytecode;
pub mod ir;
pub mod lexer;
pub mod native_pe;
pub mod optimize;
pub mod parser;
pub mod semantic;
#[cfg(test)]
mod stress;

pub mod context;
pub mod verify;
