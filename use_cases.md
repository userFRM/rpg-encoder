# RPG-Encoder: Practical Use Cases

How the Repository Planning Graph transforms code navigation from text matching into semantic understanding.

---

## The Problem RPG Solves

Coding agents (Claude Code, Gemini CLI, Cursor, etc.) navigate repositories using the same tools humans use: grep, find, and file reads. This works when you know what you're looking for, but breaks down for the queries that matter most:

- *"How does authentication work in this project?"*
- *"What handles database migrations?"*
- *"Where is the error recovery logic?"*

These are **intent queries** — you know *what* the code does, not *what it's called*. Grep can't bridge this gap. RPG-Encoder can.

---

## Use Case 1: Codebase Onboarding

### The scenario
You open a new repository for the first time. 200 files, 3,000 functions. Where do you even start?

### Without RPG
```
$ grep -r "main" --include="*.rs" -l        # 47 files match
$ grep -r "init" --include="*.rs" -l        # 23 files match
$ grep -r "config" --include="*.rs" -l      # 18 files match
# ... still no architectural understanding
```

### With RPG
```
rpg_info()
→ 9 functional areas: GraphDataModel, LanguageParsing, SemanticEncoding,
  Navigation, McpServer, CommandLineInterface, Benchmarking, Distribution, TestFixtures

search_node(query="how does the system start up", mode="features")
→ main.rs:main (score 0.82) — "parse command-line arguments, dispatch cli subcommands"
→ main.rs:RpgServer::new (score 0.71) — "initialize mcp server, load graph from disk"

explore_rpg(entity_id="crates/rpg-mcp/src/main.rs:main", direction="downstream")
→ main
  ├── RpgServer::new → loads config, loads graph
  ├── ensure_graph → lazy-loads from disk
  └── staleness_notice → checks git HEAD vs graph
```

**In 3 calls**, you understand the startup flow, the major subsystems, and the dependency chain. No guessing filenames. No reading 47 files.

---

## Use Case 2: Finding Code by Behavior

### The scenario
You need to find where the system "detects when the graph is out of date with the code."

### Without RPG
```
$ grep -r "stale" --include="*.rs"           # finds the word "stale" in strings
$ grep -r "outdated" --include="*.rs"        # nothing
$ grep -r "out of date" --include="*.rs"     # nothing
$ grep -r "changed" --include="*.rs"         # 89 matches, mostly irrelevant
# The function is actually called `detect_workdir_changes` — you'd never guess this
```

### With RPG
```
search_node(query="detect when graph is out of date with code", mode="features")
→ evolution.rs:detect_workdir_changes (score 0.78)
  features: "detect file changes in working directory against base commit"
→ main.rs:staleness_notice (score 0.65)
  features: "detect stale graph from workdir changes, generate staleness warning"
```

The semantic features describe *what the code does*, not what it's named. The vocabulary gap between your question and the function name is irrelevant.

---

## Use Case 3: Understanding a Feature End-to-End

### The scenario
You need to understand the full "semantic lifting" pipeline — from raw code to lifted features. Which functions are involved? In what order?

### Without RPG
```
$ grep -rn "lift" --include="*.rs" -l
# Returns 15 files. Which ones matter? What's the call order?
# You'd need to manually read each file, trace function calls, build a mental model.
```

### With RPG
```
search_node(query="orchestrate semantic lifting pipeline", mode="features")
→ lift.rs:resolve_scope — "resolve scope string to matching entity ids"
→ lift.rs:collect_raw_entities — "re-extract source code for entities in scope"
→ lift.rs:build_token_aware_batches — "partition entities into token-budget-aware batches"

explore_rpg(entity_id="crates/rpg-encoder/src/lift.rs:build_token_aware_batches",
            direction="upstream", depth=3)
→ build_token_aware_batches
  ├── called by: get_entities_for_lifting (MCP handler)
  │   └── called by: agent (external)
  └── calls: collect_raw_entities
      └── calls: extract_entities (parser)

fetch_node(entity_id="crates/rpg-encoder/src/lift.rs:build_token_aware_batches")
→ source code, line numbers, dependencies, hierarchy path
```

Three calls give you the full pipeline: scope resolution → entity collection → batching → MCP handler → agent. Including the source code and exact line numbers.

---

## Use Case 4: Scoped Navigation

### The scenario
You're working on the navigation layer and want to find all search-related code, excluding parser tests and benchmarks.

### Without RPG
```
$ grep -rn "search" --include="*.rs" crates/rpg-nav/
# Returns matches in test files, bench files, and production code — all mixed together
# No way to filter by "things that are about search functionality"
```

### With RPG
```
search_node(query="search entities by query",
            scope="Navigation",
            entity_type_filter="function,method")
→ search.rs:search_with_params — "search graph entities with full parameter support"
→ search.rs:search_features — "search entities by semantic feature similarity"
→ search.rs:search_snippets — "search entities by name and file path"
→ search.rs:multi_signal_score — "score text against query using multiple signals"
```

