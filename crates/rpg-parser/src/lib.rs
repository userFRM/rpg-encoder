//! Tree-sitter based code parsing for RPG entity and dependency extraction.
//!
//! Supports Python and Rust. Extracts functions, classes, methods, traits,
//! import statements, function calls, and inheritance relationships.

pub mod deps;
pub mod entities;
pub mod languages;
pub mod treesitter;
