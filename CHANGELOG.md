# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.8.3] - 2026-04-14

### Added

- `build_rpg` response now emits an action-oriented NEXT STEP directing the
  agent to lift the graph immediately — small scope lifts in the current
  context, large scope dispatches a sub-agent. Previously a passive
  "Tip: use get_entities_for_lifting" hint that agents routinely skipped.
- Canonical lock-order invariant documented on the `RpgServer` struct so
  reviewers don't have to re-derive it from scattered call sites.
- `build_semantic_hierarchy` sharded init now takes `graph.read()` to
  compute clusters *before* acquiring `hierarchy_session.write()`,
  respecting the declared graph-before-session order. Previously the
  sharded path acquired `hierarchy_session.write()` first and then
  `graph.read()`, which formed a deadlock cycle with `update_rpg`'s
  graph-then-session order under concurrent scheduling.
- `build_batch_0_domain_discovery` now takes clusters as a parameter
  instead of re-reading `self.hierarchy_session`. The previous design
  left a TOCTOU window where a concurrent `build_rpg`/`update_rpg`
  could clear the session between installation and domain-discovery
  rendering; the re-read would then panic on `unwrap()`. Callers
  snapshot the clusters while holding the session lock and pass them
  through.
- `set_project_root` no longer preserves the previous project's
  in-memory config when the new project has a malformed
  `.rpg/config.toml`. It now falls back to `RpgConfig::default()` on
  parse failure — project switch must not silently cross-contaminate
  encoding/batch settings. (Same-project `reload_rpg` keeps the
  previous config on parse failure, which is correct for that flow.)
- `lifting_status` tracks stale-feature drift across calls. A persistent
  per-server set records entities whose source was modified after they
  were lifted; the dashboard reports `stale_features: N entities modified
  since last lift` and the NEXT STEP state machine prompts re-lift even
  when coverage is 100%.
- `get_entities_for_lifting(scope="*")` now returns stale entities
  alongside unlifted ones, so a single call covers both "never lifted"
  and "lifted-but-outdated" work. Stale entities bypass the auto-lift
  skip check and flow through the normal LLM batch loop.
- `lifting_status` emits a sub-agent dispatch recommendation when the
  remaining work (unlifted + stale) is ≥100 entities. The response
  contains `LOOP` / `DISPATCH` / `FALLBACK` blocks so callers delegate
  directly without first loading a batch of source into their own context.
- `get_entities_for_lifting` batch-0 emits a one-line dispatch hint when
  ≥10 token-aware batches are queued, pointing back to `lifting_status`
  for the full recommendation.
- `submit_lift_results` NEXT action becomes scale-aware — when remaining
  work ≥100 entities, it redirects the caller to `lifting_status` instead
  of encouraging another foreground batch.
- Auto-sync notice now prescribes a verb. It distinguishes per-update
  delta from pre-existing backlog and separates new-entity drift from
  stale-feature drift, so an agent that commits code and sees the notice
  is told to re-lift rather than informed of a count.
- `server_instructions.md`: new `USE RPG FIRST` top section with a
  mapping table from shell patterns (`grep -r`, `cat`, `find`, `wc -l`,
  chained greps) to the RPG tool that replaces them.
- `server_instructions.md`: new `DRIFT MAINTENANCE` section explaining
  the three auto-sync notice variants and framing re-lift as part of
  definition-of-done for any task that wrote code.
- Tool descriptions for `search_node`, `fetch_node`, `explore_rpg`,
  `rpg_info`, `semantic_snapshot`, `context_pack`, `impact_radius`,
  `plan_change`, `analyze_health`, `detect_cycles`, `find_paths`, and
  `slice_between` now open with a `PREFER THIS OVER ...` marker naming
  the shell command or workflow they replace.
- `.claude/skills/rpg/SKILL.md` and `README.md` carry the same RPG-first
  mapping table as the server prompt.