The `scope` parameter restricts results to the Navigation area of the hierarchy. No test code, no benchmarks, no parser internals — just the search implementation.

---

## Use Case 5: Impact Analysis Before Refactoring

### The scenario
You want to refactor `RPGraph::remove_entity`. What else will break?

### Without RPG
```
$ grep -rn "remove_entity" --include="*.rs"
# Shows call sites, but not the full dependency chain
# Doesn't show what remove_entity itself calls internally
# Doesn't show which higher-level operations depend on it
```

### With RPG
```
explore_rpg(entity_id="crates/rpg-core/src/graph.rs:RPGraph::remove_entity",
            direction="both", depth=2)
→ remove_entity
  ├── UPSTREAM (what calls it):
  │   ├── apply_deletions (evolution.rs) — file deletion handler
  │   └── apply_modifications (evolution.rs) — file modification handler
  │       └── run_update — orchestrates incremental updates
  ├── DOWNSTREAM (what it calls):
  │   ├── remove_entity_from_hierarchy — cleans hierarchy tree
  │   │   └── remove_from_subtree — recursive hierarchy cleanup
  │   └── edges_for — finds all edges to clean up
```

You instantly see: changing `remove_entity` affects the incremental update pipeline (`apply_deletions`, `apply_modifications`), the hierarchy system, and edge cleanup. All without reading a single file.

---

## Use Case 6: Answering Architectural Questions

### The scenario
A new contributor asks: "What are the main subsystems and how do they relate?"

### With RPG
```
rpg_info()
→ 660 entities, 60 files, 9 functional areas:
  - GraphDataModel (115 entities) — core data structures, persistence, schema
  - LanguageParsing (192 entities) — tree-sitter extraction for 7 languages
  - SemanticEncoding (109 entities) — lifting, grounding, hierarchy, evolution
  - Navigation (105 entities) — search, explore, fetch, export
  - McpServer (48 entities) — MCP tool handlers, session management
  - CommandLineInterface (25 entities) — CLI dispatch
  - Benchmarking (40 entities) — search quality measurement
  - Distribution (4 entities) — npm binary installer
  - TestFixtures (22 entities) — Python fixture project

search_node(query="entry points that connect subsystems", mode="features")
→ main.rs:RpgServer::build_rpg — "build repository planning graph from source"
  (connects: LanguageParsing → GraphDataModel → SemanticEncoding)
→ main.rs:RpgServer::search_node — "search graph entities by query"
  (connects: Navigation → GraphDataModel)
```

The hierarchy gives immediate architectural context. No need to read dozens of files to build a mental model.

---

## The "Search-then-Zoom" Pattern

The RPG-Encoder paper observes that effective agents follow a consistent navigation pattern:

1. **Search** — Broad semantic discovery (`search_node`)
2. **Explore** — Trace dependencies from discovered entities (`explore_rpg`)
3. **Fetch** — Deep-dive into specific entities with source code (`fetch_node`)
4. **Edit** — Make precise changes with full context (grep/read/edit)

This pattern reduces the average number of agent steps from ~20 (baseline tools) to ~7 (RPG-guided), cutting cost by 60-75% while improving accuracy (paper Table 5).

RPG doesn't replace grep — it provides the **semantic map** that tells you *where* to grep.

---

## When RPG Adds the Most Value

| Scenario | Value | Why |
|----------|-------|-----|
| Unfamiliar codebase | High | Hierarchy + semantic search = instant orientation |
| Large repositories (1000+ entities) | High | Structured navigation prevents getting lost |
| Cross-cutting concerns | High | Dependency tracing across module boundaries |
| "How does X work?" questions | High | Intent-based search finds by behavior |
| Refactoring impact analysis | High | Upstream/downstream exploration reveals blast radius |
| Bug localization from issue descriptions | High | Natural language → code mapping (paper's primary evaluation) |
| Quick symbol lookup | Low | grep is faster for exact matches |
| Single-file edits | Low | Direct file read is sufficient |
| Trivial codebases (<20 files) | Low | Mental model fits in your head |

---

## Summary

RPG-Encoder transforms repository navigation from **"search for text"** to **"search for purpose"**. It gives coding agents (and humans) three capabilities that text search cannot provide:

1. **Semantic discovery** — Find code by what it does, not what it's named
2. **Architectural context** — Understand where code fits in the system hierarchy
3. **Dependency awareness** — Trace how code connects across module boundaries

The result: faster onboarding, more accurate localization, and fewer wasted exploration steps.

---

*Based on rpg-encoder v0.1.8 and the RPG-Encoder paper (Luo et al., arXiv:2602.02084, 2026).*
