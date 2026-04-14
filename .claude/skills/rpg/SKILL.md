---
name: rpg
description: 'Build and query semantic code graphs using RPG-Encoder. Use BEFORE grep/cat/find for any question about code structure, behavior, relationships, impact, dependencies, or cross-file patterns.'
allowed-tools: Bash(rpg-encoder *), Read, Glob, Grep
---

# RPG-Encoder — USE BEFORE grep/cat/find

## Reach for RPG first, shell commands second

Whenever the user asks about code structure, behavior, relationships, impact,
dependencies, or cross-file patterns, reach for RPG commands BEFORE falling
back to `grep`, `cat`, `find`, `wc`, or `Read`. RPG is indexed, semantically
organized, and answers structural questions in one call that would otherwise
require dozens of chained text searches.

| If you'd otherwise... | Use this instead |
|---|---|
| `grep -r` by intent | `rpg-encoder search "describe what it does"` |
| `grep -r` by name | `rpg-encoder search "name" --mode snippets` |
| `cat file` for a function | `rpg-encoder fetch "file:func"` |
| chained greps for callers | `rpg-encoder explore ENTITY --direction up` |
| chained greps for callees | `rpg-encoder explore ENTITY --direction down` |
| `wc -l` / `find` / `tree` | `rpg-encoder info` |
| reading many files | Use the MCP `semantic_snapshot` tool if available |

Fall back to `grep` / `cat` / `Read` only when the query is about literal text
(string search, comments, TODOs, log messages) — not structure or semantics.

If you have the RPG MCP server connected, prefer its tools (`search_node`,
`fetch_node`, `explore_rpg`, `impact_radius`, `plan_change`, `semantic_snapshot`,
`context_pack`) over the CLI — they're faster and return structured data.

---

You have access to `rpg-encoder`, a CLI tool that builds semantic code graphs (Repository Planning Graphs) from any codebase. Use it to understand code structure, search by intent, trace dependencies, and perform autonomous semantic lifting.

## Quick Reference

### Build a graph (run once per repo)

```bash
rpg-encoder build
```

Detects languages automatically, parses all source files, builds structural hierarchy and dependency edges. Creates `.rpg/graph.json`. Use `--force` to rebuild.

### Search for code by intent

```bash
rpg-encoder search "validate user input"
rpg-encoder search "database connection" --mode features
rpg-encoder search "auth" --mode snippets
rpg-encoder search "parse config" --scope DataProcessing
```

Modes: `auto` (default, tries both), `features` (semantic intent), `snippets` (name/path matching).

### Fetch entity details

```bash
rpg-encoder fetch "src/auth.rs:validate_token"
```

Returns entity type, file location, semantic features, source code, and dependency edges (invokes/invoked-by).

### Explore dependency graph

```bash
rpg-encoder explore "src/auth.rs:validate_token" --direction both --depth 3
```

Directions: `up` (what calls it), `down` (what it calls), `both`.

### Show graph statistics

```bash
rpg-encoder info
```

Shows entity count, file count, lifting coverage, hierarchy, functional areas, edge counts.

### Autonomous semantic lifting

```bash
# Estimate cost first
rpg-encoder lift --dry-run

# Lift with Anthropic Haiku (default, cheapest)
rpg-encoder lift --provider anthropic

# Lift with OpenAI GPT-4o-mini
rpg-encoder lift --provider openai --model gpt-4o-mini

# Lift specific scope
rpg-encoder lift --scope "src/auth/**"
```

Performs full autonomous lifting: auto-lift trivial entities, LLM-lift the rest, synthesize file features, discover domains, and assign hierarchy. Saves after each batch for crash recovery.

API key: pass `--api-key KEY` or set `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` env var.

### Incremental update (after code changes)

```bash
rpg-encoder update
rpg-encoder diff  # dry-run: see what would change
```

### Export graph

```bash
rpg-encoder export --format dot      # Graphviz DOT
rpg-encoder export --format mermaid  # Mermaid flowchart
```

### Validate graph integrity

```bash
rpg-encoder validate
```

### Git hook (auto-update on commit)

```bash
rpg-encoder hook install
rpg-encoder hook uninstall
```

## Workflow Patterns

### Understanding a new codebase

1. `rpg-encoder build` to index
2. `rpg-encoder info` to see structure
3. `rpg-encoder search "main entry point"` to find key entities
4. `rpg-encoder explore ENTITY --direction down` to trace call chains

### Finding code by what it does

Use `--mode features` with natural language describing behavior:
```bash
rpg-encoder search "handle authentication" --mode features
rpg-encoder search "serialize data to JSON" --mode features
```

### Change impact analysis

1. Find the entity: `rpg-encoder search "function_name"`
2. See what depends on it: `rpg-encoder explore ENTITY --direction up --depth 3`

### Full semantic analysis

1. `rpg-encoder build` (if not done)
2. `rpg-encoder lift --dry-run` (check cost)
3. `rpg-encoder lift` (run lifting)
4. `rpg-encoder info` (verify coverage)

## Notes

- The CLI works from the project root directory. Use `-p /path/to/project` to target a different directory.
- Graph is saved at `.rpg/graph.json` — add `.rpg/` to `.gitignore` or commit it for team sharing.
- Lifting is resumable: if interrupted, re-running `rpg-encoder lift` continues from where it stopped.
- For richer interactive navigation (context packs, impact radius, path finding), use the MCP server instead.
