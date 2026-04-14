RPG-Encoder: Repository Planning Graph — semantic understanding of any codebase.
No API keys or local LLMs needed. YOU are the LLM — you analyze the code directly.

## USE RPG FIRST — BEFORE grep / cat / find / file-reads

Any user question about code structure, behavior, relationships, impact,
dependencies, or cross-file patterns — reach for RPG tools BEFORE falling back
to shell commands or file reads. RPG is indexed, semantically organized, and
gives one-call answers to questions that would otherwise require dozens of
chained greps.

| If you'd otherwise reach for... | Use this instead |
|---|---|
| `grep -r` / `rg` (by intent) | `search_node(query="...")` — finds code by what it DOES |
| `grep -r` / `rg` (by name/path) | `search_node(query="...", mode="snippets")` |
| `cat file` / reading a function | `fetch_node(entity_id="file:name")` |
| chained greps for "what calls X" | `explore_rpg(entity_id="...", direction="upstream")` |
| recursive grep for "what depends on X" | `impact_radius(entity_id="...")` — with edge paths |
| `wc -l` / `find` / `tree` | `rpg_info` — counts, hierarchy, inter-area connectivity |
| reading the whole repo | `semantic_snapshot` — whole-repo view in one call |
| multi-step search + read + trace | `context_pack(query="...")` — 1 call instead of 3-5 |
| "how do I refactor X safely" | `plan_change(goal="...")` — ordered entities + blast radius |
| "find circular dependencies" | `detect_cycles` |
| "find god objects / unstable code" | `analyze_health` |
| "shortest path between A and B" | `find_paths(source, target)` |
| "minimal subgraph connecting these" | `slice_between(entity_ids=[...])` |

**Fall back to grep / cat / file-reads only when the query is about LITERAL TEXT**
(string search, comments, TODO markers, log messages, license headers) — not
about structure or semantics. This holds even if your training predisposes you
toward shell tools; the RPG is cheaper, more accurate, and more complete for
every structural question.

