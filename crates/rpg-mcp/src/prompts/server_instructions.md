RPG-Encoder: Repository Planning Graph — semantic understanding of any codebase.
No API keys or local LLMs needed. YOU are the LLM — you analyze the code directly.

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

### Large scope (100+ entities): Dispatch parallel subagents
A single conversation CANNOT lift a large repo — context will overflow.
Instead, split the work across fresh subagent conversations:

1. Call `lifting_status` to see per-area coverage and unlifted files.
2. For each area, dispatch a **foreground** subagent (Task tool, subagent_type="general-purpose"):
   - Each subagent scope: a file glob like `"crates/rpg-core/**"` or `"src/auth/**"`
   - Each subagent runs: get_entities_for_lifting -> analyze -> submit_lift_results (loop)
   - Each subagent gets a FRESH context window — no accumulation across areas
3. After all subagents complete, call `lifting_status` to verify coverage.
4. Call `finalize_lifting` then `get_files_for_synthesis` + `submit_file_syntheses`.
5. Call `build_semantic_hierarchy` + `submit_hierarchy`.

**Why subagents?** Each `get_entities_for_lifting` batch returns source code that stays in the
conversation. After ~300 entities, the context fills up and the chat breaks. Subagents solve
this by giving each chunk its own fresh context window.

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
- Use `context_pack` instead of search→fetch→explore chains (1 call vs 3-5, ~44% fewer tokens)
- Use `impact_radius` for richer reachability analysis with edge paths (1 call vs multi-step explore)

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
- **explore_rpg**: Trace dependency chains. Use `format="compact"` for pipe-delimited rows with entity_ids
- **context_pack**: Single-call search+fetch+explore. Searches, fetches source, expands neighbors, trims to token budget
- **impact_radius**: BFS reachability with edge paths. Answers "what depends on X?" in one call
- **plan_change**: Change planning — find relevant entities, dependency-safe modification order, impact radius, and related tests
- **rpg_info**: Get codebase overview and statistics
- **update_rpg**: Incrementally update after code changes
- **reload_rpg**: Reload graph from disk
