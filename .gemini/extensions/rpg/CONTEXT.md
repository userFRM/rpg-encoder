# RPG-Encoder

RPG-Encoder builds semantic code graphs (Repository Planning Graphs) for AI-assisted code understanding.

## CLI Commands

- `rpg-encoder build` — Index codebase, build graph (run once)
- `rpg-encoder search "query"` — Search entities by intent or name
- `rpg-encoder fetch ENTITY_ID` — Get entity details and source
- `rpg-encoder explore ENTITY_ID` — Trace dependency chains
- `rpg-encoder info` — Show graph statistics
- `rpg-encoder lift` — Autonomous LLM-driven semantic lifting
- `rpg-encoder lift --dry-run` — Estimate lifting cost
- `rpg-encoder update` — Incremental update after code changes
- `rpg-encoder export --format dot|mermaid` — Export graph
- `rpg-encoder validate` — Check graph integrity

## MCP Tools (via extension)

The extension also provides an MCP server with rich navigation tools: `search_node`, `fetch_node`, `explore_rpg`, `context_pack`, `impact_radius`, `plan_change`, `find_paths`, and the full lifting protocol.
