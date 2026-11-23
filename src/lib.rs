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
pub mod pe;
pub mod verify;
pub mod x64;

pub mod abi;
pub mod native_integer;
pub mod native_runtime;
pub mod pipeline;
