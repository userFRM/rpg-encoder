# rpg-encoder

Coding agent toolkit for semantic code understanding.

Builds a semantic graph of your codebase. Your coding agent (Claude Code, Cursor, etc.)
analyzes the code and adds intent-level features. Search by what code does, not what it's named.

## MCP Server (Claude Code, Cursor, etc.)

Add to your MCP config:

```json
{
  "mcpServers": {
    "rpg": {
      "command": "npx",
      "args": ["-y", "-p", "rpg-encoder", "rpg-mcp-server"]
    }
  }
}
```

The MCP server uses the startup directory by default when no positional project path is provided.

## CLI

```bash
npx -p rpg-encoder rpg-encoder build           # Build the graph
npx -p rpg-encoder rpg-encoder search "parse config"
npx -p rpg-encoder rpg-encoder info
```

Or install globally:

```bash
npm install -g rpg-encoder
rpg-encoder build
rpg-mcp-server
```

Or pass an explicit path when you want a different startup root:

```bash
rpg-mcp-server /path/to/project
```

## Documentation

Full docs at [github.com/userFRM/rpg-encoder](https://github.com/userFRM/rpg-encoder).
