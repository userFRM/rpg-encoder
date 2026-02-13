# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.5.0] - 2026-XX-XX

### Added

- **Protocol Deduplication** — Version references save 10K-40K tokens per lifting session
  - `get_files_for_synthesis` uses version references after batch 0: `[RPG Protocol: file_synthesis v<hash>]`
  - SHA256-based prompt versioning in MCP server (`PromptVersions` struct)
- **Graph Reasoning Tools** — Two new MCP tools for dependency path analysis
  - `find_paths`: K-shortest paths between entities using Yen's algorithm (returns paths of varying lengths)
  - `slice_between`: Extract minimal connecting subgraph (Steiner tree with strict edge tracking)
  - Eliminates 5-10 manual tool calls for path queries
- **Diff-Aware Search** — `search_node` accepts `since_commit` parameter for PR review workflows
  - Proximity-based ranking: 3x boost for changed entities, 2x for 1-hop neighbors, 1.5x for 2-hop
  - Automatically maps git changes to entity IDs via `rpg_encoder::evolution::detect_changes`
  - Computes dependency proximity tiers using BFS traversal
  - Boost applied before truncation to ensure changed entities can rank into results
  - 50-70% fewer irrelevant results projected in PR review tasks
- **Sharded Hierarchy Foundation** — Clustering infrastructure for repos >100 files
  - File clustering with deterministic batching (target: 70 files per cluster)
  - Representative sampling for domain discovery
  - Balance clusters to maintain manageable batch sizes
  - **FULL MCP INTEGRATION COMPLETE**: Two-phase batched workflow (domain discovery → file assignment per cluster)

### Changed

- `search_node` MCP tool supports diff-aware ranking via `since_commit` parameter
- `SearchParams` extended with `diff_context` field for proximity boosting
- Diff boost now applied before truncation (expands search limit 10x when diff_context present)
- `find_paths` defaults to max_hops=5 (use -1 for unlimited)
- File clustering uses sorted iteration for deterministic batch assignments

### Performance

- Protocol deduplication: ~75 tokens saved per batch after batch 0 (10K-40K total over 20-batch session)
- Graph reasoning tools: Single-call path queries vs 5-10 manual explore/fetch calls
- Diff-aware search: Reduces noise in PR review workflows

### Technical

- Added dependencies: `sha2 = "0.10"` (rpg-mcp), `kodama = "0.2"` (rpg-encoder, currently unused)
- New modules:
  - `rpg-nav/src/diff.rs` (189 lines, 5 tests): Proximity computation and boost application
  - `rpg-nav/src/paths.rs` (290 lines, 7 tests): K-shortest paths with Yen's algorithm
  - `rpg-nav/src/slice.rs` (300 lines, 8 tests): Minimal subgraph extraction with strict edge tracking
- Extended `rpg-encoder/src/hierarchy.rs` (+210 lines, 4 tests): Clustering and batching functions
- MCP tool count: 21 → 23 (new: `find_paths`, `slice_between`)
- Sharded hierarchy: Session management, automatic clustering, batched domain discovery and file assignment
- All tests passing (466+ tests across workspace)
- Clustering simplified from HAC to deterministic batching (kodama API complexity deferred)

## [0.4.0] - 2026-02-13

### Added

- **Incremental Embeddings** — Per-entity fingerprint sync replaces full embedding rebuild.
  Only re-embeds entities whose features changed. Stored in `embeddings.meta.json` alongside
  existing binary format.
- **Confidence-Gated Auto-Lift** — Structural signal analysis (branches, loops, calls, early
  returns) in new `signals.rs` module. Three confidence buckets:
  - *Accept*: apply features silently (simple getters, setters, constructors)
  - *Review*: show in batch 0 for LLM verification (moderate complexity)
  - *Reject*: needs full LLM analysis (high complexity)
  TOML rules extended with `max_branches`, `max_loops`, `max_calls` structural gates.
- **Lift Quality Critic** — Non-blocking feedback on `submit_lift_results`. Checks for vague
  verbs ("handle", "process"), implementation details ("loop", "iterate"), too-short/too-long
  features, and duplicates. Features are always applied; warnings help the LLM self-correct.
- **`plan_change` MCP tool** — Answers "what existing code needs to change for goal X, and in
  what order?" Orchestrates search + impact_radius + topological sort + test coverage detection.
  Returns dependency-safe modification order with blast radius analysis.
- **`feature_source` provenance** — `Option<String>` field on Entity tracks feature origin
  (`"auto"`, `"llm"`, `"synthesized"`). Backward-compatible via `serde(default)`.
- MCP tool count: 20 → 21 (new: `plan_change`)

## [0.3.0] - 2026-02-12

### Added

