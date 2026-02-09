# How RPG-Encoder Compares

RPG-Encoder occupies a distinct niche in the AI coding tools landscape. This document explains where it fits and when to use it.

## The Mental Model

```
┌─────────────────────────────────────────────────────────────────────┐
│                                                                     │
│   Serena         →   WHERE is the code?     (symbol locations)      │
│                      "UserAuth is defined at line 47"               │
│                                                                     │
│   Claude Code    →   CHANGE the code        (coding assistant)      │
│                      "I'll write the auth middleware for you"       │
│                                                                     │
│   RPG-Encoder    →   WHAT does code mean?   (semantic understanding)│
│                      "This function validates JWT tokens"           │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

These tools solve different problems. They're complementary, not competitive.

---

## Quick Comparison

| | **RPG-Encoder** | **Claude Code** | **Serena** |
|---|---|---|---|
| **Core function** | Semantic codebase indexer | AI coding assistant | LSP-based toolkit |
| **Primary question** | "What does this code do?" | "Can you build X for me?" | "Where is symbol Y?" |
| **Output** | Persistent graph with intent annotations | Code changes + explanations | Real-time LSP queries |
| **Requires LLM** | Yes (for lifting) | Yes (is the LLM) | No |
| **Requires API keys** | No | Yes | No |
| **Works offline** | Yes (after lifting) | No | Yes |

---

## Capability Matrix

### Code Understanding

| Capability | **RPG** | **Claude Code** | **Serena** |
|:---|:---:|:---:|:---:|
| "What does this codebase do?" | ✅ | ⚠️ | ❌ |
| Search by intent ("find auth code") | ✅ (hybrid embedding + lexical) | ⚠️ | ❌ |
| Architecture discovery | ✅ | ⚠️ | ❌ |
| Dependency graph traversal | ✅ | ❌ | ✅ |
| Find symbol by name | ⚠️ | ✅ | ✅ |
| Find all references | ✅ | ⚠️ | ✅ |
| Go to definition | ❌ | ⚠️ | ✅ |
| Type hierarchy | ❌ | ❌ | ✅ |

### Code Editing

| Capability | **RPG** | **Claude Code** | **Serena** |
|:---|:---:|:---:|:---:|
| Write new code | ❌ | ✅ | ❌ |
| Edit existing code | ❌ | ✅ | ✅ |
| Refactor/rename symbols | ❌ | ⚠️ | ✅ |
| Multi-file coordinated edits | ❌ | ✅ | ✅ |

### Execution & Workflow

| Capability | **RPG** | **Claude Code** | **Serena** |
|:---|:---:|:---:|:---:|
| Run shell commands | ❌ | ✅ | ✅ |
| Run tests | ❌ | ✅ | ⚠️ |
| Git operations | ❌ | ✅ | ❌ |
| Web search | ❌ | ✅ | ❌ |

### Persistence

| Capability | **RPG** | **Claude Code** | **Serena** |
|:---|:---:|:---:|:---:|
| Persistent index | ✅ | ❌ | ❌ |
| Cross-session memory | ✅ | ⚠️ | ✅ |
| Shareable with team | ✅ | ❌ | ❌ |
| Incremental updates | ✅ | N/A | N/A |

**Legend:** ✅ = Primary strength | ⚠️ = Partial/indirect support | ❌ = Not supported

---

## When to Use Each Tool

| Scenario | Best Tool | Why |
|:---|:---|:---|
| "What does this codebase do?" | **RPG** | Intent annotations reveal purpose |
| "Find all authentication code" | **RPG** | Semantic search works even if nothing is named "auth" |
| Understand architecture | **RPG** | Auto-discovered functional hierarchy |
| Write a new feature | **Claude Code** | Full coding assistant |
| Fix a bug | **Claude Code** | Debug + edit + test loop |
| Rename a function globally | **Serena** | LSP-powered refactoring |
| Find all callers of X | **RPG** or **Serena** | Both have reference tracking |
| Navigate unfamiliar codebase | **RPG** | Search by what you want, not what you know |
| Daily coding tasks | **Claude Code** | General-purpose assistant |

---

## Technical Comparison

### Architecture

| | **RPG-Encoder** | **Claude Code** | **Serena** |
|:---|:---|:---|:---|
| Written in | Rust | TypeScript | Python |
| Parsing | Tree-sitter | Text + reasoning | LSP servers |
| Semantic source | Connected agent (MCP) | Claude models | LSP type system |
| Indexing | Build once, update incrementally | On-demand (agentic search) | Real-time queries |
| Persistence | `.rpg/graph.json` | `CLAUDE.md` | Markdown memories |

### Language Support

| Language | **RPG** | **Claude Code** | **Serena** |
|:---|:---:|:---:|:---:|
| Python | ✅ | ✅ | ✅ |
| Rust | ✅ | ✅ | ✅ |
| TypeScript/JavaScript | ✅ | ✅ | ✅ |
| Go | ✅ | ✅ | ✅ |
| Java | ✅ | ✅ | ✅ |
| C/C++ | ✅ | ✅ | ✅ |
| Ruby | ❌ | ✅ | ✅ |
| PHP | ❌ | ✅ | ✅ |
| C# | ❌ | ✅ | ✅ |
| Swift | ❌ | ✅ | ✅ |
| **Coverage** | 8 languages | All (text-based) | 40+ (LSP) |

### Setup Requirements

| | **RPG-Encoder** | **Claude Code** | **Serena** |
|:---|:---|:---|:---|
| Install | `npx rpg-encoder` | `npm i -g @anthropic-ai/claude-code` | `uvx serena` |
| Runtime deps | None | Anthropic API | LSP servers |
| Setup time | Seconds (build) + minutes (lift) | None | Per-language LSP setup |
| API keys | Not required | Required | Not required |
| Python version | Any | Any | 3.11 only |

---

## Integration Patterns

### RPG + Claude Code (Recommended)

Use RPG to understand the codebase, Claude Code to make changes.

```
1. Build and lift the RPG graph
2. Search by intent: "find code that validates user input"
3. Explore dependencies: what calls this? what does it call?
4. Hand off to Claude Code: "refactor the validation in src/auth.rs"
```

### RPG + Serena

Use RPG for understanding, Serena for precise symbol operations.

```
1. Search RPG: "find all error handling code"
2. Use Serena: rename_symbol("handleError", "processError")
```

### All Three

For large refactoring projects:

```
1. RPG: Understand architecture and find all relevant code
2. Claude Code: Plan the refactoring approach
3. Serena: Execute precise symbol-level changes
```

---

## What Makes RPG Unique

### 1. Intent-Based Search

Traditional search finds code by **name**. RPG finds code by **intent**.

```
# Traditional (grep/LSP)
"Find functions named validate*"  →  validateInput, validateEmail, ...