- `reload_config_with_warning` helper (`server.rs`) distinguishes missing
  `.rpg/config.toml` (silent default) from malformed TOML (stderr warning,
  keeps previous in-memory config).
- Crate-visible `LARGE_SCOPE_ENTITIES` (100) and `LARGE_SCOPE_BATCHES` (10)
  constants replace duplicated magic numbers. Doc comments describe the
  heuristic-vs-authoritative relationship.

### Changed

- `lifting_status` NEXT STEP is runtime-neutral. No specific runtime's
  dispatch syntax appears in the response; callers use whatever sub-agent
  or cheaper-model mechanism their runtime exposes. Explicit fallbacks:
  scoped lifting for callers with no delegation mechanism, and
  `rpg-encoder lift --provider anthropic|openai` for callers with an API
  key and no sub-agent support.
- Batch-size estimates in NEXT STEP messages read from the live
  `encoding.max_batch_tokens` config instead of a hard-coded `~12K`
  figure, so the estimate scales when the user overrides the budget.
- `NEXT STEP:` remains a single parseable line; dispatch detail is
  emitted in labeled blocks immediately below.
- `server_instructions.md` LIFTING FLOW sub-section rewritten and
  shortened.
- `get_routing_candidates` response header no longer includes the graph
  revision hash — it moved to the NEXT_ACTION block at the bottom. Keeps
  the stable preamble (instructions + entity table) cache-eligible while
  still surfacing the revision where the agent needs to read it back.
- `update_rpg` now feeds `summary.modified_entity_ids` into the
  stale-tracking set so its `needs_relift: N` reply aligns with what
  `lifting_status` and `get_entities_for_lifting(scope="*")` report.
- `reload_rpg` clears the stale-tracking set only after the graph reload
  succeeds, so a transient read error no longer wipes the drift backlog
  while leaving the previous graph in memory.
- `set_project_root` resets the stale-tracking set as part of the
  project-scoped state reset.
- CLI fallback in large-scope guidance is gated to cases with actual
  unlifted work, with a note that `rpg-encoder lift` does not re-lift
  stale entities (it filters to entities with no features).

### Fixed

- `lifting_status` previously reported `Graph is complete` as soon as
  every entity had some features, ignoring stale features from modified
  sources. The state machine now considers `remaining + stale_features`
  combined.
- `get_entities_for_lifting(scope="*")` previously returned zero entities
  when all drift was stale (features present, sources modified) because
  `resolve_scope` filters to entities with no features.
- Auto-sync notice previously conflated per-update delta with global
  backlog, so a one-line edit on a partially-lifted repo could claim
  "50 new entities unlifted" when only 1 was actually new.
- `finalize_lifting` fallback guidance previously said to call it after
  each scoped subtree. That auto-routes pending entities against
  incomplete signals and locks the hierarchy early. Guidance now says
  to call `finalize_lifting` once after all scopes complete.
- `rpg-encoder lift --provider ...` (CLI fallback) left the MCP server
  on a stale in-memory graph. All docs that mention the CLI fallback
  now specify that the caller must call `reload_rpg` afterward.
- `set_project_root` and `reload_rpg` previously used
  `unwrap_or_default()` on config loads, collapsing missing-config and
  malformed-TOML into identical silent behavior. Malformed TOML now
  surfaces a stderr warning and leaves the in-memory config untouched.
- `set_project_root` failed to refresh `self.config` on project switch;
  the server kept serving the previous project's encoding settings.
- `lifting_status` large-scope recommendation previously ran off raw
  unlifted count, before auto-lift had reduced the set. On repos full
  of trivial entities (getters, setters, constructors) it could
  recommend delegation for ~0 LLM calls. The large-scope branch now
  hedges — it signals likely-large and defers the authoritative check
  to the post-auto-lift batch count in `get_entities_for_lifting`.
- `rpg_info` error wording ("No RPG found") was miscited as a friendly
  status string; corrected to "any RPG tool returns 'No RPG found'".
