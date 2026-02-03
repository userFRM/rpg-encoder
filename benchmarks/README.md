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
| `rpg-encoder` | Rust | 352 | 20 |

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

# Unlifted baseline (fast, ~10 seconds)
python3 benchmarks/search_quality.py --no-lift

# Full benchmark with lifting (requires an LLM provider)
MOONSHOT_API_KEY=xxx python3 benchmarks/search_quality.py

# Re-run measurement only (fast, uses cached graphs)
python3 benchmarks/search_quality.py --measure-only

# Force re-lift all entities
python3 benchmarks/search_quality.py --force-lift
```

Supported LLM providers for lifting: Moonshot (Kimi), OpenAI, Anthropic, Ollama.

## Results

### With Semantic Lifting (Kimi k2.5, TOON format)

352/352 entities lifted (100% coverage). Zero parse failures with TOON line format.

```
  Metric         Unlifted         Lifted    Delta
  ──────── ────────────── ────────────── ────────
  Acc@1       10/20 (50%)    13/20 (65%)     +15%
  Acc@3       13/20 (65%)    15/20 (75%)     +10%
  Acc@5       15/20 (75%)    17/20 (85%)     +10%
  Acc@10      17/20 (85%)    18/20 (90%)      +5%
  MRR               0.603          0.731   +0.128
```

Lifting improves Acc@1 by **+15%** and MRR by **+0.128**.

Notable per-query improvements with lifting:
- "resolve dependency edges between entities": miss -> @1
- "lift entities with checkpointing": @8 -> @1
- "explore dependency graph traversal": @2 -> @1
- "match glob file path patterns": @10 -> @4
- "load and save RPG graph to disk": @4 -> @2

## Architecture

The benchmark has two phases:

1. **PREPARE** (slow, cached): Copy repo, build graph, lift entities with LLM. Results cached in `/tmp/rpg-bench/rpg-encoder/.rpg/`.
2. **MEASURE** (fast, reproducible): Run search queries against cached graphs, compute Acc@k and MRR.

This separation means you only pay the lifting cost once. Subsequent runs with `--measure-only` complete in seconds.

## Reproducing

```bash
# Clean slate
rm -rf /tmp/rpg-bench

# Full reproducible run
MOONSHOT_API_KEY=xxx python3 benchmarks/search_quality.py 2>&1 | tee benchmarks/run.log

# Results saved to benchmarks/results.json
```