- **Language-Universal Auto-Lift** — 134 TOML-driven auto-lift rules across 13 languages with
  acronym-aware field normalization (`getHTTPClient` → `return http client`) (#40)
- **LLM Performance Optimizations** — `context_pack` super-tool (search→fetch→explore in 1
  call), `impact_radius` BFS reachability, dependency context in lifting batches, auto-lift
  trivial entities (≤3 lines, getter/setter/new patterns) (#38)
- Preserve semantic features on `build_rpg` rebuild and prune `.rpgignore` files (#36)
- MCP tool count: 17 → 20 (new: `context_pack`, `impact_radius`, `reconstruct_plan`)

### Fixed

- Scope auto-lift rules by language to prevent cross-language collisions (#42)

## [0.2.0] - 2026-02-11

### Added

- **Reconstruction Scheduler** — `reconstruct_plan` builds dependency-safe execution batches
  for guided code reconstruction workflows (#34)
- Validation improvements and documentation fixes (#34)

### Changed

- **Refactored rpg-mcp** — Split monolithic `tools.rs` into focused modules: `params.rs`,
  `types.rs`, `helpers.rs`, `server.rs`
- Test count: 379 → 446+

## [0.1.9] - 2026-02-09

### Added

- **LLM-Based Semantic Routing** (Algorithm 4) — Two new MCP tools (`get_routing_candidates`,
  `submit_routing_decisions`) let the connected agent perform semantic hierarchy placement
  decisions. Entities are routed via LLM judgment rather than Jaccard similarity alone.
  Persisted pending state (`.rpg/pending_routing.json`) with graph revision protection for
  crash safety and stale-decision rejection.
- **Three-Zone Drift Judgment** (Algorithm 3) — Configurable drift thresholds split
  re-lifted entities into three zones:
  - `drift < 0.3` (ignore): minor edit, in-place update
  - `0.3 <= drift <= 0.7` (borderline): surfaced for agent review via routing candidates
  - `drift > 0.7` (auto-route): automatically queued for re-routing
  New config options: `drift_ignore_threshold`, `drift_auto_threshold`.
- **Embedding-Based Semantic Search** — Feature-level embeddings via `fastembed` +
  BGE-small-en-v1.5 (384 dimensions). `search_node` features mode now uses hybrid rank-based
  scoring (0.6 semantic + 0.4 lexical) with max-cosine similarity over per-feature vectors.
  Model auto-downloads on first search, runs fully offline afterward.
- **TOML-driven paradigm pipeline** — Framework detection and entity classification via
  declarative TOML configs instead of hardcoded patterns (#28)
- **7 additional language parsers** — C, C++, Go (enhanced), Java (enhanced), with
  per-language entity and dependency modules (#28)
- Semantic drift re-routing and feature-based hierarchy routing (#30)
- Full-scale paper fidelity documentation
- MCP tool count: 15 → 17 (new: `get_routing_candidates`, `submit_routing_decisions`)

### Changed

- Parser architecture refactored into per-language modules under `crates/rpg-parser/src/languages/`
- Paradigm detection moved to `crates/rpg-parser/src/paradigms/`
- `graph_revision` now uses `updated_at` timestamp instead of `base_commit`
- `build_rpg`, `update_rpg`, `reload_rpg` now clear pending routing and invalidate embeddings
- Test count: 275 → 379+

### Fixed

- Windows `build.rs` path escaping and dropped Intel Mac from release matrix

## [0.1.8] - 2026-02-09

### Added

- **Redux Toolkit frontend adapter** for TypeScript/React/Next.js — extracts `createSlice`,
  `createAsyncThunk`, RTK Query hooks, and store configuration as first-class entities (#26)
- Benchmark comparison analysis (`benchmarks/comparison.md`)
- Use cases guide (`use_cases.md`)

## [0.1.7] - 2026-02-08

### Added

- **`.rpgignore` support** — exclude files from the RPG graph using gitignore-style patterns (#23)
- Rebuilt RPG graph with full semantic hierarchy and 100% lifting coverage

### Changed

- Updated benchmark results for 652 entities, 39 queries

## [0.1.6] - 2026-02-07

### Added

- **Enhanced TS/JS parser** for React/Next.js patterns — JSX components, hooks, pages,
  layouts, API routes (#15)
- Per-file language detection in lifting for mixed-language projects (#21)
- Scoped dependency resolution during lifting (#21)

### Fixed

- Entity IDs correctly rekeyed on file rename during incremental update (#19)
- New entities from renamed files receive hierarchy assignments (#19)
- npm publish made idempotent to avoid false-failure CI status (#14)

### Changed

- Aligned MCP synthesis instructions with file synthesis prompt (#17)
- Fixed stale benchmark data (#17)

## [0.1.5] - 2026-02-07

### Added

- **Paper alignment improvements** — closer match to Algorithms 1-4 from the paper (#11)
- Deterministic edge ordering in serialized graph output (#11)
- Improved hierarchy construction prompts

## [0.1.4] - 2026-02-06

### Fixed

- **Qualified entity IDs** — use `file:Class::method` format to resolve merged-key coverage
  ceiling where entities from different files shared names (#8)
- 100% lifting coverage now achievable (557/557 entities)

## [0.1.3] - 2026-02-06

### Fixed

- **Deterministic JSON serialization** — all maps use `BTreeMap` for reproducible
  `graph.json` output across runs (#6)

### Added

- Multi-repo setup instructions in README (#4)

## [0.1.2] - 2026-02-06

### Fixed

- **Auto-preserve semantic features** on `build_rpg` — previously, rebuilding the graph
  would discard all lifted features. Now, features from the old graph are automatically
  merged into the new graph for entities that still exist (#2)
- Collapsed nested if statements to satisfy clippy

### Added

- npm publish in release workflow (CI)
- npm OIDC trusted publishers support

## [0.1.1] - 2026-02-03

Initial npm release. Same code as v0.1.0, published to npm registry.

## [0.1.0] - 2026-02-03

Initial public release. Independent Rust implementation of the RPG-Encoder framework
described in [arXiv:2602.02084](https://arxiv.org/abs/2602.02084).

### Core Pipeline

- **Semantic Lifting** (Phase 1) — Parse code with tree-sitter, enrich entities with
  verb-object features via the connected coding agent's MCP interactive protocol
  (`get_entities_for_lifting` → `submit_lift_results`)
- **Structure Reorganization** (Phase 2) — Agent discovers functional domains and builds
  a 3-level semantic hierarchy (`build_semantic_hierarchy` → `submit_hierarchy`)
- **Artifact Grounding** (Phase 3) — Anchor hierarchy nodes to directories via LCA algorithm,
  resolve cross-file dependency edges (imports, invocations, inheritance)

### Language Support

- 8 languages via tree-sitter: Python, Rust, TypeScript, JavaScript, Go, Java, C, C++
- Per-language entity extraction (functions, classes, methods, structs, traits, interfaces)
- Per-language dependency resolution (imports, calls, inheritance, trait impls)

### Incremental Evolution

- Git-diff-based incremental updates (Algorithms 2-4 from the paper)
- Deletion pruning with hierarchy cleanup
- Modification with semantic drift detection (Jaccard distance)
- Structural entity insertion with dependency re-resolution
- Modified entities tracked for agent re-lifting

### Navigation & Search

- **search_node** — Intent-based search across 3 modes: features, snippets, auto
- **fetch_node** — Entity details with source code, dependencies, hierarchy context; V_H
  hierarchy node fetch support
- **explore_rpg** — Dependency graph traversal (upstream/downstream/both) with configurable
  depth and edge filtering by kind (imports, invokes, inherits, contains)
- **rpg_info** — Graph statistics, hierarchy overview, per-area lifting coverage
- Cross-view traversal between V_L (code entities) and V_H (hierarchy nodes)
- TOON (Token-Oriented Object Notation) serialization for token-efficient LLM output

### MCP Server

- 15 tools: `build_rpg`, `search_node`, `fetch_node`, `explore_rpg`, `rpg_info`, `update_rpg`,
  `lifting_status`, `get_entities_for_lifting`, `submit_lift_results`, `finalize_lifting`,
  `get_files_for_synthesis`, `submit_file_syntheses`, `build_semantic_hierarchy`,
  `submit_hierarchy`, `reload_rpg`
- Semantic lifting via connected coding agent — no API keys, no local LLMs, no setup
- Staleness detection on read-only tools (prepends `[stale]` notice when graph is behind HEAD)
- Auto-update on server startup when graph is stale (structural-only, sub-second)

### CLI

- Commands: `build`, `update`, `search`, `fetch`, `explore`, `info`, `diff`, `validate`,
  `export`, `hook`
- `--include` / `--exclude` glob filtering for builds
- `--since` commit override for incremental updates
- Pre-commit hook: `rpg-encoder hook install` (auto-updates and stages graph on commit)
- Graph export as DOT (Graphviz) or Mermaid flowchart
- Graph integrity validation

### Storage

- RPG graph committed to repos (`.rpg/graph.json`) — collaborators get instant semantic
  search without rebuilding
- Self-contained `.rpg/.gitignore` (ignores `config.toml`)
- Optional zstd compression for large graphs

### Configuration

- `.rpg/config.toml` with sections: `[encoding]`, `[navigation]`, `[storage]`
- Environment variable overrides (`RPG_BATCH_SIZE`, `RPG_SEARCH_LIMIT`, etc.)
- Feature normalization: trim, lowercase, sort+dedup per paper spec

### Code Quality

- Modular crate architecture: rpg-core, rpg-parser, rpg-encoder, rpg-nav, rpg-cli, rpg-mcp
- Clean `cargo clippy --workspace --all-targets -- -D warnings`
- Clean `cargo fmt --all -- --check`
