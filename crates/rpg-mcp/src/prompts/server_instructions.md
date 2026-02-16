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

## GENERATION PROTOCOL (code generation from spec)

When the user asks you to "generate code", "create a new project", "implement from spec",
or provides a natural language specification:

### Step-by-step workflow

1. `init_generation(spec="...", language="rust")` — Initialize with specification and target language
2. Decompose the spec into features using the prompt returned by init_generation
3. `submit_feature_tree({...})` — Submit the decomposed feature tree
4. `get_interfaces_for_design(batch_index=0)` — Get features for interface design
5. Design interfaces using the interface_design prompt
6. `submit_interface_design({...})` — Submit designed interfaces
7. Repeat steps 4-6 for each batch until all interfaces designed
8. `get_tasks_for_generation(batch_index=0)` — Get tasks with dependency context + TDD instructions
9. **For each task, follow the TDD loop** (see below)
10. Repeat steps 8-9 for each batch until all tasks completed
11. `validate_generation()` — Verify generated code against plan (signature comparison)
12. `finalize_generation()` — Complete the session, build RPG with pre-seeded features
13. `generation_efficiency_report()` — Export cost/latency/failure telemetry and quality-vs-cost curves

**At any point, call `generation_status` to see where you are.**

### TDD Loop (per task)

Each task returned by `get_tasks_for_generation` includes dependency source code and TDD instructions.
For every task:

1. **Write tests first** — Create tests validating the expected behavior (semantic features)
2. **Implement the code** — Write the minimal implementation matching the planned signature
3. **Run tests** — Execute the test suite (`cargo test`, `pytest`, `jest`, etc.)
4. **Report outcome** — Call `report_task_outcome` with the result
5. **Follow routing** — The tool response tells you what to do next

Optional: replace steps 3-4 with `run_task_test_loop` to execute tests in local/docker sandbox,
auto-adapt common runner flags, classify failures, and report telemetry automatically.
When using `sandbox_mode="docker"`, provide `docker_image` explicitly or configure
`.rpg/config.toml` via `[generation.docker_images]` by language key.

### Failure Routing Table

| Outcome | Routing | Action | Counts as retry? |
|---------|---------|--------|-------------------|
| `pass` | PASS | Move to next task | No |
| `test_failure` | FIX_CODE | Fix implementation, re-run tests, report again | Yes |
| `code_error` | FIX_CODE | Fix compilation error, re-run tests, report again | Yes |
| `test_error` | FIX_TEST | Fix test code, re-run tests, report again | Yes |
| `env_error` | ENV_ERROR | Fix environment, retry | **No** |

After 3 counted retries, the task is marked as **Failed** and you move on.

### Classifying Outcomes

When reporting a task outcome, classify it as:

- **pass**: All tests pass, code compiles, behavior is correct
- **test_failure**: Tests ran but some failed — the code is buggy, the tests are correct
- **code_error**: Code won't compile or has syntax errors
- **test_error**: The test code itself is broken (won't compile, bad imports, setup failures)
- **env_error**: Nothing wrong with code or tests — missing tool, wrong permissions, etc.

### Phase overview

| Phase | Tools | Description |
|-------|-------|-------------|
| 1. Planning | `init_generation`, `submit_feature_tree` | Decompose spec into features |
| 2. Design | `get_interfaces_for_design`, `submit_interface_design` | Design module interfaces |
| 3. Execute | `get_tasks_for_generation`, `report_task_outcome` | TDD loop: test → implement → verify |
| 4. Validate | `validate_generation`, `finalize_generation` | Signature validation + RPG build |
| 5. Report | `generation_efficiency_report` | Cost/latency/failure analytics |

### Key concepts

- **Feature Tree**: Hierarchical decomposition of the specification into functional areas and features
- **Interface Design**: Module contracts (public APIs, data types, dependencies) before implementation
- **Task Graph**: Dependency-ordered tasks for code generation
- **TDD Loop**: Test-first development with automated outcome routing
- **Sandboxed Runtime**: `run_task_test_loop` supports local/docker execution with auto-adaptation
- **Dependency Context**: Completed task source code included in subsequent task batches
- **Signature Validation**: rpg-parser extracts actual signatures and compares to planned
- **Pre-seeded Features**: `finalize_generation` injects planned semantic features onto RPG entities

### Generation Strategy

Choose your strategy based on project size:

**Small scope (<20 tasks)**: Generate directly in the current conversation.
Process all batches, run TDD loop, validate, and finalize.

**Large scope (20+ tasks)**: Split into subagent conversations by functional area.
Each subagent gets a fresh context window and handles one area's tasks.
After all subagents complete, call `validate_generation` + `finalize_generation`.

### Session management

- `generation_status` — Dashboard showing phase, progress, and next step
- `run_task_test_loop` — Execute tests in local/docker sandbox, auto-adapt runner args, auto-report outcome
- `generation_efficiency_report` — Cost/latency/failure telemetry + quality-vs-cost curves
- `reset_generation` — Clear session and start fresh
- `retry_failed_tasks` — Reset failed tasks to pending for re-generation

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
- **rpg_info**: Get codebase overview, statistics, and inter-area connectivity
- **update_rpg**: Incrementally update after code changes
- **reload_rpg**: Reload graph from disk

### Generation Tools
- **init_generation**: Initialize a new code generation session from a specification
- **get_feature_tree**: Get the current feature tree for review
- **submit_feature_tree**: Submit the LLM-decomposed feature tree
- **get_interfaces_for_design**: Get a batch of features for interface design
- **submit_interface_design**: Submit designed interfaces
- **get_tasks_for_generation**: Get tasks with dependency context and TDD instructions
- **report_task_outcome**: Report TDD iteration result (pass/fail) with failure routing
- **run_task_test_loop**: Run tests in sandbox, classify failure type, auto-route via report_task_outcome
- **submit_generated_code**: Report completed tasks with file paths
- **validate_generation**: Verify generated code with signature comparison
- **finalize_generation**: Complete session, build RPG with pre-seeded features
- **generation_status**: Dashboard showing phase, progress, next step
- **generation_efficiency_report**: Export cost/efficiency telemetry and backbone curves
- **reset_generation**: Clear session and start fresh
- **retry_failed_tasks**: Reset failed tasks to pending
- **seed_ontology_features**: Seed low-confidence entities with ontology hints
- **assess_representation_quality**: Confidence + drift checks at graph scale
- **run_representation_ablation**: Measure retrieval/localization ablations (Acc@k/MRR)
- **export_external_validation_bundle**: Build blinded third-party reproduction pack
