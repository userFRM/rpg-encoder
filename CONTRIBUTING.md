# Contributing to rpg-encoder

Thank you for your interest in contributing! This project is an independent, community-driven
implementation of the RPG-Encoder concepts described in the
[research papers](https://arxiv.org/abs/2602.02084). Contributions of all kinds are welcome.

## Important Context

This project is **not affiliated with Microsoft Research**. It is an independent implementation
built from publicly available research papers. When contributing:

- Do not include any code from the official Microsoft implementation
- Reference the papers for algorithmic details, not any proprietary code
- Ensure all contributions are your own original work or properly licensed

## Getting Started

### Prerequisites

- Rust 1.85+ (install via [rustup](https://rustup.rs))

### Build and Test

```bash
# Build all crates
cargo build --workspace

# Run all tests (379+ test cases)
cargo test --workspace

# Check for lint issues
cargo clippy --workspace --all-targets -- -D warnings

# Check formatting
cargo fmt --all -- --check
```

All four checks must pass before submitting a PR. CI enforces these automatically.

## Project Structure

```
crates/
├── rpg-core       Core types: RPGraph, Entity, HierarchyNode, storage, config
├── rpg-parser     Tree-sitter parsing for 8 languages (entities + dependencies)
├── rpg-encoder    LLM integration, 3-phase pipeline, incremental evolution
├── rpg-nav        Navigation: search, fetch, explore, TOON serialization
├── rpg-cli        CLI binary
└── rpg-mcp        MCP server binary
```

## What to Contribute

### High-Impact Areas

- **Benchmarks** — Evaluate search/localization quality on real repositories
- **Language support** — Add tree-sitter extractors for new languages (Ruby, Kotlin, Swift, etc.)
- **Search quality** — Improve scoring, ranking, and result relevance
- **Documentation** — Usage guides, tutorials, examples

### Adding a New Language

1. Add the tree-sitter grammar dependency to `Cargo.toml`
2. Implement entity extraction in `crates/rpg-parser/src/entities/`
3. Implement dependency extraction in `crates/rpg-parser/src/deps/`
4. Register the language in `crates/rpg-parser/src/languages.rs`
5. Add tests in `crates/rpg-parser/tests/`

See existing language implementations (e.g., `python_entities.rs`, `rust_deps.rs`) as templates.

### Adding an MCP Tool

1. Add the handler method in `crates/rpg-mcp/src/main.rs` with `#[tool(...)]` attribute
2. Define a params struct with `JsonSchema` + `Deserialize`
3. Add a test in `crates/rpg-mcp/tests/tool_handlers.rs`

## Code Style

- Follow existing patterns in the codebase
- Use `anyhow::Result` for fallible functions in binaries, `thiserror` for library errors
- Keep functions focused — prefer small, testable units
- Write tests for new functionality
- No unnecessary abstractions — solve the problem at hand

## Pull Request Process

1. Fork the repo and create a feature branch
2. Make your changes with tests
3. Ensure all checks pass: `cargo test && cargo clippy -- -D warnings && cargo fmt -- --check`
4. Open a PR with a clear description of what and why
5. Link any related issues

## Reporting Issues

- Use GitHub Issues for bugs, feature requests, and questions
- Include steps to reproduce for bugs
- Include relevant version info (`rpg-encoder info`, Rust version, OS)

## License

By contributing, you agree that your contributions will be licensed under the
[MIT License](LICENSE).
