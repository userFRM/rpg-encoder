You are an expert software architect and large-scale repository refactoring specialist.

## Goal
Reorganize the repository's parsed feature tree by assigning each entity to the most semantically appropriate location within the target architecture.

## Target Path Format (STRICT)
Each target path must have exactly three levels:
`<functional_area>/<category_level_1>/<subcategory_level_2>`
- `functional_area` must be one of the provided functional areas.
- `category_level_1` expresses broader purpose or lifecycle role.
- `subcategory_level_2` adds precise specialization or context.
- Each segment: concise (2-5 words), semantically meaningful, intent-focused.

Examples:
- "DataProcessing/data ingestion/pipeline orchestration"
- "ModelTraining/optimization strategy/hyperparameter tuning"

Avoid filler labels (e.g., "misc", "others", "core", "general").

## Semantic Naming Rules
1. Use "verb + object" phrasing.
2. Use lowercase English only for categories and subcategories.
3. Describe purpose, not implementation.
4. Each label expresses a single responsibility.
5. When multiple distinct roles exist, use multiple precise labels rather than one overloaded label.
6. Avoid vague verbs such as `handle`, `process`, `deal with`.
7. Avoid implementation details (data structures, algorithms).
8. Avoid library/framework names (React, Express, serde) — use domain terms instead.
9. Prefer domain semantics over generic terms: "authenticate user" not "check data".
10. One responsibility per label — do not chain with "and".

## Scope Constraints
- Only assign entities to functional areas from the provided list above.
- Do not invent new functional areas; you may define new categories and subcategories as needed, but they must remain meaningful and consistent.
- If a file serves multiple distinct roles, assign it based on its primary responsibility.
- Exclude docs/examples/tests/vendor code unless essential to core functionality.

## Output Format
One file per line. Format: file_path | FunctionalArea/category/subcategory

All entities within a file inherit its hierarchy assignment.

Example:
src/cli/args.rs | CommandLineInterface/parse input/read arguments
src/http/client.rs | HttpClient/manage connections/send data