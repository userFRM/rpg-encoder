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
9. Synthesize each file's entity features into a coherent 1-2 sentence summary
10. `submit_file_syntheses({...})` — apply your holistic file summaries (improves hierarchy)
11. `build_semantic_hierarchy` — get domain discovery + hierarchy assignment prompts
12. `submit_hierarchy({...})` — apply your hierarchy assignments
13. `lifting_status` — verify 100% coverage and semantic hierarchy

**At any point, call `lifting_status` to see where you are.**

## FILE SYNTHESIS (step 8-10)

After all entities are lifted, `finalize_lifting` produces dedup-aggregated file features.
These are functional but not holistic — they're just a bag of child features.

The **synthesis step** asks YOU to read each file's entity features and write a coherent
summary of what the file does as a whole. This produces better input for domain discovery
and hierarchy assignment.

Flow:
1. `get_files_for_synthesis(batch_index=0)` — returns files with entity features
2. For each file, write a holistic 1-2 sentence summary
3. `submit_file_syntheses({"path/file.rs": "summary", ...})` — applies to Module entities
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

## RESUME SUPPORT

The graph persists to disk (`.rpg/graph.json`) after every `submit_lift_results` call.
If a session ends mid-lift, starting a new session and calling `lifting_status` picks up
exactly where you left off — only unlifted entities are returned by `get_entities_for_lifting`.

## TOOLS
- **lifting_status**: Dashboard — coverage, per-area progress, unlifted files, NEXT STEP
- **build_rpg**: Index the codebase (run once, instant)
- **get_entities_for_lifting** + **submit_lift_results**: YOU analyze the code
- **finalize_lifting**: Aggregate file-level features, rebuild hierarchy metadata
- **get_files_for_synthesis** + **submit_file_syntheses**: YOU synthesize file-level summaries
- **build_semantic_hierarchy**: Get prompts for domain discovery + hierarchy assignment
- **submit_hierarchy**: Apply your hierarchy assignments to the graph
- **search_node**: Find code by intent (features/snippets/auto modes)
- **fetch_node**: Get full entity details + source code
- **explore_rpg**: Trace dependency chains
- **rpg_info**: Get codebase overview and statistics
- **update_rpg**: Incrementally update after code changes
- **reload_rpg**: Reload graph from disk
