# rpg-encoder

Semantic code graph for AI-assisted code understanding.

Extracts entities from your codebase, resolves dependency edges, and lifts semantic features via LLM — producing a navigable graph optimized for AI coding tools.

## MCP Server (Claude Code, Cursor, etc.)

Add to your MCP config:

```json
{
  "mcpServers": {
    "rpg": {
      "command": "npx",
      "args": ["-y", "-p", "rpg-encoder", "rpg-mcp-server", "/path/to/your/project"]
    }
  }
}
```

## CLI

```bash
npx -p rpg-encoder rpg-encoder build           # Build the graph
npx -p rpg-encoder rpg-encoder build --lift     # Build with semantic lifting
npx -p rpg-encoder rpg-encoder search "parse config"
npx -p rpg-encoder rpg-encoder info
```

Or install globally:

```bash
npm install -g rpg-encoder
rpg-encoder build
rpg-mcp-server /path/to/project
```

## Documentation

Full docs at [github.com/userFRM/rpg-encoder](https://github.com/userFRM/rpg-encoder).
