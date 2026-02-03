//! Tree-sitter integration for multi-language AST parsing.

use anyhow::{Context, Result};
use std::path::Path;

/// Parse a source file and return the tree-sitter tree.
pub fn parse_file(
    path: &Path,
    source: &[u8],
    language: tree_sitter::Language,
) -> Result<tree_sitter::Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&language)
        .context("failed to set tree-sitter language")?;
    parser
        .parse(source, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse {}", path.display()))
}