If a graph does not exist yet (RPG tools error with messages like "No RPG
found" or "graph: not built"), run `build_rpg` first. If entities are
unlifted and the scope is large, see the LIFTING FLOW below for delegation
guidance.

## LIFTING FLOW (step by step)

1. `build_rpg` — index the codebase (if no graph exists)
2. `lifting_status` — see coverage, unlifted files, and what to do next
3. `get_entities_for_lifting(scope="*")` — get a batch of entities with source code
4. Analyze each entity, extract verb-object features per the instructions
5. `submit_lift_results({...})` — submit features, see per-area progress
6. Repeat 3-5 until all batches done (follow the NEXT action in each response)
7. `finalize_lifting` — aggregate file-level features, rebuild hierarchy metadata
8. `get_files_for_synthesis` — get file-level entity features for holistic synthesis
9. Synthesize each file's entity features into 3-6 comma-separated high-level features
10. `submit_file_syntheses({...})` — apply your holistic file features (improves hierarchy)
11. `build_semantic_hierarchy` — get domain discovery + hierarchy assignment prompts
12. `submit_hierarchy({...})` — apply your hierarchy assignments
13. `lifting_status` — verify 100% coverage and semantic hierarchy

**At any point, call `lifting_status` to see where you are.**

## REVIEW CANDIDATES (auto-lift verification)

Some entities are auto-lifted with moderate confidence. These appear in batch 0
under a `## REVIEW CANDIDATES` section with pre-filled features. For each:

- **Accept**: If the features look correct, do nothing — they're already applied.
- **Override**: If features are wrong or incomplete, include corrected features in
  your `submit_lift_results` call. Your submission replaces the auto-generated features.

Review candidates are entities that matched an auto-lift pattern but had minor
structural complexity (exactly 1 branch, or 3+ calls). High-confidence matches
(0 branches, 0 loops, ≤2 calls) are applied silently. Entities with higher
complexity (2+ branches or any loop) are rejected and appear as regular entities
for full LLM analysis.

## SEMANTIC ROUTING (optional, after submit_lift_results)

When `submit_lift_results` reports a `## ROUTING` block, entities need semantic
placement in the hierarchy. You can:

1. **Route them** — call `get_routing_candidates` to see entities + hierarchy,
   then `submit_routing_decisions({"entity_id": "Area/cat/subcat" or "keep", ...})`.
2. **Skip routing** — the system will auto-route using Jaccard similarity when
   you call `finalize_lifting`.

LLM routing produces better hierarchy quality than auto-routing, but both work.
Borderline drift entities (0.3-0.7 Jaccard distance) are included — use "keep"
to confirm they belong in their current position.

## FILE SYNTHESIS (step 8-10)

After all entities are lifted, `finalize_lifting` produces dedup-aggregated file features.
These are functional but not holistic — they're just a bag of child features.

The **synthesis step** asks YOU to read each file's entity features and synthesize them
into 3-6 comma-separated high-level features for the file as a whole. This produces better
input for domain discovery and hierarchy assignment.

Flow:
1. `get_files_for_synthesis(batch_index=0)` — returns files with entity features
2. For each file, synthesize into 3-6 comma-separated high-level features
3. `submit_file_syntheses({"path/file.rs": "feature1, feature2, ...", ...})` — applies to Module entities
4. Repeat for all batches
5. Then proceed to `build_semantic_hierarchy`

## LIFTING STRATEGY

When the user asks you to "lift", "be the lifter", "lift this code", "lift the repo",
"add semantic features", "run the lifter", or "semantically analyze":

1. If no RPG exists yet, call `build_rpg` first.
2. Call `lifting_status` to check coverage and get the NEXT STEP.
3. **Choose your strategy based on size:**

### Small scope (< 100 entities): Lift directly
Process all batches in the current conversation:
- Call `get_entities_for_lifting` with the scope.
- Extract verb-object features for each entity per the instructions.
- Call `submit_lift_results` with JSON keys matching the headers from get_entities_for_lifting (e.g., `{"file:Class::method": ["feature1", ...]}` for methods, `{"file:func": ["feature1", ...]}` for functions).
- Continue with next batch_index until DONE.
- Call `finalize_lifting` then `get_files_for_synthesis` + `submit_file_syntheses`.
- Call `build_semantic_hierarchy` + `submit_hierarchy`.

### Large scope (100+ entities): Delegate, do not lift directly

Each batch returns a large chunk of source code that stays in your context. At the
default config ~10 batches is already ~80K tokens burned on grunt work. Feature
extraction is pattern-matching — a cheaper or delegated model handles it fine.

Delegated worker loop (run in a fresh context):

```
get_entities_for_lifting(scope="*") -> analyze -> submit_lift_results  (repeat)
finalize_lifting
```

Use whatever sub-agent or cheaper-model mechanism your runtime exposes. The graph
persists to disk after every submit, so the worker's writes survive. **After the
worker returns, call `reload_rpg`** to refresh the caller's in-memory graph —
required if the runtime gave the worker an isolated MCP session, no-op if it
shared yours.

Fallbacks when no delegation mechanism is available:
- **Scoped lifting**: narrow each call, e.g. `get_entities_for_lifting(scope="src/auth/**")`,
  then `finalize_lifting`. Each scope fits in foreground context.
- **CLI autonomous lift**: `rpg-encoder lift --provider anthropic|openai` uses an
  external API key directly — no agent subscription involvement. **After the CLI
  finishes, call `reload_rpg` in this session** so the server picks up the updated
  `.rpg/graph.json` — otherwise subsequent queries will still see the pre-lift state.

After delegation returns, call `get_files_for_synthesis` + `submit_file_syntheses`,
then `build_semantic_hierarchy` + `submit_hierarchy`.

Call `lifting_status` whenever you need the NEXT STEP with a concrete recommendation
for the current state.

## ERROR RECOVERY

If `submit_lift_results` reports unmatched keys (features that couldn't be applied):
1. Check that keys match the `### headers` from `get_entities_for_lifting` exactly
2. For methods, use the qualified format: `file:Class::method`
3. Re-submit only the corrected features — already-applied features are persisted

This implements the paper's retry-on-malformed-output pattern at the agent protocol level.

## RESUME SUPPORT

The graph persists to disk (`.rpg/graph.json`) after every `submit_lift_results` call.
If a session ends mid-lift, starting a new session and calling `lifting_status` picks up
exactly where you left off — only unlifted entities are returned by `get_entities_for_lifting`.

## NAVIGATION WORKFLOW

When using the RPG to understand or navigate a codebase (after lifting is complete):

1. **Quick context** — `context_pack(query="...", token_budget=4000)` to get a focused bundle of entities with source, features, and deps in a single call. This replaces the typical search→fetch→explore multi-step workflow.
2. **Semantic discovery** — `search_node(query="...", mode="features")` to find entities by intent. Results include `entity_id` for direct follow-up.
3. **Precision verification** — `fetch_node(entity_id="...", fields="features,deps")` to inspect specific fields without retrieving everything.
4. **Local expansion** — `explore_rpg(entity_id="...", direction="both", format="compact")` for pipe-delimited rows with entity_ids preserved.
5. **Impact analysis** — `impact_radius(entity_id="...", direction="upstream")` to find all entities that depend on a target, with edge paths.
6. **Change planning** — `plan_change(goal="add rate limiting")` to find relevant entities, compute dependency-safe modification order, and assess impact radius in one call.
7. **Pinpoint retrieval** — `search_node(query="...", mode="snippets")` for exact name/path lookups.

**Token-saving tips:**
- Use `fetch_node(fields="features,deps")` to skip source code (~80% smaller output)
- Use `explore_rpg(format="compact")` for ID-preserving machine-readable rows (enables direct follow-up calls)
- Use `explore_rpg(max_results=N)` to cap large dependency trees
- Use `context_pack` instead of search→fetch→explore chains (1 call vs 3-5)
- Use `impact_radius` for richer reachability analysis with edge paths (1 call vs multi-step explore)

## HEALTH ANALYSIS

Use `analyze_health` to assess architectural quality of the codebase. It computes
instability, centrality, coupling metrics, and optionally detects code duplication.

**When to use:** After lifting is complete, to identify refactoring targets, god objects,
unstable modules, and duplicated code.

**Parameters (all optional):**
- `instability_threshold` (default 0.7) — flag entities with instability above this
- `god_object_threshold` (default 10) — minimum degree to flag as god object
- `include_duplication` (default false) — run Rabin-Karp token-based clone detection (reads source files, slower)
- `include_semantic_duplication` (default false) — run Jaccard feature-based clone detection (in-memory, fast)
- `semantic_similarity_threshold` (default 0.6) — Jaccard threshold for semantic clones

**Output sections:**
- Summary: entity count, edges, avg instability/centrality, god objects, hubs
- God Object Candidates (degree ≥ threshold)
- Top Unstable Entities (I > 0.7)
- Hub Entities (high centrality)
- Duplication Hotspots (when `include_duplication=true`) — token-level Type-1/Type-2 clones
- Semantic Duplication (when `include_semantic_duplication=true`) — conceptual clones via lifted features
- Recommendations for refactoring

**Examples:**
```json
{}                                          // baseline health (no duplication)
{"include_duplication": true}               // + token-based clones
{"include_semantic_duplication": true}      // + conceptual clones
{"include_duplication": true, "include_semantic_duplication": true}  // both
{"god_object_threshold": 5, "instability_threshold": 0.5}           // stricter thresholds
```

## TOOLS
- **lifting_status**: Dashboard — coverage, per-area progress, unlifted files, NEXT STEP
- **build_rpg**: Index the codebase (run once, instant)
- **get_entities_for_lifting** + **submit_lift_results**: YOU analyze the code (trivial entities auto-lifted, moderate ones flagged for review)
- **get_routing_candidates** + **submit_routing_decisions**: LLM-based semantic routing (optional)
- **finalize_lifting**: Aggregate file-level features, rebuild hierarchy metadata (auto-routes pending if skipped)
- **get_files_for_synthesis** + **submit_file_syntheses**: YOU synthesize file-level features
- **build_semantic_hierarchy**: Get prompts for domain discovery + hierarchy assignment
- **submit_hierarchy**: Apply your hierarchy assignments to the graph
- **search_node**: Find code by intent (features/snippets/auto). Results include entity_id for follow-up
- **fetch_node**: Get entity details. Use `fields` param for projection (features/source/deps/hierarchy)
- **explore_rpg**: Trace dependency chains. Use `format="compact"` for pipe-delimited rows with entity_ids. Edge filter values: `imports`, `invokes`, `inherits`, `composes`, `renders`, `reads_state`, `writes_state`, `dispatches`, `data_flow`, `contains`
- **context_pack**: Single-call search+fetch+explore. Searches, fetches source, expands neighbors, trims to token budget
- **impact_radius**: BFS reachability with edge paths. Answers "what depends on X?" in one call. Traverses DataFlow edges for data lineage analysis
- **plan_change**: Change planning — find relevant entities, dependency-safe modification order, impact radius, and related tests
- **analyze_health**: Architectural health analysis — instability, centrality, god objects, duplication detection (token + semantic)
- **detect_cycles**: Find circular dependencies in the codebase. First call returns summary + area breakdown. Use filters to get cycle details.
- **rpg_info**: Get codebase overview, statistics, and inter-area connectivity
- **update_rpg**: Incrementally update after code changes
- **reload_rpg**: Reload graph from disk

## CYCLE DETECTION

Use `detect_cycles` to find circular dependencies — architectural smells where A→B→C→A.

**First call (no params):** Returns summary statistics, length distribution, area breakdown. Shows `next_step` prompting to call with filters.

**Parameters (all optional, JSON object):**
| Parameter | Type | Description |
|-----------|------|-------------|
| `area` | string | Filter to area(s), comma-separated. Example: `"Navigation"` or `"Parser,Core"` |
| `max_cycles` | number | Limit cycles returned. Example: `10` |
| `min_cycle_length` | number | Skip trivial cycles. Example: `3` (skips 2-cycles) |
| `max_cycle_length` | number | Max cycle length. Default: `20` |
| `cross_file_only` | boolean | Only cross-file cycles. Example: `true` |
| `cross_area_only` | boolean | Only cross-area cycles. Example: `true` |
| `sort_by` | string | Sort key: `"length"`, `"file_count"`, or `"entity_count"` |
| `include_files` | boolean | Include file paths. Default: `true` |
| `ignore_rpgignore` | boolean | Include ignored files. Default: `false` |

**Usage flow:**
1. Call `detect_cycles` with no params → get summary + area breakdown
2. Use `next_step` as guide → call again with filters
3. Example: `{"area": "Navigation", "max_cycles": 10}`