# RPG semantic search
"Find code that validates user input"  →  checkCredentials, sanitizeForm, verifyToken, ...
```

### 2. No API Keys Required

RPG uses the connected coding agent (Claude Code, Cursor, etc.) to analyze code directly via standard MCP tools. No external API calls, no token costs, no rate limits.

### 3. Persistent, Shareable Index

The `.rpg/` directory can be committed to your repository. Team members get instant semantic search without re-lifting.

### 4. Architectural Understanding

RPG automatically discovers functional areas in your codebase:

```
GraphManagement/
  ├── verify graph operations
  ├── configure system
  └── persist graph
SemanticEncoding/
  ├── lift semantic features
  ├── construct hierarchy
  └── ground dependencies
```

This hierarchy emerges from the code's semantics, not its file structure.

---

## Limitations

### RPG-Encoder

- **Read-only**: Cannot edit code directly
- **Requires lifting**: Initial semantic analysis takes time (minutes for large repos)
- **Limited languages**: 8 languages vs Serena's 40+
- **Context limits**: Large repos require subagent dispatch for lifting

### Claude Code

- **Requires API**: Cannot work offline
- **No persistent index**: Re-explores codebase each session
- **Rate limits**: Usage quotas based on subscription tier

### Serena

- **Python 3.11 only**: Strict version requirement
- **LSP setup**: Each language needs its own server configured
- **No semantic understanding**: Finds symbols, not intent

---

## Summary

| Tool | Use When You Need To... |
|:---|:---|
| **RPG-Encoder** | Understand what code does, search by intent, discover architecture |
| **Claude Code** | Write, edit, debug, test, commit — full development workflow |
| **Serena** | Precise symbol operations, LSP queries, refactoring across 40+ languages |

They work best together. RPG provides the understanding, Claude Code provides the action, Serena provides the precision.

---

## Links

- [RPG-Encoder](https://github.com/anthropics/rpg-encoder)
- [Claude Code](https://github.com/anthropics/claude-code)
- [Serena](https://github.com/oraios/serena)
