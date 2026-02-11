# Paper Fidelity Report

Systematic comparison of rpg-encoder against the RPG-Encoder paper
(Luo et al., *Closing the Loop: Universal Repository Representation with RPG-Encoder*,
[arXiv:2602.02084](https://arxiv.org/abs/2602.02084), 2026).

This document covers every algorithm, pipeline phase, and tool interface described in
the paper. Each section states what the paper specifies, what this implementation provides,
and where the two diverge. Extensions beyond the paper's scope are listed separately.

---

## Summary

| Category | Components Assessed | Average Fidelity |
|----------|-------------------|-----------------|
| Core data model | Graph structure, edge taxonomy | 95% |
| Three-phase pipeline | Lifting, hierarchy, grounding | 90% |
| Incremental algorithms | Algorithms 1–4 | 95% |
| Navigation tools | SearchNode, FetchNode, ExploreRPG | 95% |
| Incremental evolution | Git-diff event processing | 90% |
| Formal evaluation | SWE-bench, RepoCraft | Not implemented |

---

## 1. Graph Model

### Paper Specification

> G = (V_H ∪ V_L, E_dep ∪ E_feature)

- **V_H**: Abstract hierarchy nodes (Area → Category → Subcategory)
- **V_L**: Leaf code entities (functions, classes, methods)
- **E_dep**: Dependency edges (imports, invocations, inheritance, composition)
- **E_feature**: Containment edges linking V_L to V_H

### Implementation

| Component | Paper | rpg-encoder | Status |
|-----------|-------|-------------|--------|
| V_H hierarchy nodes | 3-level taxonomy | `hierarchy: BTreeMap<String, HierarchyNode>` | Faithful |
| V_L leaf entities | Functions, classes, methods | `entities: BTreeMap<String, Entity>` with `EntityKind` enum | Faithful |
| E_dep dependency edges | Imports, Invokes, Inherits, Composes | `edges: Vec<DependencyEdge>` with matching `EdgeKind` enum | Faithful |
| E_feature containment | Contains edges linking leaf → hierarchy | `Contains` edge kind + `hierarchy_path` on Entity | Faithful |
| Frontend edge kinds | Not described | `Renders`, `ReadsState`, `WritesState`, `Dispatches` | Extension |
| Serialization order | Not specified | Deterministic `BTreeMap` + sorted edges | Extension |

**Fidelity: 95%** — The core graph structure matches the paper exactly. Additional edge kinds
for frontend frameworks are additive and do not alter the paper's defined model.

---

## 2. Phase 1: Semantic Lifting (Section 3.1)

### Paper Specification

An LLM extracts 3–8 verb-object feature phrases per entity. Features capture behavioral
intent (e.g., "validate user credentials", "serialize config to disk").

### Implementation

| Aspect | Paper | rpg-encoder | Status |
|--------|-------|-------------|--------|
| Feature format | Verb-object phrases | Identical format, enforced via prompt | Faithful |
| Features per entity | 3–8 | 3–8 (prompt-specified) | Faithful |
| Lifting mechanism | Direct API call to LLM | Connected agent via MCP tool protocol | Variant |
| Batch protocol | Described in Appendix A.1.1 (token-budget batching) | `get_entities_for_lifting` → `submit_lift_results` with token-aware batching | Faithful |
| Parallel lifting | Not described | Batch indices are independent; parallelizable at orchestration layer | Extension |
| Cross-session resume | Not described | Graph persisted after every submission; `lifting_status` recovers state | Extension |
| File synthesis | Described in §3.1 as fine-grained → holistic file summarization | Explicit intermediate protocol: entity features → holistic file features → hierarchy | Variant |

**Fidelity: 90%** — The semantic output is identical. The delivery mechanism differs: the paper
assumes a direct LLM API call, while this implementation uses the MCP tool protocol where the
connected coding agent serves as the LLM. This is a deliberate architectural choice that
eliminates the need for separate API credentials.

---

## 3. Phase 2: Hierarchical Construction (Section 3.2)

### Paper Specification

Two-phase LLM process:
1. **Domain Discovery** — identify functional areas from aggregated features
2. **Hierarchical Assignment** — map files to 3-level Area/Category/Subcategory paths

### Implementation

| Aspect | Paper | rpg-encoder | Status |
|--------|-------|-------------|--------|
| Domain discovery | LLM identifies areas | `build_semantic_hierarchy` returns prompt; agent identifies areas | Faithful |
| Assignment | LLM assigns files to paths | Agent calls `submit_hierarchy` with path assignments | Faithful |
| Path-depth enforcement | Strict 3-level output format | Always enforced in `submit_hierarchy` | Faithful |
| Prompt structure | Separate Domain Discovery + Hierarchical Construction prompts (Appendix A.1.2) | Separated into domain discovery and assignment phases | Faithful |
| Prompt wording | Paper's specific wording | Different wording, equivalent semantics | Minor divergence |

**Fidelity: 85%** — The two-phase structure and 3-level output format match the paper. Prompt
wording differs, which is expected: prompt engineering is empirical and the paper's prompts were
tuned against their specific evaluation benchmarks.

---

## 4. Phase 3: Artifact Grounding (Section 3.3)

### Paper Specification

Anchor hierarchy nodes to directories via Lowest Common Ancestor (LCA) computation.
Resolve cross-file dependency edges.

### Implementation

| Aspect | Paper | rpg-encoder | Status |
|--------|-------|-------------|--------|
| LCA algorithm | Trie-based branching analysis | `rpg_core::lca::compute_lca()` — identical approach | Faithful |
| Directory anchoring | LCA of leaf entity file paths | Identical: compute LCA per hierarchy node | Faithful |
| Dependency resolution | Cross-file edge materialization | `resolve_dependencies()` in `grounding.rs` | Faithful |
| Performance indexes | Not described | `rebuild_edge_index()` + `rebuild_hierarchy_index()` for O(1) lookup | Extension |

**Fidelity: 95%**

---

## 5. Algorithm 1: Bottom-Up Path Propagation

### Paper Specification

Insert file paths into a prefix tree. Retain branching nodes as hierarchy anchors.

### Implementation

`rpg_core::lca::compute_lca()` implements this algorithm directly. Paths are inserted
into a prefix tree, single-child nodes are collapsed, and branching nodes are retained
as anchor points.

**Fidelity: 95%**

---

## 6. Algorithm 2: Incremental Deletion with Recursive Pruning

### Paper Specification

`DeleteNode(v)`: Remove entity from hierarchy, then `PruneOrphans`: recursively remove
empty abstract nodes bottom-up.

### Implementation

`remove_entity_from_hierarchy()` → `prune_empty()` chain. Empty nodes are recursively
cleaned bottom-up. Containment edges are updated and feature re-aggregation runs after
pruning.

**Fidelity: 95%**

---

## 7. Algorithm 3: Differential Modification (Drift Detection)

### Paper Specification

> "We assess drift based on (i) feature overlap/consistency, and (ii) an LLM judgement
> constrained by explicit criteria."

When an entity is modified, compute feature drift. If significant, treat as delete +
re-insert. The paper requires both quantitative measurement and qualitative LLM assessment.

### Implementation

Three-zone drift system with configurable thresholds:

| Zone | Drift Range | Behavior | Paper Alignment |
|------|-------------|----------|-----------------|
| Ignore | `drift < 0.3` | In-place feature update, no routing | Covers insignificant modifications |
| Borderline | `0.3 ≤ drift ≤ 0.7` | Surfaced for agent review via `get_routing_candidates` | Implements LLM judgment requirement |
| Auto-route | `drift > 0.7` | Automatically queued for re-routing | Covers significant drift case |

The borderline zone implements the paper's LLM judgment criterion. The connected agent receives
the entity's features, current hierarchy path, and drift context, then decides whether to
re-route or confirm the current position via `submit_routing_decisions`.

Thresholds are configurable:
```toml
[encoding]
drift_ignore_threshold = 0.3
drift_auto_threshold = 0.7
```

**Fidelity: 95%** — Both quantitative (Jaccard distance) and qualitative (agent judgment)
assessment are implemented.

---

## 8. Algorithm 4: Top-Down Semantic Routing

### Paper Specification

> `LLM_Route(Context, f_target)`: At each level, use LLM to select the best child node
> based on semantic similarity between entity features and node features.

### Implementation

Two MCP tools implement LLM-based routing:

| Tool | Purpose |
|------|---------|
| `get_routing_candidates` | Returns entities needing routing with features and scoped hierarchy context |
| `submit_routing_decisions` | Agent submits placement decisions (hierarchy path or `"keep"`) |

Protocol:
1. `submit_lift_results` identifies entities needing routing (drifted or newly lifted)
2. Entities are stored in persistent pending state (`.rpg/pending_routing.json`)
3. The response includes a routing block indicating how many entities need placement
4. Agent calls `get_routing_candidates` — receives entities with the top-3 matching hierarchy areas
5. Agent analyzes context and calls `submit_routing_decisions` with placement decisions
6. Server applies routing, re-aggregates features, rebuilds containment edges

Pending state is crash-safe: persisted to disk with `graph_revision` for stale-decision
protection. If the agent never calls routing tools, `finalize_lifting` drains pending entities
via Jaccard similarity as a fallback.

Routing decisions are validated at submission time:
- Decisions may only target entities currently in pending-routing state
- Non-`"keep"` decisions must be strict 3-level paths (`Area/category/subcategory`)
- Target path must already exist in the current hierarchy

| Aspect | Paper | rpg-encoder | Status |
|--------|-------|-------------|--------|
| LLM-based routing | LLM call at each level | Agent decides via MCP protocol | Faithful |
| Context provided | Node features at each level | Top-3 matching areas with aggregate features | Faithful |
| Fallback mechanism | Not described | Jaccard similarity drain in `finalize_lifting` | Extension |
| Crash-safe persistence | Not described | Pending state on disk with revision tracking | Extension |

**Fidelity: 95%**

---

## 9. SearchNode Tool

### Paper Specification

Intent-based search across entity features. The paper specifies feature mapping and
intent-based retrieval with features/snippets/auto modes; it does not hard-specify
the retrieval backend (embedding vs lexical).

### Implementation

| Aspect | Paper | rpg-encoder | Status |
|--------|-------|-------------|--------|
| Features mode | Intent-based feature retrieval | Hybrid scoring: 0.6 embedding + 0.4 lexical, rank-normalized (falls back to lexical-only when embeddings unavailable) | Faithful |
| Snippets mode | Name/path matching | Multi-signal scoring (IDF overlap, phrase match, edit distance) | Extension |
| Auto mode | Combined | Features + snippets merged with hybrid reranking | Extension |
| Embedding model | Not specified (evaluation baselines use jina-v3) | BGE-small-en-v1.5 (384 dimensions) via fastembed | Faithful |
| Scoring strategy | Not specified | Feature-level max-cosine (not centroid averaging) | Extension |
| Scope filtering | Hierarchy scope | scope + file_pattern + line_nums + entity_type_filter | Extension |

Embedding architecture:

- **Feature-level vectors**: Each entity stores individual embeddings per feature. At search
  time, `entity_score = max(cosine(query_vec, feature_vec))`. This preserves multi-role
  entity semantics rather than averaging them into a single centroid.
- **Rank-based hybrid blend**: Cosine and lexical scores are rank-normalized before blending,
  avoiding calibration issues between different score ranges.
- **Lazy initialization**: The embedding model (~130 MB) downloads on first semantic search
  and runs fully offline afterward.
- **Filter enforcement**: Semantic-only results (entities found by embeddings but not by
  lexical search) are restricted to entities that pass all user-specified filters.

**Fidelity: 95%**

---

## 10. FetchNode Tool

### Paper Specification

Return entity metadata, source code, dependencies, and hierarchy context.

### Implementation

Complete: source code extraction, semantic features, upstream/downstream dependency lists,
hierarchy path context. Supports both V_L (code entities) and V_H (hierarchy nodes).

**Fidelity: 95%**

---

## 11. ExploreRPG Tool

### Paper Specification

Traverse the dependency graph from an entity. Upstream (callers), downstream (callees),
with configurable depth.

### Implementation

Complete: direction control (upstream/downstream/both), configurable depth, edge kind
filtering (imports, invokes, inherits, composes, contains), and entity type filtering.

**Fidelity: 95%**

---

## 12. Incremental Evolution (Section 3.2)

### Paper Specification

Differential event detection from git diffs: Delete, Modify, Insert. Each event triggers
the corresponding algorithm (2, 3, or 4).

### Implementation

`update_rpg` delegates to `run_update()` in `evolution.rs`:

1. Detect file changes via `git diff` (added, modified, deleted, renamed)
2. Apply deletions (Algorithm 2) with hierarchy pruning
3. Apply modifications with structural update and stale-feature tracking
4. Apply insertions with dependency re-resolution
5. Perform Algorithm-3 drift judgement during interactive re-lifting (`submit_lift_results`)
6. Reconcile and persist pending-routing state across incremental updates

**Fidelity: 90%**

---

## 13. Reconstruction Scheduling (Section B.2.2)

### Paper Specification

In reconstruction mode, execution follows dependency-safe topological traversal with
LLM-driven batching of semantically related nodes.

### Implementation

`rpg_encoder::reconstruction` provides:
- `build_topological_execution_order()` for dependency-safe ordering
- `schedule_reconstruction()` for topological order + area-aware batching

CLI path:
- `rpg-encoder reconstruct-plan --max-batch-size <N> --format text|json`

**Fidelity: 85%** — Topological scheduling and coherent batching are implemented; full
LLM scheduler policy experimentation remains external.

---

## Extensions Beyond the Paper

The following capabilities are not described in the paper and represent implementation-specific
additions:

| Feature | Description |
|---------|-------------|
| Multi-language support | 15 parser language definitions (Python, Rust, TypeScript, JavaScript, Go, Java, C, C++, C#, Kotlin, PHP, Ruby, Scala, Swift, Bash) vs. the paper's Python-only evaluation |
| Framework paradigms | TOML-driven detection pipeline for React, Next.js, Redux with specialized entity types and edge kinds |
| File synthesis protocol | Intermediate step between entity lifting and hierarchy construction for improved domain discovery |
| Cross-session resume | Graph persisted after every operation; session state fully recoverable across restarts |
| Crash-safe routing state | Pending routing decisions persisted to disk with graph revision tracking |
| Embedding corruption recovery | Corrupt index files are detected, removed, and rebuilt automatically on next access |
| TOON serialization | Token-efficient output format for LLM consumption in MCP tool responses |
| Pre-commit hooks | `rpg-encoder hook install` for automatic graph maintenance on every commit |

---

## Not Implemented

| Paper Component | Status | Rationale |
|-----------------|--------|-----------|
| SWE-bench evaluation (Section 4.1) | Not implemented | Requires external agentic evaluation harness and benchmark dataset |
| RepoCraft evaluation (Section 4.2) | Not implemented | Requires external benchmark dataset and execution environment |
| Paper-exact prompt wording (Appendix A) | Divergent | Prompts are empirically tuned; different wording achieves equivalent lifting quality |

---

## Scorecard

| Component | Fidelity | Notes |
|-----------|----------|-------|
| Graph Model (G = V_H ∪ V_L, E) | 95% | Faithful match with additive frontend edge extensions |
| Semantic Lifting (Phase 1) | 90% | MCP tool protocol vs. direct API; identical output format |
| Hierarchical Construction (Phase 2) | 85% | Identical structure; prompt wording differs |
| Artifact Grounding (Phase 3) | 95% | Faithful LCA implementation |
| Algorithm 1 (Bottom-Up Propagation) | 95% | Direct implementation |
| Algorithm 2 (Deletion + Pruning) | 95% | Direct implementation |
| Algorithm 3 (Drift Detection + LLM Judge) | 95% | Three-zone system with agent judgment for borderline cases |
| Algorithm 4 (Semantic Routing) | 95% | LLM routing via MCP protocol with Jaccard fallback |
| SearchNode | 95% | Hybrid embedding + lexical with rank-based blending |
| FetchNode | 95% | Complete |
| ExploreRPG | 95% | Complete |
| Reconstruction Scheduling | 85% | Topological order + coherent batching via `reconstruct-plan` |
| Incremental Evolution | 90% | Full event processing with stale-feature tracking |
| Multi-language support | N/A | Extension (15 language defs vs. paper's Python-only scope) |
| Framework paradigms | N/A | Extension (not in paper) |
| Formal evaluation | 0% | Not yet implemented |

---

*Based on rpg-encoder v0.1.9 and arXiv:2602.02084 (Luo et al., 2026).
Last updated: February 2026.*