- `build_rpg` NEXT STEP previously counted `Module` entities in its
  "unlifted" total, while `lifting_status` and `get_entities_for_lifting`
  exclude them. The two could disagree by hundreds of entities on large
  codebases, tripping the delegation threshold in `build_rpg` when
  `lifting_status` would still recommend foreground lifting. Both paths
  now use `lifting_coverage()` (non-module) for the count.
- `submit_lift_results` previously emitted `DONE` as soon as coverage
  reached 100%, which could terminate a stale-only re-lift loop after
  batch 1 while later batches were still queued. The NEXT/DONE branch
  now counts unlifted + stale remaining.
- Auto-lifted features for entities that were previously flagged stale
  now drain the stale-tracking set, so the count doesn't inflate when
  the auto-lifter writes fresh features directly.

## [0.8.2] - 2026-04-14

### Added

- **`set_project_root` MCP tool** — switch the active project root at runtime
  without restarting the session. Loads the new root's `.rpg/graph.json` if
  present, resets lifting/hierarchy sessions, auto-sync markers, and pending
  routing state. Tilde-expands and canonicalizes the supplied path. Fixes the
  common case where a session is launched from `$HOME` but the user wants to
  work on `~/some-project` — previously the server's project root was locked
  to the launch directory.
- MCP tool count: 27 → 28.

### Changed

- `RpgServer::project_root` is now an async accessor backed by
  `Arc<RwLock<PathBuf>>` (previously a static `PathBuf` field). All tool
  handlers acquire a snapshot at call time, so each invocation reads whatever
  `set_project_root` most recently set.
- `get_config_blocking` renamed to `load_config` and made async.
- `staleness_detail` made async (needed to acquire the new project-root lock).

## [0.8.1] - 2026-04-14

### Fixed

- **CRITICAL: Auto-sync on workdir changes was a no-op** (caught by Codex audit) —
  v0.8.0 detected workdir changes and computed a hash, but the underlying
  `run_update` only diffed `base_commit..HEAD`, so uncommitted edits were
  silently ignored. The hash was then cached as "synced", masking the
  staleness from later queries. This made the headline v0.8.0 feature
  (workdir auto-sync) effectively non-functional.
- **Auto-sync error path no longer caches markers** — transient failures
  no longer leave the server silently stale. The next query retries.
- **Revert detection** — when a previously-dirty file returns to its HEAD
  state, the graph is now restored. Previously, the entity additions from
  the dirty version persisted forever.
- **CLI `update` now defaults to workdir-aware** — matches the MCP server.
  Pass `--since <commit>` for commit-range diffing as before.
- README said "six crates" while listing seven. Now says seven.
- `tools.rs` module comment said "17 tools" — actually 27.

### Added

- **`run_update_workdir`** in `rpg-encoder::evolution` — public API that
  applies the working-tree diff (committed + staged + unstaged) instead of
  the committed-only diff.
- **`run_update_from_changes`** in `rpg-encoder::evolution` — public API
  that applies a caller-supplied `Vec<FileChange>`. Used by the MCP server
  to compose workdir changes with revert-detection re-parses.

## [0.8.0] - 2026-04-13

### Added

- **`semantic_snapshot` MCP tool** — Whole-repo semantic understanding in one call (~25K tokens
  for 1000 entities). Includes hot spots: top-10 most-connected entities as architectural backbone.
- **`auto_lift` MCP tool** — Autonomous lifting via external LLM API (Anthropic Haiku, OpenAI
  GPT-4o-mini, OpenRouter, Google Gemini). Supports `api_key_env` parameter to resolve keys
  from environment variables (safer than raw keys in tool calls).
- **`analyze_health` MCP tool** — Code health metrics: coupling, instability, god objects,
  clone detection.
- **`detect_cycles` MCP tool** — Circular dependency detection.
- **Auto-staleness resolution** — Server auto-syncs graph when git HEAD moves. Navigation tools
  no longer show passive stale warnings, they auto-update.
