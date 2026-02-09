# rpg-encoder

[![CI](https://github.com/userFRM/rpg-encoder/workflows/CI/badge.svg)](https://github.com/userFRM/rpg-encoder/actions)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)

> [!NOTE]
> This is an **independent, community-driven implementation** inspired by the
> [RPG-Encoder paper](https://arxiv.org/abs/2602.02084) from Microsoft Research. It is **not**
> affiliated with, endorsed by, or connected to Microsoft in any way. For the official
> implementation, see [microsoft/RPG-ZeroRepo](https://github.com/microsoft/RPG-ZeroRepo).
>
> Microsoft announced *"We are in the process of preparing a full public release of the codebase,
> and all code will be released within the next two weeks."* — that was too long to wait.
> This project was built with Claude by reading the publicly available research papers and
> implementing the described algorithms from scratch in Rust. All code is original work.
> The papers are cited for attribution.

---

**Coding agent toolkit for semantic code understanding.**

rpg-encoder builds a semantic graph of your codebase. Your coding agent (Claude Code, Cursor,
etc.) analyzes the code and adds intent-level features via the MCP interactive protocol.
Search by what code *does*, not what it's named.

> [!TIP]
> **New to RPG?** See [How RPG Compares](docs/comparison.md) to understand where it fits
> alongside Claude Code, Serena, and other tools.
> For a detailed algorithm-by-algorithm comparison with the research paper, see
> [Paper Fidelity](docs/paper_fidelity.md).

## Install

Add to your MCP config (Claude Code `~/.claude.json`, Cursor settings, etc.):

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

<details>
<summary><strong>Multi-repo setup</strong></summary>

The MCP server operates on the directory passed as its first argument. For multi-repo usage:

**Option 1: Global config (single primary repo)**

Set your main development repo in `~/.claude.json`:

```json
{
  "mcpServers": {
    "rpg": {
      "command": "npx",
      "args": ["-y", "-p", "rpg-encoder", "rpg-mcp-server", "/path/to/primary/repo"]
    }
  }
}
```

**Option 2: Per-project override**

Create `.claude/mcp_servers.json` in each repo that needs RPG:

```json
{
  "rpg": {
    "type": "stdio",
    "command": "npx",
    "args": ["-y", "-p", "rpg-encoder", "rpg-mcp-server", "/path/to/this/repo"],
    "env": {}
  }
}
```

The project-level config overrides the global one. Restart Claude Code after creating/modifying configs.

</details>

## How It Works

The RPG (Repository Planning Graph) is a hierarchical, dual-view representation from the
research papers cited below:

1. **Parse** — Extract entities (functions, classes, methods) and dependency edges (imports,
   invocations, inheritance) using tree-sitter. Build a file-path hierarchy.
2. **Lift** — Your coding agent analyzes entity source code and adds verb-object semantic
   features (e.g., "validate user credentials", "serialize config to disk") via the MCP
   interactive protocol (`get_entities_for_lifting` → `submit_lift_results`).
3. **Hierarchy** — Your agent discovers functional domains and assigns entities to a 3-level
   semantic hierarchy (`build_semantic_hierarchy` → `submit_hierarchy`).
4. **Ground** — Anchor hierarchy nodes to directories via LCA algorithm, resolve cross-file
   dependency edges.

The graph is saved to `.rpg/graph.json` and **should be committed to your repo** — this way
all collaborators and AI tools get instant semantic search without rebuilding.

## MCP Tools

| Tool | Description |
|------|-------------|
| `build_rpg` | Index the codebase (run once, instant) |
| `search_node` | Search entities by intent or keywords (hybrid embedding + lexical scoring) |
| `fetch_node` | Get entity metadata, source code, dependencies, and hierarchy context |
| `explore_rpg` | Traverse dependency graph (upstream, downstream, or both) |
| `rpg_info` | Graph statistics, hierarchy overview, per-area lifting coverage |
| `update_rpg` | Incremental update from git changes |
| `lifting_status` | Dashboard — coverage, per-area progress, NEXT STEP |
| `get_entities_for_lifting` | Get entity source code for your agent to analyze |
| `submit_lift_results` | Submit the agent's semantic features back to the graph |
| `finalize_lifting` | Aggregate file-level features, rebuild hierarchy metadata |
| `get_files_for_synthesis` | Get file-level entity features for holistic synthesis |
| `submit_file_syntheses` | Submit holistic file-level summaries |
| `build_semantic_hierarchy` | Get domain discovery + hierarchy assignment prompts |
| `submit_hierarchy` | Apply hierarchy assignments to the graph |
| `get_routing_candidates` | Get entities needing semantic routing (drifted or newly lifted) |
| `submit_routing_decisions` | Submit routing decisions (hierarchy path or "keep") |
| `reload_rpg` | Reload graph from disk after external changes |

### Lifting Flow

1. Ask your agent to "lift the code" (or call `get_entities_for_lifting` with a scope)
2. The tool returns entity source code with analysis instructions
3. Your agent analyzes the code and calls `submit_lift_results` with semantic features
4. The agent continues through all batches automatically, dispatching subagents for large repos
5. After lifting, `finalize_lifting` → `build_semantic_hierarchy` → `submit_hierarchy`

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

<details>
<summary><strong>CLI</strong></summary>

The CLI provides structural operations (no semantic lifting — use the MCP server for that).

```bash
# Install
npm install -g rpg-encoder

# Build a graph
rpg-encoder build
rpg-encoder build --include "src/**/*.py" --exclude "tests/**"

# Query
rpg-encoder search "parse entities from source code"
rpg-encoder fetch "src/parser.rs:extract_entities"
rpg-encoder explore "src/parser.rs:extract_entities" --direction both --depth 2
rpg-encoder info

# Incremental update
rpg-encoder update
rpg-encoder update --since abc1234

# Pre-commit hook (auto-updates graph on every commit)
rpg-encoder hook install
```

</details>

<details>
<summary><strong>Configuration</strong></summary>

Create `.rpg/config.toml` in your project root (all fields optional):

```toml
[encoding]
batch_size = 50             # Entities per lifting batch
max_batch_tokens = 8000     # Token budget per batch
drift_threshold = 0.5       # Jaccard distance midpoint reference
drift_ignore_threshold = 0.3  # Below: minor edit, in-place update
drift_auto_threshold = 0.7    # Above: auto-queue for re-routing

[navigation]
search_result_limit = 10
```

</details>

<details>
<summary><strong>Architecture</strong></summary>

```
rpg-encoder/
├── rpg-core        Core graph types (RPGraph, Entity, HierarchyNode), storage, LCA
├── rpg-parser      Tree-sitter entity + dependency extraction (8 languages)
├── rpg-encoder     Encoding pipeline, semantic lifting utilities, incremental evolution
│   └── prompts/        Prompt templates (embedded via include_str!)
├── rpg-nav         Search, fetch, explore, TOON serialization
├── rpg-cli         CLI binary (rpg-encoder)
└── rpg-mcp         MCP server binary (rpg-mcp-server)
```

</details>

<details>
<summary><strong>How It Compares</strong></summary>

| Aspect | Paper (Microsoft) | This Repo |
|--------|-------------------|-----------|
| Implementation | Python (unreleased) | Rust (available now) |
| Lifting strategy | Full upfront via API | Progressive — your coding agent lifts via MCP |
| Semantic routing | LLM-based | LLM-based (via MCP routing protocol) |
| Feature search | Embedding-based | Hybrid embedding + lexical (BGE-small-en-v1.5) |
| MCP server | Described, not shipped | Working, with 17 tools |
| SWE-bench evaluation | 93.7% Acc@5 | Not yet evaluated |
| Languages | Python-focused | 8 languages |
| TOON format | Not described | Implemented for token efficiency |

</details>

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
