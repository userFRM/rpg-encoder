# Benchmark Comparison: Ours vs. the Paper

An honest, apples-to-apples analysis of how our `search_quality.py` benchmark compares to the RPG-Encoder paper's evaluation ([arXiv:2602.02084](https://arxiv.org/abs/2602.02084)), what the gaps are, and what we would need to change to make a fair comparison.

---

## Side-by-Side: What Each Benchmark Measures

| Dimension | Paper | Our Benchmark |
|-----------|-------|---------------|
| **Task** | Bug localization: "Given a GitHub issue, find the files/functions to fix" | Intent search: "Given a description, find the file that implements it" |
| **Dataset** | SWE-bench Verified (500 instances, 12 repos) + SWE-bench Live Lite (300 instances, 70 repos) | 39 hand-written queries on 1 repo (rpg-encoder itself) |
| **Repositories** | Large Python projects (Django, scikit-learn, sympy, matplotlib, etc.) | Single mid-size Rust project (60 files, 652 entities) |
| **Languages** | Python only | Rust only |
| **Query source** | Real GitHub issues written by developers | Hand-crafted intent descriptions written by us |
| **Ground truth** | Human-validated file/function patches from merged PRs | Manually assigned expected file path substrings |
| **Evaluation loop** | Multi-step agentic pipeline (Search → Explore → Fetch), up to 40 LLM reasoning steps | Single `rpg-encoder search` call, no agent loop |
| **Backbone LLMs** | o3-mini, GPT-4o, GPT-4.1, GPT-5, DeepSeek-V3.1, Claude-4.5-Sonnet | None (keyword + semantic feature matching, no LLM at search time) |
| **Lifting LLM** | GPT-4o for encoding | Connected coding agent (via MCP) |
| **Runs** | Averaged over 3 runs | Single deterministic run (no randomness in search) |
| **Granularity** | File-level + Function-level | File-level only |
| **Metrics** | Acc@1, Acc@5, Precision, Recall | Acc@1, Acc@3, Acc@5, Acc@10, MRR |

---

## Raw Numbers

### Paper: SWE-bench Verified (file-level, best model per metric)

| Model | Acc@1 | Acc@5 | Precision | Recall |
|-------|-------|-------|-----------|--------|
| o3-mini | 78.3% | 91.2% | 80.7% | 76.8% |
| GPT-4o | 74.5% | 89.6% | 77.0% | 72.7% |
| GPT-4.1 | 82.6% | 93.2% | 83.6% | 79.3% |
| GPT-5 | 91.9% | 97.7% | 91.1% | 89.1% |
| Claude-4.5 | 90.5% | 97.6% | 91.8% | 88.6% |

### Paper: SWE-bench Live Lite (file-level, GPT-4o)

| Metric | RPG-Encoder | Best baseline (CoSIL) | Delta |
|--------|-------------|----------------------|-------|
| Acc@1 | 69.2% | 60.1% | +9.1 |
| Acc@5 | 83.5% | 77.0% | +6.5 |

### Ours: rpg-encoder self-eval (file-level)

| Metric | Unlifted | Lifted | Delta |
|--------|----------|--------|-------|
| Acc@1 | 49% | 54% | +5% |
| Acc@3 | 69% | 79% | +10% |
| Acc@5 | 74% | 87% | +13% |
| Acc@10 | 77% | 92% | +15% |
| MRR | 0.606 | 0.683 | +0.077 |

---

## Why the Numbers Don't Compare Directly

### 1. Single-shot search vs. agentic pipeline

This is the biggest gap. The paper's agent makes **up to 40 LLM calls** per issue — it searches, reads results, explores dependencies, refines its query, and iterates. Our benchmark issues **one search call** and checks the ranked results. There is no retry, no refinement, no reasoning.

The paper itself shows (Table 5) that RPG-guided agents take an average of **6.75 steps** per task. Even at the minimum, that's 6-7x more reasoning than our single call.

**To compare fairly:** We would need to build an agentic harness that issues a GitHub issue description, then lets an LLM use `search_node`, `explore_rpg`, and `fetch_node` iteratively to produce a ranked file list. This is what the paper's RPG-Encoder agent does.

### 2. Different query types

The paper's queries are **real GitHub issues** — messy, contextual, often describing symptoms rather than solutions:
> *"BoundWidget.id_for_label ignores id set by WidgetAttrs"*

Our queries are **clean intent descriptions** that closely mirror the code's purpose:
> *"detect file changes from git diff"*

Our queries are easier because they describe *what the code does* in near-matching vocabulary. Real issues describe *what's broken*, requiring the agent to reason from symptom to implementation.

**To compare fairly:** We would need to use real issues from our own GitHub issue tracker, or adapt SWE-bench-style issue descriptions.

### 3. Self-evaluation bias

We wrote both the code and the queries for the same repository. This creates implicit vocabulary alignment — the query author knows how the code is structured and unconsciously uses similar terms.

The paper evaluates on **12-70 third-party repositories** where the query authors (issue filers) have no insider knowledge of the RPG representation.

