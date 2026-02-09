# Search Quality Benchmark

Measures whether semantic lifting improves search localization accuracy in RPG-Encoder.

## What It Measures

**Acc@k** — Does the correct file appear in the top-k search results for a natural-language intent query?

**MRR** (Mean Reciprocal Rank) — Average of `1/rank` across all queries. Higher = better.

Two search modes compared:
- **Unlifted** (`--mode snippets`): Keyword/snippet matching only (structural graph)
- **Lifted** (`--mode auto`): Semantic features + keyword matching (merged scores)

## Test Suite

The benchmark uses the rpg-encoder repository itself as the test target:

| Repo | Language | Entities | Queries |
|------|----------|----------|---------|
| `rpg-encoder` | Rust | 855 | 39 |

Each query is a natural-language intent with expected file path substrings:
```json
{
  "query": "extract semantic features from code with LLM",
  "expect": ["semantic_lifting.rs"]
}
```

## Running

```bash
# Prerequisites
cargo build --release          # Build rpg-encoder

# Re-run measurement only (fast, uses cached graphs)
python3 benchmarks/search_quality.py --measure-only

# Full benchmark with lifting (uses connected coding agent or API key)
python3 benchmarks/search_quality.py

# Force re-lift all entities
python3 benchmarks/search_quality.py --force-lift
```

## Results

### With Semantic Lifting (connected coding agent)

855/855 entities lifted (100% coverage).

```
  Metric         Unlifted         Lifted    Delta
  ──────── ────────────── ────────────── ────────
  Acc@1       13/39 (33%)    19/39 (49%)     +15%
  Acc@3       19/39 (49%)    26/39 (67%)     +18%
  Acc@5       19/39 (49%)    27/39 (69%)     +21%
  Acc@10      20/39 (51%)    33/39 (85%)     +33%
  MRR               0.409          0.589   +0.181

  MRR delta: +0.181 (95% CI [+0.012, +0.356])
```

Lifting improves Acc@1 by **+15%**, Acc@5 by **+21%**, and Acc@10 by **+33%**. The MRR improvement is statistically significant (95% CI does not cross zero).

> [!NOTE]
> These results use **lexical-only search** via the CLI binary. The MCP server uses hybrid
> embedding + lexical search (BGE-small-en-v1.5, 0.6 semantic + 0.4 lexical blending),
> which would produce even higher accuracy. The CLI does not yet enable the `embeddings`
> feature — see [Feature gap](#feature-gap-cli-vs-mcp) below.

Notable per-query improvements with lifting (20 total):
- "build token-aware entity batches": @10 -> @1
- "parse Rust functions and structs": @3 -> @1
- "detect file changes from git diff": @2 -> @1
- "incremental update from code modifications": @2 -> @1
- "serialize output in TOON format": miss -> @1
- "configure batch size and encoding settings": miss -> @1
- "parse pipe-delimited line format features": miss -> @1
- "resolve scope specification to entity IDs": miss -> @1
- "propagate dependency features bottom-up": miss -> @1
- "format search results as TOON output": miss -> @1
- "strip LLM think blocks from response": miss -> @1

## Feature Gap: CLI vs MCP

The benchmark uses the CLI binary (`rpg-encoder search`), which only performs **lexical keyword matching**. The MCP server (`rpg-mcp-server`) additionally uses **fastembed** (BGE-small-en-v1.5) for hybrid embedding + lexical search with 0.6/0.4 blending.

This gap exists because:
- `rpg-cli/Cargo.toml` depends on `rpg-nav` **without** the `embeddings` feature
- `rpg-mcp/Cargo.toml` depends on `rpg-nav` **with** `features = ["embeddings"]`
- The CLI's `cmd_search` passes `embedding_scores: None` to `search_with_params`

The benchmark therefore measures a **lower bound** on lifted search quality. MCP users get hybrid search automatically.

## Architecture

The benchmark has two phases:

1. **PREPARE** (slow, cached): Copy repo, build graph, lift entities. Results cached in `/tmp/rpg-bench/rpg-encoder/.rpg/`.
2. **MEASURE** (fast, reproducible): Run search queries against cached graphs, compute Acc@k and MRR.

This separation means you only pay the lifting cost once. Subsequent runs with `--measure-only` complete in seconds.

## Reproducing

```bash
# Clean slate
rm -rf /tmp/rpg-bench

# Full reproducible run
python3 benchmarks/search_quality.py 2>&1 | tee benchmarks/run.log

# Results saved to benchmarks/results.json
```
