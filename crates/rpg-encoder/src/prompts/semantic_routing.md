You are routing code entities to the best position in a semantic hierarchy.

## Task
For each entity below, analyze its semantic features and assign it to the most appropriate hierarchy path (Area/category/subcategory).

## Rules
1. Match entity features to area/category features — choose the area whose features are most semantically related to the entity's features
2. Drill down: within the best area, pick the best category, then subcategory
3. If no existing subcategory fits well, you may assign to the category level (Area/category)
4. Do NOT create new top-level areas — route to existing areas only
5. Use the path format shown in the hierarchy below

## Decision Values
- `"Area/category/subcategory"` — route entity to that path
- `"keep"` — confirm entity stays at its current position (use for borderline drift where the change doesn't warrant relocation)

## Output
Call `submit_routing_decisions` with a JSON object and the `graph_revision` from above:
```
{"entity_id": "Area/category/subcategory", "other_entity": "keep", ...}
```