**To compare fairly:** We would need to evaluate on external repositories where we didn't write the code or the queries.

### 4. Scale difference

| | Paper | Ours |
|---|---|---|
| Repositories | 12-70 | 1 |
| Queries per eval | 300-500 | 39 |
| Files per repo | 100s-1000s | 60 |
| Languages | Python (ecosystem diversity) | Rust (single project) |

With 39 queries, each miss or hit swings Acc@1 by 2.6 percentage points. The paper's 500-instance dataset gives much more statistical stability.

**To compare fairly:** We would need at minimum 5-10 repositories across different languages and domains, with 100+ queries total.

### 5. No Precision/Recall measurement

The paper reports Precision and Recall alongside Acc@k. These matter when the ground truth has **multiple relevant files** (a bug fix touching 3 files). Our benchmark only checks if *one expected file* appears in the results — we don't measure whether the search returns too many irrelevant results (Precision) or misses other relevant files (Recall).

**To compare fairly:** We would need multi-file ground truth annotations and Precision/Recall computation.

### 6. No function-level evaluation

The paper's strongest results are at function-level (93.7% Acc@5 with Claude-4.5). Our benchmark only evaluates at file-level. Function-level evaluation would test whether RPG can pinpoint the exact function, not just the file — which is where semantic features should excel most.

**To compare fairly:** Ground truth should include expected function/class names, and we should measure function-level Acc@k.

---

## What Does Align

Despite the methodological gaps, some findings are consistent:

### Semantic lifting improvement

| Source | Without features | With features | Delta (Acc@1) |
|--------|-----------------|---------------|---------------|
| Paper ablation (Table 3, GPT-4o) | 60.9% | 69.2% | +8.3 |
| Paper ablation (Table 3, GPT-4.1) | 71.7% | 78.0% | +6.3 |
| Our benchmark | 49% | 54% | +5% |

The **direction and magnitude of semantic lifting improvement** is consistent: +5-8% Acc@1 across all evaluations. The paper's ablation study (`w/o Feature` in Table 3) confirms that removing semantic features causes significant degradation, matching our unlifted→lifted delta.

### Acc@5 convergence

Our lifted Acc@5 of **87%** from a single search call is comparable to the paper's agentic Acc@5 of **83.5%** (GPT-4o, SWE-bench Live). This suggests that for file-level localization, a well-lifted RPG with good semantic features can approach agentic accuracy even without multi-step reasoning — at least on simpler query types.

### The lifting pattern works regardless of lifter

The paper uses GPT-4o for lifting. We use the connected coding agent via MCP. Both produce semantic features that meaningfully improve search quality, validating the agent-agnostic lifting protocol.

---

## Roadmap: Making an Apples-to-Apples Comparison

To produce results that can be directly compared to the paper's Table 1, we would need:

### Phase 1: Expand the benchmark dataset
- [ ] Add 5-10 external repositories (Python: Django, Flask; Rust: ripgrep, tokei; JS/TS: express, next.js)
- [ ] Source queries from real GitHub issues, not hand-crafted descriptions
- [ ] Target 100+ queries per language, 300+ total
- [ ] Annotate ground truth with both file and function targets
- [ ] Include multi-file ground truth where applicable

### Phase 2: Build an agentic evaluation harness
- [ ] Create an agent loop that uses `search_node` → `explore_rpg` → `fetch_node` iteratively
- [ ] Cap at 40 steps (matching the paper's protocol)
- [ ] Use the same backbone LLMs (GPT-4o, Claude-4.5)
- [ ] Collect ranked file/function predictions from the agent's final output
- [ ] Average results over 3 runs

### Phase 3: Implement full metrics
- [ ] Add Precision and Recall computation
- [ ] Add function-level evaluation with canonicalization (matching paper's Appendix B.1.2)
- [ ] Report Acc@1, Acc@5, Precision, Recall at both file and function level
- [ ] Add statistical significance testing (bootstrap CI, already implemented)

### Phase 4: Run baselines
- [ ] Implement Agentless-style text narrowing as a baseline
- [ ] Compare RPG-guided agent vs. grep-based agent on the same queries
- [ ] Measure step count and cost per query (matching paper's Table 5)

---

## Conclusion

Our benchmark validates that the core mechanism works — **semantic lifting improves search accuracy** — but it is not methodologically comparable to the paper's evaluation. The paper uses an agentic pipeline on real bug reports across dozens of repositories. We use single-shot search on self-authored queries for a single repository.

The results are **directionally consistent** (lifting helps by ~5-8% Acc@1, ~13-15% Acc@5), but the absolute numbers cannot be placed side-by-side as equivalent measurements. To make a fair comparison, we need to adopt the paper's evaluation protocol: real issues, multiple repositories, agentic multi-step search, and function-level granularity.

---

*Analysis based on rpg-encoder v0.1.7 benchmark (652 entities, 39 queries) and the RPG-Encoder paper (Luo et al., arXiv:2602.02084, 2026).*
