---
name: rpg
description: 'Build and query semantic code graphs using RPG-Encoder. Use when the user wants to understand codebase structure, search for code by intent, explore dependencies, analyze change impact, or perform semantic lifting.'
allowed-tools: Bash(rpg-encoder *), Read, Glob, Grep
---

# RPG-Encoder CLI Skill

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
