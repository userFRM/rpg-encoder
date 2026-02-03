# rpg-encoder

[![CI](https://github.com/userFRM/rpg-encoder/workflows/CI/badge.svg)](https://github.com/userFRM/rpg-encoder/actions)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)

> **Disclaimer**: This is an **independent, community-driven implementation** inspired by the
> [RPG-Encoder paper](https://arxiv.org/abs/2602.02084) from Microsoft Research. It is **not**
> affiliated with, endorsed by, or connected to Microsoft in any way. For the official
> implementation, see [microsoft/RPG-ZeroRepo](https://github.com/microsoft/RPG-ZeroRepo).
>
> This project was built by reading the publicly available research papers and implementing the
> described algorithms from scratch in Rust. All code is original work. The papers are cited
> for attribution.

---

**Build a semantic code graph of your repository. Search by intent, not keywords.**

rpg-encoder extracts entities (functions, classes, methods) from your codebase, resolves
dependency edges (imports, invocations, inheritance), and optionally lifts semantic features
via LLM — producing a navigable graph optimized for AI-assisted code understanding.

It works out of the box with [Ollama](https://ollama.com) (free, local) or API providers
(Anthropic, OpenAI), and exposes the graph as an MCP server for Claude Code, Cursor, and
other AI coding tools.

## How It Works

The RPG (Repository Planning Graph) is a hierarchical, dual-view representation introduced in
the research papers cited below:

- **V_L (Low-level nodes)**: Code entities — functions, classes, methods — with semantic
  features describing their intent
- **V_H (High-level nodes)**: Functional areas — hierarchical groupings discovered by LLM
  or derived from file paths
- **E_dep (Dependency edges)**: imports, invocations, inheritance between entities
- **E_feature (Containment edges)**: hierarchy parent-child relationships

The encoding follows a three-phase pipeline:

1. **Semantic Lifting** — Parse code with tree-sitter, enrich with LLM-generated verb-object
   features (e.g., "validate user credentials", "serialize config to disk")
2. **Structure Reorganization** — Discover functional domains and build a semantic hierarchy
3. **Artifact Grounding** — Anchor hierarchy nodes to directories via LCA algorithm, resolve
   cross-file dependency edges

## Quick Start

### Prerequisites

- **Rust 1.85+** — [Install](https://rustup.rs)
- **Ollama** (optional, for semantic lifting) — [Install](https://ollama.com)

### Install

```bash
# Via npm (no Rust needed — downloads pre-built binary)
npm install -g rpg-encoder

# Or build from source
git clone https://github.com/userFRM/rpg-encoder.git
cd rpg-encoder
cargo build --release
```

### Build a Graph

```bash
# Auto-detects language, extracts entities, builds hierarchy
rpg-encoder build

# With semantic lifting (requires Ollama or API key)
rpg-encoder build --lift

# Filter files
rpg-encoder build --include "src/**/*.py" --exclude "tests/**"
```

The graph is saved to `.rpg/graph.json` (auto-added to `.gitignore`).

### Progressive Lifting

You don't need to lift the entire repo at once. Lift the areas you're working with:

```bash
# Lift a specific directory
rpg-encoder lift "src/auth/**"

# Lift a hierarchy area
rpg-encoder lift "Auth/login"

# Lift everything
rpg-encoder lift "*"

# Check coverage
rpg-encoder info
```

### Query the Graph

```bash
# Search by intent
rpg-encoder search "parse entities from source code"

# Fetch entity details with source code
rpg-encoder fetch "src/parser.rs:extract_entities"

# Explore dependency graph
rpg-encoder explore "src/parser.rs:extract_entities" --direction both --depth 2

# Show graph statistics and lifting coverage
rpg-encoder info
```

### Incremental Updates

```bash
# Update from git changes since last build
rpg-encoder update

# Update from a specific commit
rpg-encoder update --since abc1234
```

## MCP Server

The MCP server gives AI coding tools (Claude Code, Cursor, etc.) full semantic understanding
of your codebase.

### Setup

No Rust required — just Node.js:

```json
{
  "mcpServers": {
    "rpg": {
      "command": "npx",
      "args": ["-y", "-p", "rpg-encoder", "rpg-mcp-server", "/path/to/your/project"]
    }
  }
}
```

Add this to your MCP config (e.g., Claude Code `~/.claude.json`, Cursor settings, etc.).

<details>
<summary>Alternative: build from source</summary>

```bash
git clone https://github.com/userFRM/rpg-encoder.git
cd rpg-encoder && cargo build --release
```

Then use the binary path directly:

```json
{
  "mcpServers": {
    "rpg": {
      "command": "/path/to/rpg-encoder/target/release/rpg-mcp-server",
      "args": ["/path/to/your/project"]
    }
  }
}
```

</details>

### Tools

| Tool | Description |
|------|-------------|
| `build_rpg` | Build the graph from source code (set `embed=true` for vector search) |
| `search_node` | Search entities by intent or keywords (modes: features, snippets, auto, semantic, hybrid) |
| `fetch_node` | Get entity metadata, source code, dependencies, and hierarchy context |
| `explore_rpg` | Traverse dependency graph (upstream, downstream, or both) |
| `rpg_info` | Graph statistics, hierarchy overview, per-area lifting coverage |
| `update_rpg` | Incremental update from git changes |
| `lift_area` | Semantically lift entities in a scope via local LLM |
| `get_entities_for_lifting` | Get entity source for Claude-as-Lifter (no LLM setup needed) |
| `submit_lift_results` | Submit Claude's semantic analysis back to the graph |
| `generate_embeddings` | Generate vector embeddings for semantic/hybrid search |
| `reload_rpg` | Reload graph from disk after external changes |

### Claude-as-Lifter

If you use Claude Code, you can lift entities without any LLM setup. Claude analyzes the code
directly:

1. Claude calls `get_entities_for_lifting` with a scope (e.g., `"src/auth/**"`)
2. The tool returns entity source code with instructions
3. Claude analyzes the code and calls `submit_lift_results` with semantic features
4. Repeat for the next batch until all entities are lifted

This produces frontier-quality semantic features at zero additional cost.

## Supported Languages

| Language | Entity Extraction | Dependency Resolution |
|----------|------------------|----------------------|
| Python | Functions, classes, methods | imports, calls, inheritance |
| Rust | Functions, structs, traits, impl methods | use statements, calls, trait impls |
| TypeScript | Functions, classes, methods, interfaces | imports, calls, inheritance |
| JavaScript | Functions, classes, methods | imports, calls, inheritance |
| Go | Functions, structs, methods, interfaces | imports, calls |
| Java | Classes, methods, interfaces | imports, calls, inheritance |
| C | Functions, structs | includes, calls |
| C++ | Functions, classes, methods, structs | includes, calls, inheritance |

## LLM Configuration

Semantic lifting is optional. Without it, the graph still has entities, dependencies, and
file-path hierarchy — just no semantic features for intent-based search.

### Provider Priority (auto-detected)

| Priority | Provider | Setup | Feature Quality |
|----------|----------|-------|-----------------|
| 1 | Anthropic | `export ANTHROPIC_API_KEY=...` | Excellent |
| 2 | OpenAI | `export OPENAI_API_KEY=...` | Excellent |
| 3 | Ollama (local) | Just install [Ollama](https://ollama.com) | Good (model-dependent) |
| 4 | Any OpenAI-compatible server | `export RPG_LOCAL_URL=...` | Varies |

With Ollama, the default model (`qwen3:0.6b`, 522 MB) is auto-pulled on first use. For
better quality, pull a larger model:

```bash
ollama pull qwen2.5-coder:7b
```

Then set it in `.rpg/config.toml`:

```toml
[llm]
local_model = "qwen2.5-coder:7b"
```

### Configuration

Create `.rpg/config.toml` in your project root (all fields optional):

```toml
[llm]
local_model = "qwen3:0.6b"     # Ollama model name
local_url = "http://localhost:11434"
auto_pull = true                 # Auto-pull model if missing

[encoding]
batch_size = 8                   # Entities per LLM batch

[navigation]
search_limit = 20                # Default search result limit
explore_depth = 3                # Default dependency traversal depth
```

## Architecture

```
rpg-encoder/
├── rpg-core        Core graph types (RPGraph, Entity, HierarchyNode), storage, LCA
├── rpg-parser      Tree-sitter entity + dependency extraction (8 languages)
├── rpg-encoder     3-phase pipeline, LLM integration, incremental evolution
│   ├── llm/            Provider abstraction (Anthropic, OpenAI, Ollama, local)
│   └── prompts/        LLM prompt templates (embedded via include_str!)
├── rpg-nav         Search, fetch, explore, TOON serialization
├── rpg-cli         CLI binary (rpg-encoder)
└── rpg-mcp         MCP server binary (rpg-mcp-server)
```
## How It Compares

This implementation covers the core algorithms from the paper. Key differences:

| Aspect | Paper (Microsoft) | This Repo |
|--------|-------------------|-----------|
| Implementation | Python (unreleased) | Rust (available now) |
| Default LLM | GPT-4o | Ollama qwen3:0.6b (free) or any provider |
| Lifting strategy | Full upfront | Progressive (lift what you need) |
| MCP server | Described, not shipped | Working, with 10 tools |
| Claude-as-Lifter | Not described | Supported (zero LLM setup) |
| SWE-bench evaluation | 93.7% Acc@5 | Not yet evaluated |
| Languages | Python-focused | 8 languages |
| TOON format | Not described | Implemented for token efficiency |

## References

This project is based on the following research papers. All credit for the theoretical
framework, algorithms, and evaluation methodology belongs to the original authors.

- **RPG-Encoder**: Luo, J., Yin, C., Zhang, X., et al. "Closing the Loop: Universal
  Repository Representation with RPG-Encoder." arXiv:2602.02084, 2026.
  [[Paper]](https://arxiv.org/abs/2602.02084)
  [[Project Page]](https://ayanami2003.github.io/RPG-Encoder/)
  [[Official Code]](https://github.com/microsoft/RPG-ZeroRepo)

- **RPG (ZeroRepo)**: Luo, J., Yin, C., et al. "RepoGraph: Enhancing AI Software Engineering
  with Repository-level Code Graph." arXiv:2509.16198, 2025.
  [[Paper]](https://arxiv.org/abs/2509.16198)

- **TOON**: Token-Oriented Object Notation — an LLM-optimized data format used for MCP
  tool output and LLM response parsing.
  [[Spec]](https://github.com/toon-format/toon)

## License

Licensed under the [MIT License](LICENSE).

This is an independent implementation. The RPG-Encoder paper and its associated intellectual
property belong to Microsoft Research and the paper's authors. This project implements the
publicly described algorithms and does not contain any code from Microsoft.
