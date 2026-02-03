# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.1.0] - 2026-02-05

Initial public release. Independent Rust implementation of the RPG-Encoder framework
described in [arXiv:2602.02084](https://arxiv.org/abs/2602.02084).

### Core Pipeline

- **Semantic Lifting** (Phase 1) — Parse code with tree-sitter, enrich entities with
  LLM-generated verb-object features following 11 naming rules and 6 extraction principles
- **Structure Reorganization** (Phase 2) — Discover functional domains via LLM, build
  3-level semantic hierarchy with compatibility-checked routing (Algorithm 3)
- **Artifact Grounding** (Phase 3) — Anchor hierarchy nodes to directories via LCA algorithm,
  resolve cross-file dependency edges (imports, invocations, inheritance)

### Language Support

- 8 languages via tree-sitter: Python, Rust, TypeScript, JavaScript, Go, Java, C, C++
- Per-language entity extraction (functions, classes, methods, structs, traits, interfaces)
- Per-language dependency resolution (imports, calls, inheritance, trait impls)

### LLM Integration

- Multi-provider support: Anthropic, OpenAI, Moonshot (Kimi), Ollama, any OpenAI-compatible server
- Auto-detection priority chain with zero-config Ollama fallback
- Configurable provider forcing via `config.toml` or environment variables
- Auto-pull Ollama models on first use (configurable)
- Retry with exponential backoff and format-correction retry on JSON parse failures
- Schema validation with missing-entity re-extraction
- Prompt templates externalized to `.md` files (embedded via `include_str!`)
- Temperature control (`0.0` for deterministic local inference)
- `/no_think` suffix to suppress `<think>` blocks from reasoning models (qwen3, deepseek)

### Incremental Evolution

- Git-diff-based incremental updates (Algorithms 2-4 from the paper)
- Deletion pruning with hierarchy cleanup
- Modification with semantic drift detection (Jaccard + embedding cosine distance)
- LLM drift judge for ambiguous cases (only called within +/-20% of threshold)
- Addition with semantic routing to existing hierarchy
- New entities in modified files routed to hierarchy (not just additions)

### Navigation & Search

- **search_node** — Intent-based search across 5 modes: features, snippets, auto, semantic, hybrid
- **fetch_node** — Entity details with source code, dependencies, hierarchy context; V_H hierarchy
  node fetch support
- **explore_rpg** — Dependency graph traversal (upstream/downstream/both) with configurable depth
  and edge filtering by kind (imports, invokes, inherits, contains)
- **rpg_info** — Graph statistics, hierarchy overview, per-area lifting coverage
- Cross-view traversal between V_L (code entities) and V_H (hierarchy nodes)
- TOON (Token-Oriented Object Notation) serialization for token-efficient LLM output

### Embedding Search

- Multi-provider embeddings: Ollama, OpenAI, or local fastembed (zero-setup)
- Hybrid search with configurable `semantic_weight` (keyword + embedding blend)
- Per-query weight override in MCP search tool

### MCP Server

- 10 tools: `search_node`, `fetch_node`, `explore_rpg`, `rpg_info`, `build_rpg`, `update_rpg`,
  `lift_area`, `get_entities_for_lifting`, `submit_lift_results`, `reload_rpg`
- **Claude-as-Lifter**: lift entities using Claude directly via `get_entities_for_lifting` +
  `submit_lift_results` — produces frontier-quality features at zero additional LLM cost
- Semantic/hybrid search modes with live embedding generation

### CLI

- Commands: `build`, `update`, `lift`, `search`, `fetch`, `explore`, `info`
- `--include` / `--exclude` glob filtering for builds
- `--since` commit override for incremental updates
- Progressive lifting by scope (`lift "src/auth/**"`, `lift "*"`)

### Configuration

- `.rpg/config.toml` with sections: `[llm]`, `[encoding]`, `[navigation]`, `[embeddings]`
- Configurable batch size, search limits, explore depth, retry behavior
- Feature normalization: trim, lowercase, sort+dedup per paper spec

### Code Quality

- 223 test cases across 6 crates (~3,100 lines of test code)
- Clean `cargo clippy --workspace --all-targets -- -D warnings`
- Clean `cargo fmt --all -- --check`
- Modular crate architecture: rpg-core, rpg-parser, rpg-encoder, rpg-nav, rpg-cli, rpg-mcp
- LLM module split into providers, Ollama auto-detection, and client submodules

### Benchmarks

- Search quality benchmark suite with 20 intent queries
- Acc@k and MRR metrics comparing unlifted vs lifted search
- Lifting improves Acc@1 by +15% and MRR by +0.128 (tested with Kimi k2.5)
