# rpg-encoder

[![CI](https://github.com/userFRM/rpg-encoder/workflows/CI/badge.svg)](https://github.com/userFRM/rpg-encoder/actions)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)

**Give your AI agent a brain for your codebase.**

rpg-encoder builds a semantic graph of your codebase — not just what calls what, but what
every function *does* and *why it exists*. Your coding agent lifts each entity into
intent-level features, then searches by meaning, not naming conventions.

The result: an LLM that starts every session already knowing your entire repo.

## The Problem: Your LLM Is Flying Blind

Without rpg-encoder, coding agents spend **70%+ of their tool calls** just figuring out what
your codebase does:

```
Typical 48K-call session without RPG:

  Bash   24,189  (50%)   grep, cat, find, ls — fumbling through files
  Read    7,866  (16%)   reading files without knowing which ones matter
  Grep    2,061   (4%)   text search when semantic search finds it in one call
  Glob      280   (1%)   finding files by name pattern
  ─────────────────────
  Total  34,396  (71%)   wasted on "where is the code and what does it do?"
```

Every `grep` is an admission the LLM doesn't know where things are. Every `cat` is an
admission it doesn't know what's in the file. Every `find` is an admission it doesn't
know the structure.

**With rpg-encoder:** The LLM calls `semantic_snapshot` once, reads ~25K tokens, and
knows every function's purpose, every dependency chain, every area of the codebase.
Those 34,000 exploration calls collapse into *one*.

## What Makes This Different

**Semantic understanding, not structural graphs.** Tools like GitNexus, CodeGraphContext, and
Serena build call graphs and import maps. rpg-encoder builds an *intent graph* — LLM-lifted
verb-object features that capture what code does ("validate JWT tokens", "serialize config to
disk"), not just what it's named. Search by what you mean, find code that isn't named for what
it does.

**Context injection, not tool queries.** The `semantic_snapshot` tool compresses your entire
repo's understanding into ~25K tokens — hierarchy, features, dependencies — and injects it
into the LLM's context window. The LLM reads it once and *knows the repo*. No tool calls
needed for understanding, only for fetching source code when editing.

**Self-maintaining graph.** The server auto-syncs when git HEAD moves (commits, merges,
rebases). Uncommitted changes are detected and surfaced but not auto-applied — call
`update_rpg` to sync those.

**Claude Code hooks.** PreToolUse hooks auto-inject semantic context before every file edit.
PostToolUse hooks auto-update the graph after every git commit. The LLM never has to
remember to use RPG — it's wired into the workflow.

## Install

Add to your MCP config (Claude Code `~/.claude.json`, Cursor, opencode, etc.):

```json
{
  "mcpServers": {
    "rpg": {
      "command": "npx",
      "args": ["-y", "-p", "rpg-encoder", "rpg-mcp-server"]
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
      "command": "/path/to/rpg-encoder/target/release/rpg-mcp-server"
    }
  }
}
```

</details>

<details>
<summary><strong>Multi-repo setup</strong></summary>

The server defaults to the current working directory. MCP clients launch the server from the
workspace directory, so no path argument is needed.

For an explicit path override:

```json
{
  "mcpServers": {
    "rpg": {
      "command": "npx",
      "args": ["-y", "-p", "rpg-encoder", "rpg-mcp-server", "/path/to/repo"]
    }
  }
}
```

</details>

## Getting Started

Tell your coding agent:

> "Build and lift the RPG for this repo"

That's it. The agent handles everything:

1. **Build** — Indexes all code entities and dependencies (~5 seconds)
2. **Lift** — Agent analyzes each function/class and adds semantic features (~2 min per 100 entities)
3. **Organize** — Agent discovers functional domains and builds a semantic hierarchy (~30 seconds)
4. **Save** — Graph is written to `.rpg/graph.json` — commit it so everyone benefits

Once lifted:

- *"What handles authentication?"* — finds code even if nothing is named "auth"
- *"Show me everything that depends on the database connection"*
- *"Plan a change to add rate limiting to API endpoints"*

## MCP Tools (27)

**Build & Maintain**

| Tool | Description |
|------|-------------|
| `build_rpg` | Index the codebase (run once, instant) |
| `auto_lift` | One-call autonomous lifting via cheap LLM API (Haiku, GPT-4o-mini, OpenRouter, Gemini) |
| `update_rpg` | Incremental update from git changes |
| `reload_rpg` | Reload graph from disk after external changes |
| `rpg_info` | Graph statistics, hierarchy overview, per-area lifting coverage |

**Navigate & Search**

| Tool | Description |
|------|-------------|
| `semantic_snapshot` | Whole-repo semantic understanding in one call (~25K tokens for 1000 entities) |
| `search_node` | Search entities by intent or keywords (hybrid embedding + lexical scoring) |
| `fetch_node` | Get entity metadata, source code, dependencies, and hierarchy context |
| `explore_rpg` | Traverse dependency graph (upstream, downstream, or both) |
| `context_pack` | Single-call search+fetch+explore with token budget |

**Plan & Analyze**

| Tool | Description |
|------|-------------|
| `impact_radius` | BFS reachability analysis — "what depends on X?" |
| `plan_change` | Change planning — find relevant entities, modification order, blast radius |
| `find_paths` | K-shortest dependency paths between two entities |
| `slice_between` | Extract minimal connecting subgraph between entities |
| `analyze_health` | Code health: coupling, instability, god objects, clone detection |
| `detect_cycles` | Find circular dependencies and architectural cycles |
| `reconstruct_plan` | Dependency-safe reconstruction execution plan |

**Semantic Lifting** (10 tools)

| Tool | Description |
|------|-------------|
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

### Lifting: What It Is

Lifting is the process where an LLM reads each function, class, and method in your codebase
and describes what it does — verb-object features like "validate user credentials" or
"serialize config to disk". These features power semantic search: find code by what it *does*,
not what it's named.

**Two ways to lift:**

| Mode | How | Cost | Speed |
|------|-----|------|-------|
| **Agent lifting** | Your coding agent (Claude Code, Cursor) does the analysis via MCP | Free (uses subscription) | ~2 min per 100 entities |
| **API lifting** | `auto_lift` calls a cheap external LLM directly | ~$0.02 per 100 entities (Haiku) | ~1 min per 100 entities |

API lifting supports any OpenAI-compatible endpoint:

```
auto_lift(provider="anthropic", api_key="sk-ant-...", scope="*")
auto_lift(provider="openai", api_key="sk-...", model="gpt-4o-mini")
auto_lift(provider="openai", api_key="sk-or-...", base_url="https://openrouter.ai/api/v1", model="anthropic/claude-haiku")
auto_lift(provider="openai", api_key="...", base_url="https://generativelanguage.googleapis.com/v1beta/openai", model="gemini-2.0-flash")
```

Use `dry_run=true` to estimate cost before lifting.

- **One-time cost** — lift once, commit `.rpg/`, and every future session starts instantly
- **Resumable** — if interrupted, `lifting_status` picks up exactly where you left off
- **Incremental** — after code changes, the server auto-syncs and tracks what needs re-lifting
- **Scoped** — lift the whole repo or just a subdirectory (`"src/auth/**"`)

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
| C# | Classes, methods, interfaces | using, calls, inheritance |
| PHP | Functions, classes, methods | use, calls, inheritance |
| Ruby | Classes, methods, modules | require, calls, inheritance |
| Kotlin | Functions, classes, methods | imports, calls, inheritance |
| Swift | Functions, classes, structs, protocols | imports, calls, inheritance |
| Scala | Functions, classes, objects, traits | imports, calls, inheritance |
| Bash | Functions | source, calls |

<details>
<summary><strong>CLI</strong></summary>

The CLI provides structural operations and autonomous lifting via LLM API.

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
├── rpg-parser      Tree-sitter entity + dependency extraction (15 languages)
├── rpg-encoder     Encoding pipeline, semantic lifting utilities, incremental evolution
├── rpg-nav         Search, fetch, explore, snapshot, TOON serialization
├── rpg-lift        Lifting scope resolution
├── rpg-cli         CLI binary (rpg-encoder)
└── rpg-mcp         MCP server binary (rpg-mcp-server)
```

</details>

<details>
<summary><strong>FAQ</strong></summary>

**Do I need an API key or a local LLM?**

No. Your connected coding agent (Claude Code, Cursor, etc.) *is* the LLM. rpg-encoder sends
source code to the agent via MCP tools, the agent analyzes it and sends back semantic features.
No API keys, no external services, no local model downloads.

**How long does lifting take?**

Roughly 2 minutes per 100 entities. A small project (50 files, ~200 entities) takes about
5 minutes. A large project (500+ files) should use parallel subagents — your agent handles
this automatically. Build and hierarchy steps are near-instant.

**What happens when I change code?**

The server auto-syncs the graph when git HEAD moves. Modified entities are tracked for
re-lifting. No manual intervention needed.

**Can I lift only part of the codebase?**

Yes. Pass a file glob to `get_entities_for_lifting`: `"src/auth/**"`, `"crates/rpg-core/**"`,
etc. You can also use `.rpgignore` (gitignore syntax) to permanently exclude files.

**What if lifting gets interrupted?**

The graph is saved to disk after every `submit_lift_results` call. Start a new session,
call `lifting_status`, and it picks up exactly where you left off.

**Should I commit `.rpg/` to the repo?**

Yes. Committing `.rpg/graph.json` means collaborators and CI agents get instant semantic
search without re-lifting.

</details>

## Inspirations & References

rpg-encoder is built on the theoretical framework from the RPG-Encoder research paper, with
original extensions inspired by tools across the code intelligence landscape:

- **RPG-Encoder paper** (Luo et al., 2026, Microsoft Research): The semantic lifting model,
  3-level hierarchy construction, incremental evolution algorithms, and formal graph model
  `G = (V_H ∪ V_L, E_dep ∪ E_feature)`.
  [[Paper]](https://arxiv.org/abs/2602.02084)
  [[Project Page]](https://ayanami2003.github.io/RPG-Encoder/)

- **GitNexus**: Precomputed relational intelligence, blast radius analysis, Claude Code hooks
  for seamless integration. Showed that a code graph tool must be invisible to be essential.

- **Serena**: Symbol-level precision via LSP. Demonstrated that real-time code awareness
  matters more than batch analysis.

- **TOON**: Token-Oriented Object Notation for LLM-optimized output.
  [[Spec]](https://github.com/toon-format/toon)

This is an independent implementation. All code is original work under the MIT license.
Not affiliated with or endorsed by Microsoft.

## License

Licensed under the [MIT License](LICENSE).