- **MCP tool annotations** — `read_only_hint`, `destructive_hint`, `open_world_hint` on all
  tools per MCP 2025-03-26 spec.
- **`tool_list_changed` capability** enabled for future progressive tool loading.
- **Claude Code hooks** — `PreToolUse` on Write/Edit (auto-context injection), `PostToolUse`
  on Bash (auto-update after git commits).
- MCP tool count: 23 → 27

### Fixed

- **Deadlock in `auto_sync_if_stale()` error path** — write lock held while calling read-lock
  method (#75)
- **`auto_lift` graph loss on pipeline error** — graph now stays in write lock via
  `block_in_place` (#75, #77)
- **API key exposure in Debug derive** — manual `Debug` impl redacts `api_key` (#75)
- **Concurrent `auto_lift` guard** — `lift_in_progress` AtomicBool rejects parallel calls (#76)
- **Cost estimate overcounting** — Review-confidence entities no longer counted as LLM-needed
  (#76)
- **Pre-commit hook supply-chain risk** — removed unpinned `npx -y` fallback (#77)
- **Autonomous lift name collision** — tries qualified `Class::method` names when bare name
  misses (#77)
- **Snapshot dep skeleton ambiguity** — uses qualified names instead of bare names (#77)
- **Shell hook portability** — replaced GNU `grep -P` with POSIX `sed` (#77)
- **Root scope handling in shared search** — `scope="."` and `scope=""` now behave as
  unscoped (#70)

### Removed

- Unused `kodama` dependency

### Changed

- `server.json` version updated to 0.7.0 (was stuck at 0.6.7)
- CONTRIBUTING.md corrected: tools are in `tools.rs` not `main.rs`
- README rewritten: leads with value proposition, credits inspirations (RPG paper, GitNexus,
  Serena)
- Dead code removed from rpg-lift `progress.rs`

## [0.7.0] - 2026-04-08

### Added

- **Claude Code skill** (`.claude/skills/rpg/SKILL.md`) and Gemini CLI extension (#68)
- README updated to cite arXiv paper directly

### Changed

- Entity Signatures carried over from v0.6.0
- MCP tool count: 23 (unchanged)

## [0.6.2] - 2026-02-21

### Fixed

- **Auto-migrate existing Windows graphs** — When loading graphs created on Windows
  (schema <2.2.0), all backslash entity IDs are automatically normalized to forward
  slashes across `entities`, `file_index`, `edges`, and `hierarchy`. All lifted
  semantic features and data are fully preserved. Schema version bumped to 2.2.0.

## [0.6.1] - 2026-02-21

### Fixed

- **Windows path format mismatch in entity IDs** (#49) — `Path::display()` produces
  backslashes on Windows, causing key mismatches when MCP tools receive forward-slash
  keys from users/LLMs. Added `normalize_path()` utility in `rpg-core::graph` and
  applied it at all ID-generating call sites: `RawEntity::id()`,
  `resolve_dependencies()`, `apply_renames()`, and `create_module_entities()`.
  No-op on Unix. Closes #49.
- Pre-existing clippy `inefficient_to_string` warnings in `critic.rs`, `hierarchy.rs`,
  and `evolution_test.rs`.

## [0.6.0] - 2026-02-13

### Added

- **Entity Signatures** — Typed function/method signatures extracted from AST
  (parameters with type annotations, return types). Stored on `Entity.signature`.
- **DataFlow Edges** — New `EdgeKind::DataFlow` for parameter passing and return
  value flow between entities. Adds data lineage tracking to the dependency graph.
- **Area Connectivity** — `rpg_info` now reports inter-area dependency counts,
  showing which functional areas are most tightly coupled.
- MCP tool count: 23

### Changed

- Schema version bumped for signature and data flow support

## [0.5.0] - 2026-02-13

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
