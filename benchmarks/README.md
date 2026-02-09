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
| `rpg-encoder` | Rust | 652 | 39 |

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

652/652 entities lifted (100% coverage).

```
  Metric         Unlifted         Lifted    Delta
  ──────── ────────────── ────────────── ────────
  Acc@1       19/39 (49%)    21/39 (54%)      +5%
  Acc@3       27/39 (69%)    31/39 (79%)     +10%
  Acc@5       29/39 (74%)    34/39 (87%)     +13%
  Acc@10      30/39 (77%)    36/39 (92%)     +15%
  MRR               0.606          0.683   +0.077

  MRR delta: +0.077 (95% CI [-0.094, +0.260])
```

Lifting improves Acc@5 by **+13%** and Acc@10 by **+15%**.

Notable per-query improvements with lifting:
- "configure batch size and encoding settings": @8 -> @1
- "resolve scope specification to entity IDs": @4 -> @1
- "ground hierarchy nodes to directory paths": @4 -> @1
- "detect file changes from git diff": miss -> @1
- "compute semantic drift between features": miss -> @1
- "repair failed lifting batches": miss -> @1
- "strip LLM think blocks from response": miss -> @1

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
